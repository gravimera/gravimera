// Gen3D AI orchestration and helpers.
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::object::registry::{
    builtin_object_id, ObjectPartDef, ObjectPartKind, PartAnimationDef, PartAnimationDriver,
    PartAnimationSlot,
};
use crate::types::{AnimationChannelsActive, AttackClock, BuildScene, LocomotionClock};

mod agent_loop;
mod artifacts;
mod convert;
mod copy_component;
mod headless_prefab;
mod motion_validation;
mod openai;
mod parse;
mod prompts;
mod reuse_groups;
mod schema;
mod structured_outputs;

#[cfg(test)]
mod regression_tests;

use artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_assembly_snapshot,
    write_gen3d_json_artifact,
};
pub(crate) use headless_prefab::{gen3d_generate_prefab_defs_headless, Gen3dHeadlessPrefabResult};
use prompts::{
    build_gen3d_component_system_instructions, build_gen3d_component_user_text,
    build_gen3d_plan_fill_system_instructions, build_gen3d_plan_fill_user_text,
    build_gen3d_plan_system_instructions, build_gen3d_plan_user_text,
    build_gen3d_review_delta_system_instructions, build_gen3d_review_delta_user_text,
};
use schema::*;
use structured_outputs::Gen3dAiJsonSchemaKind;

use super::state::{
    Gen3dDraft, Gen3dGenerateButton, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera,
    Gen3dSideTab, Gen3dSpeedMode, Gen3dWorkshop,
};
use super::tool_feedback::{
    append_gen3d_tool_feedback_entry, Gen3dToolFeedbackEntry, Gen3dToolFeedbackHistory,
};
use super::{GEN3D_MAX_REQUEST_IMAGES, GEN3D_PREVIEW_DEFAULT_PITCH, GEN3D_PREVIEW_DEFAULT_YAW};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Gen3dAiMode {
    #[default]
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dAgentLlmToolKind {
    GeneratePlan,
    GenerateComponent { component_idx: usize },
    GenerateComponentsBatch,
    ReviewDelta,
}

#[derive(Clone, Debug)]
struct Gen3dPendingComponentBatch {
    requested_indices: Vec<usize>,
    optimized_by_reuse_groups: bool,
    skipped_due_to_reuse_groups: Vec<usize>,
    skipped_due_to_regen_budget: Vec<usize>,
    completed_indices: std::collections::HashSet<usize>,
    failed: Vec<Gen3dComponentBatchFailure>,
}

#[derive(Clone, Debug)]
struct Gen3dComponentBatchFailure {
    index: usize,
    name: String,
    error: String,
}

#[derive(Clone, Debug)]
struct Gen3dAgentWorkspace {
    name: String,
    defs: Vec<crate::object::registry::ObjectDef>,
}

#[derive(Clone, Debug)]
enum Gen3dAgentAfterPassSnapshot {
    AdvancePassAndRequestStep,
    FinishRun {
        workshop_status: String,
        run_log: String,
        info_log: String,
    },
}

#[derive(Clone, Debug)]
struct Gen3dAgentState {
    step_actions: Vec<crate::gen3d::agent::Gen3dAgentActionJsonV1>,
    step_action_idx: usize,
    step_tool_results: Vec<crate::gen3d::agent::Gen3dToolResultJsonV1>,
    step_repair_attempt: u8,
    step_request_retry_attempt: u8,
    no_progress_steps: u32,
    last_state_hash: Option<String>,
    step_had_observable_output: bool,
    tooling_feedback_submissions: u32,
    rendered_since_last_review: bool,
    ever_rendered: bool,
    ever_reviewed: bool,
    ever_validated: bool,
    ever_smoke_checked: bool,
    last_render_images: Vec<PathBuf>,
    last_render_assembly_rev: Option<u32>,
    active_workspace_id: String,
    workspaces: std::collections::HashMap<String, Gen3dAgentWorkspace>,
    next_workspace_seq: u32,
    pending_tool_call: Option<crate::gen3d::agent::Gen3dToolCallJsonV1>,
    pending_llm_tool: Option<Gen3dAgentLlmToolKind>,
    pending_llm_repair_attempt: u8,
    pending_component_batch: Option<Gen3dPendingComponentBatch>,
    pending_render: Option<Gen3dReviewCaptureState>,
    pending_render_include_motion_sheets: bool,
    pending_pass_snapshot: Option<Gen3dReviewCaptureState>,
    pending_after_pass_snapshot: Option<Gen3dAgentAfterPassSnapshot>,
    last_smoke_ok: Option<bool>,
    last_motion_ok: Option<bool>,
    pending_regen_component_indices: Vec<usize>,
    pending_regen_component_indices_skipped_due_to_budget: Vec<usize>,
}

impl Default for Gen3dAgentState {
    fn default() -> Self {
        Self {
            step_actions: Vec::new(),
            step_action_idx: 0,
            step_tool_results: Vec::new(),
            step_repair_attempt: 0,
            step_request_retry_attempt: 0,
            no_progress_steps: 0,
            last_state_hash: None,
            step_had_observable_output: false,
            tooling_feedback_submissions: 0,
            rendered_since_last_review: false,
            ever_rendered: false,
            ever_reviewed: false,
            ever_validated: false,
            ever_smoke_checked: false,
            last_render_images: Vec::new(),
            last_render_assembly_rev: None,
            active_workspace_id: "main".to_string(),
            workspaces: std::collections::HashMap::new(),
            next_workspace_seq: 1,
            pending_tool_call: None,
            pending_llm_tool: None,
            pending_llm_repair_attempt: 0,
            pending_component_batch: None,
            pending_render: None,
            pending_render_include_motion_sheets: true,
            pending_pass_snapshot: None,
            pending_after_pass_snapshot: None,
            last_smoke_ok: None,
            last_motion_ok: None,
            pending_regen_component_indices: Vec::new(),
            pending_regen_component_indices_skipped_due_to_budget: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct Gen3dToolCallInFlight {
    call_id: String,
    tool_id: String,
    started_at: std::time::Instant,
}

#[derive(Clone, Debug)]
struct Gen3dPassMetrics {
    pass: u32,
    started_at: std::time::Instant,
    ended_at: Option<std::time::Instant>,
    agent_step_llm_ms_total: u128,
    agent_step_llm_requests: u32,
    tool_ms_total: u128,
    tool_calls: u32,
    tool_ms_by_id: std::collections::HashMap<String, u128>,
}

impl Gen3dPassMetrics {
    fn elapsed(&self, now: std::time::Instant) -> std::time::Duration {
        let end = self.ended_at.unwrap_or(now);
        end.duration_since(self.started_at)
    }
}

#[derive(Clone, Debug, Default)]
struct Gen3dCopyMetrics {
    auto_component_copies: u32,
    auto_subtree_copies: u32,
    auto_errors: u32,
    manual_component_calls: u32,
    manual_component_copies: u32,
    manual_subtree_calls: u32,
    manual_subtree_copies: u32,
    manual_failures: u32,
    last_error: Option<String>,
    recent_outcomes: std::collections::VecDeque<String>,
}

impl Gen3dCopyMetrics {
    fn push_outcome(&mut self, outcome: String) {
        const MAX_RECENT: usize = 8;
        if outcome.trim().is_empty() {
            return;
        }
        self.recent_outcomes.push_back(outcome);
        while self.recent_outcomes.len() > MAX_RECENT {
            self.recent_outcomes.pop_front();
        }
    }

    fn note_tool_result(&mut self, result: &crate::gen3d::agent::Gen3dToolResultJsonV1) {
        use crate::gen3d::agent::tools::{
            TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_LLM_GENERATE_COMPONENTS,
            TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE,
        };

        match result.tool_id.as_str() {
            TOOL_ID_COPY_COMPONENT | TOOL_ID_MIRROR_COMPONENT => {
                self.manual_component_calls = self.manual_component_calls.saturating_add(1);
                if !result.ok {
                    self.manual_failures = self.manual_failures.saturating_add(1);
                    self.last_error = result.error.clone();
                    return;
                }
                let Some(value) = result.result.as_ref() else {
                    return;
                };
                let Some(copies) = value.get("copies").and_then(|v| v.as_array()) else {
                    return;
                };
                self.manual_component_copies = self
                    .manual_component_copies
                    .saturating_add(copies.len().min(u32::MAX as usize) as u32);
                for item in copies {
                    let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
                    let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    let mode = item.get("mode").and_then(|v| v.as_str()).unwrap_or("");
                    if !source.is_empty() && !target.is_empty() {
                        self.push_outcome(format!("{source} -> {target} ({mode})"));
                    }
                }
            }
            TOOL_ID_COPY_COMPONENT_SUBTREE | TOOL_ID_MIRROR_COMPONENT_SUBTREE => {
                self.manual_subtree_calls = self.manual_subtree_calls.saturating_add(1);
                if !result.ok {
                    self.manual_failures = self.manual_failures.saturating_add(1);
                    self.last_error = result.error.clone();
                    return;
                }
                let Some(value) = result.result.as_ref() else {
                    return;
                };
                let Some(copies) = value.get("copies").and_then(|v| v.as_array()) else {
                    return;
                };
                self.manual_subtree_copies = self
                    .manual_subtree_copies
                    .saturating_add(copies.len().min(u32::MAX as usize) as u32);
                for item in copies {
                    let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
                    let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                    let mode = item.get("mode").and_then(|v| v.as_str()).unwrap_or("");
                    if !source.is_empty() && !target.is_empty() {
                        self.push_outcome(format!("{source} -> {target} ({mode})"));
                    }
                }
            }
            TOOL_ID_LLM_GENERATE_COMPONENTS => {
                let Some(value) = result.result.as_ref() else {
                    return;
                };
                let Some(auto_copy) = value.get("auto_copy").and_then(|v| v.as_object()) else {
                    return;
                };

                let component_copies_applied = auto_copy
                    .get("component_copies_applied")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let subtree_copies_applied = auto_copy
                    .get("subtree_copies_applied")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                self.auto_component_copies = self
                    .auto_component_copies
                    .saturating_add(component_copies_applied.min(u32::MAX as u64) as u32);
                self.auto_subtree_copies = self
                    .auto_subtree_copies
                    .saturating_add(subtree_copies_applied.min(u32::MAX as u64) as u32);

                if let Some(errors) = auto_copy.get("errors").and_then(|v| v.as_array()) {
                    self.auto_errors = self
                        .auto_errors
                        .saturating_add(errors.len().min(u32::MAX as usize) as u32);
                    if let Some(last) = errors.last().and_then(|v| v.as_str()) {
                        if !last.trim().is_empty() {
                            self.last_error = Some(last.trim().to_string());
                        }
                    }
                }

                if let Some(outcomes) = auto_copy.get("outcomes").and_then(|v| v.as_array()) {
                    for item in outcomes {
                        let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("");
                        let target = item.get("target").and_then(|v| v.as_str()).unwrap_or("");
                        let mode = item.get("mode").and_then(|v| v.as_str()).unwrap_or("");
                        if !source.is_empty() && !target.is_empty() {
                            self.push_outcome(format!("auto {source} -> {target} ({mode})"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Gen3dRunMetrics {
    passes: Vec<Gen3dPassMetrics>,
    current_pass_idx: Option<usize>,
    agent_step_request_started_at: Option<std::time::Instant>,
    tool_call_in_flight: Option<Gen3dToolCallInFlight>,
    copy: Gen3dCopyMetrics,
}

impl Gen3dRunMetrics {
    fn current_pass_mut(&mut self) -> Option<&mut Gen3dPassMetrics> {
        self.current_pass_idx
            .and_then(|idx| self.passes.get_mut(idx))
    }

    fn current_pass(&self) -> Option<&Gen3dPassMetrics> {
        self.current_pass_idx.and_then(|idx| self.passes.get(idx))
    }

    fn note_pass_started(&mut self, pass: u32) {
        let now = std::time::Instant::now();
        self.finish_current_pass_at(now);

        self.passes.push(Gen3dPassMetrics {
            pass,
            started_at: now,
            ended_at: None,
            agent_step_llm_ms_total: 0,
            agent_step_llm_requests: 0,
            tool_ms_total: 0,
            tool_calls: 0,
            tool_ms_by_id: std::collections::HashMap::new(),
        });
        self.current_pass_idx = Some(self.passes.len().saturating_sub(1));
        self.agent_step_request_started_at = None;
        self.tool_call_in_flight = None;
    }

    fn finish_current_pass(&mut self) {
        self.finish_current_pass_at(std::time::Instant::now());
    }

    fn finish_current_pass_at(&mut self, now: std::time::Instant) {
        let Some(pass) = self.current_pass_mut() else {
            return;
        };
        if pass.ended_at.is_none() {
            pass.ended_at = Some(now);
        }
    }

    fn note_agent_step_request_started(&mut self) {
        self.agent_step_request_started_at = Some(std::time::Instant::now());
    }

    fn note_agent_step_response_received(&mut self) {
        let Some(start) = self.agent_step_request_started_at.take() else {
            return;
        };
        let now = std::time::Instant::now();
        let ms = now.duration_since(start).as_millis() as u128;
        let Some(pass) = self.current_pass_mut() else {
            return;
        };
        pass.agent_step_llm_ms_total = pass.agent_step_llm_ms_total.saturating_add(ms);
        pass.agent_step_llm_requests = pass.agent_step_llm_requests.saturating_add(1);
    }

    fn note_tool_call_started(&mut self, call_id: &str, tool_id: &str) {
        self.tool_call_in_flight = Some(Gen3dToolCallInFlight {
            call_id: call_id.to_string(),
            tool_id: tool_id.to_string(),
            started_at: std::time::Instant::now(),
        });
    }

    fn note_tool_result(&mut self, result: &crate::gen3d::agent::Gen3dToolResultJsonV1) {
        let now = std::time::Instant::now();
        if let Some(in_flight) = self.tool_call_in_flight.take() {
            if in_flight.call_id == result.call_id {
                let ms = now.duration_since(in_flight.started_at).as_millis() as u128;
                if let Some(pass) = self.current_pass_mut() {
                    pass.tool_ms_total = pass.tool_ms_total.saturating_add(ms);
                    pass.tool_calls = pass.tool_calls.saturating_add(1);
                    let entry = pass.tool_ms_by_id.entry(in_flight.tool_id).or_insert(0);
                    *entry = entry.saturating_add(ms);
                }
            }
        }

        self.copy.note_tool_result(result);
    }

    fn agent_step_llm_ms_with_in_flight(&self, now: std::time::Instant) -> u128 {
        let Some(pass) = self.current_pass() else {
            return 0;
        };
        let mut ms = pass.agent_step_llm_ms_total;
        if let Some(start) = self.agent_step_request_started_at {
            ms = ms.saturating_add(now.duration_since(start).as_millis() as u128);
        }
        ms
    }

    fn tool_ms_with_in_flight(&self, now: std::time::Instant) -> u128 {
        let Some(pass) = self.current_pass() else {
            return 0;
        };
        let mut ms = pass.tool_ms_total;
        if let Some(in_flight) = self.tool_call_in_flight.as_ref() {
            ms = ms.saturating_add(now.duration_since(in_flight.started_at).as_millis() as u128);
        }
        ms
    }
}

#[derive(Resource, Default)]
pub(crate) struct Gen3dAiJob {
    running: bool,
    build_complete: bool,
    mode: Gen3dAiMode,
    phase: Gen3dAiPhase,
    openai: Option<crate::config::OpenAiConfig>,
    run_id: Option<Uuid>,
    attempt: u32,
    pass: u32,
    plan_hash: String,
    assembly_rev: u32,
    plan_attempt: u8,
    max_parallel_components: usize,
    review_kind: Gen3dAutoReviewKind,
    review_appearance: bool,
    review_component_idx: Option<usize>,
    auto_refine_passes_remaining: u32,
    auto_refine_passes_done: u32,
    per_component_refine_passes_remaining: u32,
    per_component_refine_passes_done: u32,
    per_component_resume: Option<Gen3dComponentBatchResume>,
    replan_attempts: u32,
    regen_total: u32,
    regen_per_component: Vec<u32>,
    user_prompt_raw: String,
    user_images: Vec<PathBuf>,
    run_dir: Option<PathBuf>,
    pass_dir: Option<PathBuf>,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    session: Gen3dAiSessionState,
    planned_components: Vec<Gen3dPlannedComponent>,
    assembly_notes: String,
    plan_collider: Option<AiColliderJson>,
    rig_move_cycle_m: Option<f32>,
    reuse_groups: Vec<reuse_groups::Gen3dValidatedReuseGroup>,
    reuse_group_warnings: Vec<String>,
    pending_plan: Option<AiPlanJsonV1>,
    component_queue: Vec<usize>,
    component_queue_pos: usize,
    component_attempts: Vec<u8>,
    component_in_flight: Vec<Gen3dInFlightComponent>,
    generation_kind: Gen3dComponentGenerationKind,
    review_capture: Option<Gen3dReviewCaptureState>,
    review_static_paths: Vec<PathBuf>,
    motion_capture: Option<Gen3dMotionCaptureState>,
    capture_previews_only: bool,
    last_review_inputs: Vec<PathBuf>,
    last_review_user_text: String,
    review_delta_repair_attempt: u8,
    shared_result: Option<Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>>>,
    shared_progress: Option<Arc<Mutex<Gen3dAiProgress>>>,
    run_started_at: Option<std::time::Instant>,
    last_run_elapsed: Option<std::time::Duration>,
    current_run_tokens: u64,
    total_tokens: u64,
    chat_fallbacks_this_run: u32,
    agent: Gen3dAgentState,
    save_seq: u32,
    metrics: Gen3dRunMetrics,
}

#[derive(Clone, Debug)]
struct Gen3dInFlightComponent {
    idx: usize,
    attempt: u8,
    sent_images: bool,
    shared_result: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>>,
    _progress: Arc<Mutex<Gen3dAiProgress>>,
}

impl Gen3dAiJob {
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn is_build_complete(&self) -> bool {
        self.build_complete
    }

    pub(crate) fn is_capturing_motion_sheets(&self) -> bool {
        self.motion_capture.is_some()
    }

    pub(crate) fn run_dir_path(&self) -> Option<&Path> {
        self.run_dir.as_deref()
    }

    pub(crate) fn pass_dir_path(&self) -> Option<&Path> {
        self.pass_dir.as_deref()
    }

    pub(crate) fn run_id(&self) -> Option<Uuid> {
        self.run_id
    }

    pub(crate) fn user_prompt_raw(&self) -> &str {
        self.user_prompt_raw.as_str()
    }

    pub(crate) fn attempt(&self) -> u32 {
        self.attempt
    }

    pub(crate) fn pass(&self) -> u32 {
        self.pass
    }

    pub(crate) fn plan_hash(&self) -> &str {
        self.plan_hash.as_str()
    }

    pub(crate) fn assembly_rev(&self) -> u32 {
        self.assembly_rev
    }

    pub(crate) fn active_workspace_id(&self) -> &str {
        self.agent.active_workspace_id.as_str()
    }

    pub(crate) fn current_save_seq(&self) -> u32 {
        self.save_seq
    }

    pub(crate) fn bump_save_seq(&mut self) -> u32 {
        self.save_seq = self.save_seq.saturating_add(1);
        self.save_seq
    }

    pub(crate) fn run_elapsed(&self) -> Option<std::time::Duration> {
        if self.running {
            self.run_started_at.map(|start| start.elapsed())
        } else {
            self.last_run_elapsed
        }
    }

    pub(crate) fn current_run_tokens(&self) -> u64 {
        self.current_run_tokens
    }

    pub(crate) fn total_tokens(&self) -> u64 {
        self.total_tokens
    }

    fn artifact_dir(&self) -> Option<&Path> {
        self.pass_dir.as_deref()
    }

    pub(crate) fn reset_session(&mut self) {
        // Each Build should start a fresh AI session, but API capability detection is provider-specific
        // (not session-specific). Keep it so we don't repeatedly "probe" unsupported features and
        // cause avoidable 0-token failed requests.
        let responses_supported = self.session.responses_supported;
        let responses_continuation_supported = self.session.responses_continuation_supported;
        let responses_background_supported = self.session.responses_background_supported;
        let responses_structured_outputs_supported =
            self.session.responses_structured_outputs_supported;
        let chat_structured_outputs_supported = self.session.chat_structured_outputs_supported;
        self.session = Gen3dAiSessionState::default();
        self.session.responses_supported = responses_supported;
        self.session.responses_continuation_supported = responses_continuation_supported;
        self.session.responses_background_supported = responses_background_supported;
        self.session.responses_structured_outputs_supported =
            responses_structured_outputs_supported;
        self.session.chat_structured_outputs_supported = chat_structured_outputs_supported;
    }

    fn start_run_metrics(&mut self) {
        self.current_run_tokens = 0;
        self.chat_fallbacks_this_run = 0;
        self.run_started_at = Some(std::time::Instant::now());
        self.last_run_elapsed = None;
    }

    fn finish_run_metrics(&mut self) {
        if let Some(start) = self.run_started_at.take() {
            self.last_run_elapsed = Some(start.elapsed());
        }

        self.metrics.finish_current_pass();
        self.stop_gen3d_log_capture();
    }

    fn add_tokens(&mut self, tokens: u64) {
        self.current_run_tokens = self.current_run_tokens.saturating_add(tokens);
        self.total_tokens = self.total_tokens.saturating_add(tokens);
    }

    fn stop_gen3d_log_capture(&mut self) {
        if let Some(sinks) = self.log_sinks.as_ref() {
            sinks.stop_gen3d_log();
        }
        self.log_sinks = None;
    }

    fn note_api_used(&mut self, api: Gen3dAiApi) {
        if matches!(api, Gen3dAiApi::ChatCompletions) {
            self.chat_fallbacks_this_run = self.chat_fallbacks_this_run.saturating_add(1);
        }
    }

    pub(crate) fn chat_fallbacks_this_run(&self) -> u32 {
        self.chat_fallbacks_this_run
    }

    pub(crate) fn progress_message(&self) -> Option<String> {
        let shared = self.shared_progress.as_ref()?;
        let guard = shared.lock().ok()?;
        Some(guard.message.clone())
    }

    pub(crate) fn status_metrics_text(&self) -> Option<String> {
        if self.metrics.passes.is_empty() {
            return None;
        }

        fn summarize_tool_for_step(tool_id: &str) -> String {
            use crate::gen3d::agent::tools::{
                TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_DETACH_COMPONENT,
                TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_STATE_SUMMARY,
                TOOL_ID_LLM_GENERATE_COMPONENT, TOOL_ID_LLM_GENERATE_COMPONENTS,
                TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_MIRROR_COMPONENT,
                TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_RENDER_PREVIEW, TOOL_ID_SMOKE_CHECK,
                TOOL_ID_SUBMIT_TOOLING_FEEDBACK, TOOL_ID_VALIDATE,
            };

            match tool_id {
                TOOL_ID_LLM_GENERATE_PLAN => "Plan".into(),
                TOOL_ID_LLM_GENERATE_COMPONENT | TOOL_ID_LLM_GENERATE_COMPONENTS => {
                    "Generate".into()
                }
                TOOL_ID_LLM_REVIEW_DELTA => "Review".into(),
                TOOL_ID_RENDER_PREVIEW => "Render".into(),
                TOOL_ID_VALIDATE => "Validate".into(),
                TOOL_ID_SMOKE_CHECK => "Smoke".into(),
                TOOL_ID_COPY_COMPONENT
                | TOOL_ID_MIRROR_COMPONENT
                | TOOL_ID_COPY_COMPONENT_SUBTREE
                | TOOL_ID_MIRROR_COMPONENT_SUBTREE
                | TOOL_ID_DETACH_COMPONENT => "Copy".into(),
                TOOL_ID_GET_STATE_SUMMARY | TOOL_ID_GET_SCENE_GRAPH_SUMMARY => "Inspect".into(),
                TOOL_ID_SUBMIT_TOOLING_FEEDBACK => "Feedback".into(),
                other => short_tool_id(other).to_string(),
            }
        }

        fn duration_from_ms(ms: u128) -> std::time::Duration {
            std::time::Duration::from_millis(ms.min(u64::MAX as u128) as u64)
        }

        fn format_duration(d: std::time::Duration) -> String {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{:.1}s", d.as_secs_f32())
            } else if secs < 60 * 60 {
                format!("{}m {}s", secs / 60, secs % 60)
            } else {
                let hours = secs / 3600;
                let mins = (secs % 3600) / 60;
                format!("{hours}h {mins}m")
            }
        }

        fn short_tool_id(tool_id: &str) -> &str {
            tool_id.strip_suffix("_v1").unwrap_or(tool_id)
        }

        let now = std::time::Instant::now();
        let mut out = String::new();
        out.push_str("\n\nMetrics:");

        // Step times (per pass). Display the most recent N to keep the panel readable.
        out.push_str("\nStep time: ");
        const MAX_PASSES: usize = 10;
        let passes = &self.metrics.passes;
        let start_idx = passes.len().saturating_sub(MAX_PASSES);
        if start_idx > 0 {
            out.push_str("… | ");
        }
        for (i, pass) in passes.iter().enumerate().skip(start_idx) {
            let is_current = self.metrics.current_pass_idx == Some(i) && pass.ended_at.is_none();
            let mut main_tool_id: Option<&str> = None;
            if is_current {
                if let Some(in_flight) = self.metrics.tool_call_in_flight.as_ref() {
                    main_tool_id = Some(in_flight.tool_id.as_str());
                }
            }
            if main_tool_id.is_none() && !pass.tool_ms_by_id.is_empty() {
                main_tool_id = pass
                    .tool_ms_by_id
                    .iter()
                    .max_by(|a, b| a.1.cmp(b.1))
                    .map(|(k, _)| k.as_str());
            }

            let label = main_tool_id
                .map(summarize_tool_for_step)
                .unwrap_or_else(|| "Think".into());

            out.push_str(&format!(
                "p{} {} {}",
                pass.pass,
                label,
                format_duration(pass.elapsed(now))
            ));
            if pass.ended_at.is_none() && self.running {
                out.push('*');
            }
            if i + 1 < passes.len() {
                out.push_str(" | ");
            }
        }

        // Current step breakdown.
        if let Some(pass) = self.metrics.current_pass() {
            let agent_ms = self.metrics.agent_step_llm_ms_with_in_flight(now);
            let tool_ms = self.metrics.tool_ms_with_in_flight(now);
            let agent_reqs = pass.agent_step_llm_requests
                + if self.metrics.agent_step_request_started_at.is_some() {
                    1
                } else {
                    0
                };
            let tool_calls = pass.tool_calls
                + if self.metrics.tool_call_in_flight.is_some() {
                    1
                } else {
                    0
                };
            let total = pass.elapsed(now);
            let main = self
                .metrics
                .tool_call_in_flight
                .as_ref()
                .map(|t| summarize_tool_for_step(t.tool_id.as_str()))
                .or_else(|| {
                    pass.tool_ms_by_id
                        .iter()
                        .max_by(|a, b| a.1.cmp(b.1))
                        .map(|(k, _)| summarize_tool_for_step(k.as_str()))
                })
                .unwrap_or_else(|| "Think".into());
            out.push_str(&format!(
                "\nThis step ({main}): agent {} ({agent_reqs} req) | tools {} ({tool_calls} call{}) | total {}",
                format_duration(duration_from_ms(agent_ms)),
                format_duration(duration_from_ms(tool_ms)),
                if tool_calls == 1 { "" } else { "s" },
                format_duration(total),
            ));

            if !pass.tool_ms_by_id.is_empty() {
                let mut tools: Vec<(&str, u128)> = pass
                    .tool_ms_by_id
                    .iter()
                    .map(|(k, v)| (k.as_str(), *v))
                    .collect();
                tools.sort_by(|a, b| b.1.cmp(&a.1));
                out.push_str("\nTop tools: ");
                for (idx, (tool_id, ms)) in tools.into_iter().take(3).enumerate() {
                    if idx > 0 {
                        out.push_str(" | ");
                    }
                    out.push_str(&format!(
                        "{} {}",
                        short_tool_id(tool_id),
                        format_duration(duration_from_ms(ms))
                    ));
                }
            }
        }

        // Copy metrics (this run): summarize without copy chains.
        let copy = &self.metrics.copy;
        let copyable_components =
            reuse_groups::copyable_target_count(&self.planned_components, &self.reuse_groups);
        let total_copied_components = copy
            .auto_component_copies
            .saturating_add(copy.manual_component_copies)
            .saturating_add(copy.manual_subtree_copies);
        if copyable_components > 0 || total_copied_components > 0 {
            out.push_str(&format!(
                "\nCopy comps: copyable {copyable_components} | total {total_copied_components}"
            ));
        }

        Some(out)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dAiApi {
    Responses,
    ChatCompletions,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dComponentGenerationKind {
    Initial,
    Regenerate,
}

impl Default for Gen3dComponentGenerationKind {
    fn default() -> Self {
        Self::Initial
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dAutoReviewKind {
    EndOfRun,
    PerComponent,
}

impl Default for Gen3dAutoReviewKind {
    fn default() -> Self {
        Self::EndOfRun
    }
}

#[derive(Clone, Debug)]
struct Gen3dComponentBatchResume {
    generation_kind: Gen3dComponentGenerationKind,
    component_queue: Vec<usize>,
    component_queue_pos: usize,
}

#[derive(Clone, Debug)]
struct Gen3dAiTextResponse {
    text: String,
    api: Gen3dAiApi,
    session: Gen3dAiSessionState,
    total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default)]
struct Gen3dAiSessionState {
    responses_supported: Option<bool>,
    responses_continuation_supported: Option<bool>,
    responses_background_supported: Option<bool>,
    responses_previous_id: Option<String>,
    responses_structured_outputs_supported: Option<bool>,
    chat_structured_outputs_supported: Option<bool>,
    chat_history: Vec<Gen3dChatHistoryMessage>,
}

#[derive(Clone, Debug)]
struct Gen3dChatHistoryMessage {
    role: String,
    content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dAiPhase {
    Idle,
    // Codex-style tool-driven agent loop.
    AgentWaitingStep,
    AgentExecutingActions,
    AgentWaitingTool,
    AgentCapturingRender,
    AgentCapturingPassSnapshot,
    WaitingPlan,
    WaitingPlanFill,
    WaitingComponent,
    CapturingReview,
    WaitingReview,
}

impl Default for Gen3dAiPhase {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Clone, Debug, Default)]
struct Gen3dAiProgress {
    message: String,
}

#[derive(Clone, Debug)]
struct Gen3dPlannedComponent {
    display_name: String,
    name: String,
    purpose: String,
    modeling_notes: String,
    /// Current resolved transform of this component in the assembled root frame.
    pos: Vec3,
    /// Current resolved transform of this component in the assembled root frame.
    rot: Quat,
    planned_size: Vec3,
    actual_size: Option<Vec3>,
    anchors: Vec<crate::object::registry::AnchorDef>,
    contacts: Vec<AiContactJson>,
    attach_to: Option<Gen3dPlannedAttachment>,
}

#[derive(Clone, Debug)]
struct Gen3dPlannedAttachment {
    parent: String,
    parent_anchor: String,
    child_anchor: String,
    offset: Transform,
    joint: Option<AiJointJson>,
    animations: Vec<PartAnimationSlot>,
}

#[derive(Clone, Copy, Debug)]
enum Gen3dReviewView {
    Front,
    FrontLeft,
    LeftBack,
    Back,
    RightBack,
    FrontRight,
    Top,
    Bottom,
}

impl Gen3dReviewView {
    fn file_stem(self) -> &'static str {
        match self {
            Self::Front => "front",
            Self::FrontLeft => "front_left",
            Self::LeftBack => "left_back",
            Self::Back => "back",
            Self::RightBack => "right_back",
            Self::FrontRight => "front_right",
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    fn orbit_angles(self, base_pitch: f32) -> (f32, f32) {
        match self {
            Self::Front => (0.0, base_pitch),
            Self::FrontLeft => (-std::f32::consts::FRAC_PI_3, base_pitch),
            Self::LeftBack => (-std::f32::consts::FRAC_PI_3 * 2.0, base_pitch),
            Self::Back => (std::f32::consts::PI, base_pitch),
            Self::RightBack => (std::f32::consts::FRAC_PI_3 * 2.0, base_pitch),
            Self::FrontRight => (std::f32::consts::FRAC_PI_3, base_pitch),
            Self::Top => (0.0, -1.52),
            Self::Bottom => (0.0, 1.52),
        }
    }
}

#[derive(Clone, Debug, Default)]
struct Gen3dReviewCaptureProgress {
    expected: usize,
    completed: usize,
}

#[derive(Clone, Debug)]
struct Gen3dReviewCaptureState {
    cameras: Vec<Entity>,
    image_paths: Vec<PathBuf>,
    progress: Arc<Mutex<Gen3dReviewCaptureProgress>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dMotionCaptureKind {
    Move,
    Attack,
}

impl Gen3dMotionCaptureKind {
    fn label(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Attack => "attack",
        }
    }

    fn sheet_filename(self) -> &'static str {
        match self {
            Self::Move => "move_sheet.png",
            Self::Attack => "attack_sheet.png",
        }
    }
}

#[derive(Clone, Debug)]
struct Gen3dMotionCaptureState {
    kind: Gen3dMotionCaptureKind,
    frame_idx: u8,
    frame_capture: Option<Gen3dReviewCaptureState>,
    frame_paths: Vec<PathBuf>,
}

impl Gen3dMotionCaptureState {
    fn new() -> Self {
        Self {
            kind: Gen3dMotionCaptureKind::Move,
            frame_idx: 0,
            frame_capture: None,
            frame_paths: Vec::new(),
        }
    }
}

pub(crate) fn gen3d_generate_button(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dGenerateButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let log_sinks = log_sinks.map(|sinks| sinks.into_inner().clone());

    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.18, 0.13, 0.92));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.12, 0.20, 0.15, 0.98));
                if job.running {
                    gen3d_cancel_build_from_api(&mut workshop, &mut job);
                    continue;
                }
                match gen3d_start_build_from_api(
                    build_scene.as_ref(),
                    &config,
                    log_sinks.clone(),
                    &mut workshop,
                    &mut job,
                    &mut draft,
                ) {
                    Ok(()) => {}
                    Err(err) => {
                        workshop.error = Some(err);
                    }
                }
            }
        }
    }
}

pub(crate) fn gen3d_cancel_build_from_api(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob) {
    if !job.running {
        return;
    }

    // Cancel the current build. The in-flight background thread can't be forcefully
    // stopped, but we ignore its eventual result and stop updating UI state.
    job.finish_run_metrics();
    job.running = false;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.capture_previews_only = false;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.agent = Gen3dAgentState::default();
    job.save_seq = 0;

    workshop.error = None;
    workshop.status = "Build cancelled. Click Build to start a new run.".to_string();
}

pub(crate) fn gen3d_start_build_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) -> Result<(), String> {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return Err("Gen3D build requires Build Preview scene.".into());
    }
    if job.running {
        return Err("Gen3D build is already running (stop it first).".into());
    }
    if workshop.images.is_empty() && workshop.prompt.trim().is_empty() {
        return Err("Provide at least 1 image or a text prompt.".into());
    }

    let Some(openai) = config.openai.clone() else {
        let details = if config.errors.is_empty() {
            "Missing config.toml. See gen_3d.md for setup.".to_string()
        } else {
            config.errors.join("\n")
        };
        return Err(details);
    };

    job.log_sinks = log_sinks;
    job.metrics = Gen3dRunMetrics::default();

    let image_paths: Vec<PathBuf> = workshop.images.iter().map(|i| i.path.clone()).collect();
    let (run_id, run_dir) = gen3d_make_run_dir(config);
    std::fs::create_dir_all(&run_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D cache dir {}: {err}",
            run_dir.display()
        )
    })?;

    write_gen3d_json_artifact(
        Some(&run_dir),
        "run.json",
        &serde_json::json!({
            "version": 1,
            "run_id": run_id.to_string(),
            "created_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            "openai": {
                "model": openai.model,
                "reasoning_effort": openai.model_reasoning_effort,
                "base_url": openai.base_url,
            },
        }),
    );

    gen3d_set_current_attempt_pass(job, &run_dir, 0, 0)?;
    let attempt_dir = gen3d_attempt_dir(&run_dir, 0);
    let Some(pass_dir) = job.pass_dir.clone() else {
        return Err("Internal error: missing Gen3D pass dir.".into());
    };

    let cached_inputs = cache_gen3d_inputs(&attempt_dir, &workshop.prompt, &image_paths);
    let cached_image_paths = cached_inputs.cached_image_paths;
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "run_start speed={} max_parallel={} model={} reasoning_effort={} base_url={} review_appearance={} images={} prompt_chars={}",
            workshop.speed_mode.short_label(),
            config.gen3d_max_parallel_components.max(1),
            openai.model,
            openai.model_reasoning_effort,
            openai.base_url,
            config.gen3d_review_appearance,
            cached_image_paths.len(),
            workshop.prompt.chars().count()
        ),
    );

    workshop.error = None;
    workshop.status = format!(
        "Planning components…\nModel: {}\nImages: {}",
        openai.model,
        cached_image_paths.len()
    );

    // Each Build is a fresh run (new cache dir + fresh AI session).
    job.reset_session();
    job.start_run_metrics();
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::AgentWaitingStep;
    job.capture_previews_only = false;
    job.plan_attempt = 0;
    job.max_parallel_components = config.gen3d_max_parallel_components.max(1);
    job.openai = Some(openai.clone());
    job.run_id = Some(run_id);
    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    job.assembly_rev = 0;
    job.user_prompt_raw = workshop.prompt.clone();
    job.user_images = cached_image_paths.clone();
    job.run_dir = Some(run_dir.clone());
    job.pass_dir = Some(pass_dir.clone());
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_appearance = config.gen3d_review_appearance;
    job.review_component_idx = None;
    job.auto_refine_passes_done = 0;
    job.auto_refine_passes_remaining = refine_passes_for_speed(config, workshop.speed_mode);
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.replan_attempts = 0;
    job.regen_total = 0;
    job.regen_per_component.clear();
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.agent = Gen3dAgentState::default();
    job.save_seq = 0;
    draft.defs.clear();

    workshop.status = format!(
        "Building…\nModel: {}\nImages: {}",
        job.openai.as_ref().map(|c| c.model.as_str()).unwrap_or(""),
        job.user_images.len()
    );

    if let Err(err) = agent_loop::spawn_agent_step_request(config, workshop, job, pass_dir.clone())
    {
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = false;
        job.phase = Gen3dAiPhase::Idle;
        return Err(err);
    }

    Ok(())
}

fn refine_passes_for_speed(config: &AppConfig, _speed: Gen3dSpeedMode) -> u32 {
    config.refine_iterations
}

fn component_refine_cycles_for_speed(_config: &AppConfig, _speed: Gen3dSpeedMode) -> u32 {
    0
}

fn max_components_for_speed(speed: Gen3dSpeedMode) -> usize {
    let _ = speed;
    24
}

pub(crate) fn gen3d_poll_ai_job(
    config: Res<AppConfig>,
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
    review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>,
) {
    if !job.running {
        return;
    }
    if matches!(job.phase, Gen3dAiPhase::Idle) {
        return;
    }

    // Hard budgets: stop the run when exceeded (best-effort draft stays in the preview).
    if config.gen3d_max_seconds > 0 {
        if let Some(elapsed) = job.run_elapsed() {
            if elapsed >= std::time::Duration::from_secs(config.gen3d_max_seconds) {
                let secs = elapsed.as_secs_f32();
                let mins = secs / 60.0;
                let max_mins = config.gen3d_max_seconds as f32 / 60.0;
                finish_job_best_effort(
                    &mut commands,
                    &review_cameras,
                    &mut workshop,
                    &mut job,
                    format!(
                        "Time budget exhausted ({secs:.1}s / {mins:.1}min >= {}s / {max_mins:.1}min).",
                        config.gen3d_max_seconds,
                    ),
                );
                return;
            }
        }
    }
    let max_tokens = config.gen3d_max_tokens;
    if max_tokens > 0 && job.current_run_tokens >= max_tokens {
        let current_tokens = job.current_run_tokens;
        finish_job_best_effort(
            &mut commands,
            &review_cameras,
            &mut workshop,
            &mut job,
            format!("Token budget exhausted ({current_tokens} >= {max_tokens})."),
        );
        return;
    }

    if matches!(job.mode, Gen3dAiMode::Agent) {
        agent_loop::poll_gen3d_agent(
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
        return;
    }

    let speed_mode = workshop.speed_mode;

    // Apply speed changes to the current run when possible.
    let desired_total_passes = refine_passes_for_speed(&config, workshop.speed_mode);
    let current_total_passes = job.auto_refine_passes_done + job.auto_refine_passes_remaining;
    if desired_total_passes == 0
        && matches!(
            job.phase,
            Gen3dAiPhase::CapturingReview | Gen3dAiPhase::WaitingReview
        )
        && matches!(job.review_kind, Gen3dAutoReviewKind::EndOfRun)
        && !job.capture_previews_only
    {
        debug!("Gen3D: auto-refine disabled mid-run; skipping review phase.");
        for entity in &review_cameras {
            commands.entity(entity).try_despawn();
        }
        job.review_capture = None;
        job.shared_result = None;
        job.shared_progress = None;
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = true;
        job.phase = Gen3dAiPhase::Idle;
        workshop.status =
            "Build finished. (Auto-review skipped due to speed mode change.) Orbit/zoom the preview. Click Build to start a new run."
                .into();
        return;
    }
    if desired_total_passes != current_total_passes {
        job.auto_refine_passes_remaining =
            desired_total_passes.saturating_sub(job.auto_refine_passes_done);
    }

    let per_component_enabled = component_refine_cycles_for_speed(&config, workshop.speed_mode) > 0;
    if !per_component_enabled
        && matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent)
        && matches!(
            job.phase,
            Gen3dAiPhase::CapturingReview | Gen3dAiPhase::WaitingReview
        )
    {
        debug!("Gen3D: per-component review disabled mid-run; resuming build.");
        for entity in &review_cameras {
            commands.entity(entity).try_despawn();
        }
        job.review_capture = None;
        job.shared_result = None;
        workshop.status = "Auto-review skipped due to speed mode change (continuing build).".into();
        resume_after_per_component_review(&mut workshop, &mut job);
        return;
    }

    // Parallel component generation: we don't use `shared_result` during this phase.
    // (Keep the legacy single-request path working if `shared_result` is still set.)
    if matches!(job.phase, Gen3dAiPhase::WaitingComponent) && job.shared_result.is_none() {
        poll_gen3d_parallel_components(&mut workshop, &mut job, &mut draft, speed_mode);
        return;
    }

    // Phase without an AI thread: capture review views as PNGs.
    if matches!(job.phase, Gen3dAiPhase::CapturingReview) {
        // 1) Poll static capture (7 views).
        if let Some(state) = &job.review_capture {
            let (done, expected) = state
                .progress
                .lock()
                .map(|g| (g.completed, g.expected))
                .unwrap_or((0, 7));
            if done < expected {
                if let Some(progress) = job.shared_progress.as_ref() {
                    if job.capture_previews_only {
                        set_progress(
                            progress,
                            format!("Capturing preview renders… ({done}/{expected})"),
                        );
                    } else {
                        set_progress(
                            progress,
                            format!("Capturing review views… ({done}/{expected})"),
                        );
                    }
                }
                return;
            }

            // Clean up capture cameras.
            for cam in state.cameras.iter().copied() {
                commands.entity(cam).try_despawn();
            }
            let review_paths = state.image_paths.clone();
            job.review_capture = None;

            for path in &review_paths {
                if std::fs::metadata(path).is_err() {
                    let label = if job.capture_previews_only {
                        "preview"
                    } else {
                        "review"
                    };
                    workshop.error = Some(format!(
                        "Failed to capture {label} image: {}",
                        path.display()
                    ));
                    if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                        debug!("Gen3D: per-component review image missing; continuing build.");
                        workshop.status = "Auto-review failed (continuing build).".into();
                        resume_after_per_component_review(&mut workshop, &mut job);
                        return;
                    }

                    job.finish_run_metrics();
                    job.running = false;
                    job.build_complete = true;
                    job.phase = Gen3dAiPhase::Idle;
                    job.shared_progress = None;
                    workshop.status =
                        "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                            .into();
                    return;
                }
            }

            if job.capture_previews_only {
                append_gen3d_run_log(job.run_dir.as_deref(), "capture_previews_done");
                job.capture_previews_only = false;
                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished. (Preview renders saved.) Orbit/zoom the preview. Click Build to start a new run."
                        .into();
                return;
            }

            // After static views, capture motion sprite sheets (move + attack), then start the review request.
            job.review_static_paths = review_paths;
            job.motion_capture = Some(Gen3dMotionCaptureState::new());
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Capturing move animation…");
            }
            return;
        }

        // 2) Poll motion capture (2× 2x2 sprite sheets), which appends to `review_static_paths`.
        if job.motion_capture.is_some() {
            poll_gen3d_motion_capture(
                &time,
                &mut commands,
                &mut images,
                &mut workshop,
                &mut job,
                &draft,
                &mut preview_model,
            );
            return;
        }

        // 3) If we have review images ready, start the AI review request.
        if !job.review_static_paths.is_empty() {
            let review_paths = job.review_static_paths.clone();
            job.review_static_paths.clear();

            let Some(openai) = job.openai.clone() else {
                workshop.error = Some("Internal error: missing OpenAI config.".into());
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
                return;
            };
            let Some(run_dir) = job.pass_dir.clone() else {
                workshop.error = Some("Internal error: missing Gen3D pass dir.".into());
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
                return;
            };

            let mut review_inputs = job.user_images.clone();
            review_inputs.extend(review_paths.clone());
            if review_inputs.len() > GEN3D_MAX_REQUEST_IMAGES {
                debug!(
                    "Gen3D: review inputs exceed max images ({} > {}), truncating extra reference photos",
                    review_inputs.len(),
                    GEN3D_MAX_REQUEST_IMAGES
                );
                review_inputs.truncate(GEN3D_MAX_REQUEST_IMAGES);
            }
            if !job.review_appearance {
                review_inputs.clear();
            }

            job.phase = Gen3dAiPhase::WaitingReview;
            job.last_review_inputs = review_inputs.clone();
            job.review_delta_repair_attempt = 0;
            let (status, prefix) = match job.review_kind {
                Gen3dAutoReviewKind::EndOfRun => (
                    format!(
                        "Auto-reviewing assembly… (pass {})",
                        job.auto_refine_passes_done.max(1)
                    ),
                    format!("review{:02}", job.auto_refine_passes_done.max(1)),
                ),
                Gen3dAutoReviewKind::PerComponent => {
                    let pass = job.per_component_refine_passes_done.max(1);
                    let total = pass + job.per_component_refine_passes_remaining;
                    let component = job
                        .review_component_idx
                        .and_then(|idx| job.planned_components.get(idx))
                        .map(|c| c.display_name.clone())
                        .unwrap_or_else(|| "unknown".into());
                    (
                        format!(
                            "Auto-reviewing assembly… (after {})\n(pass {pass}/{total})",
                            component
                        ),
                        format!(
                            "percomp_component{:02}_review{:02}",
                            job.review_component_idx.map(|idx| idx + 1).unwrap_or(0),
                            pass
                        ),
                    )
                }
            };
            workshop.status = status;
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Requesting AI auto-review…");
            }

            let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                Arc::new(Mutex::new(None));
            job.shared_result = Some(shared.clone());
            let progress = job
                .shared_progress
                .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                .clone();

            let run_id = job
                .run_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unknown".into());
            let plan_hash = compute_gen3d_plan_hash(
                &job.assembly_notes,
                job.rig_move_cycle_m,
                &job.planned_components,
            );
            job.plan_hash = plan_hash.clone();

            let scene_graph_summary = build_gen3d_scene_graph_summary(
                &run_id,
                job.attempt,
                job.pass,
                &plan_hash,
                job.assembly_rev,
                &job.planned_components,
                &draft,
            );
            let smoke_results = build_gen3d_smoke_results(
                &job.user_prompt_raw,
                !job.user_images.is_empty(),
                job.rig_move_cycle_m,
                &job.planned_components,
                &draft,
            );

            write_gen3d_json_artifact(
                job.artifact_dir(),
                "scene_graph_summary.json",
                &scene_graph_summary,
            );
            write_gen3d_json_artifact(job.artifact_dir(), "smoke_results.json", &smoke_results);

            let system = build_gen3d_review_delta_system_instructions(job.review_appearance);
            let user_text = build_gen3d_review_delta_user_text(
                &run_id,
                job.attempt,
                &plan_hash,
                job.assembly_rev,
                &job.user_prompt_raw,
                job.review_appearance && !job.user_images.is_empty(),
                &scene_graph_summary,
                &smoke_results,
            );
            job.last_review_user_text = user_text.clone();
            let reasoning_effort = openai.model_reasoning_effort.clone();
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.session.clone(),
                Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1),
                openai,
                reasoning_effort,
                system,
                user_text,
                review_inputs,
                run_dir,
                prefix,
            );
            return;
        }

        // 4) Start a new static capture.
        let Some(run_dir) = job.pass_dir.clone() else {
            workshop.error = Some("Internal error: missing Gen3D cache dir.".into());
            if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                workshop.status = "Auto-review failed (continuing build).".into();
                resume_after_per_component_review(&mut workshop, &mut job);
                return;
            }

            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            job.shared_progress = None;
            workshop.status =
                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                    .into();
            return;
        };
        let prefix = if job.capture_previews_only {
            "preview"
        } else {
            "review"
        };
        let include_overlay = !job.capture_previews_only;
        let views = [
            Gen3dReviewView::Front,
            Gen3dReviewView::FrontLeft,
            Gen3dReviewView::LeftBack,
            Gen3dReviewView::Back,
            Gen3dReviewView::RightBack,
            Gen3dReviewView::FrontRight,
            Gen3dReviewView::Top,
        ];
        match start_gen3d_review_capture(
            &mut commands,
            &mut images,
            &run_dir,
            &draft,
            include_overlay,
            prefix,
            &views,
            super::GEN3D_REVIEW_CAPTURE_WIDTH_PX,
            super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
        ) {
            Ok(state) => {
                job.review_capture = Some(state);
                if let Some(progress) = job.shared_progress.as_ref() {
                    if job.capture_previews_only {
                        set_progress(progress, "Capturing preview renders… (0/7)");
                    } else {
                        set_progress(progress, "Capturing review views… (0/7)");
                    }
                }
            }
            Err(err) => {
                workshop.error = Some(err);
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    debug!("Gen3D: per-component review capture failed; continuing build.");
                    job.review_capture = None;
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                job.review_capture = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
            }
        }
        return;
    }

    // Phases that wait for an AI response.
    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            &mut workshop,
            &mut job,
            "Internal error: missing AI job handle.",
        );
        return;
    };

    let result = {
        let Ok(mut guard) = shared.lock() else {
            return;
        };
        guard.take()
    };
    let Some(result) = result else {
        return;
    };

    job.shared_result = None;

    match result {
        Ok(resp) => {
            debug!("Gen3D: OpenAI response via {:?}", resp.api);
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                debug!("Gen3D: OpenAI usage total_tokens={tokens}");
                job.add_tokens(tokens);
            }
            let max_tokens = config.gen3d_max_tokens;
            if max_tokens > 0 && job.current_run_tokens >= max_tokens {
                let current_tokens = job.current_run_tokens;
                finish_job_best_effort(
                    &mut commands,
                    &review_cameras,
                    &mut workshop,
                    &mut job,
                    format!("Token budget exhausted ({current_tokens} >= {max_tokens})."),
                );
                return;
            }
            let text = resp.text;

            match job.phase {
                Gen3dAiPhase::AgentWaitingStep
                | Gen3dAiPhase::AgentExecutingActions
                | Gen3dAiPhase::AgentWaitingTool
                | Gen3dAiPhase::AgentCapturingRender
                | Gen3dAiPhase::AgentCapturingPassSnapshot => {
                    // Agent mode is polled via `agent_loop::poll_gen3d_agent`. If we end up here,
                    // just ignore this legacy response path.
                    debug!("Gen3D: ignoring legacy AI result while in agent phase.");
                }
                Gen3dAiPhase::WaitingPlan => {
                    debug!("Gen3D: plan request finished.");
                    let mut plan = match parse::parse_ai_plan_from_text(&text) {
                        Ok(plan) => plan,
                        Err(err) => {
                            debug!("Gen3D: failed to parse AI plan: {err}");
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    let max_components = max_components_for_speed(workshop.speed_mode);
                    if plan.components.len() > max_components {
                        debug!(
                            "Gen3D: truncating plan components from {} to {} due to speed mode",
                            plan.components.len(),
                            max_components
                        );
                        plan.components.truncate(max_components);
                    }

                    let needs_fill = plan.mobility.is_none()
                        || plan
                            .components
                            .iter()
                            .filter_map(|c| c.attach_to.as_ref())
                            .any(|att| att.animations.is_none() && att.animation.is_some());

                    if needs_fill {
                        debug!("Gen3D: plan missing mobility/animations; requesting plan-fill.");
                        job.pending_plan = Some(plan);
                        job.phase = Gen3dAiPhase::WaitingPlanFill;

                        workshop.status = format!(
                            "Planning mobility/animations…\nModel: {}\nImages: {}",
                            job.openai.as_ref().map(|c| c.model.as_str()).unwrap_or(""),
                            job.user_images.len()
                        );

                        let Some(openai) = job.openai.clone() else {
                            fail_job(
                                &mut workshop,
                                &mut job,
                                "Internal error: missing OpenAI config.",
                            );
                            return;
                        };
                        let Some(run_dir) = job.pass_dir.clone() else {
                            fail_job(
                                &mut workshop,
                                &mut job,
                                "Internal error: missing Gen3D pass dir.",
                            );
                            return;
                        };

                        let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                            Arc::new(Mutex::new(None));
                        job.shared_result = Some(shared.clone());
                        let progress = job
                            .shared_progress
                            .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                            .clone();
                        set_progress(&progress, "Requesting mobility/animations…");

                        let system = build_gen3d_plan_fill_system_instructions();
                        let user_text = build_gen3d_plan_fill_user_text(
                            &job.user_prompt_raw,
                            !job.user_images.is_empty(),
                            job.pending_plan
                                .as_ref()
                                .expect("pending_plan is set above"),
                        );
                        let reasoning_effort = openai.model_reasoning_effort.clone();
                        spawn_gen3d_ai_text_thread(
                            shared,
                            progress,
                            job.session.clone(),
                            Some(Gen3dAiJsonSchemaKind::PlanFillV1),
                            openai,
                            reasoning_effort,
                            system,
                            user_text,
                            job.user_images.clone(),
                            run_dir,
                            "plan_fill".into(),
                        );
                        return;
                    }

                    job.plan_collider = plan.collider.clone();
                    job.rig_move_cycle_m = plan
                        .rig
                        .as_ref()
                        .and_then(|r| r.move_cycle_m)
                        .filter(|v| v.is_finite())
                        .map(|v| v.abs())
                        .filter(|v| *v > 1e-3);
                    let plan_reuse_groups = plan.reuse_groups.clone();
                    match convert::ai_plan_to_initial_draft_defs(plan) {
                        Ok((planned, assembly_notes, defs)) => {
                            job.planned_components = planned;
                            job.assembly_notes = assembly_notes;
                            let (validated, warnings) = reuse_groups::validate_reuse_groups(
                                &plan_reuse_groups,
                                &job.planned_components,
                            );
                            job.reuse_groups = validated;
                            job.reuse_group_warnings = warnings;
                            job.component_queue = (0..job.planned_components.len()).collect();
                            job.component_queue_pos = 0;
                            job.generation_kind = Gen3dComponentGenerationKind::Initial;
                            job.regen_per_component = vec![0; job.planned_components.len()];
                            draft.defs = defs;
                            workshop.error = None;

                            if let Some(run_dir) = job.artifact_dir() {
                                let components: Vec<serde_json::Value> = job
                                    .planned_components
                                    .iter()
                                    .map(|c| {
                                        let forward = c.rot * Vec3::Z;
                                        let up = c.rot * Vec3::Y;
                                        serde_json::json!({
                                            "name": c.name.as_str(),
                                            "purpose": c.purpose.as_str(),
                                            "modeling_notes": c.modeling_notes.as_str(),
                                            "pos": [c.pos.x, c.pos.y, c.pos.z],
                                            "forward": [forward.x, forward.y, forward.z],
                                            "up": [up.x, up.y, up.z],
                                            "size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                                        })
                                    })
                                    .collect();
                                let extracted = serde_json::json!({
                                    "version": 2,
                                    "assembly_notes": job.assembly_notes.as_str(),
                                    "components": components,
                                });
                                write_gen3d_json_artifact(
                                    Some(run_dir),
                                    "plan_extracted.json",
                                    &extracted,
                                );
                            }
                            write_gen3d_assembly_snapshot(
                                job.artifact_dir(),
                                &job.planned_components,
                            );

                            if let Some(def) = draft.root_def() {
                                let max_dim = def.size.x.max(def.size.y).max(def.size.z).max(0.5);
                                preview.distance = (max_dim * 2.8 + 0.8).clamp(2.0, 250.0);
                                preview.pitch = GEN3D_PREVIEW_DEFAULT_PITCH;
                                preview.yaw = GEN3D_PREVIEW_DEFAULT_YAW;
                                preview.last_cursor = None;
                            }

                            if job.component_queue.is_empty() {
                                let err = "AI plan did not include any components.".to_string();
                                if retry_gen3d_plan(
                                    &mut workshop,
                                    &mut job,
                                    &mut draft,
                                    speed_mode,
                                    &err,
                                ) {
                                    return;
                                }
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }

                            job.phase = Gen3dAiPhase::WaitingComponent;
                            job.component_attempts = vec![0; job.planned_components.len()];
                            job.component_in_flight.clear();
                            job.shared_progress = None;

                            workshop.status = format!(
                                "Building components… (0/{})\nParallel: {}",
                                job.planned_components.len(),
                                job.max_parallel_components.max(1),
                            );
                        }
                        Err(err) => {
                            debug!("Gen3D: failed to build draft from plan: {err}");
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                        }
                    }
                }
                Gen3dAiPhase::WaitingPlanFill => {
                    debug!("Gen3D: plan-fill request finished.");
                    let fill = match parse::parse_ai_plan_fill_from_text(&text) {
                        Ok(fill) => fill,
                        Err(err) => {
                            debug!("Gen3D: failed to parse AI plan-fill: {err}");
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    let Some(mut plan) = job.pending_plan.take() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing pending plan for plan-fill merge.",
                        );
                        return;
                    };

                    if let Some(mobility) = fill.mobility.clone() {
                        plan.mobility = Some(mobility);
                    }

                    for component in fill.components.iter() {
                        let name = component.name.trim();
                        if name.is_empty() {
                            continue;
                        }
                        let Some(target) = plan.components.iter_mut().find(|c| c.name == name)
                        else {
                            continue;
                        };
                        let Some(att) = target.attach_to.as_mut() else {
                            continue;
                        };
                        let has_any_animation = component.animations.values().any(|v| v.is_some());
                        if !has_any_animation {
                            continue;
                        }
                        att.animations = Some(component.animations.clone());
                        att.animation = None;
                    }

                    job.plan_collider = plan.collider.clone();
                    job.rig_move_cycle_m = plan
                        .rig
                        .as_ref()
                        .and_then(|r| r.move_cycle_m)
                        .filter(|v| v.is_finite())
                        .map(|v| v.abs())
                        .filter(|v| *v > 1e-3);
                    let plan_reuse_groups = plan.reuse_groups.clone();
                    match convert::ai_plan_to_initial_draft_defs(plan) {
                        Ok((planned, assembly_notes, defs)) => {
                            job.planned_components = planned;
                            job.assembly_notes = assembly_notes;
                            let (validated, warnings) = reuse_groups::validate_reuse_groups(
                                &plan_reuse_groups,
                                &job.planned_components,
                            );
                            job.reuse_groups = validated;
                            job.reuse_group_warnings = warnings;
                            job.component_queue = (0..job.planned_components.len()).collect();
                            job.component_queue_pos = 0;
                            job.generation_kind = Gen3dComponentGenerationKind::Initial;
                            job.regen_per_component = vec![0; job.planned_components.len()];
                            draft.defs = defs;
                            workshop.error = None;

                            if let Some(run_dir) = job.artifact_dir() {
                                let components: Vec<serde_json::Value> = job
                                    .planned_components
                                    .iter()
                                    .map(|c| {
                                        let forward = c.rot * Vec3::Z;
                                        let up = c.rot * Vec3::Y;
                                        serde_json::json!({
                                            "name": c.name.as_str(),
                                            "purpose": c.purpose.as_str(),
                                            "modeling_notes": c.modeling_notes.as_str(),
                                            "pos": [c.pos.x, c.pos.y, c.pos.z],
                                            "forward": [forward.x, forward.y, forward.z],
                                            "up": [up.x, up.y, up.z],
                                            "size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                                        })
                                    })
                                    .collect();
                                let extracted = serde_json::json!({
                                    "version": 2,
                                    "assembly_notes": job.assembly_notes.as_str(),
                                    "components": components,
                                });
                                write_gen3d_json_artifact(
                                    Some(run_dir),
                                    "plan_extracted.json",
                                    &extracted,
                                );
                            }
                            write_gen3d_assembly_snapshot(
                                job.artifact_dir(),
                                &job.planned_components,
                            );

                            if let Some(def) = draft.root_def() {
                                let max_dim = def.size.x.max(def.size.y).max(def.size.z).max(0.5);
                                preview.distance = (max_dim * 2.8 + 0.8).clamp(2.0, 250.0);
                                preview.pitch = GEN3D_PREVIEW_DEFAULT_PITCH;
                                preview.yaw = GEN3D_PREVIEW_DEFAULT_YAW;
                                preview.last_cursor = None;
                            }

                            if job.component_queue.is_empty() {
                                let err = "AI plan did not include any components.".to_string();
                                if retry_gen3d_plan(
                                    &mut workshop,
                                    &mut job,
                                    &mut draft,
                                    speed_mode,
                                    &err,
                                ) {
                                    return;
                                }
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }

                            job.phase = Gen3dAiPhase::WaitingComponent;
                            job.component_attempts = vec![0; job.planned_components.len()];
                            job.component_in_flight.clear();
                            job.shared_progress = None;

                            workshop.status = format!(
                                "Building components… (0/{})\nParallel: {}",
                                job.planned_components.len(),
                                job.max_parallel_components.max(1),
                            );
                        }
                        Err(err) => {
                            debug!(
                                "Gen3D: failed to build draft from plan-fill merged plan: {err}"
                            );
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                        }
                    }
                }
                Gen3dAiPhase::WaitingComponent => {
                    if job.component_queue_pos >= job.component_queue.len() {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: component queue out of range.",
                        );
                        return;
                    }
                    let idx = job.component_queue[job.component_queue_pos];
                    if idx >= job.planned_components.len() {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: component index out of range.",
                        );
                        return;
                    }

                    let component_name = job.planned_components[idx].name.clone();
                    debug!(
                        "Gen3D: component generation finished ({}/{}, name={})",
                        job.component_queue_pos + 1,
                        job.component_queue.len(),
                        component_name
                    );

                    let ai = match parse::parse_ai_draft_from_text(&text) {
                        Ok(ai) => ai,
                        Err(err) => {
                            debug!("Gen3D: failed to parse component draft: {err}");
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    let component_def =
                        match convert::ai_to_component_def(
                            &job.planned_components[idx],
                            ai,
                            job.artifact_dir(),
                        ) {
                            Ok(def) => def,
                            Err(err) => {
                                debug!("Gen3D: failed to convert component draft: {err}");
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }
                        };

                    job.planned_components[idx].actual_size = Some(component_def.size);
                    job.planned_components[idx].anchors = component_def.anchors.clone();

                    // Replace component def in-place.
                    let target_id = component_def.object_id;
                    if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id)
                    {
                        let preserved_refs: Vec<ObjectPartDef> = existing
                            .parts
                            .iter()
                            .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
                            .cloned()
                            .collect();
                        let mut merged = component_def;
                        merged.parts.extend(preserved_refs);
                        *existing = merged;
                    } else {
                        draft.defs.push(component_def);
                    }

                    if let Some(root_idx) = job
                        .planned_components
                        .iter()
                        .position(|c| c.attach_to.is_none())
                    {
                        if let Err(err) = convert::resolve_planned_component_transforms(
                            &mut job.planned_components,
                            root_idx,
                        ) {
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    }
                    convert::update_root_def_from_planned_components(
                        &job.planned_components,
                        &job.plan_collider,
                        &mut draft,
                    );
                    write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                    job.assembly_rev = job.assembly_rev.saturating_add(1);

                    let next_pos = job.component_queue_pos + 1;
                    let per_component_refine_total =
                        component_refine_cycles_for_speed(&config, workshop.speed_mode);
                    if per_component_refine_total > 0
                        && matches!(job.generation_kind, Gen3dComponentGenerationKind::Initial)
                        && job.per_component_resume.is_none()
                    {
                        job.review_kind = Gen3dAutoReviewKind::PerComponent;
                        job.review_component_idx = Some(idx);
                        job.per_component_resume = Some(Gen3dComponentBatchResume {
                            generation_kind: job.generation_kind,
                            component_queue: job.component_queue.clone(),
                            component_queue_pos: next_pos,
                        });
                        job.component_queue_pos = next_pos;
                        job.per_component_refine_passes_done = 1;
                        job.per_component_refine_passes_remaining =
                            per_component_refine_total.saturating_sub(1);
                        job.phase = Gen3dAiPhase::CapturingReview;
                        job.review_capture = None;
                        job.review_static_paths.clear();
                        job.motion_capture = None;
                        workshop.status = format!(
                            "Auto-reviewing assembly… (after {})\n(pass {}/{})",
                            job.planned_components[idx].display_name,
                            job.per_component_refine_passes_done,
                            job.per_component_refine_passes_done
                                + job.per_component_refine_passes_remaining
                        );
                        if let Some(progress) = job.shared_progress.as_ref() {
                            set_progress(progress, "Preparing review capture…");
                        }
                        return;
                    }
                    if next_pos >= job.component_queue.len() {
                        if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent)
                            && matches!(
                                job.generation_kind,
                                Gen3dComponentGenerationKind::Regenerate
                            )
                            && job.per_component_resume.is_some()
                        {
                            if job.per_component_refine_passes_remaining > 0 {
                                job.per_component_refine_passes_remaining -= 1;
                                job.per_component_refine_passes_done += 1;
                                job.phase = Gen3dAiPhase::CapturingReview;
                                job.review_capture = None;
                                job.review_static_paths.clear();
                                job.motion_capture = None;
                                workshop.status = format!(
                                    "Auto-reviewing assembly… (component pass {}/{})",
                                    job.per_component_refine_passes_done,
                                    job.per_component_refine_passes_done
                                        + job.per_component_refine_passes_remaining
                                );
                                if let Some(progress) = job.shared_progress.as_ref() {
                                    set_progress(progress, "Preparing review capture…");
                                }
                                return;
                            }

                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }

                        // Finished this generation batch.
                        match job.generation_kind {
                            Gen3dComponentGenerationKind::Initial
                            | Gen3dComponentGenerationKind::Regenerate => {
                                if job.auto_refine_passes_remaining > 0 {
                                    job.auto_refine_passes_remaining -= 1;
                                    job.auto_refine_passes_done += 1;
                                    job.phase = Gen3dAiPhase::CapturingReview;
                                    job.review_capture = None;
                                    job.review_static_paths.clear();
                                    job.motion_capture = None;
                                    workshop.status = format!(
                                        "Auto-reviewing assembly… (pass {}/{})",
                                        job.auto_refine_passes_done,
                                        job.auto_refine_passes_done
                                            + job.auto_refine_passes_remaining
                                    );
                                    if let Some(progress) = job.shared_progress.as_ref() {
                                        set_progress(progress, "Preparing review capture…");
                                    }
                                    return;
                                }
                            }
                        }

                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    job.component_queue_pos = next_pos;
                    let next_idx = job.component_queue[next_pos];

                    let Some(openai) = job.openai.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing OpenAI config.",
                        );
                        return;
                    };
                    let Some(run_dir) = job.pass_dir.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing Gen3D pass dir.",
                        );
                        return;
                    };

                    let phase_label = match job.generation_kind {
                        Gen3dComponentGenerationKind::Initial => "Building components…",
                        Gen3dComponentGenerationKind::Regenerate => "Regenerating components…",
                    };
                    let comp = &job.planned_components[next_idx];
                    let forward = comp.rot * Vec3::Z;
                    let up = comp.rot * Vec3::Y;
                    workshop.status = format!(
                        "{phase_label} ({}/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
                        next_pos + 1,
                        job.component_queue.len(),
                        comp.display_name,
                        comp.pos.x,
                        comp.pos.y,
                        comp.pos.z,
                        forward.x,
                        forward.y,
                        forward.z,
                        up.x,
                        up.y,
                        up.z,
                    );
                    if let Some(progress) = job.shared_progress.as_ref() {
                        set_progress(progress, "Starting next component…");
                    }

                    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                        Arc::new(Mutex::new(None));
                    job.shared_result = Some(shared.clone());

                    let progress = job
                        .shared_progress
                        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                        .clone();

                    let system = build_gen3d_component_system_instructions();
                    let user_text = build_gen3d_component_user_text(
                        &job.user_prompt_raw,
                        !job.user_images.is_empty(),
                        workshop.speed_mode,
                        &job.assembly_notes,
                        &job.planned_components,
                        next_idx,
                    );
                    let prefix = match job.generation_kind {
                        Gen3dComponentGenerationKind::Initial => format!(
                            "component{:02}_{}",
                            next_idx + 1,
                            job.planned_components[next_idx].name
                        ),
                        Gen3dComponentGenerationKind::Regenerate => match job.review_kind {
                            Gen3dAutoReviewKind::EndOfRun => format!(
                                "review{:02}_regen_component{:02}_{}",
                                job.auto_refine_passes_done.max(1),
                                next_idx + 1,
                                job.planned_components[next_idx].name
                            ),
                            Gen3dAutoReviewKind::PerComponent => format!(
                                "percomp_component{:02}_pass{:02}_regen_component{:02}_{}",
                                job.review_component_idx
                                    .map(|idx| idx + 1)
                                    .unwrap_or(next_idx + 1),
                                job.per_component_refine_passes_done.max(1),
                                next_idx + 1,
                                job.planned_components[next_idx].name
                            ),
                        },
                    };
                    let reasoning_effort = openai.model_reasoning_effort.clone();
                    spawn_gen3d_ai_text_thread(
                        shared,
                        progress,
                        job.session.clone(),
                        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
                        openai,
                        reasoning_effort,
                        system,
                        user_text,
                        job.user_images.clone(),
                        run_dir,
                        prefix,
                    );
                }
                Gen3dAiPhase::WaitingReview => {
                    debug!("Gen3D: auto-review delta request finished.");
                    let delta = match parse::parse_ai_review_delta_from_text(&text) {
                        Ok(delta) => delta,
                        Err(err) => {
                            warn!("Gen3D: failed to parse AI review-delta: {err}");
                            if retry_gen3d_review_delta(
                                &mut workshop,
                                &mut job,
                                &config,
                                speed_mode,
                                &format!("Parse error: {err}"),
                            ) {
                                return;
                            }
                            workshop.error = Some(err);
                            if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                                debug!("Gen3D: per-component review failed; continuing build.");
                                workshop.status = "Auto-review failed (continuing build).".into();
                                resume_after_per_component_review(&mut workshop, &mut job);
                                return;
                            }

                            job.finish_run_metrics();
                            job.running = false;
                            job.build_complete = true;
                            job.phase = Gen3dAiPhase::Idle;
                            job.shared_progress = None;
                            workshop.status =
                                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                    .into();
                            return;
                        }
                    };

                    if let Some(summary) = delta.summary.as_deref() {
                        if !summary.trim().is_empty() {
                            debug!(
                                "Gen3D: review-delta summary: {}",
                                truncate_for_ui(summary.trim(), 800)
                            );
                        }
                    }
                    if let Some(notes) = delta.notes.as_deref() {
                        if !notes.trim().is_empty() {
                            debug!(
                                "Gen3D: review-delta notes: {}",
                                truncate_for_ui(notes.trim(), 800)
                            );
                        }
                    }

                    let expected_run_id = job
                        .run_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unknown".into());
                    if delta.applies_to.run_id != expected_run_id
                        || delta.applies_to.attempt != job.attempt
                        || delta.applies_to.plan_hash != job.plan_hash
                        || delta.applies_to.assembly_rev != job.assembly_rev
                    {
                        let msg = format!(
                            "applies_to mismatch (expected run_id={}, attempt={}, plan_hash={}, assembly_rev={})",
                            expected_run_id, job.attempt, job.plan_hash, job.assembly_rev
                        );
                        warn!("Gen3D: review-delta rejected: {msg}");
                        if retry_gen3d_review_delta(
                            &mut workshop,
                            &mut job,
                            &config,
                            speed_mode,
                            &msg,
                        ) {
                            return;
                        }
                        workshop.error = Some(msg);
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                .into();
                        return;
                    }

                    let plan_collider = job.plan_collider.clone();
                    let artifact_dir = job.pass_dir.clone();
                    let apply = match convert::apply_ai_review_delta_actions(
                        delta,
                        &mut job.planned_components,
                        &plan_collider,
                        &mut draft,
                        artifact_dir.as_deref(),
                    ) {
                        Ok(apply) => apply,
                        Err(err) => {
                            warn!("Gen3D: failed to apply review-delta actions: {err}");
                            if retry_gen3d_review_delta(
                                &mut workshop,
                                &mut job,
                                &config,
                                speed_mode,
                                &format!("Apply error: {err}"),
                            ) {
                                return;
                            }
                            workshop.error = Some(err);
                            job.finish_run_metrics();
                            job.running = false;
                            job.build_complete = true;
                            job.phase = Gen3dAiPhase::Idle;
                            job.shared_progress = None;
                            workshop.status =
                                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                    .into();
                            return;
                        }
                    };

                    if !apply.tooling_feedback.is_empty() {
                        record_gen3d_tooling_feedback(
                            &config,
                            &mut workshop,
                            &mut feedback_history,
                            &job,
                            &apply.tooling_feedback,
                        );
                    }

                    if apply.accepted {
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. (Reviewer accepted.) Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    if apply.had_actions {
                        job.assembly_rev = job.assembly_rev.saturating_add(1);
                        write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                    }

                    if let Some(reason) = apply.replan_reason {
                        if try_start_gen3d_replan(
                            &config,
                            &mut workshop,
                            &mut job,
                            &mut draft,
                            reason,
                        ) {
                            return;
                        }
                        finish_job_best_effort(
                            &mut commands,
                            &review_cameras,
                            &mut workshop,
                            &mut job,
                            format!(
                                "Replan budget exhausted (max_replans={}).",
                                config.gen3d_max_replans
                            ),
                        );
                        return;
                    }

                    if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                        // Keep the old per-component review behavior for now.
                        if !apply.had_actions {
                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }
                        if apply.regen_indices.is_empty() {
                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }
                    } else if !apply.had_actions {
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. (Auto-review made no changes.) Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    if matches!(job.review_kind, Gen3dAutoReviewKind::EndOfRun)
                        && apply.regen_indices.is_empty()
                    {
                        if job.auto_refine_passes_remaining > 0 {
                            if let Err(err) = gen3d_advance_pass(&mut job) {
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }
                            job.auto_refine_passes_remaining -= 1;
                            job.auto_refine_passes_done += 1;
                            job.phase = Gen3dAiPhase::CapturingReview;
                            job.review_capture = None;
                            job.review_static_paths.clear();
                            job.motion_capture = None;
                            workshop.status = format!(
                                "Auto-reviewing assembly… (pass {}/{})",
                                job.auto_refine_passes_done,
                                job.auto_refine_passes_done + job.auto_refine_passes_remaining
                            );
                            if let Some(progress) = job.shared_progress.as_ref() {
                                set_progress(progress, "Preparing review capture…");
                            }
                            return;
                        }

                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished (auto-review applied tweaks). Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    let requested_regen = apply.regen_indices;
                    let mut regen_allowed = Vec::new();
                    let mut regen_skipped = Vec::new();
                    if !requested_regen.is_empty() {
                        let max_total = config.gen3d_max_regen_total;
                        let max_per_component = config.gen3d_max_regen_per_component;
                        let planned_len = job.planned_components.len();
                        if job.regen_per_component.len() != planned_len {
                            job.regen_per_component.resize(planned_len, 0);
                        }

                        for idx in requested_regen {
                            if idx >= planned_len {
                                continue;
                            }
                            if max_total > 0 && job.regen_total >= max_total {
                                regen_skipped.push(idx);
                                continue;
                            }
                            if max_per_component > 0
                                && job.regen_per_component[idx] >= max_per_component
                            {
                                regen_skipped.push(idx);
                                continue;
                            }

                            job.regen_total = job.regen_total.saturating_add(1);
                            job.regen_per_component[idx] =
                                job.regen_per_component[idx].saturating_add(1);
                            regen_allowed.push(idx);
                        }

                        if regen_allowed.is_empty() {
                            finish_job_best_effort(
                                &mut commands,
                                &review_cameras,
                                &mut workshop,
                                &mut job,
                                format!(
                                    "Regen budget exhausted (max_regen_total={}, max_regen_per_component={}).",
                                    config.gen3d_max_regen_total, config.gen3d_max_regen_per_component
                                ),
                            );
                            return;
                        }
                        if !regen_skipped.is_empty() {
                            regen_skipped.sort();
                            regen_skipped.dedup();
                            warn!(
                                "Gen3D: regen budget reached; skipping regen for {} component(s): {:?}",
                                regen_skipped.len(),
                                regen_skipped
                            );
                            append_gen3d_run_log(
                                job.artifact_dir(),
                                format!(
                                    "regen_budget_skip skipped={} max_total={} max_per_component={}",
                                    regen_skipped.len(),
                                    config.gen3d_max_regen_total,
                                    config.gen3d_max_regen_per_component
                                ),
                            );
                        }
                    }

                    if !regen_allowed.is_empty() {
                        if let Err(err) = gen3d_advance_pass(&mut job) {
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    }

                    job.generation_kind = Gen3dComponentGenerationKind::Regenerate;
                    job.component_queue = regen_allowed;
                    job.component_queue_pos = 0;

                    let Some(openai) = job.openai.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing OpenAI config.",
                        );
                        return;
                    };
                    let Some(run_dir) = job.pass_dir.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing Gen3D pass dir.",
                        );
                        return;
                    };

                    let idx = job.component_queue[0];
                    job.phase = Gen3dAiPhase::WaitingComponent;
                    let comp = &job.planned_components[idx];
                    let forward = comp.rot * Vec3::Z;
                    let up = comp.rot * Vec3::Y;
                    workshop.status = format!(
                        "Regenerating components… (1/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
                        job.component_queue.len(),
                        comp.display_name,
                        comp.pos.x,
                        comp.pos.y,
                        comp.pos.z,
                        forward.x,
                        forward.y,
                        forward.z,
                        up.x,
                        up.y,
                        up.z,
                    );
                    if let Some(progress) = job.shared_progress.as_ref() {
                        set_progress(progress, "Starting component regeneration…");
                    }

                    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                        Arc::new(Mutex::new(None));
                    job.shared_result = Some(shared.clone());
                    let progress = job
                        .shared_progress
                        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                        .clone();
                    let system = build_gen3d_component_system_instructions();
                    let user_text = build_gen3d_component_user_text(
                        &job.user_prompt_raw,
                        !job.user_images.is_empty(),
                        workshop.speed_mode,
                        &job.assembly_notes,
                        &job.planned_components,
                        idx,
                    );
                    let prefix = match job.review_kind {
                        Gen3dAutoReviewKind::EndOfRun => format!(
                            "review{:02}_regen_component{:02}_{}",
                            job.auto_refine_passes_done.max(1),
                            idx + 1,
                            job.planned_components[idx].name
                        ),
                        Gen3dAutoReviewKind::PerComponent => format!(
                            "percomp_component{:02}_pass{:02}_regen_component{:02}_{}",
                            job.review_component_idx
                                .map(|idx| idx + 1)
                                .unwrap_or(idx + 1),
                            job.per_component_refine_passes_done.max(1),
                            idx + 1,
                            job.planned_components[idx].name
                        ),
                    };
                    let reasoning_effort = openai.model_reasoning_effort.clone();
                    spawn_gen3d_ai_text_thread(
                        shared,
                        progress,
                        job.session.clone(),
                        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
                        openai,
                        reasoning_effort,
                        system,
                        user_text,
                        job.user_images.clone(),
                        run_dir,
                        prefix,
                    );
                }
                Gen3dAiPhase::CapturingReview | Gen3dAiPhase::Idle => {}
            }
        }
        Err(err) => {
            debug!("Gen3D: AI job failed: {err}");
            if matches!(job.phase, Gen3dAiPhase::WaitingPlan)
                && retry_gen3d_plan(&mut workshop, &mut job, &mut draft, speed_mode, &err)
            {
                return;
            }

            fail_job(&mut workshop, &mut job, err);
        }
    }
}

const GEN3D_MAX_PLAN_RETRIES: u8 = 1;
const GEN3D_MAX_COMPONENT_RETRIES: u8 = 1;
const GEN3D_MAX_REVIEW_DELTA_REPAIRS: u8 = 1;

fn retry_gen3d_review_delta(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    _config: &AppConfig,
    _speed: Gen3dSpeedMode,
    reason: &str,
) -> bool {
    if job.review_delta_repair_attempt >= GEN3D_MAX_REVIEW_DELTA_REPAIRS {
        return false;
    }
    job.review_delta_repair_attempt += 1;

    let Some(openai) = job.openai.clone() else {
        return false;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        return false;
    };
    if job.last_review_inputs.is_empty() || job.last_review_user_text.trim().is_empty() {
        return false;
    }

    warn!(
        "Gen3D: retrying review-delta (repair {}/{}) reason={}",
        job.review_delta_repair_attempt,
        GEN3D_MAX_REVIEW_DELTA_REPAIRS,
        truncate_for_ui(reason, 800)
    );

    workshop.error = None;
    workshop.status = format!(
        "Auto-reviewing assembly… (repair {}/{})",
        job.review_delta_repair_attempt, GEN3D_MAX_REVIEW_DELTA_REPAIRS
    );

    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
        Arc::new(Mutex::new(None));
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();
    set_progress(&progress, "Repairing review-delta JSON…");
    job.phase = Gen3dAiPhase::WaitingReview;

    let system = build_gen3d_review_delta_system_instructions(job.review_appearance);
    let mut user_text = job.last_review_user_text.clone();
    user_text.push_str("\n\nYour previous response was invalid.\nError:\n");
    user_text.push_str(reason.trim());
    user_text.push_str("\n\nReturn corrected JSON ONLY. No markdown.\n");
    job.last_review_user_text = user_text.clone();

    let prefix = format!(
        "review{:02}_repair{:02}",
        job.auto_refine_passes_done.max(1),
        job.review_delta_repair_attempt
    );
    let images = job.last_review_inputs.clone();
    let reasoning_effort = openai.model_reasoning_effort.clone();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1),
        openai,
        reasoning_effort,
        system,
        user_text,
        images,
        run_dir,
        prefix,
    );

    true
}

fn retry_gen3d_plan(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: Gen3dSpeedMode,
    reason: &str,
) -> bool {
    if job.plan_attempt >= GEN3D_MAX_PLAN_RETRIES {
        return false;
    }

    job.plan_attempt += 1;
    warn!(
        "Gen3D: plan failed; retrying (attempt {}/{}) reason={}",
        job.plan_attempt + 1,
        GEN3D_MAX_PLAN_RETRIES + 1,
        truncate_for_ui(reason, 600)
    );

    job.reset_session();
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    draft.defs.clear();

    workshop.error = None;
    workshop.status = format!(
        "Planning components… (attempt {}/{})\nImages: {}",
        job.plan_attempt + 1,
        GEN3D_MAX_PLAN_RETRIES + 1,
        job.user_images.len()
    );

    let Some(openai) = job.openai.clone() else {
        fail_job(workshop, job, "Internal error: missing OpenAI config.");
        return true;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return true;
    };

    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
        Arc::new(Mutex::new(None));
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Starting…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    job.phase = Gen3dAiPhase::WaitingPlan;

    let system = build_gen3d_plan_system_instructions();
    let user_text =
        build_gen3d_plan_user_text(&job.user_prompt_raw, !job.user_images.is_empty(), speed);
    let prefix = format!("plan_retry{}", job.plan_attempt);
    let reasoning_effort = openai.model_reasoning_effort.clone();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::PlanV1),
        openai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        run_dir,
        prefix,
    );

    true
}

fn poll_gen3d_parallel_components(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: Gen3dSpeedMode,
) {
    let total = job.planned_components.len();
    if total == 0 {
        fail_job(workshop, job, "Internal error: no planned components.");
        return;
    }

    // 1) Apply any completed component results.
    let mut i = 0usize;
    while i < job.component_in_flight.len() {
        let maybe_result = {
            let task = &job.component_in_flight[i];
            let Ok(mut guard) = task.shared_result.lock() else {
                i += 1;
                continue;
            };
            guard.take()
        };

        let Some(result) = maybe_result else {
            i += 1;
            continue;
        };

        let task = job.component_in_flight.swap_remove(i);
        let idx = task.idx;
        let component_name = job
            .planned_components
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("component_{idx}"));

        match result {
            Ok(resp) => {
                debug!(
                    "Gen3D: component generation finished (idx={}, name={}, api={:?}, sent_images={})",
                    idx,
                    component_name,
                    resp.api,
                    task.sent_images
                );
                job.note_api_used(resp.api);
                if let Some(tokens) = resp.total_tokens {
                    job.add_tokens(tokens);
                }
                if let Some(flag) = resp.session.responses_supported {
                    job.session.responses_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_continuation_supported {
                    job.session.responses_continuation_supported = Some(flag);
                }

                let ai = match parse::parse_ai_draft_from_text(&resp.text) {
                    Ok(ai) => ai,
                    Err(err) => {
                        if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D: component draft parse failed; retrying component {} (idx={}, attempt {}/{}) err={}",
                                component_name,
                                idx,
                                next + 1,
                                GEN3D_MAX_COMPONENT_RETRIES + 1,
                                truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        fail_job(workshop, job, err);
                        return;
                    }
                };

                let component_def = match job
                    .planned_components
                    .get(idx)
                    .ok_or_else(|| {
                        format!("Internal error: missing planned component for idx={idx}")
                    })
                    .and_then(|planned| convert::ai_to_component_def(planned, ai, job.artifact_dir()))
                {
                    Ok(def) => def,
                    Err(err) => {
                        if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D: component draft conversion failed; retrying component {} (idx={}, attempt {}/{}) err={}",
                                component_name,
                                idx,
                                next + 1,
                                GEN3D_MAX_COMPONENT_RETRIES + 1,
                                truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        fail_job(workshop, job, err);
                        return;
                    }
                };

                if let Some(comp) = job.planned_components.get_mut(idx) {
                    comp.actual_size = Some(component_def.size);
                    comp.anchors = component_def.anchors.clone();
                }

                // Replace component def in-place.
                let target_id = component_def.object_id;
                if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
                    let preserved_refs: Vec<ObjectPartDef> = existing
                        .parts
                        .iter()
                        .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
                        .cloned()
                        .collect();
                    let mut merged = component_def;
                    merged.parts.extend(preserved_refs);
                    *existing = merged;
                } else {
                    draft.defs.push(component_def);
                }

                if let Some(root_idx) = job
                    .planned_components
                    .iter()
                    .position(|c| c.attach_to.is_none())
                {
                    if let Err(err) = convert::resolve_planned_component_transforms(
                        &mut job.planned_components,
                        root_idx,
                    ) {
                        fail_job(workshop, job, err);
                        return;
                    }
                }
                convert::update_root_def_from_planned_components(
                    &job.planned_components,
                    &job.plan_collider,
                    draft,
                );
                write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                job.assembly_rev = job.assembly_rev.saturating_add(1);
            }
            Err(err) => {
                if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                    let next = task.attempt + 1;
                    warn!(
                        "Gen3D: component request failed; retrying component {} (idx={}, attempt {}/{}, sent_images={}) err={}",
                        component_name,
                        idx,
                        next + 1,
                        GEN3D_MAX_COMPONENT_RETRIES + 1,
                        task.sent_images,
                        truncate_for_ui(&err, 600),
                    );
                    if idx >= job.component_attempts.len() {
                        job.component_attempts
                            .resize(job.planned_components.len(), 0);
                    }
                    job.component_attempts[idx] = next;
                    job.component_queue.insert(0, idx);
                    continue;
                }
                fail_job(workshop, job, err);
                return;
            }
        }
    }

    // 2) Start new component requests up to the parallel limit.
    let mut parallel = job.max_parallel_components.max(1).min(total);
    // Some providers support `/responses` but do not support `previous_response_id` continuation.
    // When that support is unknown, "probe" with a single request first to avoid spamming 400s.
    if job.session.responses_previous_id.is_some()
        && job.session.responses_continuation_supported.is_none()
    {
        parallel = parallel.min(1);
    }
    while job.component_in_flight.len() < parallel && !job.component_queue.is_empty() {
        let idx = job.component_queue.remove(0);
        if idx >= job.planned_components.len() {
            continue;
        }

        let Some(openai) = job.openai.clone() else {
            fail_job(workshop, job, "Internal error: missing OpenAI config.");
            return;
        };
        let Some(run_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return;
        };

        let attempt = *job.component_attempts.get(idx).unwrap_or(&0);
        let sent_images = !job.user_images.is_empty();
        let image_paths = if sent_images {
            job.user_images.clone()
        } else {
            Vec::new()
        };

        let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
            Arc::new(Mutex::new(None));
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let system = build_gen3d_component_system_instructions();
        let user_text = build_gen3d_component_user_text(
            &job.user_prompt_raw,
            !job.user_images.is_empty(),
            speed,
            &job.assembly_notes,
            &job.planned_components,
            idx,
        );
        let prefix = if attempt == 0 {
            format!(
                "component{:02}_{}",
                idx + 1,
                job.planned_components[idx].name
            )
        } else {
            format!(
                "component{:02}_{}_retry{}",
                idx + 1,
                job.planned_components[idx].name,
                attempt
            )
        };
        let reasoning_effort = openai.model_reasoning_effort.clone();
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.session.clone(),
            Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
            openai,
            reasoning_effort,
            system,
            user_text,
            image_paths,
            run_dir,
            prefix,
        );

        job.component_in_flight.push(Gen3dInFlightComponent {
            idx,
            attempt,
            sent_images,
            shared_result: shared,
            _progress: progress,
        });
    }

    // 3) Update status and complete if finished.
    let done = job
        .planned_components
        .iter()
        .filter(|c| c.actual_size.is_some())
        .count();
    let in_flight = job.component_in_flight.len();
    let pending = job.component_queue.len();
    workshop.status = format!(
        "Building components… ({done}/{total})\nIn flight: {in_flight} | pending: {pending}\nParallel: {parallel}"
    );

    if done == total && in_flight == 0 && pending == 0 {
        if job.auto_refine_passes_remaining > 0 {
            job.auto_refine_passes_remaining -= 1;
            job.auto_refine_passes_done += 1;
            job.phase = Gen3dAiPhase::CapturingReview;
            job.review_capture = None;
            job.review_static_paths.clear();
            job.motion_capture = None;
            job.capture_previews_only = false;
            workshop.status = format!(
                "Auto-reviewing assembly… (pass {}/{})",
                job.auto_refine_passes_done,
                job.auto_refine_passes_done + job.auto_refine_passes_remaining
            );
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Preparing review capture…");
            }
            append_gen3d_run_log(job.artifact_dir(), "auto_review_start");
        } else if job.pass_dir.is_some() {
            job.phase = Gen3dAiPhase::CapturingReview;
            job.review_capture = None;
            job.review_static_paths.clear();
            job.motion_capture = None;
            job.capture_previews_only = true;
            workshop.status = "Capturing preview renders…".into();
            append_gen3d_run_log(job.artifact_dir(), "capture_previews_start");
        } else {
            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            workshop.status =
                "Build finished. Orbit/zoom the preview. Click Build to start a new run.".into();
        }
    }
}

fn fail_job(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob, err: impl Into<String>) {
    let err = err.into();
    error!("Gen3D: build failed: {}", truncate_for_ui(&err, 1200));
    abort_pending_agent_tool_call(job, format!("Run failed: {err}"));
    workshop.error = Some(err);
    workshop.status = "Build failed.".into();
    job.finish_run_metrics();
    job.running = false;
    job.build_complete = false;
    job.phase = Gen3dAiPhase::Idle;
    job.plan_attempt = 0;
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.capture_previews_only = false;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.replan_attempts = 0;
    job.regen_total = 0;
    job.regen_per_component.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
}

fn abort_pending_agent_tool_call(job: &mut Gen3dAiJob, reason: String) {
    let Some(call) = job.agent.pending_tool_call.take() else {
        return;
    };
    let result = crate::gen3d::agent::Gen3dToolResultJsonV1::err(
        call.call_id.clone(),
        call.tool_id.clone(),
        reason.clone(),
    );
    append_gen3d_jsonl_artifact(
        job.artifact_dir(),
        "tool_results.jsonl",
        &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
    );
    append_gen3d_run_log(
        job.artifact_dir(),
        format!(
            "tool_call_aborted call_id={} tool_id={} reason={}",
            call.call_id,
            call.tool_id,
            truncate_for_ui(reason.trim(), 360)
        ),
    );
    job.agent.step_tool_results.push(result);
    job.agent.pending_llm_tool = None;
    job.agent.pending_component_batch = None;
    job.agent.pending_render = None;
    job.agent.pending_pass_snapshot = None;
    job.agent.pending_after_pass_snapshot = None;
}

fn finish_job_best_effort(
    commands: &mut Commands,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    reason: String,
) {
    warn!(
        "Gen3D: stopping run due to budget: {}",
        truncate_for_ui(&reason, 800)
    );
    append_gen3d_run_log(
        job.artifact_dir(),
        format!("budget_stop reason={}", truncate_for_ui(&reason, 600)),
    );
    abort_pending_agent_tool_call(job, format!("Run stopped (best effort): {reason}"));

    workshop.error = None;
    workshop.status = format!(
        "Build finished (best effort).\nReason: {}\nYou can Save this draft or click Build to start a new run.",
        truncate_for_ui(&reason, 600)
    );

    for entity in review_cameras {
        commands.entity(entity).try_despawn();
    }
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.capture_previews_only = false;

    job.finish_run_metrics();
    job.running = false;
    job.build_complete = true;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
}

fn resume_after_per_component_review(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob) {
    let Some(resume) = job.per_component_resume.take() else {
        return;
    };

    job.component_queue = resume.component_queue;
    job.component_queue_pos = resume.component_queue_pos;
    job.generation_kind = resume.generation_kind;
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;

    let next_pos = job.component_queue_pos;
    if next_pos >= job.component_queue.len() {
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = true;
        job.phase = Gen3dAiPhase::Idle;
        job.shared_progress = None;
        workshop.status =
            "Build finished. Orbit/zoom the preview. Click Build to start a new run.".into();
        return;
    }

    let Some(openai) = job.openai.clone() else {
        fail_job(workshop, job, "Internal error: missing OpenAI config.");
        return;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return;
    };

    let next_idx = job.component_queue[next_pos];
    job.phase = Gen3dAiPhase::WaitingComponent;
    let comp = &job.planned_components[next_idx];
    let forward = comp.rot * Vec3::Z;
    let up = comp.rot * Vec3::Y;
    workshop.status = format!(
        "Building components… ({}/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
        next_pos + 1,
        job.component_queue.len(),
        comp.display_name,
        comp.pos.x,
        comp.pos.y,
        comp.pos.z,
        forward.x,
        forward.y,
        forward.z,
        up.x,
        up.y,
        up.z,
    );

    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
        Arc::new(Mutex::new(None));
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();

    let system = build_gen3d_component_system_instructions();
    let user_text = build_gen3d_component_user_text(
        &job.user_prompt_raw,
        !job.user_images.is_empty(),
        workshop.speed_mode,
        &job.assembly_notes,
        &job.planned_components,
        next_idx,
    );
    let prefix = format!(
        "component{:02}_{}",
        next_idx + 1,
        job.planned_components[next_idx].name
    );
    let reasoning_effort = openai.model_reasoning_effort.clone();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
        openai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        run_dir,
        prefix,
    );
}

fn try_start_gen3d_replan(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    reason: String,
) -> bool {
    let max_replans = config.gen3d_max_replans;
    if job.replan_attempts >= max_replans {
        debug!("Gen3D: replan requested, but max attempts reached; ignoring.");
        return false;
    }
    job.replan_attempts += 1;

    let Some(openai) = job.openai.clone() else {
        fail_job(workshop, job, "Internal error: missing OpenAI config.");
        return true;
    };
    let Some(run_dir) = job.run_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D cache dir.");
        return true;
    };

    debug!(
        "Gen3D: starting replan attempt {}: {}",
        job.replan_attempts, reason
    );

    let next_attempt = job.replan_attempts;
    if let Err(err) = gen3d_set_current_attempt_pass(job, &run_dir, next_attempt, 0) {
        fail_job(workshop, job, err);
        return true;
    }
    let attempt_dir = gen3d_attempt_dir(&run_dir, next_attempt);
    let cached_inputs = cache_gen3d_inputs(&attempt_dir, &job.user_prompt_raw, &job.user_images);
    job.user_images = cached_inputs.cached_image_paths;

    // Reset build state but keep the same run/session/tokens.
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.auto_refine_passes_done = 0;
    job.auto_refine_passes_remaining = refine_passes_for_speed(config, workshop.speed_mode);
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.generation_kind = Gen3dComponentGenerationKind::Initial;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.plan_hash.clear();
    job.assembly_rev = 0;
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.regen_per_component.clear();
    draft.defs.clear();

    workshop.error = None;
    workshop.status = format!(
        "Re-planning components…\nReason: {}\nModel: {}\nImages: {}",
        reason,
        openai.model,
        job.user_images.len()
    );

    job.phase = Gen3dAiPhase::WaitingPlan;
    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
        Arc::new(Mutex::new(None));
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();
    set_progress(&progress, "Starting re-plan…");

    let system = build_gen3d_plan_system_instructions();
    let mut user_text = build_gen3d_plan_user_text(
        &job.user_prompt_raw,
        !job.user_images.is_empty(),
        workshop.speed_mode,
    );
    user_text.push_str("\n\nReplan requested by reviewer.\nReason:\n");
    user_text.push_str(reason.trim());
    user_text.push('\n');
    let prefix = format!("attempt{:02}_plan", job.attempt);
    let Some(pass_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return true;
    };
    let reasoning_effort = openai.model_reasoning_effort.clone();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::PlanV1),
        openai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        pass_dir,
        prefix,
    );

    true
}

fn default_gen3d_cache_dir() -> PathBuf {
    crate::paths::default_gen3d_cache_dir()
}

fn gen3d_make_run_dir(config: &AppConfig) -> (Uuid, PathBuf) {
    let base = config
        .gen3d_cache_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(default_gen3d_cache_dir);
    let run_id = Uuid::new_v4();
    (run_id, base.join(run_id.to_string()))
}

fn gen3d_attempt_dir(run_dir: &Path, attempt: u32) -> PathBuf {
    run_dir.join(format!("attempt_{attempt}"))
}

fn gen3d_set_current_attempt_pass(
    job: &mut Gen3dAiJob,
    run_dir: &Path,
    attempt: u32,
    pass: u32,
) -> Result<(), String> {
    let attempt_dir = gen3d_attempt_dir(run_dir, attempt);
    std::fs::create_dir_all(&attempt_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D attempt dir {}: {err}",
            attempt_dir.display()
        )
    })?;
    let pass_dir = attempt_dir.join(format!("pass_{pass}"));
    std::fs::create_dir_all(&pass_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D pass dir {}: {err}",
            pass_dir.display()
        )
    })?;

    job.attempt = attempt;
    job.pass = pass;
    job.pass_dir = Some(pass_dir.clone());

    if let Some(sinks) = job.log_sinks.as_ref() {
        if let Err(err) = sinks.start_gen3d_pass_log(pass_dir.join("gravimera.log")) {
            warn!("Gen3D: failed to start per-pass log capture: {err}");
        }
    }

    write_gen3d_json_artifact(
        Some(&pass_dir),
        "pass.json",
        &serde_json::json!({
            "version": 1,
            "attempt": attempt,
            "pass": pass,
            "created_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        }),
    );

    job.metrics.note_pass_started(pass);

    Ok(())
}

fn gen3d_advance_pass(job: &mut Gen3dAiJob) -> Result<(), String> {
    let run_dir = job
        .run_dir
        .clone()
        .ok_or_else(|| "Internal error: missing Gen3D run dir.".to_string())?;
    let next = job.pass.saturating_add(1);
    gen3d_set_current_attempt_pass(job, &run_dir, job.attempt, next)
}

#[derive(Clone, Debug)]
struct Gen3dCachedInputs {
    cached_image_paths: Vec<PathBuf>,
}

fn cache_gen3d_inputs(
    attempt_dir: &Path,
    prompt_raw: &str,
    image_paths: &[PathBuf],
) -> Gen3dCachedInputs {
    let inputs_dir = attempt_dir.join("inputs");
    let images_dir = inputs_dir.join("images");
    if let Err(err) = std::fs::create_dir_all(&images_dir) {
        debug!(
            "Gen3D: failed to create inputs dir {}: {err}",
            images_dir.display()
        );
    }

    let prompt_path = inputs_dir.join("user_prompt.txt");
    if let Err(err) = std::fs::write(&prompt_path, prompt_raw) {
        debug!(
            "Gen3D: failed to write prompt {}: {err}",
            prompt_path.display()
        );
    }

    let mut cached_image_paths = Vec::with_capacity(image_paths.len());
    let mut manifest_images: Vec<serde_json::Value> = Vec::with_capacity(image_paths.len());

    for (idx, src) in image_paths.iter().enumerate() {
        let file_name = src.file_name().and_then(|s| s.to_str()).unwrap_or("image");
        let sanitized = file_name
            .chars()
            .map(|ch| if ch == '/' || ch == '\\' { '_' } else { ch })
            .collect::<String>();
        let dst_name = format!("{:02}_{sanitized}", idx + 1);
        let dst = images_dir.join(dst_name);
        let copied = match std::fs::copy(src, &dst) {
            Ok(bytes) => {
                cached_image_paths.push(dst.clone());
                manifest_images.push(serde_json::json!({
                    "index": idx + 1,
                    "original_path": src.display().to_string(),
                    "cached_path": dst.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                    "bytes": bytes,
                }));
                true
            }
            Err(err) => {
                debug!(
                    "Gen3D: failed to cache input image {}: {err}",
                    src.display()
                );
                cached_image_paths.push(src.clone());
                manifest_images.push(serde_json::json!({
                    "index": idx + 1,
                    "original_path": src.display().to_string(),
                    "cached_path": null,
                    "error": err.to_string(),
                }));
                false
            }
        };
        if copied {
            debug!(
                "Gen3D: cached input image {}/{} to {}",
                idx + 1,
                image_paths.len(),
                dst.display()
            );
        }
    }

    let manifest = serde_json::json!({
        "version": 1,
        "user_prompt_file": "inputs/user_prompt.txt",
        "images_dir": "inputs/images",
        "images": manifest_images,
    });
    write_gen3d_json_artifact(Some(attempt_dir), "inputs_manifest.json", &manifest);

    Gen3dCachedInputs { cached_image_paths }
}

fn create_gen3d_review_render_target(
    images: &mut Assets<Image>,
    width_px: u32,
    height_px: u32,
) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        width_px.max(1),
        height_px.max(1),
        TextureFormat::bevy_default(),
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(image)
}

fn gen3d_orbit_transform(yaw: f32, pitch: f32, distance: f32, focus: Vec3) -> Transform {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let pos = focus + rot * Vec3::new(0.0, 0.0, distance);
    Transform::from_translation(pos).looking_at(focus, Vec3::Y)
}

fn gen3d_required_distance_for_view(
    half_extents: Vec3,
    yaw: f32,
    pitch: f32,
    fov_y: f32,
    aspect: f32,
    near: f32,
) -> f32 {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let mut view_dir = -rot * Vec3::Z;
    if !view_dir.is_finite() || view_dir.length_squared() <= 1e-6 {
        view_dir = -Vec3::Z;
    } else {
        view_dir = view_dir.normalize();
    }

    let mut right = Vec3::Y.cross(view_dir);
    if !right.is_finite() || right.length_squared() <= 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let mut up = view_dir.cross(right);
    if !up.is_finite() || up.length_squared() <= 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    let extent_right = half_extents.x * right.x.abs()
        + half_extents.y * right.y.abs()
        + half_extents.z * right.z.abs();
    let extent_up =
        half_extents.x * up.x.abs() + half_extents.y * up.y.abs() + half_extents.z * up.z.abs();
    let extent_forward = half_extents.x * view_dir.x.abs()
        + half_extents.y * view_dir.y.abs()
        + half_extents.z * view_dir.z.abs();

    let tan_y = (fov_y * 0.5).tan().max(1e-4);
    let tan_x = (tan_y * aspect).max(1e-4);
    let dist_y = extent_up / tan_y;
    let dist_x = extent_right / tan_x;

    // Ensure the near plane won't clip the bounds.
    dist_x.max(dist_y).max(extent_forward + near + 0.05)
}

fn start_gen3d_review_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    run_dir: &Path,
    draft: &Gen3dDraft,
    include_overlay: bool,
    file_prefix: &str,
    views: &[Gen3dReviewView],
    width_px: u32,
    height_px: u32,
) -> Result<Gen3dReviewCaptureState, String> {
    let Some(root) = draft.root_def() else {
        return Err("Internal error: missing Gen3D draft root.".into());
    };

    let focus = super::preview::compute_draft_focus(draft);
    let half_extents = root.size.abs().max(Vec3::splat(0.01)) * 0.5;
    let aspect = width_px.max(1) as f32 / height_px.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    // Use a slightly above-the-object pitch for the horizontal ring views.
    // `GEN3D_PREVIEW_DEFAULT_PITCH` is used by the interactive preview and may be negative.
    let base_pitch = -super::GEN3D_PREVIEW_DEFAULT_PITCH.abs();

    if views.is_empty() {
        return Err("Internal error: no review views requested.".into());
    }

    // Pick a single distance that fits all views. This keeps scales comparable across screenshots,
    // and makes the object fill more of the frame than the previous overly conservative formula.
    let base_distance = views
        .iter()
        .map(|view| {
            let (yaw, pitch) = view.orbit_angles(base_pitch);
            gen3d_required_distance_for_view(half_extents, yaw, pitch, fov_y, aspect, near)
        })
        .fold(0.0f32, f32::max);
    let margin = if include_overlay { 1.15 } else { 1.08 };
    let distance = (base_distance * margin).clamp(near + 0.2, 250.0);

    let progress = Arc::new(Mutex::new(Gen3dReviewCaptureProgress {
        expected: views.len(),
        completed: 0,
    }));

    let mut cameras = Vec::with_capacity(views.len());
    let mut image_paths = Vec::with_capacity(views.len());

    for &view in views {
        let target = create_gen3d_review_render_target(images, width_px, height_px);
        let (yaw, pitch) = view.orbit_angles(base_pitch);
        let transform = gen3d_orbit_transform(yaw, pitch, distance, focus);

        let render_layers = if include_overlay {
            bevy::camera::visibility::RenderLayers::from_layers(&[
                super::GEN3D_PREVIEW_LAYER,
                super::GEN3D_REVIEW_LAYER,
            ])
        } else {
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER)
        };

        let camera = commands
            .spawn((
                Camera3d::default(),
                bevy::camera::Projection::Perspective(projection.clone()),
                Camera {
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                Tonemapping::TonyMcMapface,
                render_layers,
                transform,
                Gen3dReviewCaptureCamera,
            ))
            .id();
        cameras.push(camera);

        let path = run_dir.join(format!("{file_prefix}_{}.png", view.file_stem()));
        image_paths.push(path.clone());

        let progress_clone = progress.clone();
        commands
            .spawn(Screenshot::image(target))
            .observe(move |event: On<ScreenshotCaptured>| {
                let mut saver = save_to_disk(path.clone());
                saver(event);
                if let Ok(mut guard) = progress_clone.lock() {
                    guard.completed = guard.completed.saturating_add(1);
                }
            });
    }

    Ok(Gen3dReviewCaptureState {
        cameras,
        image_paths,
        progress,
    })
}

fn write_gen3d_sprite_sheet_2x2(sheet_path: &Path, frames: &[PathBuf]) -> Result<(), String> {
    use image::GenericImage;

    if frames.len() != 4 {
        return Err(format!(
            "Expected 4 frames for sprite sheet, got {}.",
            frames.len()
        ));
    }

    let imgs: Vec<image::RgbaImage> = frames
        .iter()
        .map(|path| {
            image::open(path)
                .map(|img| img.to_rgba8())
                .map_err(|err| format!("Failed to read frame {}: {err}", path.display()))
        })
        .collect::<Result<_, _>>()?;

    let (w, h) = imgs[0].dimensions();
    for (idx, img) in imgs.iter().enumerate().skip(1) {
        if img.dimensions() != (w, h) {
            return Err(format!(
                "Sprite sheet frame size mismatch at index {idx}: expected {w}x{h}, got {:?}.",
                img.dimensions()
            ));
        }
    }

    let mut sheet = image::RgbaImage::new(w.saturating_mul(2), h.saturating_mul(2));
    sheet
        .copy_from(&imgs[0], 0, 0)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[1], w, 0)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[2], 0, h)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[3], w, h)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;

    image::DynamicImage::ImageRgba8(sheet)
        .save(sheet_path)
        .map_err(|err| {
            format!(
                "Failed to write sprite sheet {}: {err}",
                sheet_path.display()
            )
        })?;

    Ok(())
}

fn poll_gen3d_motion_capture(
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    let Some(pass_dir) = job.pass_dir.clone() else {
        debug!("Gen3D: motion capture skipped (missing pass dir).");
        job.motion_capture = None;
        return;
    };

    let mut iter = preview_model.iter_mut();
    let Some((mut channels, mut locomotion, mut attack_clock)) = iter.next() else {
        debug!("Gen3D: motion capture skipped (missing preview model root).");
        job.motion_capture = None;
        return;
    };

    let Some(motion) = job.motion_capture.as_mut() else {
        return;
    };

    const FRAMES: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
    if motion.frame_idx as usize >= FRAMES.len() {
        motion.frame_idx = 0;
    }

    let total_frames = FRAMES.len() as u8;
    let frame_idx = motion.frame_idx.min(total_frames.saturating_sub(1));
    let sample_phase_01 = FRAMES[frame_idx as usize];

    fn infer_move_cycle_m(
        rig_move_cycle_m: Option<f32>,
        components: &[Gen3dPlannedComponent],
    ) -> f32 {
        if let Some(v) = rig_move_cycle_m
            .filter(|v| v.is_finite())
            .map(|v| v.abs())
            .filter(|v| *v > 1e-3)
        {
            return v;
        }

        let mut best: Option<f32> = None;
        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            let Some(slot) = att.animations.iter().find(|s| s.channel.as_ref() == "move") else {
                continue;
            };
            if !matches!(
                slot.spec.driver,
                PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
            ) {
                continue;
            }
            let (duration_secs, repeats) = match &slot.spec.clip {
                PartAnimationDef::Loop { duration_secs, .. }
                | PartAnimationDef::Once { duration_secs, .. } => (*duration_secs, 1.0),
                PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0),
                PartAnimationDef::Spin { .. } => continue,
            };
            if !duration_secs.is_finite() || duration_secs <= 0.0 {
                continue;
            }
            let speed_scale = slot.spec.speed_scale.max(1e-6);
            let effective = (repeats * duration_secs / speed_scale).abs();
            if !effective.is_finite() || effective <= 1e-3 {
                continue;
            }
            best = Some(best.map_or(effective, |b| b.max(effective)));
        }

        best.unwrap_or(1.0)
    }

    fn infer_attack_window_secs(draft: &Gen3dDraft, components: &[Gen3dPlannedComponent]) -> f32 {
        if let Some(v) = draft
            .root_def()
            .and_then(|def| def.attack.as_ref())
            .map(|a| a.anim_window_secs)
            .filter(|v| v.is_finite())
            .map(|v| v.abs())
            .filter(|v| *v > 1e-3)
        {
            return v;
        }

        let mut best: Option<f32> = None;
        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            for slot in att.animations.iter() {
                if slot.channel.as_ref() != "attack_primary" {
                    continue;
                }
                let (duration_secs, repeats) = match &slot.spec.clip {
                    PartAnimationDef::Loop { duration_secs, .. }
                    | PartAnimationDef::Once { duration_secs, .. } => (*duration_secs, 1.0),
                    PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0),
                    PartAnimationDef::Spin { .. } => continue,
                };
                if !duration_secs.is_finite() || duration_secs <= 0.0 {
                    continue;
                }
                let speed = slot.spec.speed_scale.max(1e-3);
                let wall_duration = (repeats * duration_secs / speed).abs();
                if !wall_duration.is_finite() || wall_duration <= 1e-3 {
                    continue;
                }
                best = Some(best.map_or(wall_duration, |b| b.max(wall_duration)));
            }
        }

        best.unwrap_or(1.0)
    }

    if motion.frame_capture.is_none() {
        match motion.kind {
            Gen3dMotionCaptureKind::Move => {
                let cycle_m =
                    infer_move_cycle_m(job.rig_move_cycle_m, &job.planned_components).max(1e-3);
                let sample_m = (sample_phase_01 * cycle_m).clamp(0.0, cycle_m);
                channels.moving = true;
                channels.attacking_primary = false;
                locomotion.t = sample_m;
                locomotion.distance_m = sample_m;
                locomotion.signed_distance_m = sample_m;
                locomotion.speed_mps = 1.0;
            }
            Gen3dMotionCaptureKind::Attack => {
                let window_secs =
                    infer_attack_window_secs(draft, &job.planned_components).max(1e-3);
                let sample_secs = (sample_phase_01 * window_secs).clamp(0.0, window_secs);
                channels.moving = false;
                channels.attacking_primary = true;
                attack_clock.duration_secs = window_secs;
                attack_clock.started_at_secs = time.elapsed_secs() - sample_secs;
            }
        }

        let prefix = format!("{}_frame{:02}", motion.kind.label(), frame_idx + 1);
        if let Some(progress) = job.shared_progress.as_ref() {
            set_progress(
                progress,
                format!(
                    "Capturing {} animation… (frame {}/{})",
                    motion.kind.label(),
                    frame_idx + 1,
                    total_frames
                ),
            );
        }

        let views = [Gen3dReviewView::Front];
        match start_gen3d_review_capture(
            commands,
            images,
            &pass_dir,
            draft,
            false,
            &prefix,
            &views,
            super::GEN3D_REVIEW_CAPTURE_WIDTH_PX,
            super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
        ) {
            Ok(state) => {
                motion.frame_capture = Some(state);
            }
            Err(err) => {
                warn!("Gen3D: motion capture failed to start: {err}");
                workshop.error = Some(err);
                job.motion_capture = None;
            }
        }
        return;
    }

    let Some(state) = motion.frame_capture.as_ref() else {
        return;
    };
    let (done, expected) = state
        .progress
        .lock()
        .map(|g| (g.completed, g.expected))
        .unwrap_or((0, 1));
    if done < expected {
        return;
    }

    // Clean up capture cameras.
    for cam in state.cameras.iter().copied() {
        commands.entity(cam).try_despawn();
    }
    let paths = state.image_paths.clone();
    motion.frame_capture = None;

    for path in &paths {
        if std::fs::metadata(path).is_err() {
            warn!("Gen3D: motion capture missing frame: {}", path.display());
            job.motion_capture = None;
            return;
        }
    }
    motion.frame_paths.extend(paths);

    motion.frame_idx = motion.frame_idx.saturating_add(1);
    if motion.frame_idx < total_frames {
        return;
    }

    let sheet_path = pass_dir.join(motion.kind.sheet_filename());
    if let Err(err) = write_gen3d_sprite_sheet_2x2(&sheet_path, &motion.frame_paths) {
        warn!("Gen3D: failed to compose {}: {err}", sheet_path.display());
    } else {
        job.review_static_paths.push(sheet_path);
    }

    // Prepare next sheet or finish.
    motion.frame_idx = 0;
    motion.frame_paths.clear();
    motion.frame_capture = None;
    motion.kind = match motion.kind {
        Gen3dMotionCaptureKind::Move => Gen3dMotionCaptureKind::Attack,
        Gen3dMotionCaptureKind::Attack => {
            channels.moving = false;
            channels.attacking_primary = false;
            job.motion_capture = None;
            return;
        }
    };
}

fn compute_gen3d_plan_hash(
    assembly_notes: &str,
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
) -> String {
    let mut comps: Vec<&Gen3dPlannedComponent> = components.iter().collect();
    comps.sort_by(|a, b| a.name.cmp(&b.name));
    let comps_json: Vec<serde_json::Value> = comps
        .into_iter()
        .map(|c| {
            let anchors: Vec<serde_json::Value> = c
                .anchors
                .iter()
                .map(|a| {
                    let pos = a.transform.translation;
                    let q = a.transform.rotation.normalize();
                    serde_json::json!({
                        "name": a.name.as_ref(),
                        "pos": [pos.x, pos.y, pos.z],
                        "rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                    })
                })
                .collect();

            let contacts: Vec<serde_json::Value> = {
                let mut contacts: Vec<&AiContactJson> = c.contacts.iter().collect();
                contacts.sort_by(|a, b| a.name.cmp(&b.name));
                contacts
                    .into_iter()
                    .map(|contact| {
                        let stance = contact.stance.as_ref().map(|s| {
                            serde_json::json!({
                                "phase_01": s.phase_01,
                                "duty_factor_01": s.duty_factor_01,
                            })
                        });
                        serde_json::json!({
                            "name": contact.name.as_str(),
                            "kind": match contact.kind {
                                AiContactKindJson::Ground => "ground",
                                AiContactKindJson::Unknown => "unknown",
                            },
                            "anchor": contact.anchor.as_str(),
                            "stance": stance,
                        })
                    })
                    .collect()
            };

            let attach_to = c.attach_to.as_ref().map(|att| {
                let pos = att.offset.translation;
                let q = att.offset.rotation.normalize();
                let s = att.offset.scale;
                let channels: Vec<&str> = att
                    .animations
                    .iter()
                    .map(|slot| slot.channel.as_ref())
                    .collect();
                let joint = att.joint.as_ref().map(|j| {
                    serde_json::json!({
                        "kind": match j.kind {
                            AiJointKindJson::Fixed => "fixed",
                            AiJointKindJson::Hinge => "hinge",
                            AiJointKindJson::Ball => "ball",
                            AiJointKindJson::Free => "free",
                            AiJointKindJson::Unknown => "unknown",
                        },
                        "axis_join": j.axis_join,
                        "limits_degrees": j.limits_degrees,
                        "swing_limits_degrees": j.swing_limits_degrees,
                        "twist_limits_degrees": j.twist_limits_degrees,
                    })
                });
                serde_json::json!({
                    "parent": att.parent.as_str(),
                    "parent_anchor": att.parent_anchor.as_str(),
                    "child_anchor": att.child_anchor.as_str(),
                    "offset_pos": [pos.x, pos.y, pos.z],
                    "offset_rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                    "offset_scale": [s.x, s.y, s.z],
                    "joint": joint,
                    "animation_channels": channels,
                })
            });

            serde_json::json!({
                "name": c.name.as_str(),
                "purpose": c.purpose.as_str(),
                "modeling_notes": c.modeling_notes.as_str(),
                "planned_size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                "attach_to": attach_to,
                "anchors": anchors,
                "contacts": contacts,
            })
        })
        .collect();

    let plan_state = serde_json::json!({
        "version": 1,
        "assembly_notes": assembly_notes.trim(),
        "rig_move_cycle_m": rig_move_cycle_m,
        "components": comps_json,
    });
    let text = serde_json::to_string(&plan_state).unwrap_or_else(|_| plan_state.to_string());
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("sha256:{hex}")
}

fn build_gen3d_scene_graph_summary(
    run_id: &str,
    attempt: u32,
    pass: u32,
    plan_hash: &str,
    assembly_rev: u32,
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    fn anchor_frame_json(
        anchors: &[crate::object::registry::AnchorDef],
        name: &str,
    ) -> Option<(serde_json::Value, Transform)> {
        if name == "origin" {
            return Some((
                serde_json::json!({
                    "pos": [0.0, 0.0, 0.0],
                    "forward": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                }),
                Transform::IDENTITY,
            ));
        }
        let anchor = anchors.iter().find(|a| a.name.as_ref() == name)?;
        let pos = anchor.transform.translation;
        let forward = anchor.transform.rotation * Vec3::Z;
        let up = anchor.transform.rotation * Vec3::Y;
        Some((
            serde_json::json!({
                "pos": [pos.x, pos.y, pos.z],
                "forward": [forward.x, forward.y, forward.z],
                "up": [up.x, up.y, up.z],
            }),
            anchor.transform,
        ))
    }

    let mut name_to_component: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for c in components.iter() {
        name_to_component.insert(c.name.as_str(), c);
    }

    let root = draft.root_def();
    let root_json = root.map(|root| {
        let collider = match root.collider {
            crate::object::registry::ColliderProfile::None => serde_json::json!({"kind":"none"}),
            crate::object::registry::ColliderProfile::CircleXZ { radius } => {
                serde_json::json!({"kind":"circle_xz","radius": radius})
            }
            crate::object::registry::ColliderProfile::AabbXZ { half_extents } => serde_json::json!({
                "kind":"aabb_xz",
                "half_extents":[half_extents.x, half_extents.y]
            }),
        };
        let mobility = root.mobility.as_ref().map(|m| {
            serde_json::json!({
                "kind": match m.mode {
                    crate::object::registry::MobilityMode::Ground => "ground",
                    crate::object::registry::MobilityMode::Air => "air",
                },
                "max_speed": m.max_speed,
            })
        });
        let attack = root.attack.as_ref().map(|a| {
            serde_json::json!({
                "kind": match a.kind {
                    crate::object::registry::UnitAttackKind::Melee => "melee",
                    crate::object::registry::UnitAttackKind::RangedProjectile => "ranged_projectile",
                },
                "cooldown_secs": a.cooldown_secs,
                "damage": a.damage,
                "anim_window_secs": a.anim_window_secs,
            })
        });
        let object_id_uuid = Uuid::from_u128(root.object_id).to_string();
        serde_json::json!({
            "object_id_uuid": object_id_uuid,
            "size": [root.size.x, root.size.y, root.size.z],
            "collider": collider,
            "mobility": mobility,
            "attack": attack,
        })
    });

    let components_json: Vec<serde_json::Value> = components
        .iter()
        .map(|c| {
            let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{}", c.name));
            let component_id_uuid = Uuid::from_u128(object_id).to_string();
            let forward = c.rot * Vec3::Z;
            let up = c.rot * Vec3::Y;
            let anchors: Vec<serde_json::Value> = c
                .anchors
                .iter()
                .map(|a| {
                    let pos = a.transform.translation;
                    let forward = a.transform.rotation * Vec3::Z;
                    let up = a.transform.rotation * Vec3::Y;
                    serde_json::json!({
                        "name": a.name.as_ref(),
                        "pos": [pos.x, pos.y, pos.z],
                        "forward": [forward.x, forward.y, forward.z],
                        "up": [up.x, up.y, up.z],
                    })
                })
                .collect();

            let attach_to = c.attach_to.as_ref().map(|att| {
                let parent_id =
                    builtin_object_id(&format!("gravimera/gen3d/component/{}", att.parent));
                let parent_component_id_uuid = Uuid::from_u128(parent_id).to_string();
                let parent_component = name_to_component.get(att.parent.as_str()).copied();
                let parent_anchor = parent_component
                    .and_then(|pc| anchor_frame_json(&pc.anchors, att.parent_anchor.as_str()));
                let child_anchor = anchor_frame_json(&c.anchors, att.child_anchor.as_str());
                let pos = att.offset.translation;
                let q = att.offset.rotation.normalize();
                let s = att.offset.scale;
                let forward = att.offset.rotation * Vec3::Z;
                let up = att.offset.rotation * Vec3::Y;
                let join_forward_world = parent_component
                    .zip(parent_anchor.as_ref())
                    .map(|(pc, (_, t))| pc.rot * (t.rotation * Vec3::Z));
                let join_up_world = parent_component
                    .zip(parent_anchor.as_ref())
                    .map(|(pc, (_, t))| pc.rot * (t.rotation * Vec3::Y));
                let join_right_world = join_up_world
                    .zip(join_forward_world)
                    .and_then(|(u, f)| {
                        let v = u.cross(f);
                        if !v.is_finite() || v.length_squared() <= 1e-6 {
                            None
                        } else {
                            Some(v.normalize())
                        }
                    });
                let joint = att.joint.as_ref().map(|joint| {
                    let mut json = serde_json::Map::new();
                    json.insert(
                        "kind".into(),
                        serde_json::Value::String(match joint.kind {
                            AiJointKindJson::Fixed => "fixed",
                            AiJointKindJson::Hinge => "hinge",
                            AiJointKindJson::Ball => "ball",
                            AiJointKindJson::Free => "free",
                            AiJointKindJson::Unknown => "unknown",
                        }
                        .to_string()),
                    );
                    if let Some(axis) = joint.axis_join {
                        json.insert(
                            "axis_join".into(),
                            serde_json::json!([axis[0], axis[1], axis[2]]),
                        );
                    }
                    if let Some(limits) = joint.limits_degrees {
                        json.insert(
                            "limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    if let Some(limits) = joint.swing_limits_degrees {
                        json.insert(
                            "swing_limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    if let Some(limits) = joint.twist_limits_degrees {
                        json.insert(
                            "twist_limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    serde_json::Value::Object(json)
                });
                let animations: Vec<serde_json::Value> = att
                    .animations
                    .iter()
                    .map(|slot| {
                        let spec = &slot.spec;
                        let clip = match &spec.clip {
                            crate::object::registry::PartAnimationDef::Loop {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"loop",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::Once {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"once",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::PingPong {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"ping_pong",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::Spin { axis, radians_per_unit } => {
                                serde_json::json!({
                                    "kind":"spin",
                                    "axis":[axis.x, axis.y, axis.z],
                                    "radians_per_unit": radians_per_unit,
                                })
                            }
                        };
                        serde_json::json!({
                            "channel": slot.channel.as_ref(),
                            "driver": match spec.driver {
                                crate::object::registry::PartAnimationDriver::Always => "always",
                                crate::object::registry::PartAnimationDriver::MovePhase => "move_phase",
                                crate::object::registry::PartAnimationDriver::MoveDistance => "move_distance",
                                crate::object::registry::PartAnimationDriver::AttackTime => "attack_time",
                            },
                            "speed_scale": spec.speed_scale,
                            "clip": clip,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "parent_component_id_uuid": parent_component_id_uuid,
                    "parent_component_name": att.parent.as_str(),
                    "parent_anchor": att.parent_anchor.as_str(),
                    "child_anchor": att.child_anchor.as_str(),
                    "parent_anchor_frame": parent_anchor.as_ref().map(|(json, _)| json.clone()),
                    "child_anchor_frame": child_anchor.as_ref().map(|(json, _)| json.clone()),
                    "join_forward_world": join_forward_world.map(|v| [v.x, v.y, v.z]),
                    "join_up_world": join_up_world.map(|v| [v.x, v.y, v.z]),
                    "join_right_world": join_right_world.map(|v| [v.x, v.y, v.z]),
                    "offset": {
                        "pos": [pos.x, pos.y, pos.z],
                        "forward": [forward.x, forward.y, forward.z],
                        "up": [up.x, up.y, up.z],
                        "rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                        "scale": [s.x, s.y, s.z],
                    },
                    "joint": joint,
                    "animations": animations,
                })
            });

            let geometry = draft.defs.iter().find(|d| d.object_id == object_id).map(|def| {
                let geometry_parts: Vec<&crate::object::registry::ObjectPartDef> = def
                    .parts
                    .iter()
                    .filter(|p| {
                        !(matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::ObjectRef { .. }
                        ) && p.attachment.is_some())
                    })
                    .collect();

                if geometry_parts.len() == 1 {
                    let part = geometry_parts[0];
                    if part.attachment.is_none() {
                        if let crate::object::registry::ObjectPartKind::ObjectRef { object_id } =
                            &part.kind
                        {
                            let source_id = *object_id;
                            let source_uuid = Uuid::from_u128(source_id).to_string();
                            let source_name = components
                                .iter()
                                .find(|cmp| {
                                    builtin_object_id(&format!(
                                        "gravimera/gen3d/component/{}",
                                        cmp.name
                                    )) == source_id
                                })
                                .map(|cmp| cmp.name.as_str());
                            return serde_json::json!({
                                "kind": "linked_copy",
                                "source_component_id_uuid": source_uuid,
                                "source_component_name": source_name,
                            });
                        }
                    }
                }

                let primitive_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::Primitive { .. }
                        )
                    })
                    .count();
                let model_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(&p.kind, crate::object::registry::ObjectPartKind::Model { .. })
                    })
                    .count();
                let object_ref_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::ObjectRef { .. }
                        )
                    })
                    .count();

                serde_json::json!({
                    "kind": if geometry_parts.is_empty() { "empty" } else { "geometry" },
                    "parts_total": def.parts.len(),
                    "geometry_parts": geometry_parts.len(),
                    "primitive_parts": primitive_parts,
                    "model_parts": model_parts,
                    "object_ref_parts": object_ref_parts,
                })
            });

            serde_json::json!({
                "component_id_uuid": component_id_uuid,
                "name": c.name.as_str(),
                "generated": c.actual_size.is_some(),
                "planned_size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                "actual_size": c.actual_size.map(|s| [s.x, s.y, s.z]),
                "resolved_transform": {
                    "pos": [c.pos.x, c.pos.y, c.pos.z],
                    "forward": [forward.x, forward.y, forward.z],
                    "up": [up.x, up.y, up.z],
                },
                "geometry": geometry,
                "anchors": anchors,
                "attach_to": attach_to,
            })
        })
        .collect();

    serde_json::json!({
        "version": 1,
        "run_id": run_id,
        "attempt": attempt,
        "pass": pass,
        "plan_hash": plan_hash,
        "assembly_rev": assembly_rev,
        "root": root_json,
        "components": components_json,
    })
}

fn build_gen3d_smoke_results(
    raw_prompt: &str,
    has_images: bool,
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    let prompt = raw_prompt.trim().to_ascii_lowercase();
    let attack_required = prompt.contains("can attack")
        || prompt.contains("attackable")
        || prompt.contains("weapon")
        || prompt.contains("gun")
        || prompt.contains("shoot")
        || prompt.contains("spear")
        || prompt.contains("axe")
        || prompt.contains("bow");

    let root = draft.root_def();
    let mobility_present = root.and_then(|r| r.mobility.as_ref()).is_some();
    let attack_present = root.and_then(|r| r.attack.as_ref()).is_some();

    let mut issues: Vec<serde_json::Value> = Vec::new();
    if attack_required && (!mobility_present || !attack_present) {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Prompt implies the object should be attack-capable, but the draft has no mobility/attack profile.",
        }));
    }
    if attack_present && !mobility_present {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Draft has an attack profile but is not movable (missing mobility).",
        }));
    }

    for c in components {
        let component_id = Uuid::from_u128(builtin_object_id(&format!(
            "gravimera/gen3d/component/{}",
            c.name
        )))
        .to_string();
        if !c.pos.is_finite() || !c.rot.is_finite() || !c.planned_size.is_finite() {
            issues.push(serde_json::json!({
                "severity":"error",
                "component_id": component_id.as_str(),
                "component": c.name.as_str(),
                "message":"Component has non-finite transform or size.",
            }));
        }
        if let Some(att) = c.attach_to.as_ref() {
            if !att.offset.translation.is_finite()
                || !att.offset.rotation.is_finite()
                || !att.offset.scale.is_finite()
            {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "component_id": component_id.as_str(),
                    "component": c.name.as_str(),
                    "message":"Attachment offset has non-finite values.",
                }));
            }

            // Animation sanity: for most attached spinning parts (wheels, propellers, turrets),
            // the spin axis should be the attachment direction (+Z in the join frame). If this is
            // wrong, the part will visibly spin around a strange axis.
            for slot in att.animations.iter() {
                let crate::object::registry::PartAnimationDef::Spin { axis, .. } = &slot.spec.clip
                else {
                    continue;
                };
                let axis = *axis;
                if !axis.is_finite() || axis.length_squared() <= 1e-6 {
                    continue;
                }
                let axis = axis.normalize();
                let align = axis.dot(Vec3::Z).abs();
                if align < 0.7 {
                    // Provide a robust suggestion: in component-local space, set the spin axis to
                    // the child anchor's forward vector (attachment direction).
                    let child_forward = if att.child_anchor == "origin" {
                        Vec3::Z
                    } else {
                        c.anchors
                            .iter()
                            .find(|a| a.name.as_ref() == att.child_anchor)
                            .map(|a| a.transform.rotation * Vec3::Z)
                            .unwrap_or(Vec3::Z)
                    };
                    issues.push(serde_json::json!({
                        "severity":"warn",
                        "component_id": component_id.as_str(),
                        "component": c.name.as_str(),
                        "channel": slot.channel.as_ref(),
                        "message":"Spin axis is not aligned with the attachment direction (+Z in the join frame). This often makes wheels/props/turrets spin around the wrong axis.",
                        "suggested_component_local_axis": [child_forward.x, child_forward.y, child_forward.z],
                    }));
                }
            }
        }
        for a in c.anchors.iter() {
            if !a.transform.translation.is_finite() || !a.transform.rotation.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "component_id": component_id.as_str(),
                    "component": c.name.as_str(),
                    "anchor": a.name.as_ref(),
                    "message":"Anchor has non-finite transform.",
                }));
            }
        }
    }

    let ok = issues
        .iter()
        .all(|i| i.get("severity").and_then(|v| v.as_str()) != Some("error"));

    let motion_report =
        motion_validation::build_motion_validation_report(rig_move_cycle_m, components);
    let motion_ok = motion_report
        .motion_validation
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let ok = ok && motion_ok;

    serde_json::json!({
        "version": 1,
        "has_images": has_images,
        "attack_required_by_prompt": attack_required,
        "mobility_present": mobility_present,
        "attack_present": attack_present,
        "components_total": components.len(),
        "components_generated": components.iter().filter(|c| c.actual_size.is_some()).count(),
        "draft_defs": draft.defs.len(),
        "rig_summary": motion_report.rig_summary,
        "motion_validation": motion_report.motion_validation,
        "issues": issues,
        "ok": ok,
    })
}

fn build_gen3d_validate_results(
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    use crate::object::registry::ObjectPartKind;

    let mut issues: Vec<serde_json::Value> = Vec::new();

    let root_id = super::gen3d_draft_object_id();
    let root_present = draft.defs.iter().any(|d| d.object_id == root_id);
    if !root_present {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Draft is missing the Gen3D root object def.",
        }));
    }

    let mut seen_ids: std::collections::HashSet<u128> = std::collections::HashSet::new();
    for def in &draft.defs {
        if !seen_ids.insert(def.object_id) {
            issues.push(serde_json::json!({
                "severity":"error",
                "object_id": format!("{:#x}", def.object_id),
                "message":"Duplicate object_id in draft defs.",
            }));
        }
        if !def.size.is_finite() || def.size.abs().max_element() <= 1e-6 {
            issues.push(serde_json::json!({
                "severity":"error",
                "object_id": format!("{:#x}", def.object_id),
                "label": def.label.as_ref(),
                "message":"ObjectDef.size is non-finite or near-zero.",
            }));
        }

        let mut anchor_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for a in &def.anchors {
            let name = a.name.as_ref().trim();
            if name.is_empty() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "message":"Anchor has empty name.",
                }));
                continue;
            }
            if !anchor_names.insert(name.to_string()) {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "anchor": name,
                    "message":"Duplicate anchor name on ObjectDef.",
                }));
            }
            if !a.transform.translation.is_finite() || !a.transform.rotation.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "anchor": name,
                    "message":"Anchor transform is non-finite.",
                }));
            }
        }
    }

    let defs_map: std::collections::HashMap<u128, &crate::object::registry::ObjectDef> =
        draft.defs.iter().map(|d| (d.object_id, d)).collect();
    for def in &draft.defs {
        let mut anchor_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for a in &def.anchors {
            anchor_names.insert(a.name.as_ref());
        }

        for (idx, part) in def.parts.iter().enumerate() {
            let t = part.transform;
            if !t.translation.is_finite() || !t.rotation.is_finite() || !t.scale.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "message":"Part transform has non-finite values.",
                }));
            }
            if t.scale.abs().max_element() <= 1e-6 {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "message":"Part scale is near-zero.",
                }));
            }

            let ObjectPartKind::ObjectRef {
                object_id: child_id,
            } = &part.kind
            else {
                continue;
            };
            let child_id = *child_id;
            let Some(child_def) = defs_map.get(&child_id).copied() else {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "missing_child_object_id": format!("{:#x}", child_id),
                    "message":"ObjectRef points at a missing object def.",
                }));
                continue;
            };

            if let Some(att) = part.attachment.as_ref() {
                let parent_anchor = att.parent_anchor.as_ref();
                let child_anchor = att.child_anchor.as_ref();
                if parent_anchor != "origin" && !anchor_names.contains(parent_anchor) {
                    issues.push(serde_json::json!({
                        "severity":"error",
                        "object_id": format!("{:#x}", def.object_id),
                        "label": def.label.as_ref(),
                        "part_index": idx,
                        "parent_anchor": parent_anchor,
                        "message":"Attachment references a missing parent_anchor.",
                    }));
                }
                if child_anchor != "origin"
                    && !child_def
                        .anchors
                        .iter()
                        .any(|a| a.name.as_ref() == child_anchor)
                {
                    issues.push(serde_json::json!({
                        "severity":"error",
                        "object_id": format!("{:#x}", def.object_id),
                        "label": def.label.as_ref(),
                        "part_index": idx,
                        "child_object_id": format!("{:#x}", child_id),
                        "child_anchor": child_anchor,
                        "message":"Attachment references a missing child_anchor.",
                    }));
                }
            }
        }
    }

    // Ensure planned components point at existing defs (useful for diagnosing plan/draft mismatches).
    for c in components {
        let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{}", c.name));
        if !defs_map.contains_key(&object_id) {
            issues.push(serde_json::json!({
                "severity":"warn",
                "component": c.name.as_str(),
                "message":"Planned component has no matching object def in draft.",
            }));
        }
    }

    let ok = issues
        .iter()
        .all(|i| i.get("severity").and_then(|v| v.as_str()) != Some("error"));

    serde_json::json!({
        "version": 1,
        "draft_defs": draft.defs.len(),
        "components_total": components.len(),
        "components_generated": components.iter().filter(|c| c.actual_size.is_some()).count(),
        "issues": issues,
        "ok": ok,
    })
}

fn spawn_gen3d_ai_text_thread(
    shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>>,
    progress: Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    openai: crate::config::OpenAiConfig,
    reasoning_effort: String,
    system_instructions: String,
    user_text: String,
    image_paths: Vec<PathBuf>,
    run_dir: PathBuf,
    prefix: String,
) {
    let url = crate::config::join_base_url(&openai.base_url, "responses");
    std::thread::spawn(move || {
        let thread_id = std::thread::current().id();
        let started_at = std::time::Instant::now();
        append_gen3d_run_log(
            Some(&run_dir),
            format!(
                "request_thread_started prefix={} model={} images={} url={} reasoning_effort={} thread={:?}",
                prefix,
                openai.model,
                image_paths.len(),
                url,
                reasoning_effort,
                thread_id
            ),
        );
        debug!(
            "Gen3D: request started (prefix={}, model={}, images={}, url={}, cache_dir={}, thread={:?})",
            prefix,
            openai.model,
            image_paths.len(),
            url,
            run_dir.display(),
            thread_id,
        );
        let result = openai::generate_text_via_openai(
            &progress,
            session,
            expected_schema,
            &openai.base_url,
            &openai.api_key,
            &openai.model,
            &reasoning_effort,
            &system_instructions,
            &user_text,
            &image_paths,
            Some(&run_dir),
            &prefix,
        );
        let openai_elapsed_ms = started_at.elapsed().as_millis();
        append_gen3d_run_log(
            Some(&run_dir),
            format!(
                "request_thread_openai_done prefix={} ok={} elapsed_ms={}",
                prefix,
                result.is_ok(),
                openai_elapsed_ms
            ),
        );
        debug!(
            "Gen3D: request thread OpenAI done (prefix={}, ok={}, elapsed_ms={}, thread={:?})",
            prefix,
            result.is_ok(),
            openai_elapsed_ms,
            thread_id,
        );

        let shared_lock_started_at = std::time::Instant::now();
        let mut guard = match shared.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                warn!(
                    "Gen3D: shared_result lock poisoned; continuing (prefix={}, thread={:?})",
                    prefix, thread_id
                );
                poisoned.into_inner()
            }
        };
        let shared_lock_wait_ms = shared_lock_started_at.elapsed().as_millis();
        append_gen3d_run_log(
            Some(&run_dir),
            format!(
                "request_thread_shared_lock_acquired prefix={} wait_ms={}",
                prefix, shared_lock_wait_ms
            ),
        );
        if shared_lock_wait_ms >= 1_000 {
            warn!(
                "Gen3D: shared_result lock wait high (prefix={}, wait_ms={}, thread={:?})",
                prefix, shared_lock_wait_ms, thread_id
            );
        } else {
            debug!(
                "Gen3D: shared_result lock acquired (prefix={}, wait_ms={}, thread={:?})",
                prefix, shared_lock_wait_ms, thread_id
            );
        }

        *guard = Some(result);
        append_gen3d_run_log(
            Some(&run_dir),
            format!("request_thread_shared_set prefix={}", prefix),
        );
    });
}

pub(super) fn spawn_prefab_descriptor_meta_enrichment_thread_best_effort(
    job: &Gen3dAiJob,
    descriptor_path: PathBuf,
    prefab_label: String,
    roles: Vec<String>,
    size_m: Vec3,
    ground_origin_y_m: f32,
    mobility: Option<String>,
    attack_kind: Option<String>,
    anchors: Vec<String>,
    animation_channels: Vec<String>,
    plan_extracted_text: Option<String>,
    motion_summary_json: Option<serde_json::Value>,
) {
    let Some(openai) = job.openai.clone() else {
        return;
    };

    let session = job.session.clone();
    let pass_dir = job.pass_dir.clone();
    let user_prompt = job.user_prompt_raw.clone();

    std::thread::spawn(move || {
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Generating prefab metadata…".into(),
        }));
        let system = prompts::build_gen3d_descriptor_meta_system_instructions();
        let user_text = prompts::build_gen3d_descriptor_meta_user_text(
            &prefab_label,
            &user_prompt,
            &roles,
            size_m,
            ground_origin_y_m,
            mobility.as_deref(),
            attack_kind.as_deref(),
            &anchors,
            &animation_channels,
            plan_extracted_text.as_deref(),
            motion_summary_json.as_ref(),
        );

        let reasoning_effort = openai::cap_reasoning_effort(&openai.model_reasoning_effort, "low");
        let resp = openai::generate_text_via_openai(
            &progress,
            session,
            Some(Gen3dAiJsonSchemaKind::DescriptorMetaV1),
            &openai.base_url,
            &openai.api_key,
            &openai.model,
            &reasoning_effort,
            &system,
            &user_text,
            &[],
            pass_dir.as_deref(),
            "descriptor_meta",
        );

        let meta = match resp {
            Ok(resp) => match parse::parse_ai_descriptor_meta_from_text(&resp.text) {
                Ok(meta) => meta,
                Err(err) => {
                    warn!("Gen3D: failed to parse descriptor-meta response: {err}");
                    return;
                }
            },
            Err(err) => {
                warn!("Gen3D: descriptor-meta request failed: {err}");
                return;
            }
        };

        let bytes = match std::fs::read(&descriptor_path) {
            Ok(b) => b,
            Err(err) => {
                warn!(
                    "Gen3D: descriptor-meta could not read {}: {err}",
                    descriptor_path.display()
                );
                return;
            }
        };
        let json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "Gen3D: descriptor-meta invalid JSON {}: {err}",
                    descriptor_path.display()
                );
                return;
            }
        };
        let mut doc: crate::prefab_descriptors::PrefabDescriptorFileV1 =
            match serde_json::from_value(json) {
                Ok(v) => v,
                Err(err) => {
                    warn!(
                        "Gen3D: descriptor-meta schema mismatch {}: {err}",
                        descriptor_path.display()
                    );
                    return;
                }
            };

        let mut should_update_short = true;
        if let Some(text) = doc.text.as_ref().and_then(|t| t.short.as_deref()) {
            if !text.trim().is_empty() {
                should_update_short = false;
                if let Some(prompt) = doc
                    .provenance
                    .as_ref()
                    .and_then(|p| p.gen3d.as_ref())
                    .and_then(|g| g.prompt.as_deref())
                {
                    if let Some(first_line) = prompt.lines().find(|l| !l.trim().is_empty()) {
                        if text.trim() == first_line.trim() {
                            should_update_short = true;
                        }
                    }
                }
            }
        }

        if should_update_short && !meta.short.trim().is_empty() {
            let text = doc.text.get_or_insert_with(Default::default);
            text.short = Some(meta.short.trim().to_string());
        }

        let mut merged_tags: Vec<String> = doc.tags;
        merged_tags.extend(meta.tags);
        doc.tags = merged_tags;

        if let Err(err) =
            crate::prefab_descriptors::save_prefab_descriptor_file(&descriptor_path, &doc)
        {
            warn!(
                "Gen3D: descriptor-meta failed to save {}: {err}",
                descriptor_path.display()
            );
        }
    });
}

fn set_progress(progress: &Arc<Mutex<Gen3dAiProgress>>, message: impl Into<String>) {
    if let Ok(mut guard) = progress.lock() {
        guard.message = message.into();
    }
}

fn truncate_for_ui(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_chars + 32);
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("…(truncated)");
    out
}

fn record_gen3d_tooling_feedback(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    history: &mut Gen3dToolFeedbackHistory,
    job: &Gen3dAiJob,
    feedbacks: &[AiToolingFeedbackJsonV1],
) {
    use bevy::log::{info, warn};

    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let run_id = job.run_id.map(|id| id.to_string());
    let attempt = Some(job.attempt);
    let pass = Some(job.pass);
    let run_dir = job.run_dir.as_deref();
    let pass_dir = job.pass_dir.as_deref();

    for feedback in feedbacks {
        let priority = feedback.priority.trim();
        let priority = if priority.is_empty() {
            "medium".to_string()
        } else {
            priority.to_string()
        };
        let title = feedback.title.trim();
        let title = if title.is_empty() {
            "Tooling feedback".to_string()
        } else {
            title.to_string()
        };
        let summary = feedback.summary.trim();
        let summary = if summary.is_empty() {
            "No summary provided.".to_string()
        } else {
            summary.to_string()
        };

        let mut evidence_paths: Vec<String> = Vec::new();
        if let Some(dir) = run_dir {
            evidence_paths.push(dir.display().to_string());
            evidence_paths.push(dir.join("tool_feedback.jsonl").display().to_string());
        }
        if let Some(dir) = pass_dir {
            evidence_paths.push(dir.display().to_string());
            evidence_paths.push(dir.join("gen3d_run.log").display().to_string());
            evidence_paths.push(dir.join("gravimera.log").display().to_string());
            evidence_paths.push(dir.join("review_*.png").display().to_string());
        }
        evidence_paths.push(
            super::tool_feedback::gen3d_tool_feedback_history_path(config)
                .display()
                .to_string(),
        );

        let raw = serde_json::to_value(feedback).unwrap_or(serde_json::Value::Null);

        let entry = Gen3dToolFeedbackEntry {
            version: 1,
            entry_id: Uuid::new_v4().to_string(),
            created_at_ms,
            run_id: run_id.clone(),
            attempt,
            pass,
            priority,
            title,
            summary,
            feedback: raw,
            evidence_paths,
        };

        let entry_priority = entry.priority.clone();
        let entry_title = entry.title.clone();
        let entry_summary = entry.summary.clone();
        let entry_id = entry.entry_id.clone();

        append_gen3d_tool_feedback_entry(config, run_dir, &entry);
        history.entries.push(entry);
        if matches!(workshop.side_tab, Gen3dSideTab::Status) {
            workshop.tool_feedback_unread = true;
        }

        // Codex-style developer breadcrumbs: surface tool feedback in terminal/logs.
        if let Some(pass_dir) = pass_dir {
            super::ai::artifacts::append_gen3d_run_log(
                Some(pass_dir),
                format!(
                    "tool_feedback_received priority={} title={:?} entry_id={} summary={:?}",
                    entry_priority,
                    entry_title.trim(),
                    entry_id,
                    entry_summary.trim()
                ),
            );
        } else if let Some(run_dir) = run_dir {
            // Best effort: if we don't have a pass_dir, at least write into the run root.
            super::ai::artifacts::append_gen3d_run_log(
                Some(run_dir),
                format!(
                    "tool_feedback_received priority={} title={:?} entry_id={} summary={:?}",
                    entry_priority,
                    entry_title.trim(),
                    entry_id,
                    entry_summary.trim()
                ),
            );
        }

        if entry_priority.trim().eq_ignore_ascii_case("high")
            || entry_priority.trim().eq_ignore_ascii_case("critical")
        {
            warn!(
                "Gen3D tooling feedback ({}) {}: {}",
                entry_priority,
                entry_title.trim(),
                entry_summary.trim()
            );
        } else {
            info!(
                "Gen3D tooling feedback ({}) {}: {}",
                entry_priority,
                entry_title.trim(),
                entry_summary.trim()
            );
        }
    }
}
