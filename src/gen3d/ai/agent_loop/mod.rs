use bevy::log::debug;
use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
#[cfg(test)]
use crate::gen3d::agent::Gen3dToolResultJsonV1;
use crate::gen3d::agent::{append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolRegistryV1};

use super::artifacts::append_gen3d_run_log;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase, Gen3dAiProgress,
    Gen3dAiTextResponse,
};
#[cfg(test)]
use crate::gen3d::agent::tools::TOOL_ID_QA;
use crate::threaded_result::{new_shared_result, SharedResult};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

#[cfg(test)]
use super::agent_parsing::{parse_delta_transform, resolve_component_index_by_name_hint};
use super::agent_prompt::{build_agent_system_instructions, build_agent_user_text, draft_summary};
use super::agent_render_capture::poll_agent_render_capture;
#[cfg(test)]
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_images_from_args,
    review_capture_dimensions_for_max_dim, select_review_preview_images,
};
use super::agent_step::poll_agent_descriptor_meta;
use super::agent_step::{execute_agent_actions, poll_agent_pass_snapshot_capture, poll_agent_step};
use super::agent_tool_poll::poll_agent_tool;
#[cfg(test)]
use super::agent_utils::compute_agent_state_hash;

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;

pub(super) fn poll_gen3d_agent(
    config: &AppConfig,
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    preview: &mut Gen3dPreview,
    preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    match job.phase {
        Gen3dAiPhase::AgentWaitingStep => poll_agent_step(
            config,
            commands,
            review_cameras,
            workshop,
            feedback_history,
            job,
            draft,
        ),
        Gen3dAiPhase::AgentExecutingActions => execute_agent_actions(
            config,
            time,
            commands,
            images,
            workshop,
            feedback_history,
            job,
            draft,
            preview,
            preview_model,
        ),
        Gen3dAiPhase::AgentWaitingTool => poll_agent_tool(
            config,
            commands,
            images,
            workshop,
            feedback_history,
            job,
            draft,
            preview,
        ),
        Gen3dAiPhase::AgentCapturingRender => {
            poll_agent_render_capture(
                config,
                time,
                commands,
                images,
                workshop,
                job,
                draft,
                preview_model,
            );
        }
        Gen3dAiPhase::AgentCapturingPassSnapshot => poll_agent_pass_snapshot_capture(
            config,
            commands,
            images,
            workshop,
            feedback_history,
            job,
        ),
        Gen3dAiPhase::AgentWaitingDescriptorMeta => {
            poll_agent_descriptor_meta(config, commands, images, workshop, job, draft)
        }
        _ => fail_job(
            workshop,
            job,
            "Internal error: agent entered an unexpected phase.",
        ),
    }
}

pub(super) fn spawn_agent_step_request(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    let Some(ai) = job.ai.clone() else {
        return Err("Internal error: missing AI config.".into());
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Starting agent…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    job.metrics.note_agent_step_request_started();

    set_progress(&progress, "Thinking…");

    let registry = Gen3dToolRegistryV1::default();
    let system = build_agent_system_instructions();
    let user_text = build_agent_user_text(
        config,
        job,
        workshop,
        draft_summary(config, job),
        &job.agent.step_tool_results,
        &registry,
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: "Gen3D agent: requesting next step".into(),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "agent_step_request attempt={} pass={} plan_hash={} assembly_rev={} active_ws={} components_generated={}/{}",
            job.attempt,
            job.pass,
            job.plan_hash(),
            job.assembly_rev(),
            job.active_workspace_id(),
            job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
            job.planned_components.len(),
        ),
    );
    debug!(
        "Gen3D agent: requesting step (attempt={}, pass={}, plan_hash={}, assembly_rev={}, active_ws={}, components_generated={}/{})",
        job.attempt,
        job.pass,
        job.plan_hash(),
        job.assembly_rev(),
        job.active_workspace_id(),
        job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
        job.planned_components.len(),
    );

    let reasoning_effort = super::openai::cap_reasoning_effort(
        ai.model_reasoning_effort(),
        &config.gen3d_reasoning_effort_agent_step,
    );
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(super::structured_outputs::Gen3dAiJsonSchemaKind::AgentStepV1),
        config.gen3d_require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        Vec::new(),
        pass_dir,
        "agent_step".into(),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, OpenAiConfig};
    use crate::gen3d::ai::ai_service::Gen3dAiServiceConfig;
    use crate::gen3d::state::{Gen3dDraft, Gen3dPreview, Gen3dSpeedMode, Gen3dWorkshop};
    use crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory;
    use uuid::Uuid;

    #[test]
    fn gen3d_tool_transform_parsing_preserves_negative_and_zero_scale() {
        let value = serde_json::json!({
            "pos": [1.0, 2.0, 3.0],
            "scale": [-1.0, 0.0, 2.0],
        });
        let t = parse_delta_transform(Some(&value));
        assert_eq!(t.translation, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(t.scale, Vec3::new(-1.0, 0.0, 2.0));
    }

    #[test]
    fn select_review_preview_images_prefers_five_static_views() {
        let images = vec![
            PathBuf::from("render_front.png"),
            PathBuf::from("render_left_back.png"),
            PathBuf::from("render_right_back.png"),
            PathBuf::from("render_top.png"),
            PathBuf::from("render_bottom.png"),
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, false, false);
        assert_eq!(
            selected,
            vec![
                PathBuf::from("render_front.png"),
                PathBuf::from("render_left_back.png"),
                PathBuf::from("render_right_back.png"),
                PathBuf::from("render_top.png"),
                PathBuf::from("render_bottom.png"),
            ]
        );
    }

    #[test]
    fn select_review_preview_images_includes_motion_sheets_when_requested() {
        let images = vec![
            PathBuf::from("render_front.png"),
            PathBuf::from("render_left_back.png"),
            PathBuf::from("render_right_back.png"),
            PathBuf::from("render_top.png"),
            PathBuf::from("render_bottom.png"),
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, true, true);
        assert_eq!(
            selected,
            vec![
                PathBuf::from("render_front.png"),
                PathBuf::from("render_left_back.png"),
                PathBuf::from("render_right_back.png"),
                PathBuf::from("render_top.png"),
                PathBuf::from("render_bottom.png"),
                PathBuf::from("move_sheet.png"),
                PathBuf::from("attack_sheet.png"),
            ]
        );
    }

    #[test]
    fn review_capture_dimensions_for_max_dim_matches_expected_scale() {
        assert_eq!(review_capture_dimensions_for_max_dim(960), (960, 540));
        assert_eq!(review_capture_dimensions_for_max_dim(768), (768, 432));
        assert_eq!(review_capture_dimensions_for_max_dim(1920), (1920, 1080));
    }

    #[test]
    fn select_review_preview_images_falls_back_when_only_motion_present() {
        let images = vec![
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, false, false);
        assert_eq!(selected, images);
    }

    #[test]
    fn motion_sheets_needed_from_smoke_results_is_channel_specific() {
        let smoke_results = serde_json::json!({
            "motion_validation": {
                "ok": false,
                "issues": [
                    { "severity": "error", "kind": "chain_axis_mismatch", "channel": "move" },
                    { "severity": "error", "kind": "attack_self_intersection", "channel": "attack_primary" },
                    { "severity": "warn", "kind": "some_warn", "channel": "move" },
                ]
            }
        });
        assert_eq!(
            motion_sheets_needed_from_smoke_results(&smoke_results),
            (true, true)
        );
    }

    #[test]
    fn motion_sheets_needed_from_smoke_results_falls_back_when_issues_missing() {
        let smoke_results = serde_json::json!({
            "motion_validation": {
                "ok": false
            }
        });
        assert_eq!(
            motion_sheets_needed_from_smoke_results(&smoke_results),
            (true, false)
        );
    }

    #[test]
    fn gen3d_no_progress_state_hash_ignores_assembly_rev_churn() {
        let mut job = Gen3dAiJob::default();
        job.plan_hash = "sha256:deadbeef".to_string();

        let draft = Gen3dDraft::default();

        job.assembly_rev = 1;
        let h1 = compute_agent_state_hash(&job, &draft);
        job.assembly_rev = 999;
        let h2 = compute_agent_state_hash(&job, &draft);

        assert_eq!(h1, h2, "state hash should ignore assembly_rev");
    }

    #[test]
    fn gen3d_agent_state_summary_exposes_budget_remaining() {
        let mut config = AppConfig::default();
        config.gen3d_max_seconds = 200;
        config.gen3d_max_tokens = 2000;
        config.gen3d_no_progress_max_steps = 12;
        config.gen3d_max_regen_total = 16;
        config.gen3d_max_regen_per_component = 2;

        let mut job = Gen3dAiJob::default();
        job.running = false;
        job.last_run_elapsed = Some(std::time::Duration::from_secs_f64(100.25));
        job.current_run_tokens = 1234;
        job.regen_total = 15;
        job.regen_per_component = vec![2, 1];
        job.agent.no_progress_steps = 5;
        job.planned_components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "A".into(),
                name: "a".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "B".into(),
                name: "b".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        let summary = draft_summary(&config, &job);
        let budgets = summary
            .get("budgets")
            .and_then(|v| v.as_object())
            .expect("expected budgets object in state summary");

        let regen_remaining = budgets
            .get("regen")
            .and_then(|v| v.get("remaining_total"))
            .and_then(|v| v.as_u64())
            .expect("expected regen remaining_total");
        assert_eq!(regen_remaining, 1);

        let no_progress_remaining = budgets
            .get("no_progress")
            .and_then(|v| v.get("remaining_steps"))
            .and_then(|v| v.as_u64())
            .expect("expected no_progress remaining_steps");
        assert_eq!(no_progress_remaining, 7);

        let token_remaining = budgets
            .get("tokens")
            .and_then(|v| v.get("remaining_run_tokens"))
            .and_then(|v| v.as_u64())
            .expect("expected tokens remaining_run_tokens");
        assert_eq!(token_remaining, 766);

        let elapsed = budgets
            .get("time")
            .and_then(|v| v.get("elapsed_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time elapsed_seconds");
        assert!(
            (elapsed - 100.3).abs() < 1e-6,
            "unexpected elapsed={elapsed}"
        );

        let remaining = budgets
            .get("time")
            .and_then(|v| v.get("remaining_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time remaining_seconds");
        assert!(
            (remaining - 99.8).abs() < 1e-6,
            "unexpected remaining={remaining}"
        );

        let components = summary
            .get("components")
            .and_then(|v| v.as_array())
            .expect("expected components array");
        assert_eq!(
            components[0]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(0)
        );
        assert_eq!(
            components[0]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            components[1]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            components[1]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn gen3d_agent_state_summary_includes_seed_kind_for_seeded_sessions() {
        let config = AppConfig::default();
        let mut job = Gen3dAiJob::default();

        let summary = draft_summary(&config, &job);
        assert!(summary
            .get("seed")
            .expect("expected seed field in state summary")
            .is_null());

        let prefab_uuid = Uuid::from_u128(123);
        let prefab_id_str = prefab_uuid.to_string();
        job.set_edit_base_prefab_id(Some(prefab_uuid.as_u128()));
        job.set_save_overwrite_prefab_id(Some(prefab_uuid.as_u128()));
        let summary = draft_summary(&config, &job);
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str()),
            Some("edit_overwrite")
        );
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("prefab_id"))
                .and_then(|v| v.as_str()),
            Some(prefab_id_str.as_str())
        );

        job.set_save_overwrite_prefab_id(None);
        let summary = draft_summary(&config, &job);
        assert_eq!(
            summary
                .get("seed")
                .and_then(|v| v.get("kind"))
                .and_then(|v| v.as_str()),
            Some("fork")
        );
    }

    #[test]
    fn gen3d_motion_authoring_apply_to_current_ignores_run_id_and_attempt_for_restart_safety() {
        let config = AppConfig::default();

        let mut job = Gen3dAiJob::default();
        job.plan_hash = "sha256:abc123".into();
        job.assembly_rev = 7;
        job.attempt = 0;
        job.run_id = Some(Uuid::from_u128(123));

        job.motion_authoring = Some(super::super::schema::AiMotionAuthoringJsonV1 {
            version: 1,
            applies_to: super::super::schema::AiReviewDeltaAppliesToJsonV1 {
                run_id: "some_prior_run".into(),
                attempt: 999,
                plan_hash: job.plan_hash.clone(),
                assembly_rev: job.assembly_rev,
            },
            decision: super::super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips,
            reason: String::new(),
            replace_channels: Vec::new(),
            edges: Vec::new(),
            notes: None,
        });

        assert!(
            job.motion_authoring_for_current_draft().is_some(),
            "expected persisted motion_authoring to apply when plan_hash+assembly_rev match"
        );

        let summary = draft_summary(&config, &job);
        let applies = summary
            .get("motion_authoring")
            .and_then(|v| v.get("applies_to_current"))
            .and_then(|v| v.as_bool())
            .expect("expected motion_authoring.applies_to_current bool");
        assert!(applies);
    }

    #[test]
    fn gen3d_agent_step_prompt_compacts_large_tool_payloads() {
        let config = AppConfig::default();
        let mut job = Gen3dAiJob::default();
        job.user_prompt_raw = "test prompt".into();

        let workshop = Gen3dWorkshop::default();
        let state_summary = serde_json::json!({"run_id":"test"});
        let registry = crate::gen3d::agent::tools::Gen3dToolRegistryV1::default();

        let huge = "x".repeat(50_000);
        let tool_result = Gen3dToolResultJsonV1::err("call_0".into(), TOOL_ID_QA.into(), huge);

        let prompt = build_agent_user_text(
            &config,
            &job,
            &workshop,
            state_summary,
            &[tool_result],
            &registry,
        );

        // This prompt is written to disk each pass; keep it comfortably small so later passes don't balloon.
        assert!(
            prompt.len() < 24_000,
            "agent_step prompt unexpectedly large: {} bytes",
            prompt.len()
        );
        assert!(
            prompt.contains("…(truncated)"),
            "expected truncation marker in compact prompt summary"
        );
    }

    #[test]
    fn gen3d_component_name_hint_resolves_common_aliases() {
        let components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "Turret Base".into(),
                name: "turret_base".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "Cannon Barrel".into(),
                name: "cannon_barrel".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        assert_eq!(
            resolve_component_index_by_name_hint(&components, "roof_turret_base"),
            Some(0)
        );
        assert_eq!(
            resolve_component_index_by_name_hint(&components, "cannon_weapon"),
            Some(1)
        );
    }

    #[test]
    fn gen3d_review_preview_image_args_ignore_tool_placeholders() {
        let args = serde_json::json!({
            "preview_images": [
                "$CALL_1.images[0]",
                "$CALL_2.render_paths[0]",
            ]
        });
        let paths = parse_review_preview_images_from_args(&args);
        assert!(
            paths.is_empty(),
            "expected placeholder-only paths to be ignored"
        );
    }

    #[test]
    fn gen3d_mock_agent_builds_warcar_prompt_end_to_end() {
        let prompt = "A warcar with a cannon as weapon";

        let run_id = Uuid::new_v4();
        let base_dir = std::env::temp_dir().join(format!("gravimera_gen3d_test_{run_id}"));
        let run_dir = base_dir.join(run_id.to_string());
        let pass_dir = run_dir.join("attempt_0").join("pass_0");
        std::fs::create_dir_all(&pass_dir).expect("create temp gen3d pass dir");

        let openai = OpenAiConfig {
            base_url: "mock://gen3d".into(),
            model: "mock".into(),
            model_reasoning_effort: "none".into(),
            api_key: "mock".into(),
        };

        let mut config = AppConfig {
            openai: Some(openai.clone()),
            ..Default::default()
        };
        // Keep budgets unlimited for the test, but bound execution time in the loop below.
        config.gen3d_max_seconds = 0;
        config.gen3d_max_tokens = 0;

        let mut workshop = Gen3dWorkshop::default();
        workshop.prompt = prompt.to_string();
        workshop.speed_mode = Gen3dSpeedMode::Level3;

        let mut job = Gen3dAiJob::default();
        job.running = true;
        job.build_complete = false;
        job.mode = super::super::Gen3dAiMode::Agent;
        job.phase = super::super::Gen3dAiPhase::AgentWaitingStep;
        job.ai = Some(Gen3dAiServiceConfig::OpenAi(openai));
        job.run_id = Some(run_id);
        job.attempt = 0;
        job.pass = 0;
        job.plan_hash.clear();
        job.assembly_rev = 0;
        job.max_parallel_components = 1;
        job.user_prompt_raw = prompt.to_string();
        job.user_images.clear();
        job.run_dir = Some(run_dir);
        job.pass_dir = Some(pass_dir.clone());
        job.agent = super::super::Gen3dAgentState::default();

        let draft = Gen3dDraft::default();
        let preview = Gen3dPreview::default();
        let feedback_history = Gen3dToolFeedbackHistory::default();

        spawn_agent_step_request(&config, &mut workshop, &mut job, pass_dir)
            .expect("spawn mock agent step");

        let mut app = App::new();
        app.insert_resource(config);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(workshop);
        app.insert_resource(feedback_history);
        app.insert_resource(job);
        app.insert_resource(draft);
        app.insert_resource(preview);

        app.add_systems(
            Update,
            |config: Res<AppConfig>,
             time: Res<Time>,
             mut commands: Commands,
             mut images: ResMut<Assets<Image>>,
             mut workshop: ResMut<Gen3dWorkshop>,
             mut feedback_history: ResMut<Gen3dToolFeedbackHistory>,
             mut job: ResMut<Gen3dAiJob>,
             mut draft: ResMut<Gen3dDraft>,
             mut preview: ResMut<Gen3dPreview>,
             mut preview_model: Query<
                (
                    &mut AnimationChannelsActive,
                    &mut LocomotionClock,
                    &mut AttackClock,
                ),
                With<Gen3dPreviewModelRoot>,
            >,
             review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>| {
                poll_gen3d_agent(
                    &config,
                    &time,
                    &mut commands,
                    &mut images,
                    &review_cameras,
                    &mut workshop,
                    &mut feedback_history,
                    &mut job,
                    &mut draft,
                    &mut preview,
                    &mut preview_model,
                );
            },
        );

        let started = std::time::Instant::now();
        loop {
            app.update();
            let running = app.world().resource::<Gen3dAiJob>().is_running();
            if !running {
                break;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                panic!("Gen3D mock agent test timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let draft = app.world().resource::<Gen3dDraft>();
        assert!(
            draft.total_non_projectile_primitive_parts() > 0,
            "expected generated primitive parts"
        );
        let root = draft.root_def().expect("expected Gen3D root def");
        assert!(root.mobility.is_some(), "expected mobility on root def");
        assert!(root.attack.is_some(), "expected attack on root def");

        let job = app.world().resource::<Gen3dAiJob>();
        let meta = job
            .descriptor_meta_for_current_draft()
            .expect("expected descriptor-meta cached before run completion");
        assert!(
            !meta.short.trim().is_empty(),
            "expected descriptor-meta short to be non-empty"
        );
        assert!(
            !meta.tags.is_empty(),
            "expected descriptor-meta tags to be non-empty"
        );
    }
}
