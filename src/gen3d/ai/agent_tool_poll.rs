use bevy::log::{debug, warn};
use bevy::prelude::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::tools::TOOL_ID_LLM_GENERATE_PLAN;
use crate::gen3d::agent::{append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolResultJsonV1};
use crate::threaded_result::{new_shared_result, SharedResult};

use super::super::state::{Gen3dDraft, Gen3dPreview, Gen3dWorkshop};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_component_batch::poll_agent_component_batch;
use super::agent_regen_budget::{ensure_agent_regen_budget_len, regen_budget_allows};
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_images_from_args,
    select_review_preview_images,
};
use super::agent_utils::{note_observable_tool_result, sanitize_prefix, truncate_json_for_log};
use super::artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_assembly_snapshot,
    write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::parse;
use super::{
    fail_job, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase, Gen3dAiProgress,
    Gen3dAiTextResponse,
};
use super::{
    GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS, GEN3D_MAX_REQUEST_IMAGES,
    GEN3D_PREVIEW_DEFAULT_PITCH, GEN3D_PREVIEW_DEFAULT_YAW,
};

#[derive(Debug, Default)]
struct ReviewDeltaRegenBuckets {
    allowed: Vec<usize>,
    skipped_due_to_budget: Vec<usize>,
    blocked_due_to_qa_gate: Vec<usize>,
}

fn bucket_review_delta_regen_requests(
    config: &AppConfig,
    job: &mut Gen3dAiJob,
    requested_indices: &[usize],
) -> ReviewDeltaRegenBuckets {
    let mut buckets = ReviewDeltaRegenBuckets::default();
    if requested_indices.is_empty() {
        return buckets;
    }

    ensure_agent_regen_budget_len(job);
    let preserve_mode = job.preserve_existing_components_mode;
    let qa_gate_open =
        job.agent.last_validate_ok == Some(false) || job.agent.last_smoke_ok == Some(false);

    let mut seen = std::collections::HashSet::<usize>::new();
    for idx in requested_indices.iter().copied() {
        if idx >= job.planned_components.len() {
            continue;
        }
        if !seen.insert(idx) {
            continue;
        }
        let is_regen = job
            .planned_components
            .get(idx)
            .map(|c| c.actual_size.is_some())
            .unwrap_or(false);
        if preserve_mode && is_regen && !qa_gate_open {
            buckets.blocked_due_to_qa_gate.push(idx);
            continue;
        }
        if is_regen && !regen_budget_allows(config, job, idx) {
            buckets.skipped_due_to_budget.push(idx);
            continue;
        }
        buckets.allowed.push(idx);
    }

    buckets.allowed.sort_unstable();
    buckets.skipped_due_to_budget.sort_unstable();
    buckets.blocked_due_to_qa_gate.sort_unstable();
    buckets
}

pub(super) fn poll_agent_tool(
    config: &AppConfig,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    preview: &mut Gen3dPreview,
) {
    if matches!(
        job.agent.pending_llm_tool,
        Some(super::Gen3dAgentLlmToolKind::GenerateComponentsBatch)
    ) {
        if let Some(tool_result) =
            poll_agent_component_batch(config, workshop, job, draft, workshop.speed_mode)
        {
            job.metrics.note_tool_result(&tool_result);
            append_agent_trace_event_v1(
                job.run_dir.as_deref(),
                &AgentTraceEventV1::ToolResult {
                    call_id: tool_result.call_id.clone(),
                    tool_id: tool_result.tool_id.clone(),
                    ok: tool_result.ok,
                    result: tool_result.result.clone(),
                    error: tool_result.error.clone(),
                },
            );
            append_gen3d_jsonl_artifact(
                job.pass_dir.as_deref(),
                "tool_results.jsonl",
                &serde_json::to_value(&tool_result).unwrap_or(serde_json::Value::Null),
            );
            append_gen3d_run_log(
                job.pass_dir.as_deref(),
                format!(
                    "tool_call_result call_id={} tool_id={} ok={} {}",
                    tool_result.call_id,
                    tool_result.tool_id,
                    tool_result.ok,
                    if tool_result.ok {
                        tool_result
                            .result
                            .as_ref()
                            .map(|v| format!("result={}", truncate_json_for_log(v, 900)))
                            .unwrap_or_else(|| "result=<none>".into())
                    } else {
                        format!("error={}", tool_result.error.as_deref().unwrap_or("<none>"))
                    }
                ),
            );
            if tool_result.ok {
                debug!(
                    "Gen3D tool call ok: call_id={} tool_id={}",
                    tool_result.call_id, tool_result.tool_id
                );
            } else {
                warn!(
                    "Gen3D tool call failed: call_id={} tool_id={} error={}",
                    tool_result.call_id,
                    tool_result.tool_id,
                    tool_result.error.as_deref().unwrap_or("<none>")
                );
            }
            note_observable_tool_result(job, &tool_result);
            job.agent.step_tool_results.push(tool_result);

            job.phase = Gen3dAiPhase::AgentExecutingActions;
        }
        return;
    }

    let Some(shared) = job.shared_result.as_ref() else {
        return;
    };
    let result = shared.lock().ok().and_then(|mut g| g.take());
    let Some(result) = result else {
        return;
    };
    job.shared_result = None;

    let Some(call) = job.agent.pending_tool_call.take() else {
        fail_job(workshop, job, "Internal error: missing pending tool call");
        return;
    };
    let Some(kind) = job.agent.pending_llm_tool.take() else {
        fail_job(workshop, job, "Internal error: missing pending tool kind");
        return;
    };

    append_gen3d_run_log(
        job.pass_dir.as_deref(),
        format!(
            "shared_result_taken tool_id={} call_id={} kind={kind:?}",
            call.tool_id, call.call_id
        ),
    );
    debug!(
        "Gen3D: shared result taken (tool_id={}, call_id={}, kind={kind:?})",
        call.tool_id, call.call_id
    );

    let mut stop_best_effort_after_tool: Option<String> = None;

    fn schedule_llm_tool_schema_repair(
        job: &mut Gen3dAiJob,
        workshop: &mut Gen3dWorkshop,
        call: &crate::gen3d::agent::Gen3dToolCallJsonV1,
        kind: super::Gen3dAgentLlmToolKind,
        ai: super::ai_service::Gen3dAiServiceConfig,
        reasoning_effort_cap: &str,
        pass_dir: PathBuf,
        system: String,
        base_user_text: String,
        images_to_send: Vec<PathBuf>,
        err: &str,
        _previous_output: &str,
        prefix_base: &str,
    ) -> bool {
        if job.agent.pending_llm_repair_attempt >= GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS {
            return false;
        }
        job.agent.pending_llm_repair_attempt =
            job.agent.pending_llm_repair_attempt.saturating_add(1);
        let attempt = job.agent.pending_llm_repair_attempt;

        let mut user_text = base_user_text;
        user_text.push_str("\n\nREPAIR REQUEST:\n");
        user_text.push_str(
            "Your previous output could not be parsed/applied by the engine.\n\
	Return ONLY a single JSON object that matches the schema exactly.\n\
	Do not include markdown or extra commentary.\n",
        );
        user_text.push_str(&format!("Error: {}\n", err.trim()));
        user_text.push_str(
            "IMPORTANT: Your previous output may contain INVALID field names.\n\
             Do NOT copy/paste keys from it. Use ONLY the schema-defined keys.\n\
             If you want to reuse values (numbers/strings), retype them under the correct keys.\n\
             (The raw previous output is omitted here to avoid repeating invalid keys.)\n",
        );

        let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
        job.shared_result = Some(shared.clone());
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: format!("Repairing tool output (attempt {attempt})…"),
        }));
        job.shared_progress = Some(progress.clone());
        set_progress(
            &progress,
            format!(
                "Repairing tool output… ({attempt}/{GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS})"
            ),
        );

        let prefix = sanitize_prefix(&format!("{prefix_base}_repair{attempt}"));
        append_agent_trace_event_v1(
            job.run_dir.as_deref(),
            &AgentTraceEventV1::Info {
                message: format!(
                    "Gen3D: repairing tool output (tool_id={}, call_id={}, attempt={}/{})",
                    call.tool_id, call.call_id, attempt, GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS
                ),
            },
        );
        append_gen3d_run_log(
            Some(&pass_dir),
            format!(
                "tool_schema_repair_start tool_id={} call_id={} attempt={}/{} err={}",
                call.tool_id,
                call.call_id,
                attempt,
                GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS,
                super::truncate_for_ui(err, 240)
            ),
        );

        let reasoning_effort =
            super::openai::cap_reasoning_effort(ai.model_reasoning_effort(), reasoning_effort_cap);
        let expected_schema = match kind {
            super::Gen3dAgentLlmToolKind::GeneratePlan => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanV1)
            }
            super::Gen3dAgentLlmToolKind::GenerateComponent { .. }
            | super::Gen3dAgentLlmToolKind::GenerateComponentsBatch => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1)
            }
            super::Gen3dAgentLlmToolKind::GenerateMotionAuthoring => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::MotionAuthoringV1)
            }
            super::Gen3dAgentLlmToolKind::ReviewDelta => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ReviewDeltaV1)
            }
        };
        spawn_gen3d_ai_text_thread(
            shared,
            progress,
            job.cancel_flag.clone(),
            job.session.clone(),
            expected_schema,
            job.require_structured_outputs,
            ai,
            reasoning_effort,
            system,
            user_text,
            images_to_send,
            pass_dir,
            prefix,
        );

        job.agent.pending_tool_call = Some(call.clone());
        job.agent.pending_llm_tool = Some(kind);
        job.phase = Gen3dAiPhase::AgentWaitingTool;
        workshop.status = format!(
            "Repairing tool output… ({attempt}/{GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS})"
        );
        true
    }

    let tool_result = match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }

            match kind {
                super::Gen3dAgentLlmToolKind::GeneratePlan => {
                    let text = resp.text;
                    let preserve_existing_components = call
                        .args
                        .get("constraints")
                        .and_then(|v| v.get("preserve_existing_components"))
                        .and_then(|v| v.as_bool())
                        .unwrap_or(job.preserve_existing_components_mode);
                    let preserve_edit_policy_raw = call
                        .args
                        .get("constraints")
                        .and_then(|v| v.get("preserve_edit_policy"))
                        .and_then(|v| v.as_str());
                    let preserve_edit_policy = super::preserve_plan_policy::parse_preserve_edit_policy(
                        preserve_edit_policy_raw,
                    );
                    let rewire_components: Vec<String> = call
                        .args
                        .get("constraints")
                        .and_then(|v| v.get("rewire_components"))
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    match parse::parse_ai_plan_from_text(&text) {
                        Ok(plan) => {
                            let plan_reuse_groups = plan.reuse_groups.clone();
                            match super::convert::ai_plan_to_initial_draft_defs(plan.clone()) {
                                Ok((planned, notes, defs)) => {
                                    let old_components = job.planned_components.clone();
                                    let old_root_name = old_components
                                        .iter()
                                        .find(|c| c.attach_to.is_none())
                                        .map(|c| c.name.clone());

                                    let rig_move_cycle_m = plan
                                        .rig
                                        .as_ref()
                                        .and_then(|r| r.move_cycle_m)
                                        .filter(|v| v.is_finite())
                                        .map(f32::abs)
                                        .filter(|v| *v > 1e-3);
                                    let plan_collider = plan.collider.clone();

                                    let (validated, warnings) =
                                        super::reuse_groups::validate_reuse_groups(
                                            &plan_reuse_groups,
                                            &planned,
                                        );

                                    let can_preserve_geometry = preserve_existing_components
                                        && !old_components.is_empty()
                                        && draft.total_non_projectile_primitive_parts() > 0;

                                    let preserve_error = if can_preserve_geometry {
                                        if preserve_edit_policy.is_none() {
                                            let raw = preserve_edit_policy_raw
                                                .unwrap_or("<none>")
                                                .trim();
                                            Some(format!(
                                                "Invalid constraints.preserve_edit_policy={raw:?}. Expected one of: \"additive\", \"allow_offsets\", \"allow_rewire\"."
                                            ))
                                        } else {
                                            let preserve_edit_policy = preserve_edit_policy
                                                .unwrap_or(super::preserve_plan_policy::PreserveEditPolicy::Additive);

                                            let old_names: std::collections::HashSet<String> =
                                                old_components
                                                    .iter()
                                                    .map(|c| c.name.clone())
                                                    .collect();
                                            let new_names: std::collections::HashSet<String> = planned
                                                .iter()
                                                .map(|c| c.name.clone())
                                                .collect();
                                            let mut missing: Vec<String> = old_names
                                                .difference(&new_names)
                                                .cloned()
                                                .collect::<Vec<_>>();
                                            missing.sort();
                                            if !missing.is_empty() {
                                                Some(format!(
                                                    "llm_generate_plan_v1 preserve_existing_components=true requires the plan to include ALL existing component names. Missing: {missing:?}"
                                                ))
                                            } else {
                                                let root_error = if let Some(old_root_name) =
                                                    old_root_name.as_ref()
                                                {
                                                    let new_root_name = planned
                                                        .iter()
                                                        .find(|c| c.attach_to.is_none())
                                                        .map(|c| c.name.as_str())
                                                        .unwrap_or("");
                                                    (new_root_name != old_root_name.as_str())
                                                        .then(|| {
                                                            format!(
                                                                "llm_generate_plan_v1 preserve_existing_components=true must keep the same root component. Old root=`{}`, new root=`{}`",
                                                                old_root_name, new_root_name
                                                            )
                                                        })
                                                } else {
                                                    None
                                                };
                                                if let Some(err) = root_error {
                                                    Some(err)
                                                } else {
                                                    let violations = super::preserve_plan_policy::validate_preserve_mode_plan_diff(
                                                        &old_components,
                                                        &planned,
                                                        preserve_edit_policy,
                                                        &rewire_components,
                                                    );
                                                    if violations.is_empty() {
                                                        None
                                                    } else {
                                                        let mut lines: Vec<String> = Vec::new();
                                                        lines.push(format!(
                                                            "llm_generate_plan_v1 preserve_existing_components=true edit_policy={} rejected plan diff:",
                                                            preserve_edit_policy.as_str()
                                                        ));
                                                        for v in violations.iter().take(24) {
                                                            lines.push(format!(
                                                                "- component={} kind={:?} field={} old={} new={}",
                                                                v.component, v.kind, v.field, v.old, v.new
                                                            ));
                                                        }
                                                        if violations.len() > 24 {
                                                            lines.push(format!(
                                                                "- … ({} more)",
                                                                violations.len().saturating_sub(24)
                                                            ));
                                                        }
                                                        lines.push(
                                                            "Hint: Use `apply_draft_ops_v1` to adjust offsets/parts, or re-run `llm_generate_plan_v1` with a broader preserve_edit_policy (and explicit rewire_components for allow_rewire), or disable preserve mode for a full rebuild.".into(),
                                                        );
                                                        Some(lines.join("\n"))
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        None
                                    };

                                    if let Some(err) = preserve_error {
                                        let mut existing_component_names: Vec<String> =
                                            old_components
                                                .iter()
                                                .map(|c| c.name.clone())
                                                .collect();
                                        existing_component_names.sort();
                                        existing_component_names.dedup();

                                        job.pending_plan_attempt =
                                            Some(super::Gen3dPendingPlanAttempt {
                                                call_id: call.call_id.clone(),
                                                error: err.clone(),
                                                preserve_existing_components,
                                                preserve_edit_policy: preserve_edit_policy_raw
                                                    .map(|s| s.trim().to_string())
                                                    .filter(|s| !s.is_empty()),
                                                rewire_components: rewire_components.clone(),
                                                existing_component_names,
                                                existing_root_component: old_root_name.clone(),
                                                plan,
                                            });

                                        Gen3dToolResultJsonV1::err(
                                            call.call_id.clone(),
                                            call.tool_id.clone(),
                                            format!("{err}\nHint: Run `inspect_plan_v1` for computed preserve-mode constraints (allowed names/root/policy) before replanning."),
                                        )
                                    } else {
                                        job.pending_plan_attempt = None;
                                        let mut planned_components = planned;
                                        let mut apply_err: Option<String> = None;
                                        if can_preserve_geometry {
                                            // Preserve existing component generation status and motion metadata.
                                            let mut old_by_name: std::collections::HashMap<
                                                &str,
                                                &super::Gen3dPlannedComponent,
                                            > = std::collections::HashMap::new();
                                            let mut old_component_ids: std::collections::HashSet<u128> =
                                                std::collections::HashSet::new();
                                            for comp in old_components.iter() {
                                                old_by_name.insert(comp.name.as_str(), comp);
                                                old_component_ids.insert(
                                                    crate::object::registry::builtin_object_id(
                                                        &format!(
                                                            "gravimera/gen3d/component/{}",
                                                            comp.name
                                                        ),
                                                    ),
                                                );
                                            }
                                            for comp in planned_components.iter_mut() {
                                                let Some(old) = old_by_name.get(comp.name.as_str())
                                                else {
                                                    continue;
                                                };
                                                comp.actual_size = old.actual_size;
                                                comp.contacts = old.contacts.clone();

                                                // Preserve anchor frames for existing anchors; allow the plan to add
                                                // new anchors without shifting existing attachments.
                                                let mut merged_anchors = old.anchors.clone();
                                                let mut seen_anchor_names: std::collections::HashSet<
                                                    String,
                                                > = merged_anchors
                                                    .iter()
                                                    .map(|a| a.name.as_ref().to_string())
                                                    .collect();
                                                for a in comp.anchors.iter() {
                                                    if seen_anchor_names
                                                        .insert(a.name.as_ref().to_string())
                                                    {
                                                        merged_anchors.push(a.clone());
                                                    }
                                                }
                                                comp.anchors = merged_anchors;

                                                if let (Some(new_att), Some(old_att)) =
                                                    (comp.attach_to.as_mut(), old.attach_to.as_ref())
                                                {
                                                    let same_interface = new_att.parent.trim()
                                                        == old_att.parent.trim()
                                                        && new_att.parent_anchor.trim()
                                                            == old_att.parent_anchor.trim()
                                                        && new_att.child_anchor.trim()
                                                            == old_att.child_anchor.trim();
                                                    if same_interface {
                                                        new_att.animations =
                                                            old_att.animations.clone();
                                                    }
                                                }
                                            }

                                            // Preserve existing geometry: merge plan defs into the draft without
                                            // overwriting primitive/model parts.
                                            let mut idx_by_id: std::collections::HashMap<u128, usize> =
                                                std::collections::HashMap::new();
                                            for (idx, def) in draft.defs.iter().enumerate() {
                                                idx_by_id.insert(def.object_id, idx);
                                            }
                                            for next in defs {
                                                if let Some(idx) = idx_by_id.get(&next.object_id).copied()
                                                {
                                                    let def = &mut draft.defs[idx];
                                                    let preserve_size_and_anchors =
                                                        old_component_ids.contains(&def.object_id)
                                                            && def.parts.iter().any(|p| {
                                                                matches!(
                                                                    p.kind,
                                                                    crate::object::registry::ObjectPartKind::Primitive { .. }
                                                                        | crate::object::registry::ObjectPartKind::Model { .. }
                                                                )
                                                            });

                                                    let parts = std::mem::take(&mut def.parts);
                                                    let old_size = def.size;
                                                    let old_anchors = std::mem::take(&mut def.anchors);
                                                    let next_size = next.size;
                                                    let next_anchors = next.anchors;

                                                    def.label = next.label;
                                                    def.ground_origin_y = next.ground_origin_y;
                                                    def.collider = next.collider;
                                                    def.interaction = next.interaction;
                                                    def.aim = next.aim;
                                                    def.mobility = next.mobility;
                                                    def.minimap_color = next.minimap_color;
                                                    def.health_bar_offset_y = next.health_bar_offset_y;
                                                    def.enemy = next.enemy;
                                                    def.muzzle = next.muzzle;
                                                    def.projectile = next.projectile;
                                                    def.attack = next.attack;

                                                    if preserve_size_and_anchors {
                                                        def.size = old_size;
                                                        let mut merged_anchors = old_anchors;
                                                        let mut seen_anchor_names: std::collections::HashSet<
                                                            String,
                                                        > = merged_anchors
                                                            .iter()
                                                            .map(|a| a.name.as_ref().to_string())
                                                            .collect();
                                                        for a in next_anchors.iter() {
                                                            if seen_anchor_names.insert(
                                                                a.name.as_ref().to_string(),
                                                            ) {
                                                                merged_anchors.push(a.clone());
                                                            }
                                                        }
                                                        def.anchors = merged_anchors;
                                                    } else {
                                                        def.size = next_size;
                                                        def.anchors = next_anchors;
                                                    }

                                                    def.parts = parts;
                                                } else {
                                                    draft.defs.push(next);
                                                }
                                            }

                                        job.planned_components = planned_components;
                                        job.assembly_notes = notes;
                                        job.rig_move_cycle_m = rig_move_cycle_m;
                                        job.plan_collider = plan_collider;
                                        job.reuse_groups = validated;
                                        job.reuse_group_warnings = warnings;
                                        job.plan_hash = super::compute_gen3d_plan_hash(
                                            &job.assembly_notes,
                                            job.rig_move_cycle_m,
                                            &job.planned_components,
                                        );

                                        job.component_queue.clear();
                                        job.component_queue_pos = 0;
                                        job.component_attempts
                                            .resize(job.planned_components.len(), 0);
                                        job.component_in_flight.clear();
                                        ensure_agent_regen_budget_len(job);

                                        if let Err(err) = super::convert::sync_attachment_tree_to_defs(
                                            &job.planned_components,
                                            draft,
                                        ) {
                                            apply_err = Some(format!(
                                                "Failed to sync attachments after plan merge: {err}"
                                            ));
                                        } else {
                                            super::convert::update_root_def_from_planned_components(
                                                &job.planned_components,
                                                &job.plan_collider,
                                                draft,
                                            );
                                            write_gen3d_assembly_snapshot(
                                                job.pass_dir.as_deref(),
                                                &job.planned_components,
                                            );
                                            job.assembly_rev = job.assembly_rev.saturating_add(1);
                                        }
                                    } else {
                                        job.planned_components = planned_components;
                                        job.assembly_notes = notes;
                                        job.rig_move_cycle_m = rig_move_cycle_m;
                                        job.plan_collider = plan_collider;
                                        job.reuse_groups = validated;
                                        job.reuse_group_warnings = warnings;
                                        job.plan_hash = super::compute_gen3d_plan_hash(
                                            &job.assembly_notes,
                                            job.rig_move_cycle_m,
                                            &job.planned_components,
                                        );
                                        job.assembly_rev = 0;
                                        draft.defs = defs;
                                    }

                                    job.preserve_existing_components_mode =
                                        preserve_existing_components;
                                    job.agent.workspaces.clear();
                                    job.agent.active_workspace_id = "main".to_string();
                                    job.agent.next_workspace_seq = 1;
                                    job.agent.rendered_since_last_review = false;
                                    job.agent.last_render_images.clear();
                                    job.agent.last_render_assembly_rev = None;
                                    job.agent.pending_regen_component_indices.clear();
                                    job.agent
                                        .pending_regen_component_indices_skipped_due_to_budget
                                        .clear();
                                    job.agent
                                        .pending_regen_component_indices_blocked_due_to_qa_gate
                                        .clear();
                                    job.agent.pending_llm_repair_attempt = 0;

                                    if let Some(def) = draft.root_def() {
                                        let max_dim =
                                            def.size.x.max(def.size.y).max(def.size.z).max(0.5);
                                        preview.distance = (max_dim * 2.8 + 0.8).clamp(2.0, 250.0);
                                        preview.pitch = GEN3D_PREVIEW_DEFAULT_PITCH;
                                        preview.yaw = GEN3D_PREVIEW_DEFAULT_YAW;
                                        preview.last_cursor = None;
                                    }

                                    if let Some(dir) = job.pass_dir.as_deref() {
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
                                            Some(dir),
                                            "plan_extracted.json",
                                            &extracted,
                                        );
                                        write_gen3d_assembly_snapshot(
                                            Some(dir),
                                            &job.planned_components,
                                        );
                                        write_gen3d_text_artifact(
                                            Some(dir),
                                            "plan_raw.txt",
                                            text.trim(),
                                        );
                                    }

                                    if let Some(err) = apply_err {
                                        Gen3dToolResultJsonV1::err(
                                            call.call_id.clone(),
                                            call.tool_id.clone(),
                                            err,
                                        )
                                    } else {
                                        Gen3dToolResultJsonV1::ok(
                                            call.call_id.clone(),
                                            call.tool_id.clone(),
                                            serde_json::json!({
                                                "ok": true,
                                                "components_total": job.planned_components.len(),
                                                "plan_hash": job.plan_hash,
                                            }),
                                        )
                                    }
                                    }
                                }
                                Err(err) => {
                                    let mut existing_component_names: Vec<String> = job
                                        .planned_components
                                        .iter()
                                        .map(|c| c.name.clone())
                                        .collect();
                                    existing_component_names.sort();
                                    existing_component_names.dedup();
                                    let existing_root_component = job
                                        .planned_components
                                        .iter()
                                        .find(|c| c.attach_to.is_none())
                                        .map(|c| c.name.clone());

                                    job.pending_plan_attempt =
                                        Some(super::Gen3dPendingPlanAttempt {
                                            call_id: call.call_id.clone(),
                                            error: err.clone(),
                                            preserve_existing_components,
                                            preserve_edit_policy: preserve_edit_policy_raw
                                                .map(|s| s.trim().to_string())
                                                .filter(|s| !s.is_empty()),
                                            rewire_components: rewire_components.clone(),
                                            existing_component_names,
                                            existing_root_component,
                                            plan,
                                        });

                                    Gen3dToolResultJsonV1::err(
                                        call.call_id.clone(),
                                        call.tool_id.clone(),
                                        format!(
                                            "{err}\nHint: This is a semantic plan error (not JSON/schema). Run `inspect_plan_v1` for computed constraints, or use `get_plan_template_v1` + llm_generate_plan_v1.plan_template_artifact_ref to replan safely."
                                        ),
                                    )
                                }
                            }
                        }
	                        Err(err) => {
	                            job.pending_plan_attempt = None;
	                            match (job.ai.clone(), job.pass_dir.clone()) {
	                            (Some(ai), Some(pass_dir)) => {
	                                let system = super::prompts::build_gen3d_plan_system_instructions();
	                                let prompt_override =
	                                    call.args.get("prompt").and_then(|v| v.as_str());
	                                let style_hint = call.args.get("style").and_then(|v| v.as_str());
	                                let plan_template_artifact_ref = call
	                                    .args
	                                    .get("plan_template_artifact_ref")
	                                    .and_then(|v| v.as_str())
	                                    .map(|s| s.trim().to_string())
	                                    .filter(|s| !s.is_empty());
	                                let mut required_component_names: Vec<String> = call
	                                    .args
	                                    .get("components")
	                                    .and_then(|v| v.as_array())
	                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|v| v.as_str())
                                            .map(|s| s.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .collect::<Vec<_>>()
                                    })
                                    .unwrap_or_default();
                                if required_component_names.len()
                                    > super::max_components_for_speed(workshop.speed_mode)
                                {
                                    required_component_names.truncate(
                                        super::max_components_for_speed(workshop.speed_mode),
                                    );
                                }

	                                let prompt_text = prompt_override
	                                    .map(|s| s.trim())
	                                    .filter(|s| !s.is_empty())
	                                    .unwrap_or(job.user_prompt_raw.as_str());
	                                let preserve_edit_policy = preserve_edit_policy_raw
	                                    .map(|s| s.trim())
	                                    .filter(|s| !s.is_empty())
	                                    .unwrap_or("additive");

	                                let plan_template_json: Option<serde_json::Value> =
	                                    if let Some(template_ref) =
	                                        plan_template_artifact_ref.as_deref()
	                                    {
	                                        job.run_dir
	                                            .as_deref()
	                                            .and_then(|run_dir| {
	                                                super::artifacts::read_artifact_v1(
	                                                    run_dir,
	                                                    template_ref,
	                                                    64 * 1024,
	                                                    None,
	                                                    None,
	                                                )
	                                                .ok()
	                                            })
	                                            .and_then(|v| {
	                                                let truncated = v
	                                                    .get("truncated")
	                                                    .and_then(|v| v.as_bool())
	                                                    .unwrap_or(false);
	                                                (!truncated).then(|| v.get("json").cloned())
	                                            })
	                                            .flatten()
	                                    } else {
	                                        None
	                                    };

	                                let user_text = if preserve_existing_components
	                                    && !job.planned_components.is_empty()
	                                {
	                                    super::prompts::build_gen3d_plan_user_text_preserve_existing_components(
	                                        prompt_text,
	                                        !job.user_images.is_empty(),
	                                        workshop.speed_mode,
	                                        style_hint,
	                                        &job.planned_components,
	                                        &job.assembly_notes,
	                                        preserve_edit_policy,
	                                        &rewire_components,
	                                        plan_template_json.as_ref(),
	                                    )
	                                } else {
	                                    super::prompts::build_gen3d_plan_user_text_with_hints(
	                                        prompt_text,
	                                        !job.user_images.is_empty(),
	                                        workshop.speed_mode,
	                                        style_hint,
	                                        &required_component_names,
	                                    )
	                                };

	                                if schedule_llm_tool_schema_repair(
	                                    job,
	                                    workshop,
	                                    &call,
                                    kind,
                                    ai,
                                    &config.gen3d_reasoning_effort_repair,
                                    pass_dir,
                                    system,
                                    user_text,
                                    job.user_images.clone(),
                                    &err,
                                    &text,
                                    &format!("tool_plan_{}", call.call_id),
                                ) {
                                    return;
                                }
                                Gen3dToolResultJsonV1::err(
                                    call.call_id.clone(),
                                    call.tool_id.clone(),
                                    err,
                                )
                            }
                            _ => Gen3dToolResultJsonV1::err(
                                call.call_id.clone(),
                                call.tool_id.clone(),
                                err,
                            ),
                        }
                        }
                    }
                }
                super::Gen3dAgentLlmToolKind::GenerateComponentsBatch => Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Internal error: llm_generate_components_v1 batch tool should be handled by poll_agent_component_batch.".into(),
                ),
                super::Gen3dAgentLlmToolKind::GenerateComponent { component_idx } => {
                    let text = resp.text;
                    match parse::parse_ai_draft_from_text(&text) {
                        Ok(ai) => match super::convert::ai_to_component_def(
                            &job.planned_components[component_idx],
                            ai,
                            job.pass_dir.as_deref(),
                        ) {
                            Ok(def) => {
                                let object_id = def.object_id;
                                job.planned_components[component_idx].actual_size = Some(def.size);
                                job.planned_components[component_idx].anchors = def.anchors.clone();
                                job.agent.pending_llm_repair_attempt = 0;

                                if let Some(existing) =
                                    draft.defs.iter_mut().find(|d| d.object_id == object_id)
                                {
                                    let preserved_refs: Vec<crate::object::registry::ObjectPartDef> = existing
                                        .parts
                                        .iter()
                                        .filter(|p| {
                                            matches!(
                                                p.kind,
                                                crate::object::registry::ObjectPartKind::ObjectRef { .. }
                                            )
                                        })
                                        .cloned()
                                        .collect();
                                    let mut new_def = def;
                                    new_def.parts.extend(preserved_refs);
                                    *existing = new_def;
                                } else {
                                    draft.defs.push(def);
                                }

                                if let Some(root_idx) = job
                                    .planned_components
                                    .iter()
                                    .position(|c| c.attach_to.is_none())
                                {
                                    if let Err(err) =
                                        super::convert::resolve_planned_component_transforms(
                                            &mut job.planned_components,
                                            root_idx,
                                        )
                                    {
                                        warn!(
                                            "Gen3D agent: failed to resolve transforms after component update: {err}"
                                        );
                                    }
                                }
                                super::convert::update_root_def_from_planned_components(
                                    &job.planned_components,
                                    &job.plan_collider,
                                    draft,
                                );
                                write_gen3d_assembly_snapshot(
                                    job.pass_dir.as_deref(),
                                    &job.planned_components,
                                );
                                job.assembly_rev = job.assembly_rev.saturating_add(1);

                                Gen3dToolResultJsonV1::ok(
                                    call.call_id,
                                    call.tool_id,
                                    serde_json::json!({
                                        "ok": true,
                                        "component_index": component_idx,
                                        "component_name": job.planned_components[component_idx].name,
                                    }),
                                )
                            }
                            Err(err) => match (job.ai.clone(), job.pass_dir.clone()) {
                                (Some(ai), Some(pass_dir)) => {
                                    let system =
                                        super::prompts::build_gen3d_component_system_instructions();
                                    let user_text = super::prompts::build_gen3d_component_user_text(
                                        &job.user_prompt_raw,
                                        !job.user_images.is_empty(),
                                        workshop.speed_mode,
                                        &job.assembly_notes,
                                        &job.planned_components,
                                        component_idx,
                                    );
                                    if schedule_llm_tool_schema_repair(
                                        job,
                                        workshop,
                                        &call,
                                        kind,
                                        ai,
                                        &config.gen3d_reasoning_effort_repair,
                                        pass_dir,
                                        system,
                                        user_text,
                                        job.user_images.clone(),
                                        &err,
                                        &text,
                                        &format!(
                                            "tool_component{}_{}",
                                            component_idx.saturating_add(1),
                                            call.call_id
                                        ),
                                    ) {
                                        return;
                                    }
                                    Gen3dToolResultJsonV1::err(
                                        call.call_id.clone(),
                                        call.tool_id.clone(),
                                        err,
                                    )
                                }
                                _ => Gen3dToolResultJsonV1::err(
                                    call.call_id.clone(),
                                    call.tool_id.clone(),
                                    err,
                                ),
                            },
                        },
                        Err(err) => match (job.ai.clone(), job.pass_dir.clone()) {
                            (Some(ai), Some(pass_dir)) => {
                                let system =
                                    super::prompts::build_gen3d_component_system_instructions();
                                let user_text = super::prompts::build_gen3d_component_user_text(
                                    &job.user_prompt_raw,
                                    !job.user_images.is_empty(),
                                    workshop.speed_mode,
                                    &job.assembly_notes,
                                    &job.planned_components,
                                    component_idx,
                                );
                                if schedule_llm_tool_schema_repair(
                                    job,
                                    workshop,
                                    &call,
                                    kind,
                                    ai,
                                    &config.gen3d_reasoning_effort_repair,
                                    pass_dir,
                                    system,
                                    user_text,
                                    job.user_images.clone(),
                                    &err,
                                    &text,
                                    &format!(
                                        "tool_component{}_{}",
                                        component_idx.saturating_add(1),
                                        call.call_id
                                    ),
                                ) {
                                    return;
                                }
                                Gen3dToolResultJsonV1::err(
                                    call.call_id.clone(),
                                    call.tool_id.clone(),
                                    err,
                                )
                            }
                            _ => Gen3dToolResultJsonV1::err(
                                call.call_id.clone(),
                                call.tool_id.clone(),
                                err,
                            ),
                        },
                    }
                }
                super::Gen3dAgentLlmToolKind::GenerateMotionAuthoring => {
                    use crate::object::registry::{
                        PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
                        PartAnimationSlot, PartAnimationSpec,
                    };

                    let text = resp.text;
                    if let Some(dir) = job.pass_dir.as_deref() {
                        write_gen3d_text_artifact(
                            Some(dir),
                            "motion_authoring_raw.txt",
                            text.trim(),
                        );
                    }

                    match super::parse::parse_ai_motion_authoring_from_text(&text) {
                        Ok(authored) => {
                            let expected_run_id =
                                job.run_id.map(|id| id.to_string()).unwrap_or_default();
                            let mut issues: Vec<String> = Vec::new();
                            if !expected_run_id.trim().is_empty()
                                && authored.applies_to.run_id.trim() != expected_run_id.trim()
                            {
                                issues.push(format!(
                                    "applies_to.run_id mismatch (got {}, expected {})",
                                    authored.applies_to.run_id.trim(),
                                    expected_run_id.trim()
                                ));
                            }
                            if authored.applies_to.attempt != job.attempt
                                || authored.applies_to.plan_hash.trim() != job.plan_hash.trim()
                                || authored.applies_to.assembly_rev != job.assembly_rev
                            {
                                issues.push(format!(
                                    "applies_to mismatch (got attempt={} plan_hash={} assembly_rev={}, expected attempt={} plan_hash={} assembly_rev={})",
                                    authored.applies_to.attempt,
                                    authored.applies_to.plan_hash.trim(),
                                    authored.applies_to.assembly_rev,
                                    job.attempt,
                                    job.plan_hash.trim(),
                                    job.assembly_rev,
                                ));
                            }

                            if issues.is_empty() {
                                match authored.decision {
                                    super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired => {
                                        if !authored.replace_channels.is_empty()
                                            || !authored.edges.is_empty()
                                        {
                                            issues.push("decision=regen_geometry_required must set replace_channels=[] and edges=[] (do not author clips).".to_string());
                                        }
                                        if authored.reason.trim().is_empty() {
                                            issues.push("decision=regen_geometry_required must include a non-empty `reason`.".to_string());
                                        }
                                    }
                                    super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips => {
                                        if authored.replace_channels.is_empty() {
                                            issues.push(
                                                "replace_channels must be non-empty when decision=author_clips"
                                                    .to_string(),
                                            );
                                        }
                                        if authored.edges.is_empty() {
                                            issues.push(
                                                "edges must be non-empty when decision=author_clips"
                                                    .to_string(),
                                            );
                                        }
                                    }
                                    _ => {
                                        issues.push("AI motion-authoring has invalid `decision` value (expected `author_clips` or `regen_geometry_required`).".to_string());
                                    }
                                }
                            }

                            if issues.is_empty()
                                && matches!(
                                    authored.decision,
                                    super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips
                                )
                            {

                                let mut name_to_idx: std::collections::HashMap<String, usize> =
                                    std::collections::HashMap::new();
                                for (idx, c) in job.planned_components.iter().enumerate() {
                                    name_to_idx.insert(c.name.clone(), idx);
                                }

                                let replace: std::collections::HashSet<&str> = authored
                                    .replace_channels
                                    .iter()
                                    .map(|s| s.as_str())
                                    .collect();

                                fn driver_from_ai(
                                    driver: super::schema::AiAnimationDriverJsonV1,
                                ) -> Option<PartAnimationDriver> {
                                    match driver {
                                        super::schema::AiAnimationDriverJsonV1::Always => {
                                            Some(PartAnimationDriver::Always)
                                        }
                                        super::schema::AiAnimationDriverJsonV1::MovePhase => {
                                            Some(PartAnimationDriver::MovePhase)
                                        }
                                        super::schema::AiAnimationDriverJsonV1::MoveDistance => {
                                            Some(PartAnimationDriver::MoveDistance)
                                        }
                                        super::schema::AiAnimationDriverJsonV1::AttackTime => {
                                            Some(PartAnimationDriver::AttackTime)
                                        }
                                        super::schema::AiAnimationDriverJsonV1::Unknown => None,
                                    }
                                }

                                fn transform_from_delta(
                                    delta: &super::schema::AiAnimationDeltaTransformJsonV1,
                                ) -> Transform {
                                    let translation = delta
                                        .pos
                                        .unwrap_or([0.0, 0.0, 0.0])
                                        .map(|v| if v.is_finite() { v } else { 0.0 });
                                    let translation = Vec3::new(
                                        translation[0],
                                        translation[1],
                                        translation[2],
                                    );

                                    let scale = delta
                                        .scale
                                        .unwrap_or([1.0, 1.0, 1.0])
                                        .map(|v| if v.is_finite() { v } else { 1.0 });
                                    let scale = Vec3::new(scale[0], scale[1], scale[2]);

                                    let rotation = match delta.rot_quat_xyzw {
                                        Some([x, y, z, w]) => {
                                            let q = Quat::from_xyzw(x, y, z, w);
                                            if q.is_finite() {
                                                q.normalize()
                                            } else {
                                                Quat::IDENTITY
                                            }
                                        }
                                        _ => Quat::IDENTITY,
                                    };

                                    Transform {
                                        translation,
                                        rotation,
                                        scale,
                                    }
                                }

                                for edge in authored.edges.iter() {
                                    let name = edge.component.trim();
                                    if name.is_empty() {
                                        continue;
                                    }
                                    let Some(&component_idx) = name_to_idx.get(name) else {
                                        issues.push(format!("Unknown component: {name}"));
                                        continue;
                                    };
                                    if job.planned_components[component_idx].attach_to.is_none() {
                                        issues.push(format!(
                                            "Component {name} is the root (no attach_to); cannot author edge motion"
                                        ));
                                        continue;
                                    }

                                    let mut replacement_slots: Vec<PartAnimationSlot> = Vec::new();
                                    let mut channels_seen: std::collections::HashSet<&str> =
                                        std::collections::HashSet::new();
                                    for slot in edge.slots.iter() {
                                        let channel = slot.channel.trim();
                                        if channel.is_empty() {
                                            continue;
                                        }
                                        if channel != "attack_primary"
                                            && !channels_seen.insert(channel)
                                        {
                                            issues.push(format!(
                                                "Duplicate channel `{channel}` for component `{name}` (only attack_primary may have multiple variants)"
                                            ));
                                            continue;
                                        }

                                        let Some(driver) = driver_from_ai(slot.driver) else {
                                            issues.push(format!(
                                                "Unknown driver for component `{name}` channel `{channel}`"
                                            ));
                                            continue;
                                        };

                                        let speed_scale = slot.speed_scale.abs().max(1e-3);
                                        let time_offset_units = slot.time_offset_units;

                                        let clip = match &slot.clip {
                                            super::schema::AiAnimationClipJsonV1::Loop {
                                                duration_units,
                                                keyframes,
                                            } => PartAnimationDef::Loop {
                                                duration_secs: duration_units.abs().max(1e-3),
                                                keyframes: keyframes
                                                    .iter()
                                                    .map(|kf| PartAnimationKeyframeDef {
                                                        time_secs: kf.t_units,
                                                        delta: transform_from_delta(&kf.delta),
                                                    })
                                                    .collect(),
                                            },
                                            super::schema::AiAnimationClipJsonV1::Once {
                                                duration_units,
                                                keyframes,
                                            } => PartAnimationDef::Once {
                                                duration_secs: duration_units.abs().max(1e-3),
                                                keyframes: keyframes
                                                    .iter()
                                                    .map(|kf| PartAnimationKeyframeDef {
                                                        time_secs: kf.t_units,
                                                        delta: transform_from_delta(&kf.delta),
                                                    })
                                                    .collect(),
                                            },
                                            super::schema::AiAnimationClipJsonV1::PingPong {
                                                duration_units,
                                                keyframes,
                                            } => PartAnimationDef::PingPong {
                                                duration_secs: duration_units.abs().max(1e-3),
                                                keyframes: keyframes
                                                    .iter()
                                                    .map(|kf| PartAnimationKeyframeDef {
                                                        time_secs: kf.t_units,
                                                        delta: transform_from_delta(&kf.delta),
                                                    })
                                                    .collect(),
                                            },
                                            super::schema::AiAnimationClipJsonV1::Spin {
                                                axis,
                                                radians_per_unit,
                                            } => PartAnimationDef::Spin {
                                                axis: Vec3::new(axis[0], axis[1], axis[2]),
                                                radians_per_unit: *radians_per_unit,
                                            },
                                        };

                                        replacement_slots.push(PartAnimationSlot {
                                            channel: channel.to_string().into(),
                                            spec: PartAnimationSpec {
                                                driver,
                                                speed_scale,
                                                time_offset_units,
                                                clip,
                                            },
                                        });
                                    }

                                    if let Some(att) =
                                        job.planned_components[component_idx].attach_to.as_mut()
                                    {
                                        att.animations.retain(|slot| {
                                            !replace.contains(slot.channel.as_ref())
                                        });
                                        att.animations.extend(replacement_slots);
                                    }
                                }

                                if issues.is_empty() {
                                    let movable = draft
                                        .root_def()
                                        .and_then(|def| def.mobility.as_ref())
                                        .is_some();
                                    if movable {
                                        let has_move = job.planned_components.iter().any(|c| {
                                            c.attach_to.as_ref().is_some_and(|att| {
                                                att.animations
                                                    .iter()
                                                    .any(|slot| slot.channel.as_ref() == "move")
                                            })
                                        });
                                        if !has_move {
                                            issues.push("decision=author_clips must produce at least one `move` animation slot for movable drafts.".to_string());
                                        }
                                    }
                                }
                            }

                            if !issues.is_empty() {
                                issues.sort();
                                issues.dedup();
                                Gen3dToolResultJsonV1::err(
                                    call.call_id,
                                    call.tool_id,
                                    format!(
                                        "motion-authoring validation failed:\n- {}",
                                        issues.join("\n- ")
                                    ),
                                )
                            } else {
                                if let Some(dir) = job.pass_dir.as_deref() {
                                    write_gen3d_json_artifact(
                                        Some(dir),
                                        "motion_authoring.json",
                                        &serde_json::to_value(&authored)
                                            .unwrap_or(serde_json::Value::Null),
                                    );
                                }

                                if matches!(
                                    authored.decision,
                                    super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips
                                ) {
                                    if let Err(err) = super::convert::sync_attachment_tree_to_defs(
                                        &job.planned_components,
                                        draft,
                                    ) {
                                        return fail_job(
                                            workshop,
                                            job,
                                            format!("Failed to apply motion-authoring: {err}"),
                                        );
                                    }
                                    write_gen3d_assembly_snapshot(
                                        job.pass_dir.as_deref(),
                                        &job.planned_components,
                                    );
                                }

                                job.motion_authoring = Some(authored.clone());
                                job.agent.pending_llm_repair_attempt = 0;

                                Gen3dToolResultJsonV1::ok(
                                    call.call_id,
                                    call.tool_id,
                                    serde_json::json!({
                                        "ok": true,
                                        "decision": match authored.decision {
                                            super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips => "author_clips",
                                            super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired => "regen_geometry_required",
                                            _ => "unknown",
                                        },
                                        "edges": authored.edges.len(),
                                    }),
                                )
                            }
                        }
                        Err(err) => match (job.ai.clone(), job.pass_dir.clone()) {
	                            (Some(ai), Some(pass_dir)) => {
	                                let system = super::prompts::build_gen3d_motion_authoring_system_instructions();
	                                let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
	                                let (mut has_idle_slot, mut has_move_slot) = (false, false);
	                                for comp in job.planned_components.iter() {
	                                    let Some(att) = comp.attach_to.as_ref() else {
	                                        continue;
	                                    };
	                                    for slot in att.animations.iter() {
	                                        match slot.channel.as_ref() {
	                                            "idle" => has_idle_slot = true,
	                                            "move" => has_move_slot = true,
	                                            _ => {}
	                                        }
	                                    }
	                                }
	                                let user_text = super::prompts::build_gen3d_motion_authoring_user_text(
	                                    &job.user_prompt_raw,
	                                    !job.user_images.is_empty(),
	                                    &run_id,
	                                    job.attempt,
	                                    &job.plan_hash,
	                                    job.assembly_rev,
	                                    job.rig_move_cycle_m,
	                                    has_idle_slot,
	                                    has_move_slot,
	                                    &job.planned_components,
	                                    draft,
	                                );
	                                if schedule_llm_tool_schema_repair(
                                    job,
                                    workshop,
                                    &call,
                                    kind,
                                    ai,
                                    &config.gen3d_reasoning_effort_repair,
                                    pass_dir,
                                    system,
                                    user_text,
                                    Vec::new(),
                                    &err,
                                    &text,
                                    &format!("tool_motion_authoring_{}", call.call_id),
                                ) {
                                    return;
                                }
                                Gen3dToolResultJsonV1::err(
                                    call.call_id.clone(),
                                    call.tool_id.clone(),
                                    err,
                                )
                            }
                            _ => Gen3dToolResultJsonV1::err(
                                call.call_id.clone(),
                                call.tool_id.clone(),
                                err,
                            ),
                        },
                    }
                }
                super::Gen3dAgentLlmToolKind::ReviewDelta => {
                    let text = resp.text;
                    if let Some(dir) = job.pass_dir.as_deref() {
                        write_gen3d_text_artifact(Some(dir), "review_delta_raw.txt", text.trim());
                    }

                    match super::parse::parse_ai_review_delta_from_text(&text) {
                        Ok(delta) => {
                            let delta_requested_regen = delta.actions.iter().any(|action| {
                                matches!(
                                    action,
                                    super::schema::AiReviewDeltaActionJsonV1::RegenComponent { .. }
                                )
                            });
                            let delta_has_non_regen_actions = delta.actions.iter().any(|action| {
                                !matches!(
                                    action,
                                    super::schema::AiReviewDeltaActionJsonV1::Accept
                                        | super::schema::AiReviewDeltaActionJsonV1::ToolingFeedback {
                                            ..
                                        }
                                        | super::schema::AiReviewDeltaActionJsonV1::RegenComponent {
                                            ..
                                        }
                                )
                            });
                            let extracted_feedback: Vec<super::schema::AiToolingFeedbackJsonV1> =
                                delta
                                    .actions
                                    .iter()
                                    .filter_map(|action| {
                                        match action {
                                        super::schema::AiReviewDeltaActionJsonV1::ToolingFeedback {
                                            feedback,
                                        } => Some(feedback.clone()),
                                        _ => None,
                                    }
                                    })
                                    .collect();
                            let plan_collider = job.plan_collider.clone();
                            match super::convert::apply_ai_review_delta_actions(
                                delta,
                                &mut job.planned_components,
                                &plan_collider,
                                draft,
                                job.pass_dir.as_deref(),
                            ) {
                                Ok(apply) => {
                                    if !apply.tooling_feedback.is_empty() {
                                        super::record_gen3d_tooling_feedback(
                                            config,
                                            workshop,
                                            feedback_history,
                                            job,
                                            &apply.tooling_feedback,
                                        );
                                    }

                                    // Budget-gate regen requests so the agent doesn't loop forever on a single component.
                                    let regen_buckets = bucket_review_delta_regen_requests(
                                        config,
                                        job,
                                        &apply.regen_indices,
                                    );
                                    if !regen_buckets.skipped_due_to_budget.is_empty() {
                                        append_gen3d_run_log(
                                            job.pass_dir.as_deref(),
                                            format!(
                                                "regen_budget_skip_review skipped={} max_total={} max_per_component={}",
                                                regen_buckets.skipped_due_to_budget.len(),
                                                config.gen3d_max_regen_total,
                                                config.gen3d_max_regen_per_component
                                            ),
                                        );
                                    }
                                    job.agent.pending_regen_component_indices =
                                        regen_buckets.allowed.clone();
                                    job.agent.pending_regen_component_indices_skipped_due_to_budget =
                                        regen_buckets.skipped_due_to_budget.clone();
                                    job.agent.pending_regen_component_indices_blocked_due_to_qa_gate =
                                        regen_buckets.blocked_due_to_qa_gate.clone();

                                    let non_actionable_regen_only = delta_requested_regen
                                        && regen_buckets.allowed.is_empty()
                                        && !regen_buckets.skipped_due_to_budget.is_empty()
                                        && !delta_has_non_regen_actions
                                        && apply.replan_reason.is_none();

                                    let qa_gated_regen_only = delta_requested_regen
                                        && regen_buckets.allowed.is_empty()
                                        && !regen_buckets.blocked_due_to_qa_gate.is_empty()
                                        && !delta_has_non_regen_actions
                                        && apply.replan_reason.is_none()
                                        && !apply.had_actions;

                                    if qa_gated_regen_only {
                                        job.agent.rendered_since_last_review = false;
                                        job.agent.ever_reviewed = true;
                                        job.agent.pending_llm_repair_attempt = 0;

                                        let validate_ok = job.agent.last_validate_ok;
                                        let smoke_ok = job.agent.last_smoke_ok;
                                        let reason = if validate_ok.is_none() || smoke_ok.is_none()
                                        {
                                            "qa_v1 has not been run (or is incomplete)"
                                        } else {
                                            "qa_v1 reports no errors"
                                        };

                                        Gen3dToolResultJsonV1::err(
                                            call.call_id,
                                            call.tool_id,
                                            format!(
                                                "Regen request blocked by QA gate because {reason}. validate_ok={validate_ok:?} smoke_ok={smoke_ok:?}. blocked_component_indices={:?}. In preserve-existing-components mode, regenerating already-generated components is only allowed when QA reports errors. Prefer `apply_draft_ops_v1` / non-regen `llm_review_delta_v1` actions, OR disable preserve mode via `llm_generate_plan_v1` with `constraints.preserve_existing_components=false` and rebuild.",
                                                regen_buckets.blocked_due_to_qa_gate
                                            ),
                                        )
                                    } else {
                                        if non_actionable_regen_only {
                                            let llm_available = job
                                                .ai
                                                .as_ref()
                                                .map(|ai| {
                                                    !ai.base_url().starts_with("mock://gen3d")
                                                })
                                                .unwrap_or(true);
                                            let appearance_review_enabled =
                                                llm_available && job.review_appearance;
                                            let qa_ok = job.agent.ever_validated
                                                && job.agent.ever_smoke_checked
                                                && (!appearance_review_enabled
                                                    || (job.agent.ever_rendered
                                                        && job.agent.ever_reviewed));
                                            if qa_ok {
                                                stop_best_effort_after_tool = Some(format!(
                                                    "Regen budget exhausted for requested component(s) (max_regen_total={}, max_regen_per_component={}).",
                                                    config.gen3d_max_regen_total,
                                                    config.gen3d_max_regen_per_component
                                                ));
                                            }
                                        }

                                        if apply.had_actions && !non_actionable_regen_only {
                                            job.assembly_rev =
                                                job.assembly_rev.saturating_add(1);
                                            write_gen3d_assembly_snapshot(
                                                job.pass_dir.as_deref(),
                                                &job.planned_components,
                                            );
                                        }
                                        job.agent.rendered_since_last_review = false;
                                        job.agent.ever_reviewed = true;
                                        job.agent.pending_llm_repair_attempt = 0;

                                        Gen3dToolResultJsonV1::ok(
                                            call.call_id,
                                            call.tool_id,
                                            serde_json::json!({
                                                "ok": true,
                                                "accepted": apply.accepted,
                                                "had_actions": apply.had_actions && !non_actionable_regen_only,
                                                "regen_component_indices": regen_buckets.allowed,
                                                "regen_component_indices_skipped_due_to_budget": regen_buckets.skipped_due_to_budget,
                                                "regen_component_indices_blocked_due_to_qa_gate": regen_buckets.blocked_due_to_qa_gate,
                                                "replan_reason": apply.replan_reason,
                                            }),
                                        )
                                    }
                                }
                                Err(err) => {
                                    if !extracted_feedback.is_empty() {
                                        super::record_gen3d_tooling_feedback(
                                            config,
                                            workshop,
                                            feedback_history,
                                            job,
                                            &extracted_feedback,
                                        );
                                    }
                                    match (job.ai.clone(), job.pass_dir.clone()) {
                                        (Some(ai), Some(pass_dir)) => {
                                            let run_id = job
                                                .run_id
                                                .map(|id| id.to_string())
                                                .unwrap_or_default();
                                            let scene_graph_summary =
                                                super::build_gen3d_scene_graph_summary(
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
                                            let review_appearance = job.review_appearance;
                                            let edit_session = job.edit_base_prefab_id.is_some()
                                                && !job.user_prompt_raw.trim().is_empty();
                                            let system = super::prompts::build_gen3d_review_delta_system_instructions(
                                                review_appearance,
                                                edit_session,
                                            );
                                            let include_original_images = review_appearance
                                                && call
                                                    .args
                                                    .get("include_original_images")
                                                    .and_then(|v| v.as_bool())
                                                    .unwrap_or(true);
                                            let user_text =
                                                super::prompts::build_gen3d_review_delta_user_text(
                                                    &run_id,
                                                    job.attempt,
                                                    &job.plan_hash,
                                                    job.assembly_rev,
                                                    &job.user_prompt_raw,
                                                    include_original_images
                                                        && !job.user_images.is_empty(),
                                                    &scene_graph_summary,
                                                &smoke_results,
                                            );

                                            let mut preview_images = if review_appearance {
                                                parse_review_preview_images_from_args(&call.args)
                                            } else {
                                                Vec::new()
                                            };
                                            let preview_images_were_explicit =
                                                !preview_images.is_empty();
                                            if review_appearance && preview_images.is_empty() {
                                                preview_images =
                                                    job.agent.last_render_images.clone();
                                            }

                                            let mut images_to_send: Vec<PathBuf> = Vec::new();
                                            if review_appearance {
                                                let (include_move_sheet, include_attack_sheet) =
                                                    motion_sheets_needed_from_smoke_results(
                                                        &smoke_results,
                                                    );
                                                if !preview_images_were_explicit {
                                                    preview_images = select_review_preview_images(
                                                        &preview_images,
                                                        include_move_sheet,
                                                        include_attack_sheet,
                                                    );
                                                }
                                                if include_original_images {
                                                    images_to_send
                                                        .extend(job.user_images.clone());
                                                }
                                                images_to_send.extend(preview_images);
                                                if images_to_send.len()
                                                    > GEN3D_MAX_REQUEST_IMAGES
                                                {
                                                    images_to_send
                                                        .truncate(GEN3D_MAX_REQUEST_IMAGES);
                                                }
                                            }

                                            if schedule_llm_tool_schema_repair(
                                                job,
                                                workshop,
                                                &call,
                                                kind,
                                                ai,
                                                &config.gen3d_reasoning_effort_repair,
                                                pass_dir,
                                                system,
                                                user_text,
                                                images_to_send,
                                                &err,
                                                &text,
                                                &format!("tool_review_{}", call.call_id),
                                            ) {
                                                return;
                                            }
                                            Gen3dToolResultJsonV1::err(
                                                call.call_id.clone(),
                                                call.tool_id.clone(),
                                                err,
                                            )
                                        }
                                        _ => Gen3dToolResultJsonV1::err(
                                            call.call_id.clone(),
                                            call.tool_id.clone(),
                                            err,
                                        ),
                                    }
                                }
                            }
                        }
                        Err(err) => match (job.ai.clone(), job.pass_dir.clone()) {
                            (Some(ai), Some(pass_dir)) => {
                                let run_id =
                                    job.run_id.map(|id| id.to_string()).unwrap_or_default();
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
                                let review_appearance = job.review_appearance;
                                let edit_session = job.edit_base_prefab_id.is_some()
                                    && !job.user_prompt_raw.trim().is_empty();
                                let system = super::prompts::build_gen3d_review_delta_system_instructions(
                                    review_appearance,
                                    edit_session,
                                );
                                let include_original_images = review_appearance
                                    && call
                                        .args
                                        .get("include_original_images")
                                        .and_then(|v| v.as_bool())
                                        .unwrap_or(true);
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

                                let mut preview_images = if review_appearance {
                                    parse_review_preview_images_from_args(&call.args)
                                } else {
                                    Vec::new()
                                };
                                let preview_images_were_explicit = !preview_images.is_empty();
                                if review_appearance && preview_images.is_empty() {
                                    preview_images = job.agent.last_render_images.clone();
                                }

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

                                if schedule_llm_tool_schema_repair(
                                    job,
                                    workshop,
                                    &call,
                                    kind,
                                    ai,
                                    &config.gen3d_reasoning_effort_repair,
                                    pass_dir,
                                    system,
                                    user_text,
                                    images_to_send,
                                    &err,
                                    &text,
                                    &format!("tool_review_{}", call.call_id),
                                ) {
                                    return;
                                }
                                Gen3dToolResultJsonV1::err(
                                    call.call_id.clone(),
                                    call.tool_id.clone(),
                                    err,
                                )
                            }
                            _ => Gen3dToolResultJsonV1::err(
                                call.call_id.clone(),
                                call.tool_id.clone(),
                                err,
                            ),
                        },
                    }
                }
            }
        }
        Err(err) => Gen3dToolResultJsonV1::err(call.call_id, call.tool_id, err),
    };

    job.metrics.note_tool_result(&tool_result);
    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::ToolResult {
            call_id: tool_result.call_id.clone(),
            tool_id: tool_result.tool_id.clone(),
            ok: tool_result.ok,
            result: tool_result.result.clone(),
            error: tool_result.error.clone(),
        },
    );
    append_gen3d_jsonl_artifact(
        job.pass_dir.as_deref(),
        "tool_results.jsonl",
        &serde_json::to_value(&tool_result).unwrap_or(serde_json::Value::Null),
    );
    append_gen3d_run_log(
        job.pass_dir.as_deref(),
        format!(
            "tool_call_result call_id={} tool_id={} ok={} {}",
            tool_result.call_id,
            tool_result.tool_id,
            tool_result.ok,
            if tool_result.ok {
                tool_result
                    .result
                    .as_ref()
                    .map(|v| format!("result={}", truncate_json_for_log(v, 900)))
                    .unwrap_or_else(|| "result=<none>".into())
            } else {
                format!("error={}", tool_result.error.as_deref().unwrap_or("<none>"))
            }
        ),
    );
    if tool_result.ok {
        debug!(
            "Gen3D tool call ok: call_id={} tool_id={}",
            tool_result.call_id, tool_result.tool_id
        );
    } else {
        warn!(
            "Gen3D tool call failed: call_id={} tool_id={} error={}",
            tool_result.call_id,
            tool_result.tool_id,
            tool_result.error.as_deref().unwrap_or("<none>")
        );
    }
    let tool_id_for_guard = tool_result.tool_id.clone();
    let tool_ok_for_guard = tool_result.ok;
    note_observable_tool_result(job, &tool_result);
    job.agent.step_tool_results.push(tool_result);

    if let Some(reason) = stop_best_effort_after_tool.take() {
        workshop.error = None;
        let status = format!(
            "Build finished (best effort).\nReason: {}",
            super::truncate_for_ui(reason.trim(), 600)
        );
        super::agent_step::start_finish_run_sequence(
            config,
            commands,
            images,
            workshop,
            job,
            draft,
            super::Gen3dPendingFinishRun {
                workshop_status: status,
                run_log: format!(
                    "budget_stop reason={}",
                    super::truncate_for_ui(reason.trim(), 600)
                ),
                info_log: format!(
                    "Gen3D agent: best-effort stop (regen budget exhausted). reason={:?}",
                    reason.trim()
                ),
            },
        );
        return;
    }

    if !tool_ok_for_guard || tool_id_for_guard == TOOL_ID_LLM_GENERATE_PLAN {
        // End the step early on async tool failures (avoid cascades), and also enforce
        // a hard phase split after planning so the next step can observe the plan state
        // (including any reuse_groups) before deciding what to generate.
        job.agent.step_action_idx = job.agent.step_actions.len();
    }

    job.phase = Gen3dAiPhase::AgentExecutingActions;

    let _ = commands;
    let _ = images;
    let _ = feedback_history;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::object::registry::AnchorDef;
    use bevy::prelude::{Quat, Transform, Vec3};

    fn make_job_with_components(generated_flags: &[bool]) -> Gen3dAiJob {
        let mut job = Gen3dAiJob::default();
        job.planned_components = generated_flags
            .iter()
            .enumerate()
            .map(
                |(idx, generated)| super::super::job::Gen3dPlannedComponent {
                    display_name: format!("{}. c{idx}", idx + 1),
                    name: format!("c{idx}"),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    pos: Vec3::ZERO,
                    rot: Quat::IDENTITY,
                    planned_size: Vec3::ONE,
                    actual_size: generated.then_some(Vec3::ONE),
                    anchors: vec![AnchorDef {
                        name: "mount".into(),
                        transform: Transform::IDENTITY,
                    }],
                    contacts: Vec::new(),
                    attach_to: if idx == 0 {
                        None
                    } else {
                        Some(super::super::job::Gen3dPlannedAttachment {
                            parent: "c0".to_string(),
                            parent_anchor: "mount".to_string(),
                            child_anchor: "mount".to_string(),
                            offset: Transform::IDENTITY,
                            joint: None,
                            animations: Vec::new(),
                        })
                    },
                },
            )
            .collect();
        job
    }

    #[test]
    fn bucket_regen_requests_blocks_force_regen_when_preserve_mode_and_qa_clean() {
        let config = AppConfig::default();
        let mut job = make_job_with_components(&[true, true]);
        job.preserve_existing_components_mode = true;
        job.agent.last_validate_ok = Some(true);
        job.agent.last_smoke_ok = Some(true);

        let buckets = bucket_review_delta_regen_requests(&config, &mut job, &[1]);
        assert_eq!(buckets.allowed, Vec::<usize>::new());
        assert_eq!(buckets.skipped_due_to_budget, Vec::<usize>::new());
        assert_eq!(buckets.blocked_due_to_qa_gate, vec![1]);
    }

    #[test]
    fn bucket_regen_requests_allows_force_regen_when_qa_has_errors() {
        let config = AppConfig::default();
        let mut job = make_job_with_components(&[true, true]);
        job.preserve_existing_components_mode = true;
        job.agent.last_validate_ok = Some(false);
        job.agent.last_smoke_ok = Some(true);

        let buckets = bucket_review_delta_regen_requests(&config, &mut job, &[1]);
        assert_eq!(buckets.allowed, vec![1]);
        assert!(buckets.blocked_due_to_qa_gate.is_empty());
    }

    #[test]
    fn bucket_regen_requests_keeps_missing_components_actionable_even_if_regen_budget_exhausted() {
        let mut config = AppConfig::default();
        config.gen3d_max_regen_total = 1;

        let mut job = make_job_with_components(&[true, false]);
        job.preserve_existing_components_mode = true;
        job.agent.last_validate_ok = Some(true);
        job.agent.last_smoke_ok = Some(true);
        job.regen_total = 1;

        let buckets = bucket_review_delta_regen_requests(&config, &mut job, &[1]);
        assert_eq!(buckets.allowed, vec![1]);
        assert!(buckets.skipped_due_to_budget.is_empty());
        assert!(buckets.blocked_due_to_qa_gate.is_empty());
    }

    #[test]
    fn bucket_regen_requests_respects_regen_budget_for_generated_components() {
        let mut config = AppConfig::default();
        config.gen3d_max_regen_total = 1;

        let mut job = make_job_with_components(&[true, true]);
        job.preserve_existing_components_mode = false;
        job.regen_total = 1;

        let buckets = bucket_review_delta_regen_requests(&config, &mut job, &[1]);
        assert!(buckets.allowed.is_empty());
        assert_eq!(buckets.skipped_due_to_budget, vec![1]);
    }
}
