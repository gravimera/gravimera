use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT, TOOL_ID_APPLY_LAST_DRAFT_OPS,
    TOOL_ID_GET_PLAN_TEMPLATE, TOOL_ID_LLM_GENERATE_COMPONENT, TOOL_ID_LLM_GENERATE_COMPONENTS,
    TOOL_ID_LLM_GENERATE_DRAFT_OPS, TOOL_ID_LLM_GENERATE_MOTION, TOOL_ID_LLM_GENERATE_MOTIONS,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_GENERATE_PLAN_OPS, TOOL_ID_LLM_REVIEW_DELTA,
    TOOL_ID_LLM_SELECT_EDIT_STRATEGY, TOOL_ID_QA, TOOL_ID_QUERY_COMPONENT_PARTS,
    TOOL_ID_RENDER_PREVIEW, TOOL_ID_SMOKE_CHECK, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::Gen3dToolCallJsonV1;
use crate::gen3d::agent::Gen3dToolResultJsonV1;

use super::super::state::{Gen3dDraft, Gen3dWorkshop};
use super::job::Gen3dAiJob;
use super::orchestration::truncate_for_ui;

fn short_tool_id(tool_id: &str) -> &str {
    tool_id.strip_suffix("_v1").unwrap_or(tool_id)
}

fn tool_step_label(tool_id: &str, call: Option<&Gen3dToolCallJsonV1>) -> String {
    match tool_id {
        TOOL_ID_LLM_SELECT_EDIT_STRATEGY => "Edit strategy".into(),
        TOOL_ID_LLM_GENERATE_PLAN => "Plan".into(),
        TOOL_ID_GET_PLAN_TEMPLATE => "Plan template".into(),
        TOOL_ID_LLM_GENERATE_PLAN_OPS => "Plan ops".into(),
        TOOL_ID_LLM_GENERATE_COMPONENTS => "Generate components".into(),
        TOOL_ID_LLM_GENERATE_COMPONENT => {
            let component = call
                .and_then(|c| c.args.get("component"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            if let Some(component) = component {
                format!("Generate component: {component}")
            } else {
                "Generate component".into()
            }
        }
        TOOL_ID_QUERY_COMPONENT_PARTS => {
            let component = call
                .and_then(|c| c.args.get("component"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            if let Some(component) = component {
                format!("Capture parts: {component}")
            } else {
                "Capture parts".into()
            }
        }
        TOOL_ID_LLM_GENERATE_DRAFT_OPS => "Draft ops (suggest)".into(),
        TOOL_ID_APPLY_DRAFT_OPS => "Draft ops (apply)".into(),
        TOOL_ID_APPLY_LAST_DRAFT_OPS => "Draft ops (apply last)".into(),
        TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT => "Draft ops (apply from event)".into(),
        TOOL_ID_VALIDATE => "Validate".into(),
        TOOL_ID_SMOKE_CHECK => "Smoke check".into(),
        TOOL_ID_QA => "QA".into(),
        TOOL_ID_LLM_GENERATE_MOTION => {
            let channel = call
                .and_then(|c| c.args.get("channel"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty());
            if let Some(channel) = channel {
                format!("Motion: {channel}")
            } else {
                "Motion".into()
            }
        }
        TOOL_ID_LLM_GENERATE_MOTIONS => "Motion (batch)".into(),
        TOOL_ID_RENDER_PREVIEW => "Render preview".into(),
        TOOL_ID_LLM_REVIEW_DELTA => "Review delta".into(),
        other => short_tool_id(other).to_string(),
    }
}

fn tool_step_why(tool_id: &str, call: Option<&Gen3dToolCallJsonV1>) -> String {
    match tool_id {
        TOOL_ID_LLM_SELECT_EDIT_STRATEGY => {
            "Select seeded-edit strategy and DraftOps snapshot scope.".into()
        }
        TOOL_ID_LLM_GENERATE_PLAN => "Generate a component plan from the prompt.".into(),
        TOOL_ID_GET_PLAN_TEMPLATE => "Fetch a plan template for preserve-mode replanning.".into(),
        TOOL_ID_LLM_GENERATE_PLAN_OPS => "Propose a small diff to the existing plan.".into(),
        TOOL_ID_LLM_GENERATE_COMPONENTS => {
            let forced = call
                .and_then(|c| c.args.get("force"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if forced {
                "Regenerate selected components.".into()
            } else {
                let missing_only = call
                    .and_then(|c| c.args.get("missing_only"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if missing_only {
                    "Generate missing components from the plan.".into()
                } else {
                    "Generate components from the plan.".into()
                }
            }
        }
        TOOL_ID_LLM_GENERATE_COMPONENT => "Generate primitives for one component.".into(),
        TOOL_ID_QUERY_COMPONENT_PARTS => "Capture part snapshots for deterministic editing.".into(),
        TOOL_ID_LLM_GENERATE_DRAFT_OPS => "Suggest safe primitive edits (no regeneration).".into(),
        TOOL_ID_APPLY_DRAFT_OPS => "Apply DraftOps atomically (gated by assembly_rev).".into(),
        TOOL_ID_APPLY_LAST_DRAFT_OPS => {
            "Apply the latest suggested DraftOps atomically (gated by assembly_rev).".into()
        }
        TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT => {
            "Apply DraftOps from a previous suggestion event atomically (gated by assembly_rev)."
                .into()
        }
        TOOL_ID_VALIDATE => "Validate structural rules for the draft.".into(),
        TOOL_ID_SMOKE_CHECK => "Run bounded behavior/motion checks for the draft.".into(),
        TOOL_ID_QA => "Run validate + smoke checks; collect errors and warnings.".into(),
        TOOL_ID_LLM_GENERATE_MOTION => "Generate one motion channel (animation clips).".into(),
        TOOL_ID_LLM_GENERATE_MOTIONS => "Generate multiple motion channels in parallel.".into(),
        TOOL_ID_RENDER_PREVIEW => "Render images for appearance review.".into(),
        TOOL_ID_LLM_REVIEW_DELTA => "Review the draft and produce bounded fix actions.".into(),
        other => format!("Run tool `{}`.", short_tool_id(other)),
    }
}

fn summarize_tool_error(err: Option<&str>) -> String {
    let err = err.unwrap_or("").trim();
    let first_line = err.lines().next().unwrap_or("").trim();
    if first_line.is_empty() {
        "Error.".into()
    } else {
        format!("Error: {}", truncate_for_ui(first_line, 180))
    }
}

fn summarize_batch_task_counts(total: usize) -> Option<String> {
    (total > 0).then(|| format!("tasks: running 0 | queued 0 | total {total}"))
}

pub(super) fn log_tool_call_started(workshop: &mut Gen3dWorkshop, call: &Gen3dToolCallJsonV1) {
    let label = tool_step_label(call.tool_id.as_str(), Some(call));
    let why = tool_step_why(call.tool_id.as_str(), Some(call));
    workshop.status_log.start_step(label, why);
}

pub(super) fn log_tool_call_finished(
    workshop: &mut Gen3dWorkshop,
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    result: &Gen3dToolResultJsonV1,
) {
    let summary = if !result.ok {
        summarize_tool_error(result.error.as_deref())
    } else {
        match result.tool_id.as_str() {
            TOOL_ID_LLM_SELECT_EDIT_STRATEGY => {
                let strategy = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("strategy"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let scoped = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("snapshot_components"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if strategy.trim().is_empty() {
                    "OK".into()
                } else if scoped > 0 {
                    format!("OK ({strategy}, scope: {scoped})")
                } else {
                    format!("OK ({strategy})")
                }
            }
            TOOL_ID_LLM_GENERATE_PLAN => {
                let total = job.planned_components.len();
                if total > 0 {
                    format!("OK (components: {total})")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_LLM_GENERATE_COMPONENTS | TOOL_ID_LLM_GENERATE_COMPONENT => {
                let total = job.planned_components.len();
                let generated = job
                    .planned_components
                    .iter()
                    .filter(|c| c.actual_size.is_some())
                    .count();
                let batch_total = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("requested"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                if total > 0 {
                    if let Some(tasks) = summarize_batch_task_counts(batch_total) {
                        format!("OK (generated: {generated}/{total}; {tasks})")
                    } else {
                        format!("OK (generated: {generated}/{total})")
                    }
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_LLM_GENERATE_DRAFT_OPS => {
                let ops = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("ops"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if ops > 0 {
                    format!("OK (ops: {ops})")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_APPLY_DRAFT_OPS => {
                let rejected = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("rejected_ops"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if rejected > 0 {
                    format!("Rejected ops: {rejected}")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_APPLY_LAST_DRAFT_OPS | TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT => {
                let rejected = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("rejected_ops"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if rejected > 0 {
                    format!("Rejected ops: {rejected}")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_QA | TOOL_ID_VALIDATE | TOOL_ID_SMOKE_CHECK => {
                let ok = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("ok"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let errors = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("errors"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                let warnings = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("warnings"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if ok {
                    if warnings > 0 {
                        format!("OK (warnings: {warnings})")
                    } else {
                        "OK".into()
                    }
                } else {
                    format!("Not OK (errors: {errors}, warnings: {warnings})")
                }
            }
            TOOL_ID_QUERY_COMPONENT_PARTS => {
                let key = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("info_kv"))
                    .and_then(|v| v.get("key"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty());
                if let Some(key) = key {
                    format!("OK (saved: {key})")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_RENDER_PREVIEW => {
                let views = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("preview_blob_ids"))
                    .and_then(|v| v.as_array())
                    .map(|arr| arr.len())
                    .unwrap_or(0);
                if views > 0 {
                    format!("OK (views: {views})")
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_LLM_REVIEW_DELTA => {
                if let Some(reason) = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("replan_reason"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                {
                    format!("Replan: {}", truncate_for_ui(reason, 180))
                } else {
                    let regen = result
                        .result
                        .as_ref()
                        .and_then(|v| v.get("regen_component_indices"))
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.len())
                        .unwrap_or(0);
                    if regen > 0 {
                        format!("Regenerate components: {regen}")
                    } else {
                        "OK".into()
                    }
                }
            }
            TOOL_ID_LLM_GENERATE_MOTION => "OK".into(),
            TOOL_ID_LLM_GENERATE_MOTIONS => {
                let total = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("requested"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;
                let succeeded = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("succeeded"))
                    .map(|value| match value {
                        serde_json::Value::Array(items) => items.len(),
                        serde_json::Value::Number(value) => value.as_u64().unwrap_or(0) as usize,
                        _ => 0,
                    })
                    .unwrap_or(0);
                if total > 0 {
                    format!(
                        "OK (channels: {succeeded}/{total}; tasks: running 0 | queued 0 | total {total})"
                    )
                } else {
                    "OK".into()
                }
            }
            TOOL_ID_GET_PLAN_TEMPLATE | TOOL_ID_LLM_GENERATE_PLAN_OPS => "OK".into(),
            other => {
                let _ = other;
                let primitives = draft.total_primitive_parts();
                if primitives > 0 {
                    format!("OK (primitives: {primitives})")
                } else {
                    "OK".into()
                }
            }
        }
    };

    workshop.status_log.finish_step_if_active(summary);
}

pub(super) fn log_ai_request_started(workshop: &mut Gen3dWorkshop, step: &str, why: &str) {
    workshop
        .status_log
        .start_step(step.to_string(), why.to_string());
}

pub(super) fn log_note(workshop: &mut Gen3dWorkshop, message: &str) {
    let step = "Note";
    workshop
        .status_log
        .start_step(step.to_string(), message.to_string());
    workshop.status_log.finish_step("OK".to_string());
}
