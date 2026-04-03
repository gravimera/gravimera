// Gen3D AI orchestration and helpers.

mod agent_component_batch;
mod agent_motion_batch;
mod agent_parsing;
mod agent_regen_budget;
mod agent_render_capture;
mod agent_review_delta;
mod agent_review_images;
mod agent_step;
mod agent_tool_dispatch;
mod agent_tool_poll;
mod agent_utils;
mod ai_service;
mod artifacts;
mod attachment_motion_basis;
mod basis_from_up_forward;
mod bootstrap_requests;
mod claude;
mod component_regen;
mod convert;
mod copy_component;
mod draft_ops;
mod edit_bundle;
mod gemini;
mod headless_prefab;
mod info_store;
mod job;
mod mimo;
mod motion_validation;
mod openai;
mod orchestration;
mod parse;
mod pipeline_orchestrator;
mod plan_ops;
mod plan_tools;
mod preserve_plan_policy;
mod prompts;
mod repair_hints;
mod reuse_groups;
mod schema;
mod snapshots;
mod status_steps;
mod structured_outputs;
mod workspaces;

#[cfg(test)]
mod pipeline_orchestrator_tests;
#[cfg(test)]
mod regression_tests;

use super::{GEN3D_MAX_REQUEST_IMAGES, GEN3D_PREVIEW_DEFAULT_PITCH, GEN3D_PREVIEW_DEFAULT_YAW};

use crate::config::AppConfig;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use job::*;
#[cfg(test)]
use schema::{AiContactJson, AiContactStanceJson, AiJointJson, AiJointKindJson};

use orchestration::{
    build_gen3d_scene_graph_summary, build_gen3d_smoke_results, build_gen3d_validate_results,
    compute_gen3d_plan_hash, fail_job, finish_job_best_effort, max_components_for_speed,
    poll_gen3d_motion_capture, record_gen3d_tooling_feedback, set_progress,
    spawn_gen3d_ai_text_thread, start_gen3d_review_capture, truncate_for_ui,
};

pub(crate) use draft_ops::gen3d_apply_draft_ops_from_api;
pub(crate) use edit_bundle::{
    gen3d_build_edit_bundle_v1, gen3d_hydrate_seeded_job_from_edit_bundle_v1,
    gen3d_load_edit_bundle_v1, gen3d_write_edit_bundle_v1,
};
pub(crate) use headless_prefab::{gen3d_generate_prefab_defs_headless, Gen3dHeadlessPrefabResult};
pub(crate) use job::Gen3dAiJob;
pub(crate) use job::Gen3dDescriptorMetaPolicy;
pub(crate) use job::Gen3dParallelTaskCounts;
pub(crate) use orchestration::{
    gen3d_apply_pending_seed_from_prefab, gen3d_cancel_build_from_api, gen3d_generate_button,
    gen3d_poll_ai_job, gen3d_resume_build_from_api, gen3d_start_build_from_api,
    gen3d_start_edit_run_from_current_draft_from_api,
    gen3d_start_edit_session_from_prefab_id_from_api,
    gen3d_start_fork_session_from_prefab_id_from_api,
};
pub(crate) use snapshots::{
    gen3d_diff_snapshots_from_api, gen3d_list_snapshots_from_api, gen3d_restore_snapshot_from_api,
    gen3d_snapshot_from_api,
};
pub(crate) use workspaces::{
    gen3d_copy_from_workspace_from_api, gen3d_create_workspace_from_api,
    gen3d_delete_workspace_from_api, gen3d_diff_workspaces_from_api,
    gen3d_merge_workspace_from_api, gen3d_set_active_workspace_from_api,
};

#[derive(Clone, Debug)]
pub(crate) struct Gen3dSimpleTextResponse {
    pub(crate) text: String,
    pub(crate) total_tokens: Option<u64>,
}

pub(crate) fn gen3d_generate_text_simple(
    config: &AppConfig,
    system_instructions: &str,
    user_text: &str,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<Gen3dSimpleTextResponse, String> {
    gen3d_generate_text_simple_with_prefix(
        config,
        system_instructions,
        user_text,
        cancel,
        "genfloor",
    )
}

pub(crate) fn gen3d_generate_text_simple_with_prefix(
    config: &AppConfig,
    system_instructions: &str,
    user_text: &str,
    cancel: Option<Arc<AtomicBool>>,
    artifact_prefix: &str,
) -> Result<Gen3dSimpleTextResponse, String> {
    let ai = orchestration::resolve_gen3d_ai_service_config(config)?;
    let progress = Arc::new(Mutex::new(Gen3dAiProgress::default()));
    let session = Gen3dAiSessionState::default();
    let reasoning_effort = ai.model_reasoning_effort().to_string();

    let response = ai_service::generate_text_via_ai_service(
        &progress,
        session,
        cancel,
        std::time::Duration::from_secs(config.ai_request_timeout_secs.max(1)),
        None,
        config.gen3d_require_structured_outputs,
        &ai,
        reasoning_effort.as_str(),
        system_instructions,
        user_text,
        &[],
        None,
        artifact_prefix,
    )?;

    Ok(Gen3dSimpleTextResponse {
        text: response.text,
        total_tokens: response.total_tokens,
    })
}

pub(super) fn spawn_prefab_descriptor_meta_enrichment_thread_best_effort(
    job: &Gen3dAiJob,
    descriptor_path: std::path::PathBuf,
    prefab_label: String,
    roles: Vec<String>,
    size_m: bevy::prelude::Vec3,
    ground_origin_y_m: f32,
    mobility: Option<String>,
    attack_kind: Option<String>,
    anchors: Vec<String>,
    animation_channels: Vec<String>,
    plan_extracted_text: Option<String>,
    motion_summary_json: Option<serde_json::Value>,
) {
    orchestration::spawn_prefab_descriptor_meta_enrichment_thread_best_effort(
        job,
        descriptor_path,
        prefab_label,
        roles,
        size_m,
        ground_origin_y_m,
        mobility,
        attack_kind,
        anchors,
        animation_channels,
        plan_extracted_text,
        motion_summary_json,
    );
}
