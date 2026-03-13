use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::Gen3dToolCallJsonV1;
use crate::threaded_result::{new_shared_result, SharedResult};

use super::super::state::Gen3dDraft;
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_blob_ids_from_args,
    select_review_preview_blob_ids, validate_review_images_for_llm,
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
    let Some(ai) = job.ai.clone() else {
        return Err("Missing AI config".into());
    };
    let Some(pass_dir) = job.pass_dir.clone() else {
        return Err("Missing pass dir".into());
    };

    let mut preview_blob_ids = if review_appearance {
        parse_review_preview_blob_ids_from_args(&call.args)
    } else {
        Vec::new()
    };
    let preview_blob_ids_were_explicit = !preview_blob_ids.is_empty();
    if review_appearance && preview_blob_ids.is_empty() {
        preview_blob_ids = job.agent.last_render_blob_ids.clone();
    }
    let include_original_images_requested = call
        .args
        .get("include_original_images")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if include_original_images_requested {
        return Err(
            "`include_original_images=true` is not supported: user reference photos are pre-summarized into text and are not sent to the LLM. Use the prompt + image summary only (and optionally preview renders when review_appearance=true).".into(),
        );
    }

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

        let selected_blob_ids = {
            let store = job.ensure_info_store()?;
            if preview_blob_ids_were_explicit {
                preview_blob_ids.clone()
            } else {
                select_review_preview_blob_ids(
                    store,
                    &preview_blob_ids,
                    include_move_sheet,
                    include_attack_sheet,
                )
            }
        };

        let mut selected_blob_ids = selected_blob_ids;
        if selected_blob_ids.len() > GEN3D_MAX_REQUEST_IMAGES {
            selected_blob_ids.truncate(GEN3D_MAX_REQUEST_IMAGES);
        }

        if !selected_blob_ids.is_empty() {
            let run_dir = job
                .run_dir
                .clone()
                .ok_or_else(|| "Missing run dir (needed to resolve preview_blob_ids).".to_string())?;
            let resolved = {
                let store = job.ensure_info_store()?;
                let mut out = Vec::with_capacity(selected_blob_ids.len());
                for blob_id in &selected_blob_ids {
                    out.push(store.resolve_blob_run_cache_path(blob_id.as_str())?);
                }
                out
            };
            images_to_send = validate_review_images_for_llm(run_dir.as_path(), &resolved)?;
        }
    }

    if let Some(dir) = job.pass_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "scene_graph_summary.json", &scene_graph_summary);
        write_gen3d_json_artifact(Some(dir), "smoke_results.json", &smoke_results);
    }

    let edit_session = job.edit_base_prefab_id.is_some() && !job.user_prompt_raw.trim().is_empty();
    let system = super::prompts::build_gen3d_review_delta_system_instructions(
        review_appearance,
        edit_session,
    );
    let image_object_summary = job
        .user_image_object_summary
        .as_ref()
        .map(|s| s.text.as_str());
    let user_text = super::prompts::build_gen3d_review_delta_user_text(
        &run_id,
        job.attempt,
        &job.plan_hash,
        job.assembly_rev,
        &job.user_prompt_raw,
        image_object_summary,
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
        ai.model_reasoning_effort(),
        &config.gen3d_reasoning_effort_review,
    );
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ReviewDeltaV1),
        config.gen3d_require_structured_outputs,
        ai,
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
