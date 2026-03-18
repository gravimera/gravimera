use bevy::log::warn;
use bevy::prelude::*;

use crate::config::AppConfig;
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::{fail_job, Gen3dAiJob, Gen3dAiMode, Gen3dAiPhase};

pub(super) fn poll_gen3d_pipeline(
    config: &AppConfig,
    _time: &Time,
    _commands: &mut Commands,
    _images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    _feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    _draft: &mut Gen3dDraft,
    _preview: &mut Gen3dPreview,
    _preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    // The deterministic pipeline is implemented in follow-up steps of the ExecPlan.
    // Until then, keep pipeline mode safe by falling back to the existing agent-step loop.
    // (This preserves UX and ensures the game remains buildable/runnable in pipeline mode.)
    if !matches!(job.mode, Gen3dAiMode::Pipeline) {
        return;
    }

    warn!("Gen3D: pipeline orchestrator not implemented; falling back to agent-step");
    fallback_to_agent_step(config, workshop, job, "pipeline_orchestrator_wip".into());
}

fn fallback_to_agent_step(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    reason: String,
) {
    let Some(pass_dir) = job.pass_dir.clone() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pass dir for pipeline fallback.",
        );
        return;
    };

    job.append_info_event_best_effort(
        super::info_store::InfoEventKindV1::EngineLog,
        None,
        None,
        format!("Pipeline fallback → agent-step (reason: {reason})"),
        serde_json::json!({
            "reason": reason,
        }),
    );

    workshop.status = format!("Pipeline fallback → agent-step (reason: {reason})");
    job.mode = Gen3dAiMode::Agent;

    let needs_user_image_summary =
        !job.user_images.is_empty() && job.user_image_object_summary.is_none();
    job.phase = if needs_user_image_summary {
        Gen3dAiPhase::AgentWaitingUserImageSummary
    } else {
        Gen3dAiPhase::AgentWaitingStep
    };

    let spawn_result = if needs_user_image_summary {
        super::agent_loop::spawn_agent_user_image_summary_request(config, workshop, job, pass_dir)
    } else {
        super::agent_loop::spawn_agent_step_request(config, workshop, job, pass_dir)
    };

    if let Err(err) = spawn_result {
        fail_job(workshop, job, err);
    }
}
