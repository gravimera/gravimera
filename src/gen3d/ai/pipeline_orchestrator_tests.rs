use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use uuid::Uuid;

use crate::config::{AppConfig, OpenAiConfig};
use crate::gen3d::state::{
    Gen3dDraft, Gen3dPreview, Gen3dSpeedMode, Gen3dWorkshop,
};
use crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory;

use super::ai_service::Gen3dAiServiceConfig;
use super::{Gen3dAgentState, Gen3dAiJob, Gen3dAiMode, Gen3dAiPhase, Gen3dPipelineState};

fn make_temp_gen3d_run_dir(prefix: &str, run_id: Uuid) -> PathBuf {
    let base_dir = std::env::temp_dir().join(format!("{prefix}_{run_id}"));
    base_dir.join(run_id.to_string())
}

fn run_app_until_build_stops(mut app: App, timeout: Duration) -> App {
    let started = std::time::Instant::now();
    loop {
        app.update();
        let running = app.world().resource::<Gen3dAiJob>().is_running();
        if !running {
            break;
        }
        if started.elapsed() > timeout {
            panic!("Gen3D pipeline test timed out");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    app
}

fn build_test_app(
    config: AppConfig,
    workshop: Gen3dWorkshop,
    feedback_history: Gen3dToolFeedbackHistory,
    job: Gen3dAiJob,
    draft: Gen3dDraft,
    preview: Gen3dPreview,
) -> App {
    let mut app = App::new();
    app.insert_resource(config);
    app.insert_resource(Time::<()>::default());
    app.insert_resource(Assets::<Image>::default());
    app.insert_resource(workshop);
    app.insert_resource(feedback_history);
    app.insert_resource(job);
    app.insert_resource(draft);
    app.insert_resource(preview);

    app.add_systems(Update, super::gen3d_poll_ai_job);
    app
}

#[test]
fn gen3d_mock_pipeline_builds_warcar_prompt_end_to_end() {
    let prompt = "A warcar with a cannon as weapon";

    let run_id = Uuid::new_v4();
    let run_dir = make_temp_gen3d_run_dir("gravimera_gen3d_pipeline_test", run_id);
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
    config.gen3d_max_seconds = 0;
    config.gen3d_max_tokens = 0;
    config.gen3d_no_progress_tries_max = 0;

    let mut workshop = Gen3dWorkshop::default();
    workshop.prompt = prompt.to_string();
    workshop.speed_mode = Gen3dSpeedMode::Level3;

    let mut job = Gen3dAiJob::default();
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Pipeline;
    job.phase = Gen3dAiPhase::AgentExecutingActions;
    job.ai = Some(Gen3dAiServiceConfig::OpenAi(openai));
    job.run_id = Some(run_id);
    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    job.assembly_rev = 0;
    job.max_parallel_components = 1;
    job.user_prompt_raw = prompt.to_string();
    job.user_images.clear();
    job.run_dir = Some(run_dir.clone());
    job.pass_dir = Some(pass_dir.clone());
    job.agent = Gen3dAgentState::default();
    job.pipeline = Gen3dPipelineState::default();

    let app = build_test_app(
        config,
        workshop,
        Gen3dToolFeedbackHistory::default(),
        job,
        Gen3dDraft::default(),
        Gen3dPreview::default(),
    );

    let app = run_app_until_build_stops(app, Duration::from_secs(5));

    let draft = app.world().resource::<Gen3dDraft>();
    assert!(
        draft.total_non_projectile_primitive_parts() > 0,
        "expected generated primitive parts"
    );

    let job = app.world().resource::<Gen3dAiJob>();
    assert!(
        matches!(job.mode, Gen3dAiMode::Pipeline),
        "expected pipeline to complete without fallback (mode={:?})",
        job.mode
    );

    let trace_path = run_dir.join("agent_trace.jsonl");
    let trace = std::fs::read_to_string(&trace_path).expect("read agent_trace.jsonl");
    assert!(
        !trace.contains("\"artifact_prefix\":\"agent_step\""),
        "pipeline run unexpectedly called agent_step"
    );
}

#[test]
fn gen3d_mock_pipeline_seeded_edit_prefers_draft_ops_and_does_not_regen() {
    let create_prompt = "A warcar with a cannon as weapon";
    let edit_prompt = "Make the cannon longer and darken it.";

    let run_id = Uuid::new_v4();
    let run_dir = make_temp_gen3d_run_dir("gravimera_gen3d_pipeline_edit_test", run_id);
    let pass0 = run_dir.join("attempt_0").join("pass_0");
    std::fs::create_dir_all(&pass0).expect("create temp gen3d pass_0 dir");

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
    config.gen3d_max_seconds = 0;
    config.gen3d_max_tokens = 0;
    config.gen3d_no_progress_tries_max = 0;

    let mut workshop = Gen3dWorkshop::default();
    workshop.prompt = create_prompt.to_string();
    workshop.speed_mode = Gen3dSpeedMode::Level3;

    let mut job = Gen3dAiJob::default();
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Pipeline;
    job.phase = Gen3dAiPhase::AgentExecutingActions;
    job.ai = Some(Gen3dAiServiceConfig::OpenAi(openai));
    job.run_id = Some(run_id);
    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    job.assembly_rev = 0;
    job.max_parallel_components = 1;
    job.user_prompt_raw = create_prompt.to_string();
    job.user_images.clear();
    job.run_dir = Some(run_dir.clone());
    job.pass_dir = Some(pass0.clone());
    job.agent = Gen3dAgentState::default();
    job.pipeline = Gen3dPipelineState::default();

    let mut app = build_test_app(
        config,
        workshop,
        Gen3dToolFeedbackHistory::default(),
        job,
        Gen3dDraft::default(),
        Gen3dPreview::default(),
    );

    app = run_app_until_build_stops(app, Duration::from_secs(5));

    let before_edit_rev = app.world().resource::<Gen3dAiJob>().assembly_rev;
    assert!(before_edit_rev > 0, "expected non-zero assembly_rev after create");

    // Start a seeded edit run using the existing in-memory draft + plan.
    let pass1 = run_dir.join("attempt_0").join("pass_1");
    std::fs::create_dir_all(&pass1).expect("create temp gen3d pass_1 dir");
    {
        let mut workshop = app.world_mut().resource_mut::<Gen3dWorkshop>();
        workshop.prompt = edit_prompt.to_string();

        let mut job = app.world_mut().resource_mut::<Gen3dAiJob>();
        job.running = true;
        job.build_complete = false;
        job.mode = Gen3dAiMode::Pipeline;
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        job.user_prompt_raw = edit_prompt.to_string();
        job.edit_base_prefab_id = Some(Uuid::new_v4().as_u128());
        job.preserve_existing_components_mode = true;
        job.shared_progress = None;
        job.shared_result = None;
        job.pending_finish_run = None;
        job.agent = Gen3dAgentState::default();
        job.pipeline = Gen3dPipelineState::default();
        job.pass = 1;
        job.pass_dir = Some(pass1.clone());
    }

    app = run_app_until_build_stops(app, Duration::from_secs(8));

    let job = app.world().resource::<Gen3dAiJob>();
    assert!(
        matches!(job.mode, Gen3dAiMode::Pipeline),
        "expected edit pipeline to complete without fallback (mode={:?})",
        job.mode
    );
    assert!(
        job.assembly_rev > before_edit_rev,
        "expected DraftOps to increment assembly_rev (before={before_edit_rev} after={})",
        job.assembly_rev
    );

    assert!(
        pass1.join("apply_draft_ops_last.json").exists(),
        "expected apply_draft_ops_last.json artifact in edit pass"
    );

    let tool_calls = std::fs::read_to_string(pass1.join("tool_calls.jsonl"))
        .expect("read tool_calls.jsonl");
    assert!(
        tool_calls.contains("llm_generate_draft_ops_v1"),
        "expected llm_generate_draft_ops_v1 tool call in edit run"
    );
    assert!(
        tool_calls.contains("apply_draft_ops_v1"),
        "expected apply_draft_ops_v1 tool call in edit run"
    );
    assert!(
        !tool_calls.contains("llm_generate_components_v1"),
        "expected edit run to avoid component regeneration by default"
    );
}

#[test]
fn gen3d_mock_pipeline_falls_back_to_agent_step_on_persistent_draft_ops_schema_failure() {
    let create_prompt = "A warcar with a cannon as weapon";
    let edit_prompt = "__MOCK_INVALID_DRAFT_OPS_ALWAYS__ Make the cannon longer and darken it.";

    let run_id = Uuid::new_v4();
    let run_dir = make_temp_gen3d_run_dir("gravimera_gen3d_pipeline_fallback_test", run_id);
    let pass0 = run_dir.join("attempt_0").join("pass_0");
    std::fs::create_dir_all(&pass0).expect("create temp gen3d pass_0 dir");

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
    config.gen3d_max_seconds = 0;
    config.gen3d_max_tokens = 0;
    config.gen3d_no_progress_tries_max = 0;

    let mut workshop = Gen3dWorkshop::default();
    workshop.prompt = create_prompt.to_string();
    workshop.speed_mode = Gen3dSpeedMode::Level3;

    let mut job = Gen3dAiJob::default();
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Pipeline;
    job.phase = Gen3dAiPhase::AgentExecutingActions;
    job.ai = Some(Gen3dAiServiceConfig::OpenAi(openai));
    job.run_id = Some(run_id);
    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    job.assembly_rev = 0;
    job.max_parallel_components = 1;
    job.user_prompt_raw = create_prompt.to_string();
    job.user_images.clear();
    job.run_dir = Some(run_dir.clone());
    job.pass_dir = Some(pass0.clone());
    job.agent = Gen3dAgentState::default();
    job.pipeline = Gen3dPipelineState::default();

    let mut app = build_test_app(
        config,
        workshop,
        Gen3dToolFeedbackHistory::default(),
        job,
        Gen3dDraft::default(),
        Gen3dPreview::default(),
    );

    app = run_app_until_build_stops(app, Duration::from_secs(5));

    // Start seeded edit run that forces DraftOps tool failures until pipeline falls back.
    let pass1 = run_dir.join("attempt_0").join("pass_1");
    std::fs::create_dir_all(&pass1).expect("create temp gen3d pass_1 dir");
    {
        let mut workshop = app.world_mut().resource_mut::<Gen3dWorkshop>();
        workshop.prompt = edit_prompt.to_string();

        let mut job = app.world_mut().resource_mut::<Gen3dAiJob>();
        job.running = true;
        job.build_complete = false;
        job.mode = Gen3dAiMode::Pipeline;
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        job.user_prompt_raw = edit_prompt.to_string();
        job.edit_base_prefab_id = Some(Uuid::new_v4().as_u128());
        job.preserve_existing_components_mode = true;
        job.shared_progress = None;
        job.shared_result = None;
        job.pending_finish_run = None;
        job.agent = Gen3dAgentState::default();
        job.pipeline = Gen3dPipelineState::default();
        job.pass = 1;
        job.pass_dir = Some(pass1.clone());
    }

    app = run_app_until_build_stops(app, Duration::from_secs(8));

    let job = app.world().resource::<Gen3dAiJob>();
    assert!(
        matches!(job.mode, Gen3dAiMode::Agent),
        "expected pipeline fallback to agent-step (mode={:?})",
        job.mode
    );

    let events_path = run_dir.join("info_store_v1").join("events.jsonl");
    let events = std::fs::read_to_string(&events_path).expect("read info_store events.jsonl");
    assert!(
        events.contains("Pipeline fallback"),
        "expected info_store to record pipeline fallback (events_path={})",
        events_path.display()
    );
}
