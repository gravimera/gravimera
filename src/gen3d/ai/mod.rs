// Gen3D AI orchestration and helpers.

mod agent_component_batch;
mod agent_loop;
mod agent_parsing;
mod agent_prompt;
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
mod convert;
mod copy_component;
mod gemini;
mod headless_prefab;
mod job;
mod motion_validation;
mod openai;
mod orchestration;
mod parse;
mod prompts;
mod reuse_groups;
mod schema;
mod structured_outputs;

#[cfg(test)]
mod regression_tests;

use super::{GEN3D_MAX_REQUEST_IMAGES, GEN3D_PREVIEW_DEFAULT_PITCH, GEN3D_PREVIEW_DEFAULT_YAW};

use job::*;
#[cfg(test)]
use schema::{AiContactJson, AiContactStanceJson, AiJointJson, AiJointKindJson};

use orchestration::{
    build_gen3d_scene_graph_summary, build_gen3d_smoke_results, build_gen3d_validate_results,
    compute_gen3d_plan_hash, fail_job, finish_job_best_effort, gen3d_advance_pass,
    max_components_for_speed, poll_gen3d_motion_capture, record_gen3d_tooling_feedback,
    set_progress, spawn_gen3d_ai_text_thread, start_gen3d_review_capture, truncate_for_ui,
};

pub(crate) use headless_prefab::{gen3d_generate_prefab_defs_headless, Gen3dHeadlessPrefabResult};
pub(crate) use job::Gen3dAiJob;
pub(crate) use orchestration::{
    gen3d_cancel_build_from_api, gen3d_generate_button, gen3d_poll_ai_job,
    gen3d_start_build_from_api,
};
pub(crate) use schema::{AiMotionRolesJsonV1, AiMoveEffectorRoleJsonV1};

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
