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
use super::agent_step::maybe_start_pass_snapshot_capture;
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
        openai: crate::config::OpenAiConfig,
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

        let reasoning_effort = super::openai::cap_reasoning_effort(
            &openai.model_reasoning_effort,
            reasoning_effort_cap,
        );
        let expected_schema = match kind {
            super::Gen3dAgentLlmToolKind::GeneratePlan => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanV1)
            }
            super::Gen3dAgentLlmToolKind::GenerateComponent { .. }
            | super::Gen3dAgentLlmToolKind::GenerateComponentsBatch => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1)
            }
            super::Gen3dAgentLlmToolKind::GenerateMotionRoles => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::MotionRolesV1)
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
            openai,
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
                    match parse::parse_ai_plan_from_text(&text) {
                        Ok(plan) => {
                            let plan_reuse_groups = plan.reuse_groups.clone();
                            match super::convert::ai_plan_to_initial_draft_defs(plan.clone()) {
                                Ok((planned, notes, defs)) => {
                                    job.planned_components = planned;
                                    job.assembly_notes = notes;
                                    let (validated, warnings) = super::reuse_groups::validate_reuse_groups(
                                        &plan_reuse_groups,
                                        &job.planned_components,
                                    );
                                    job.reuse_groups = validated;
                                    job.reuse_group_warnings = warnings;
                                    job.plan_hash = super::compute_gen3d_plan_hash(
                                        &job.assembly_notes,
                                        job.rig_move_cycle_m,
                                        &job.planned_components,
                                    );
                                    job.assembly_rev = 0;
                                    job.rig_move_cycle_m = plan
                                        .rig
                                        .as_ref()
                                        .and_then(|r| r.move_cycle_m)
                                        .filter(|v| v.is_finite())
                                        .map(f32::abs)
                                        .filter(|v| *v > 1e-3);
                                    job.plan_collider = plan.collider;
                                    draft.defs = defs;
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

                                    Gen3dToolResultJsonV1::ok(
                                        call.call_id,
                                        call.tool_id,
                                        serde_json::json!({
                                            "ok": true,
                                            "components_total": job.planned_components.len(),
                                            "plan_hash": job.plan_hash,
                                        }),
                                    )
                                }
                                Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                                    (Some(openai), Some(pass_dir)) => {
                                        let system =
                                            super::prompts::build_gen3d_plan_system_instructions();
                                        let prompt_override =
                                            call.args.get("prompt").and_then(|v| v.as_str());
                                        let style_hint =
                                            call.args.get("style").and_then(|v| v.as_str());
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
                                                super::max_components_for_speed(
                                                    workshop.speed_mode,
                                                ),
                                            );
                                        }

                                        let prompt_text = prompt_override
                                            .map(|s| s.trim())
                                            .filter(|s| !s.is_empty())
                                            .unwrap_or(job.user_prompt_raw.as_str());
                                        let user_text =
                                            super::prompts::build_gen3d_plan_user_text_with_hints(
                                                prompt_text,
                                                !job.user_images.is_empty(),
                                                workshop.speed_mode,
                                                style_hint,
                                                &required_component_names,
                                            );

                                        if schedule_llm_tool_schema_repair(
                                            job,
                                            workshop,
                                            &call,
                                            kind,
                                            openai,
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
                                },
                            }
                        }
                        Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                            (Some(openai), Some(pass_dir)) => {
                                let system = super::prompts::build_gen3d_plan_system_instructions();
                                let prompt_override =
                                    call.args.get("prompt").and_then(|v| v.as_str());
                                let style_hint = call.args.get("style").and_then(|v| v.as_str());
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
                                let user_text =
                                    super::prompts::build_gen3d_plan_user_text_with_hints(
                                        prompt_text,
                                        !job.user_images.is_empty(),
                                        workshop.speed_mode,
                                        style_hint,
                                        &required_component_names,
                                    );

                                if schedule_llm_tool_schema_repair(
                                    job,
                                    workshop,
                                    &call,
                                    kind,
                                    openai,
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
                        },
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
                            Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                                (Some(openai), Some(pass_dir)) => {
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
                                        openai,
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
                        Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                            (Some(openai), Some(pass_dir)) => {
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
                                    openai,
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
                super::Gen3dAgentLlmToolKind::GenerateMotionRoles => {
                    let text = resp.text;
                    if let Some(dir) = job.pass_dir.as_deref() {
                        write_gen3d_text_artifact(Some(dir), "motion_roles_raw.txt", text.trim());
                    }

                    match super::parse::parse_ai_motion_roles_from_text(&text) {
                        Ok(mut roles) => {
                            let expected_run_id =
                                job.run_id.map(|id| id.to_string()).unwrap_or_default();
                            let mut issues: Vec<String> = Vec::new();
                            if !expected_run_id.trim().is_empty()
                                && roles.applies_to.run_id.trim() != expected_run_id.trim()
                            {
                                issues.push(format!(
                                    "applies_to.run_id mismatch (got {}, expected {})",
                                    roles.applies_to.run_id.trim(),
                                    expected_run_id.trim()
                                ));
                            }
                            if roles.applies_to.attempt != job.attempt
                                || roles.applies_to.plan_hash.trim() != job.plan_hash.trim()
                                || roles.applies_to.assembly_rev != job.assembly_rev
                            {
                                issues.push(format!(
                                    "applies_to mismatch (got attempt={} plan_hash={} assembly_rev={}, expected attempt={} plan_hash={} assembly_rev={})",
                                    roles.applies_to.attempt,
                                    roles.applies_to.plan_hash.trim(),
                                    roles.applies_to.assembly_rev,
                                    job.attempt,
                                    job.plan_hash.trim(),
                                    job.assembly_rev,
                                ));
                            }
                            for effector in roles.move_effectors.iter() {
                                let name = effector.component.trim();
                                let Some(component) = job
                                    .planned_components
                                    .iter()
                                    .find(|c| c.name == name)
                                else {
                                    issues.push(format!("Unknown component: {name}"));
                                    continue;
                                };
                                if component.attach_to.is_none() {
                                    issues.push(format!(
                                        "Component {name} is the root (no attach_to); cannot be a move effector"
                                    ));
                                }
                                match effector.role {
                                    super::schema::AiMoveEffectorRoleJsonV1::Leg => {
                                        if !matches!(effector.phase_group, Some(0) | Some(1)) {
                                            issues.push(format!(
                                                "Component {name} role=leg must have phase_group=0 or 1"
                                            ));
                                        }
                                        if effector.spin_axis_local.is_some() {
                                            issues.push(format!(
                                                "Component {name} role=leg must have spin_axis_local=null"
                                            ));
                                        }
                                    }
                                    super::schema::AiMoveEffectorRoleJsonV1::Arm => {
                                        if effector.phase_group.is_some()
                                            && !matches!(effector.phase_group, Some(0) | Some(1))
                                        {
                                            issues.push(format!(
                                                "Component {name} role=arm phase_group must be 0 or 1 (or null)"
                                            ));
                                        }
                                        if effector.spin_axis_local.is_some() {
                                            issues.push(format!(
                                                "Component {name} role=arm must have spin_axis_local=null"
                                            ));
                                        }
                                    }
                                    super::schema::AiMoveEffectorRoleJsonV1::Wheel => {
                                        if effector.phase_group.is_some() {
                                            issues.push(format!(
                                                "Component {name} role=wheel must have phase_group=null"
                                            ));
                                        }
                                    }
                                    super::schema::AiMoveEffectorRoleJsonV1::Propeller
                                    | super::schema::AiMoveEffectorRoleJsonV1::Rotor => {
                                        if effector.phase_group.is_some() {
                                            issues.push(format!(
                                                "Component {name} role={:?} must have phase_group=null",
                                                effector.role
                                            ));
                                        }
                                    }
                                    super::schema::AiMoveEffectorRoleJsonV1::Head
                                    | super::schema::AiMoveEffectorRoleJsonV1::Ear
                                    | super::schema::AiMoveEffectorRoleJsonV1::Tail
                                    | super::schema::AiMoveEffectorRoleJsonV1::Wing => {
                                        if effector.phase_group.is_some() {
                                            issues.push(format!(
                                                "Component {name} role={:?} must have phase_group=null",
                                                effector.role
                                            ));
                                        }
                                        if effector.spin_axis_local.is_some() {
                                            issues.push(format!(
                                                "Component {name} role={:?} must have spin_axis_local=null",
                                                effector.role
                                            ));
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
                                        "motion-roles validation failed:\n- {}",
                                        issues.join("\n- ")
                                    ),
                                )
                            } else {
                                roles.move_effectors.retain(|e| {
                                    job.planned_components
                                        .iter()
                                        .any(|c| c.name == e.component.trim())
                                });

                                if let Some(dir) = job.pass_dir.as_deref() {
                                    write_gen3d_json_artifact(
                                        Some(dir),
                                        "motion_roles.json",
                                        &serde_json::to_value(&roles)
                                            .unwrap_or(serde_json::Value::Null),
                                    );
                                }

                                job.motion_roles = Some(roles.clone());
                                job.agent.pending_llm_repair_attempt = 0;

                                Gen3dToolResultJsonV1::ok(
                                    call.call_id,
                                    call.tool_id,
                                    serde_json::json!({
                                        "ok": true,
                                        "move_effectors": roles.move_effectors.len(),
                                    }),
                                )
                            }
                        }
                        Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                            (Some(openai), Some(pass_dir)) => {
                                let system = super::prompts::build_gen3d_motion_roles_system_instructions();
                                let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
                                let user_text = super::prompts::build_gen3d_motion_roles_user_text(
                                    &job.user_prompt_raw,
                                    !job.user_images.is_empty(),
                                    &run_id,
                                    job.attempt,
                                    &job.plan_hash,
                                    job.assembly_rev,
                                    &job.planned_components,
                                );
                                if schedule_llm_tool_schema_repair(
                                    job,
                                    workshop,
                                    &call,
                                    kind,
                                    openai,
                                    &config.gen3d_reasoning_effort_repair,
                                    pass_dir,
                                    system,
                                    user_text,
                                    Vec::new(),
                                    &err,
                                    &text,
                                    &format!("tool_motion_roles_{}", call.call_id),
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

                            let mobility_mode = draft
                                .root_def()
                                .and_then(|def| def.mobility.as_ref())
                                .map(|m| m.mode);
                            let runtime_candidate = super::agent_utils::motion_runtime_candidate_kind(
                                job.motion_roles_for_current_draft(),
                                &job.planned_components,
                                mobility_mode,
                            );
                            let has_move_existing = job.planned_components.iter().any(|c| {
                                c.attach_to.as_ref().is_some_and(|att| {
                                    att.animations
                                        .iter()
                                        .any(|slot| slot.channel.as_ref() == "move")
                                })
                            });

                            if issues.is_empty() {
                                match authored.decision {
                                    super::schema::AiMotionAuthoringDecisionJsonV1::RuntimeOk => {
                                        if !authored.replace_channels.is_empty()
                                            || !authored.edges.is_empty()
                                        {
                                            issues.push("decision=runtime_ok must set replace_channels=[] and edges=[] (do not author clips).".to_string());
                                        }
                                        if runtime_candidate.is_none() && !has_move_existing {
                                            issues.push("decision=runtime_ok is invalid because there is no runtime motion rig candidate and no authored `move` slots. Use decision=author_clips or decision=regen_geometry_required.".to_string());
                                        }
                                    }
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
                                    super::schema::AiMotionAuthoringDecisionJsonV1::Unknown => {
                                        issues.push("AI motion-authoring has unknown `decision` value.".to_string());
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
                                            super::schema::AiMotionAuthoringDecisionJsonV1::RuntimeOk => "runtime_ok",
                                            super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips => "author_clips",
                                            super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired => "regen_geometry_required",
                                            super::schema::AiMotionAuthoringDecisionJsonV1::Unknown => "unknown",
                                        },
                                        "edges": authored.edges.len(),
                                    }),
                                )
                            }
                        }
                        Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
	                            (Some(openai), Some(pass_dir)) => {
	                                let system = super::prompts::build_gen3d_motion_authoring_system_instructions();
	                                let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
	                                let roles = job.motion_roles_for_current_draft();
	                                let mobility_mode = draft
	                                    .root_def()
	                                    .and_then(|def| def.mobility.as_ref())
	                                    .map(|m| m.mode);
	                                let runtime_candidate = super::agent_utils::motion_runtime_candidate_kind(
	                                    roles,
	                                    &job.planned_components,
	                                    mobility_mode,
	                                );
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
	                                    roles.is_some(),
	                                    runtime_candidate,
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
                                    openai,
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
                                    let mut regen_allowed: Vec<usize> = Vec::new();
                                    let mut regen_skipped: Vec<usize> = Vec::new();
                                    if !apply.regen_indices.is_empty() {
                                        ensure_agent_regen_budget_len(job);
                                        let mut seen = std::collections::HashSet::<usize>::new();
                                        for idx in apply.regen_indices.iter().copied() {
                                            if idx >= job.planned_components.len() {
                                                continue;
                                            }
                                            if !seen.insert(idx) {
                                                continue;
                                            }
                                            if regen_budget_allows(config, job, idx) {
                                                regen_allowed.push(idx);
                                            } else {
                                                regen_skipped.push(idx);
                                            }
                                        }
                                        if !regen_skipped.is_empty() {
                                            regen_skipped.sort_unstable();
                                            append_gen3d_run_log(
                                                job.pass_dir.as_deref(),
                                                format!(
                                                    "regen_budget_skip_review skipped={} max_total={} max_per_component={}",
                                                    regen_skipped.len(),
                                                    config.gen3d_max_regen_total,
                                                    config.gen3d_max_regen_per_component
                                                ),
                                            );
                                        }
                                    }
                                    regen_allowed.sort_unstable();
                                    job.agent.pending_regen_component_indices = regen_allowed.clone();
                                    job.agent.pending_regen_component_indices_skipped_due_to_budget =
                                        regen_skipped.clone();

                                    let non_actionable_regen_only = delta_requested_regen
                                        && regen_allowed.is_empty()
                                        && !regen_skipped.is_empty()
                                        && !delta_has_non_regen_actions
                                        && apply.replan_reason.is_none();

                                    if non_actionable_regen_only {
                                        let visual_qa_required = job
                                            .openai
                                            .as_ref()
                                            .map(|openai| {
                                                !openai.base_url.starts_with("mock://gen3d")
                                            })
                                            .unwrap_or(true);
                                        let qa_ok = job.agent.ever_validated
                                            && job.agent.ever_smoke_checked
                                            && (!visual_qa_required
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
                                        job.assembly_rev = job.assembly_rev.saturating_add(1);
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
                                            "regen_component_indices": regen_allowed,
                                            "regen_component_indices_skipped_due_to_budget": regen_skipped,
                                            "replan_reason": apply.replan_reason,
                                        }),
                                    )
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
                                    match (job.openai.clone(), job.pass_dir.clone()) {
                                        (Some(openai), Some(pass_dir)) => {
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
                                            let system = super::prompts::build_gen3d_review_delta_system_instructions(review_appearance);
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
                                                openai,
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
                        Err(err) => match (job.openai.clone(), job.pass_dir.clone()) {
                            (Some(openai), Some(pass_dir)) => {
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
                                let system = super::prompts::build_gen3d_review_delta_system_instructions(review_appearance);
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
                                    openai,
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
        if maybe_start_pass_snapshot_capture(
            config,
            commands,
            images,
            workshop,
            job,
            draft,
            super::Gen3dAgentAfterPassSnapshot::FinishRun {
                workshop_status: status.clone(),
                run_log: format!(
                    "budget_stop reason={}",
                    super::truncate_for_ui(reason.trim(), 600)
                ),
                info_log: format!(
                    "Gen3D agent: best-effort stop (regen budget exhausted). reason={:?}",
                    reason.trim()
                ),
            },
        ) {
            workshop.status = status;
            return;
        }

        workshop.status = status;
        append_gen3d_run_log(
            job.pass_dir.as_deref(),
            format!(
                "budget_stop reason={}",
                super::truncate_for_ui(reason.trim(), 600)
            ),
        );
        info!(
            "Gen3D agent: best-effort stop (regen budget exhausted). reason={:?}",
            reason.trim()
        );
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = true;
        job.phase = Gen3dAiPhase::Idle;
        job.shared_progress = None;
        job.shared_result = None;
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
