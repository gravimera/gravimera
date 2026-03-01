use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::Gen3dToolCallJsonV1;
use crate::threaded_result::{new_shared_result, SharedResult};

use super::super::state::Gen3dDraft;
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_images_from_args,
    select_review_preview_images,
};
use super::agent_utils::sanitize_prefix;
use super::artifacts::write_gen3d_json_artifact;
use super::{
    set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase, Gen3dAiProgress,
    Gen3dAiTextResponse, GEN3D_MAX_REQUEST_IMAGES,
};

pub(super) fn start_agent_llm_review_delta_call(
    config: &AppConfig,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    call: Gen3dToolCallJsonV1,
) -> Result<(), String> {
    let review_appearance = job.review_appearance;
    let Some(openai) = job.openai.clone() else {
        return Err("Missing OpenAI config".into());
    };
    let Some(pass_dir) = job.pass_dir.clone() else {
        return Err("Missing pass dir".into());
    };

    let mut preview_images = if review_appearance {
        parse_review_preview_images_from_args(&call.args)
    } else {
        Vec::new()
    };
    let preview_images_were_explicit = !preview_images.is_empty();
    if review_appearance && preview_images.is_empty() {
        preview_images = job.agent.last_render_images.clone();
    }
    let include_original_images = review_appearance
        && call
            .args
            .get("include_original_images")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

    let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
    let scene_graph_summary = super::build_gen3d_scene_graph_summary(
        &run_id,
        job.attempt,
        job.pass,
        &job.plan_hash,
        job.assembly_rev,
        &job.planned_components,
        draft,
    );
    let smoke_results = super::build_gen3d_smoke_results(
        &job.user_prompt_raw,
        !job.user_images.is_empty(),
        job.rig_move_cycle_m,
        &job.planned_components,
        draft,
    );

    let mut images_to_send: Vec<PathBuf> = Vec::new();
    if review_appearance {
        let (include_move_sheet, include_attack_sheet) =
            motion_sheets_needed_from_smoke_results(&smoke_results);
        if !preview_images_were_explicit {
            preview_images = select_review_preview_images(
                &preview_images,
                include_move_sheet,
                include_attack_sheet,
            );
        }

        if include_original_images {
            images_to_send.extend(job.user_images.clone());
        }
        images_to_send.extend(preview_images);
        if images_to_send.len() > GEN3D_MAX_REQUEST_IMAGES {
            images_to_send.truncate(GEN3D_MAX_REQUEST_IMAGES);
        }
    }

    if let Some(dir) = job.pass_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "scene_graph_summary.json", &scene_graph_summary);
        write_gen3d_json_artifact(Some(dir), "smoke_results.json", &smoke_results);
    }

    let system = super::prompts::build_gen3d_review_delta_system_instructions(review_appearance);
    let user_text = super::prompts::build_gen3d_review_delta_user_text(
        &run_id,
        job.attempt,
        &job.plan_hash,
        job.assembly_rev,
        &job.user_prompt_raw,
        include_original_images && !job.user_images.is_empty(),
        &scene_graph_summary,
        &smoke_results,
    );

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Reviewing…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    set_progress(&progress, "Calling model for review delta…");
    job.agent.pending_llm_repair_attempt = 0;

    let reasoning_effort = super::openai::cap_reasoning_effort(
        &openai.model_reasoning_effort,
        &config.gen3d_reasoning_effort_review,
    );
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ReviewDeltaV1),
        openai,
        reasoning_effort,
        system,
        user_text,
        images_to_send,
        pass_dir,
        sanitize_prefix(&format!("tool_review_{}", &call.call_id)),
    );
    job.agent.pending_tool_call = Some(call);
    job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::ReviewDelta);
    job.phase = Gen3dAiPhase::AgentWaitingTool;
    Ok(())
}
