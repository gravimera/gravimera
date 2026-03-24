use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::{append_agent_trace_event_v1, AgentTraceEventV1};
use crate::threaded_result::{new_shared_result, SharedResult};

use super::artifacts::append_gen3d_run_log;
use super::status_steps;
use super::{
    set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiProgress, Gen3dAiTextResponse,
};

pub(super) fn spawn_gen3d_user_image_summary_request(
    config: &AppConfig,
    workshop: &mut super::super::state::Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    if job.user_images.is_empty() {
        return Err("Internal error: requested user-image summary but no user images exist.".into());
    }
    if job.user_image_object_summary.is_some() {
        return Err("Internal error: requested user-image summary but summary already exists.".into());
    }

    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Summarizing images…".into(),
    }));
    job.shared_progress = Some(progress.clone());

    set_progress(&progress, "Analyzing reference images…");
    status_steps::log_ai_request_started(
        workshop,
        "Analyze reference images",
        &format!(
            "You provided {} image(s); extracting a short object summary for downstream steps.",
            job.user_images.len()
        ),
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: format!(
                "Gen3D: summarizing {} reference image(s) into text",
                job.user_images.len()
            ),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "user_image_summary_request attempt={} pass={} images={}",
            job.attempt,
            job.pass,
            job.user_images.len()
        ),
    );

    let system = super::prompts::build_gen3d_user_image_object_summary_system_instructions();
    let user_text = super::prompts::build_gen3d_user_image_object_summary_user_text(
        &job.user_prompt_raw,
        job.user_images.len(),
    );
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        None,
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        pass_dir,
        "user_image_summary".into(),
    );

    Ok(())
}

pub(super) fn spawn_gen3d_prompt_intent_request(
    config: &AppConfig,
    workshop: &mut super::super::state::Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    if job.prompt_intent.is_some() {
        return Err("Internal error: requested prompt-intent analysis but it already exists.".into());
    }

    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Analyzing prompt…".into(),
    }));
    job.shared_progress = Some(progress.clone());

    set_progress(&progress, "Determining prompt intent…");
    status_steps::log_ai_request_started(
        workshop,
        "Analyze prompt intent",
        "Determining whether the prompt requires gameplay attack capability.",
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: "Gen3D: analyzing prompt intent".into(),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "prompt_intent_request attempt={} pass={}",
            job.attempt, job.pass
        ),
    );

    let system = super::prompts::build_gen3d_prompt_intent_system_instructions();
    let image_object_summary = job
        .user_image_object_summary
        .as_ref()
        .map(|s| s.text.as_str());
    let user_text = super::prompts::build_gen3d_prompt_intent_user_text(
        &job.user_prompt_raw,
        image_object_summary,
    );
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PromptIntentV1),
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images_component.clone(),
        pass_dir,
        "prompt_intent".into(),
    );

    Ok(())
}
