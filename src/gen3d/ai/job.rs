// Gen3D AI job state and shared types.
use bevy::prelude::*;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::object::registry::PartAnimationSlot;
use crate::threaded_result::SharedResult;

use super::ai_service::Gen3dAiServiceConfig;
use super::info_store::{Gen3dInfoStore, InfoEventKindV1};
use super::reuse_groups;
use super::schema::*;

pub(super) const GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS: u8 = 2;
pub(super) const GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES: u8 = 6;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Gen3dAiMode {
    #[default]
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dAgentLlmToolKind {
    GeneratePlan,
    GeneratePlanOps,
    GenerateComponent { component_idx: usize },
    GenerateComponentsBatch,
    GenerateMotionAuthoring,
    ReviewDelta,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPendingComponentBatch {
    pub(super) requested_indices: Vec<usize>,
    pub(super) optimized_by_reuse_groups: bool,
    pub(super) skipped_due_to_reuse_groups: Vec<usize>,
    pub(super) skipped_due_to_preserve_existing_components: Vec<usize>,
    pub(super) skipped_due_to_regen_budget: Vec<usize>,
    pub(super) completed_indices: std::collections::HashSet<usize>,
    pub(super) failed: Vec<Gen3dComponentBatchFailure>,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dComponentBatchFailure {
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) error: String,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dAgentWorkspace {
    pub(super) name: String,
    pub(super) defs: Vec<crate::object::registry::ObjectDef>,
    pub(super) planned_components: Vec<Gen3dPlannedComponent>,
    pub(super) plan_hash: String,
    pub(super) assembly_rev: u32,
    pub(super) assembly_notes: String,
    pub(super) plan_collider: Option<AiColliderJson>,
    pub(super) rig_move_cycle_m: Option<f32>,
    pub(super) motion_authoring: Option<AiMotionAuthoringJsonV1>,
    pub(super) reuse_groups: Vec<reuse_groups::Gen3dValidatedReuseGroup>,
    pub(super) reuse_group_warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dAgentSnapshot {
    pub(super) workspace_id: String,
    pub(super) label: String,
    pub(super) created_at_ms: u64,
    pub(super) defs: Vec<crate::object::registry::ObjectDef>,
    pub(super) planned_components: Vec<Gen3dPlannedComponent>,
    pub(super) plan_hash: String,
    pub(super) assembly_rev: u32,
    pub(super) assembly_notes: String,
    pub(super) plan_collider: Option<AiColliderJson>,
    pub(super) rig_move_cycle_m: Option<f32>,
    pub(super) motion_authoring: Option<AiMotionAuthoringJsonV1>,
    pub(super) reuse_groups: Vec<reuse_groups::Gen3dValidatedReuseGroup>,
    pub(super) reuse_group_warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub(super) enum Gen3dAgentAfterPassSnapshot {
    AdvancePassAndRequestStep,
    FinishRun {
        workshop_status: String,
        run_log: String,
        info_log: String,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPendingFinishRun {
    pub(super) workshop_status: String,
    pub(super) run_log: String,
    pub(super) info_log: String,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dDescriptorMetaCache {
    pub(super) plan_hash: String,
    pub(super) meta: AiDescriptorMetaJsonV1,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dInFlightDescriptorMeta {
    pub(super) run_id: Uuid,
    pub(super) plan_hash: String,
    pub(super) shared_result: SharedResult<Gen3dAiTextResponse, String>,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPendingPlanAttempt {
    pub(super) call_id: String,
    pub(super) error: String,
    pub(super) preserve_existing_components: bool,
    pub(super) preserve_edit_policy: Option<String>,
    pub(super) rewire_components: Vec<String>,
    pub(super) existing_component_names: Vec<String>,
    pub(super) existing_root_component: Option<String>,
    pub(super) plan: AiPlanJsonV1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen3dDescriptorMetaPolicy {
    Suggest,
    Preserve,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dAgentState {
    pub(super) step_actions: Vec<crate::gen3d::agent::Gen3dAgentActionJsonV1>,
    pub(super) step_action_idx: usize,
    pub(super) step_tool_results: Vec<crate::gen3d::agent::Gen3dToolResultJsonV1>,
    pub(super) step_repair_attempt: u8,
    pub(super) step_request_retry_attempt: u8,
    pub(super) no_progress_tries: u32,
    pub(super) no_progress_inspection_steps: u32,
    pub(super) last_state_hash: Option<String>,
    pub(super) step_had_observable_output: bool,
    pub(super) info_store_inspection_cache: std::collections::HashMap<String, serde_json::Value>,
    pub(super) tooling_feedback_submissions: u32,
    pub(super) rendered_since_last_review: bool,
    pub(super) ever_rendered: bool,
    pub(super) ever_reviewed: bool,
    pub(super) ever_validated: bool,
    pub(super) ever_smoke_checked: bool,
    pub(super) last_render_blob_ids: Vec<String>,
    pub(super) last_render_assembly_rev: Option<u32>,
    pub(super) active_workspace_id: String,
    pub(super) workspaces: std::collections::HashMap<String, Gen3dAgentWorkspace>,
    pub(super) next_workspace_seq: u32,
    pub(super) snapshots: std::collections::HashMap<String, Gen3dAgentSnapshot>,
    pub(super) next_snapshot_seq: u32,
    pub(super) pending_tool_call: Option<crate::gen3d::agent::Gen3dToolCallJsonV1>,
    pub(super) pending_llm_tool: Option<Gen3dAgentLlmToolKind>,
    pub(super) pending_review_delta_regen_allowed: Option<bool>,
    pub(super) pending_llm_repair_attempt: u8,
    pub(super) pending_component_batch: Option<Gen3dPendingComponentBatch>,
    pub(super) pending_render: Option<Gen3dReviewCaptureState>,
    pub(super) pending_render_include_motion_sheets: bool,
    pub(super) pending_pass_snapshot: Option<Gen3dReviewCaptureState>,
    pub(super) pending_after_pass_snapshot: Option<Gen3dAgentAfterPassSnapshot>,
    pub(super) last_validate_ok: Option<bool>,
    pub(super) last_smoke_ok: Option<bool>,
    pub(super) last_motion_ok: Option<bool>,
    pub(super) last_qa_warnings_count: Option<u32>,
    pub(super) last_qa_warning_example: Option<String>,
    pub(super) last_qa_basis_workspace_id: Option<String>,
    pub(super) last_qa_basis_state_hash: Option<String>,
    pub(super) last_qa_result_json: Option<serde_json::Value>,
    pub(super) pending_regen_component_indices: Vec<usize>,
    pub(super) pending_regen_component_indices_skipped_due_to_budget: Vec<usize>,
    pub(super) pending_regen_component_indices_blocked_due_to_qa_gate: Vec<usize>,
}

impl Default for Gen3dAgentState {
    fn default() -> Self {
        Self {
            step_actions: Vec::new(),
            step_action_idx: 0,
            step_tool_results: Vec::new(),
            step_repair_attempt: 0,
            step_request_retry_attempt: 0,
            no_progress_tries: 0,
            no_progress_inspection_steps: 0,
            last_state_hash: None,
            step_had_observable_output: false,
            info_store_inspection_cache: std::collections::HashMap::new(),
            tooling_feedback_submissions: 0,
            rendered_since_last_review: false,
            ever_rendered: false,
            ever_reviewed: false,
            ever_validated: false,
            ever_smoke_checked: false,
            last_render_blob_ids: Vec::new(),
            last_render_assembly_rev: None,
            active_workspace_id: "main".to_string(),
            workspaces: std::collections::HashMap::new(),
            next_workspace_seq: 1,
            snapshots: std::collections::HashMap::new(),
            next_snapshot_seq: 1,
            pending_tool_call: None,
            pending_llm_tool: None,
            pending_review_delta_regen_allowed: None,
            pending_llm_repair_attempt: 0,
            pending_component_batch: None,
            pending_render: None,
            pending_render_include_motion_sheets: true,
            pending_pass_snapshot: None,
            pending_after_pass_snapshot: None,
            last_validate_ok: None,
            last_smoke_ok: None,
            last_motion_ok: None,
            last_qa_warnings_count: None,
            last_qa_warning_example: None,
            last_qa_basis_workspace_id: None,
            last_qa_basis_state_hash: None,
            last_qa_result_json: None,
            pending_regen_component_indices: Vec::new(),
            pending_regen_component_indices_skipped_due_to_budget: Vec::new(),
            pending_regen_component_indices_blocked_due_to_qa_gate: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dToolCallInFlight {
    pub(super) call_id: String,
    pub(super) tool_id: String,
    pub(super) started_at: std::time::Instant,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPassMetrics {
    pub(super) pass: u32,
    pub(super) started_at: std::time::Instant,
    pub(super) ended_at: Option<std::time::Instant>,
    pub(super) agent_step_llm_ms_total: u128,
    pub(super) agent_step_llm_requests: u32,
    pub(super) tool_ms_total: u128,
    pub(super) tool_calls: u32,
    pub(super) tool_ms_by_id: std::collections::HashMap<String, u128>,
}

impl Gen3dPassMetrics {
    fn elapsed(&self, now: std::time::Instant) -> std::time::Duration {
        let end = self.ended_at.unwrap_or(now);
        end.duration_since(self.started_at)
    }
}

#[derive(Clone, Debug, Default)]
pub(super) struct Gen3dCopyMetrics {
    pub(super) auto_component_copies: u32,
    pub(super) auto_subtree_copies: u32,
    pub(super) auto_errors: u32,
    pub(super) manual_component_calls: u32,
    pub(super) manual_component_copies: u32,
    pub(super) manual_subtree_calls: u32,
    pub(super) manual_subtree_copies: u32,
    pub(super) manual_failures: u32,
    pub(super) last_error: Option<String>,
    pub(super) recent_outcomes: std::collections::VecDeque<String>,
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
            TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE,
            TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_MIRROR_COMPONENT,
            TOOL_ID_MIRROR_COMPONENT_SUBTREE,
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
pub(super) struct Gen3dRunMetrics {
    pub(super) passes: Vec<Gen3dPassMetrics>,
    pub(super) current_pass_idx: Option<usize>,
    pub(super) agent_step_request_started_at: Option<std::time::Instant>,
    pub(super) tool_call_in_flight: Option<Gen3dToolCallInFlight>,
    pub(super) copy: Gen3dCopyMetrics,
}

impl Gen3dRunMetrics {
    fn current_pass_mut(&mut self) -> Option<&mut Gen3dPassMetrics> {
        self.current_pass_idx
            .and_then(|idx| self.passes.get_mut(idx))
    }

    fn current_pass(&self) -> Option<&Gen3dPassMetrics> {
        self.current_pass_idx.and_then(|idx| self.passes.get(idx))
    }

    pub(super) fn note_pass_started(&mut self, pass: u32) {
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

    pub(super) fn note_agent_step_request_started(&mut self) {
        self.agent_step_request_started_at = Some(std::time::Instant::now());
    }

    pub(super) fn note_agent_step_response_received(&mut self) {
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

    pub(super) fn note_tool_call_started(&mut self, call_id: &str, tool_id: &str) {
        self.tool_call_in_flight = Some(Gen3dToolCallInFlight {
            call_id: call_id.to_string(),
            tool_id: tool_id.to_string(),
            started_at: std::time::Instant::now(),
        });
    }

    pub(super) fn note_tool_result(&mut self, result: &crate::gen3d::agent::Gen3dToolResultJsonV1) {
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
    pub(super) running: bool,
    pub(super) cancel_flag: Option<Arc<AtomicBool>>,
    pub(super) build_complete: bool,
    pub(super) mode: Gen3dAiMode,
    pub(super) phase: Gen3dAiPhase,
    pub(super) ai: Option<Gen3dAiServiceConfig>,
    pub(super) require_structured_outputs: bool,
    pub(super) run_id: Option<Uuid>,
    pub(super) attempt: u32,
    pub(super) pass: u32,
    pub(super) plan_hash: String,
    pub(super) preserve_existing_components_mode: bool,
    pub(super) assembly_rev: u32,
    pub(super) plan_attempt: u8,
    pub(super) max_parallel_components: usize,
    pub(super) review_kind: Gen3dAutoReviewKind,
    pub(super) review_appearance: bool,
    pub(super) review_component_idx: Option<usize>,
    pub(super) auto_refine_passes_remaining: u32,
    pub(super) auto_refine_passes_done: u32,
    pub(super) per_component_refine_passes_remaining: u32,
    pub(super) per_component_refine_passes_done: u32,
    pub(super) per_component_resume: Option<Gen3dComponentBatchResume>,
    pub(super) replan_attempts: u32,
    pub(super) regen_total: u32,
    pub(super) regen_per_component: Vec<u32>,
    pub(super) user_prompt_raw: String,
    pub(super) user_images: Vec<PathBuf>,
    pub(super) user_image_object_summary: Option<Gen3dUserImageObjectSummary>,
    pub(super) run_dir: Option<PathBuf>,
    pub(super) pass_dir: Option<PathBuf>,
    pub(super) info_store: Option<Gen3dInfoStore>,
    pub(super) log_sinks: Option<crate::app::Gen3dLogSinks>,
    pub(super) session: Gen3dAiSessionState,
    pub(super) planned_components: Vec<Gen3dPlannedComponent>,
    pub(super) assembly_notes: String,
    pub(super) plan_collider: Option<AiColliderJson>,
    pub(super) rig_move_cycle_m: Option<f32>,
    pub(super) motion_authoring: Option<AiMotionAuthoringJsonV1>,
    pub(super) descriptor_meta_cache: Option<Gen3dDescriptorMetaCache>,
    pub(super) descriptor_meta_in_flight: Option<Gen3dInFlightDescriptorMeta>,
    pub(super) seed_descriptor_meta: Option<AiDescriptorMetaJsonV1>,
    pub(super) descriptor_meta_override: Option<AiDescriptorMetaJsonV1>,
    pub(super) pending_finish_run: Option<Gen3dPendingFinishRun>,
    pub(super) reuse_groups: Vec<reuse_groups::Gen3dValidatedReuseGroup>,
    pub(super) reuse_group_warnings: Vec<String>,
    pub(super) pending_plan_attempt: Option<Gen3dPendingPlanAttempt>,
    pub(super) component_queue: Vec<usize>,
    pub(super) component_queue_pos: usize,
    pub(super) component_attempts: Vec<u8>,
    pub(super) component_in_flight: Vec<Gen3dInFlightComponent>,
    pub(super) generation_kind: Gen3dComponentGenerationKind,
    pub(super) review_capture: Option<Gen3dReviewCaptureState>,
    pub(super) review_static_paths: Vec<PathBuf>,
    pub(super) motion_capture: Option<Gen3dMotionCaptureState>,
    pub(super) capture_previews_only: bool,
    pub(super) last_review_inputs: Vec<PathBuf>,
    pub(super) last_review_user_text: String,
    pub(super) review_delta_repair_attempt: u8,
    pub(super) shared_result: Option<SharedResult<Gen3dAiTextResponse, String>>,
    pub(super) shared_progress: Option<Arc<Mutex<Gen3dAiProgress>>>,
    pub(super) run_started_at: Option<std::time::Instant>,
    pub(super) last_run_elapsed: Option<std::time::Duration>,
    pub(super) current_run_tokens: u64,
    pub(super) total_tokens: u64,
    pub(super) chat_fallbacks_this_run: u32,
    pub(super) agent: Gen3dAgentState,
    pub(super) save_seq: u32,
    pub(super) edit_base_prefab_id: Option<u128>,
    pub(super) save_overwrite_prefab_id: Option<u128>,
    pub(super) seed_target_entity: Option<Entity>,
    pub(super) metrics: Gen3dRunMetrics,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dUserImageObjectSummary {
    pub(super) text: String,
    pub(super) truncated: bool,
    pub(super) word_count: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dInFlightComponent {
    pub(super) idx: usize,
    pub(super) attempt: u8,
    pub(super) sent_images: bool,
    pub(super) shared_result: SharedResult<Gen3dAiTextResponse, String>,
    pub(super) _progress: Arc<Mutex<Gen3dAiProgress>>,
}

impl Gen3dAiJob {
    pub(crate) fn is_running(&self) -> bool {
        self.running
    }

    pub(crate) fn is_build_complete(&self) -> bool {
        self.build_complete
    }

    pub(crate) fn can_resume(&self) -> bool {
        !self.running
            && !self.build_complete
            && self.ai.is_some()
            && self.run_id.is_some()
            && self.run_dir.is_some()
            && self.pass_dir.is_some()
    }

    pub(crate) fn is_capturing_motion_sheets(&self) -> bool {
        self.motion_capture.is_some()
    }

    pub(crate) fn edit_base_prefab_id(&self) -> Option<u128> {
        self.edit_base_prefab_id
    }

    pub(crate) fn set_edit_base_prefab_id(&mut self, id: Option<u128>) {
        self.edit_base_prefab_id = id;
    }

    pub(crate) fn save_overwrite_prefab_id(&self) -> Option<u128> {
        self.save_overwrite_prefab_id
    }

    pub(crate) fn set_save_overwrite_prefab_id(&mut self, id: Option<u128>) {
        self.save_overwrite_prefab_id = id;
    }

    pub(crate) fn seed_target_entity(&self) -> Option<Entity> {
        self.seed_target_entity
    }

    pub(crate) fn set_seed_target_entity(&mut self, entity: Option<Entity>) {
        self.seed_target_entity = entity;
    }

    pub(crate) fn run_dir_path(&self) -> Option<&Path> {
        self.run_dir.as_deref()
    }

    pub(crate) fn pass_dir_path(&self) -> Option<&Path> {
        self.pass_dir.as_deref()
    }

    pub(super) fn ensure_info_store(&mut self) -> Result<&mut Gen3dInfoStore, String> {
        let run_dir = self
            .run_dir
            .as_deref()
            .ok_or_else(|| "No active Gen3D run (missing run_dir).".to_string())?;
        let needs_reload = self
            .info_store
            .as_ref()
            .map(|s| s.run_dir() != run_dir)
            .unwrap_or(true);
        if needs_reload {
            self.info_store = Some(Gen3dInfoStore::open_or_create(run_dir)?);
        }
        Ok(self
            .info_store
            .as_mut()
            .expect("ensure_info_store sets info_store"))
    }

    pub(super) fn append_info_event_best_effort(
        &mut self,
        kind: InfoEventKindV1,
        tool_id: Option<String>,
        call_id: Option<String>,
        message: String,
        data: serde_json::Value,
    ) {
        fn redact_run_dir_paths_in_string(
            s: &str,
            run_dir_display: &str,
            run_dir_slashes: &str,
        ) -> String {
            let mut out = s.to_string();
            if !run_dir_display.is_empty() {
                out = out.replace(run_dir_display, "<gen3d_run_dir>");
            }
            if run_dir_slashes != run_dir_display && !run_dir_slashes.is_empty() {
                out = out.replace(run_dir_slashes, "<gen3d_run_dir>");
            }
            out
        }

        fn redact_run_dir_paths_in_json(
            value: &mut serde_json::Value,
            run_dir_display: &str,
            run_dir_slashes: &str,
        ) {
            match value {
                serde_json::Value::String(s) => {
                    *s = redact_run_dir_paths_in_string(s, run_dir_display, run_dir_slashes);
                }
                serde_json::Value::Array(arr) => {
                    for v in arr {
                        redact_run_dir_paths_in_json(v, run_dir_display, run_dir_slashes);
                    }
                }
                serde_json::Value::Object(obj) => {
                    for v in obj.values_mut() {
                        redact_run_dir_paths_in_json(v, run_dir_display, run_dir_slashes);
                    }
                }
                _ => {}
            }
        }

        let attempt = self.attempt;
        let pass = self.pass;
        let assembly_rev = self.assembly_rev;
        let Ok(store) = self.ensure_info_store() else {
            return;
        };

        let run_dir_display = store.run_dir().display().to_string();
        let run_dir_slashes = run_dir_display.replace('\\', "/");
        let message = redact_run_dir_paths_in_string(
            message.as_str(),
            run_dir_display.as_str(),
            run_dir_slashes.as_str(),
        );
        let mut data = data;
        redact_run_dir_paths_in_json(
            &mut data,
            run_dir_display.as_str(),
            run_dir_slashes.as_str(),
        );

        let _ = store.append_event(
            attempt,
            pass,
            assembly_rev,
            kind,
            tool_id,
            call_id,
            message,
            data,
        );
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

    pub(crate) fn min_ground_contact_y_in_root(&self) -> Option<f32> {
        let mut min_y: Option<f32> = None;
        for comp in &self.planned_components {
            if !comp.pos.is_finite() || !comp.rot.is_finite() {
                continue;
            }

            for contact in &comp.contacts {
                if contact.kind != AiContactKindJson::Ground {
                    continue;
                }
                let anchor_name = contact.anchor.trim();
                let anchor_local = if anchor_name == "origin" {
                    Some(Vec3::ZERO)
                } else {
                    comp.anchors
                        .iter()
                        .find(|a| a.name.as_ref() == anchor_name)
                        .map(|a| a.transform.translation)
                };
                let Some(anchor_local) = anchor_local else {
                    continue;
                };

                let p = comp.pos + comp.rot * anchor_local;
                if !p.y.is_finite() {
                    continue;
                }
                min_y = Some(min_y.map_or(p.y, |prev| prev.min(p.y)));
            }
        }
        min_y
    }

    pub(crate) fn motion_authoring_for_current_draft(&self) -> Option<&AiMotionAuthoringJsonV1> {
        let authored = self.motion_authoring.as_ref()?;
        // `gen3d_edit_bundle_v1.json` persists motion metadata so Edit/Fork remains restart-safe.
        // A resumed session will typically have a NEW run_id (fresh cache dir), so treat
        // run_id/attempt as provenance-only and gate freshness on plan_hash+assembly_rev.
        (authored.applies_to.plan_hash.trim() == self.plan_hash.trim()
            && authored.applies_to.assembly_rev == self.assembly_rev)
            .then_some(authored)
    }

    pub(crate) fn descriptor_meta_for_current_draft(&self) -> Option<&AiDescriptorMetaJsonV1> {
        let cached = self.descriptor_meta_cache.as_ref()?;
        // Descriptor meta is a semantic "best effort" label/short/tags suggestion. It should stay
        // usable across assembly revisions within the same plan hash.
        (cached.plan_hash.trim() == self.plan_hash.trim()).then_some(&cached.meta)
    }

    pub(crate) fn descriptor_meta_for_save(
        &self,
    ) -> Option<(Gen3dDescriptorMetaPolicy, &AiDescriptorMetaJsonV1)> {
        if let Some(meta) = self.descriptor_meta_override.as_ref() {
            return Some((Gen3dDescriptorMetaPolicy::Preserve, meta));
        }

        // Seeded Edit/Fork sessions preserve existing `text.short` + `tags` by default, unless the
        // agent explicitly overrides them.
        if self.edit_base_prefab_id.is_some() {
            if let Some(meta) = self.seed_descriptor_meta.as_ref() {
                return Some((Gen3dDescriptorMetaPolicy::Preserve, meta));
            }
        }

        self.descriptor_meta_for_current_draft()
            .map(|meta| (Gen3dDescriptorMetaPolicy::Suggest, meta))
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
        let previous = self.last_run_elapsed.unwrap_or_default();
        if self.running {
            self.run_started_at
                .map(|start| previous.saturating_add(start.elapsed()))
                .or_else(|| self.last_run_elapsed)
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

    pub(super) fn artifact_dir(&self) -> Option<&Path> {
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

    pub(super) fn start_run_metrics(&mut self) {
        self.current_run_tokens = 0;
        self.chat_fallbacks_this_run = 0;
        self.run_started_at = Some(std::time::Instant::now());
        self.last_run_elapsed = None;
    }

    pub(super) fn resume_run_metrics(&mut self) {
        self.run_started_at = Some(std::time::Instant::now());
    }

    pub(super) fn finish_run_metrics(&mut self) {
        if let Some(start) = self.run_started_at.take() {
            let elapsed = start.elapsed();
            let total = self
                .last_run_elapsed
                .unwrap_or_default()
                .saturating_add(elapsed);
            self.last_run_elapsed = Some(total);
        }

        self.metrics.finish_current_pass();
        self.stop_gen3d_log_capture();
    }

    pub(super) fn add_tokens(&mut self, tokens: u64) {
        self.current_run_tokens = self.current_run_tokens.saturating_add(tokens);
        self.total_tokens = self.total_tokens.saturating_add(tokens);
    }

    fn stop_gen3d_log_capture(&mut self) {
        if let Some(sinks) = self.log_sinks.as_ref() {
            sinks.stop_gen3d_log();
        }
        self.log_sinks = None;
    }

    pub(super) fn note_api_used(&mut self, api: Gen3dAiApi) {
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
                TOOL_ID_GET_PLAN_TEMPLATE, TOOL_ID_GET_SCENE_GRAPH_SUMMARY,
                TOOL_ID_GET_STATE_SUMMARY, TOOL_ID_INSPECT_PLAN, TOOL_ID_LLM_GENERATE_COMPONENT,
                TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
                TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_MIRROR_COMPONENT,
                TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_RENDER_PREVIEW, TOOL_ID_SMOKE_CHECK,
                TOOL_ID_SUBMIT_TOOLING_FEEDBACK, TOOL_ID_VALIDATE,
            };

            match tool_id {
                TOOL_ID_LLM_GENERATE_PLAN => "Plan".into(),
                TOOL_ID_LLM_GENERATE_COMPONENT | TOOL_ID_LLM_GENERATE_COMPONENTS => {
                    "Generate".into()
                }
                TOOL_ID_LLM_GENERATE_MOTION_AUTHORING => "Motion".into(),
                TOOL_ID_LLM_REVIEW_DELTA => "Review".into(),
                TOOL_ID_RENDER_PREVIEW => "Render".into(),
                TOOL_ID_VALIDATE => "Validate".into(),
                TOOL_ID_SMOKE_CHECK => "Smoke".into(),
                TOOL_ID_COPY_COMPONENT
                | TOOL_ID_MIRROR_COMPONENT
                | TOOL_ID_COPY_COMPONENT_SUBTREE
                | TOOL_ID_MIRROR_COMPONENT_SUBTREE
                | TOOL_ID_DETACH_COMPONENT => "Copy".into(),
                TOOL_ID_GET_STATE_SUMMARY
                | TOOL_ID_GET_SCENE_GRAPH_SUMMARY
                | TOOL_ID_INSPECT_PLAN
                | TOOL_ID_GET_PLAN_TEMPLATE => "Inspect".into(),
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
pub(super) enum Gen3dAiApi {
    Responses,
    ChatCompletions,
    GeminiStreamGenerateContent,
    ClaudeMessages,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dComponentGenerationKind {
    Initial,
    Regenerate,
}

impl Default for Gen3dComponentGenerationKind {
    fn default() -> Self {
        Self::Initial
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dAutoReviewKind {
    EndOfRun,
    PerComponent,
}

impl Default for Gen3dAutoReviewKind {
    fn default() -> Self {
        Self::EndOfRun
    }
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dComponentBatchResume {
    pub(super) generation_kind: Gen3dComponentGenerationKind,
    pub(super) component_queue: Vec<usize>,
    pub(super) component_queue_pos: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dAiTextResponse {
    pub(super) text: String,
    pub(super) api: Gen3dAiApi,
    pub(super) session: Gen3dAiSessionState,
    pub(super) total_tokens: Option<u64>,
}

#[derive(Clone, Debug, Default)]
pub(super) struct Gen3dAiSessionState {
    pub(super) responses_supported: Option<bool>,
    pub(super) responses_continuation_supported: Option<bool>,
    pub(super) responses_background_supported: Option<bool>,
    pub(super) responses_previous_id: Option<String>,
    pub(super) responses_structured_outputs_supported: Option<bool>,
    pub(super) chat_structured_outputs_supported: Option<bool>,
    pub(super) chat_history: Vec<Gen3dChatHistoryMessage>,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dChatHistoryMessage {
    pub(super) role: String,
    pub(super) content: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dAiPhase {
    Idle,
    // Codex-style tool-driven agent loop.
    AgentWaitingUserImageSummary,
    AgentWaitingStep,
    AgentExecutingActions,
    AgentWaitingTool,
    AgentCapturingRender,
    AgentCapturingPassSnapshot,
    AgentWaitingDescriptorMeta,
    WaitingPlan,
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
pub(super) struct Gen3dAiProgress {
    pub(super) message: String,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPlannedComponent {
    pub(super) display_name: String,
    pub(super) name: String,
    pub(super) purpose: String,
    pub(super) modeling_notes: String,
    /// Current resolved transform of this component in the assembled root frame.
    pub(super) pos: Vec3,
    /// Current resolved transform of this component in the assembled root frame.
    pub(super) rot: Quat,
    pub(super) planned_size: Vec3,
    pub(super) actual_size: Option<Vec3>,
    pub(super) anchors: Vec<crate::object::registry::AnchorDef>,
    pub(super) contacts: Vec<AiContactJson>,
    pub(super) attach_to: Option<Gen3dPlannedAttachment>,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dPlannedAttachment {
    pub(super) parent: String,
    pub(super) parent_anchor: String,
    pub(super) child_anchor: String,
    pub(super) offset: Transform,
    pub(super) joint: Option<AiJointJson>,
    pub(super) animations: Vec<PartAnimationSlot>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Gen3dReviewView {
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
    pub(super) fn file_stem(self) -> &'static str {
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

    pub(super) fn orbit_angles(self, base_pitch: f32) -> (f32, f32) {
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
pub(super) struct Gen3dReviewCaptureProgress {
    pub(super) expected: usize,
    pub(super) completed: usize,
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dReviewCaptureState {
    pub(super) cameras: Vec<Entity>,
    pub(super) image_paths: Vec<PathBuf>,
    pub(super) progress: Arc<Mutex<Gen3dReviewCaptureProgress>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Gen3dMotionCaptureKind {
    Move,
    Attack,
}

impl Gen3dMotionCaptureKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Move => "move",
            Self::Attack => "attack",
        }
    }

    pub(super) fn sheet_filename(self) -> &'static str {
        match self {
            Self::Move => "move_sheet.png",
            Self::Attack => "attack_sheet.png",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct Gen3dMotionCaptureState {
    pub(super) kind: Gen3dMotionCaptureKind,
    pub(super) frame_idx: u8,
    pub(super) frame_capture: Option<Gen3dReviewCaptureState>,
    pub(super) frame_paths: Vec<PathBuf>,
}

impl Gen3dMotionCaptureState {
    pub(super) fn new() -> Self {
        Self {
            kind: Gen3dMotionCaptureKind::Move,
            frame_idx: 0,
            frame_capture: None,
            frame_paths: Vec::new(),
        }
    }
}
