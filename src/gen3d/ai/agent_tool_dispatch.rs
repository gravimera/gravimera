use bevy::prelude::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT, TOOL_ID_APPLY_LAST_DRAFT_OPS,
    TOOL_ID_APPLY_PLAN_OPS, TOOL_ID_APPLY_REUSE_GROUPS, TOOL_ID_BASIS_FROM_UP_FORWARD,
    TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_COPY_FROM_WORKSPACE,
    TOOL_ID_CREATE_WORKSPACE, TOOL_ID_DELETE_WORKSPACE, TOOL_ID_DETACH_COMPONENT,
    TOOL_ID_DIFF_SNAPSHOTS, TOOL_ID_DIFF_WORKSPACES, TOOL_ID_GET_PLAN_TEMPLATE,
    TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_USER_INPUTS, TOOL_ID_INFO_BLOBS_GET,
    TOOL_ID_INFO_BLOBS_LIST, TOOL_ID_INFO_EVENTS_GET, TOOL_ID_INFO_EVENTS_LIST,
    TOOL_ID_INFO_EVENTS_SEARCH, TOOL_ID_INFO_KV_GET, TOOL_ID_INFO_KV_GET_MANY,
    TOOL_ID_INFO_KV_GET_PAGED, TOOL_ID_INFO_KV_LIST_HISTORY, TOOL_ID_INFO_KV_LIST_KEYS,
    TOOL_ID_INSPECT_PLAN, TOOL_ID_LIST_SNAPSHOTS, TOOL_ID_LLM_GENERATE_COMPONENT,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_DRAFT_OPS, TOOL_ID_LLM_GENERATE_MOTION,
    TOOL_ID_LLM_GENERATE_MOTIONS, TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_GENERATE_PLAN_OPS,
    TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_LLM_SELECT_EDIT_STRATEGY, TOOL_ID_MERGE_WORKSPACE,
    TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_MOTION_METRICS, TOOL_ID_QA,
    TOOL_ID_QUERY_COMPONENT_PARTS, TOOL_ID_RENDER_PREVIEW, TOOL_ID_RESTORE_SNAPSHOT,
    TOOL_ID_SET_ACTIVE_WORKSPACE, TOOL_ID_SET_DESCRIPTOR_META, TOOL_ID_SMOKE_CHECK,
    TOOL_ID_SNAPSHOT, TOOL_ID_SUBMIT_TOOLING_FEEDBACK, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{Gen3dToolCallJsonV1, Gen3dToolResultJsonV1};
use crate::threaded_result::{new_shared_result, SharedResult};
use crate::types::{ActionClock, AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dWorkshop};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_parsing::{
    normalize_identifier_for_match, parse_delta_transform, resolve_component_index_by_name_hint,
};
use super::agent_regen_budget::consume_regen_budget;
use super::agent_review_delta::start_agent_llm_review_delta_call;
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_blob_ids_from_args,
    review_capture_dimensions_for_max_dim,
};
use super::agent_step::ToolCallOutcome;
use super::agent_utils::{
    build_component_subset_workspace_defs, compute_agent_state_hash, sanitize_prefix,
};
use super::artifacts::{
    append_gen3d_run_log, write_gen3d_assembly_snapshot, write_gen3d_json_artifact,
};
use super::basis_from_up_forward::basis_from_up_forward_v1;
use super::info_store::InfoPage;
use super::{
    set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob, Gen3dAiPhase, Gen3dAiProgress,
    Gen3dAiTextResponse,
};

fn normalize_tool_call_args(call: &mut Gen3dToolCallJsonV1) -> Result<(), String> {
    let args = std::mem::take(&mut call.args);
    match args {
        serde_json::Value::Null => {
            call.args = serde_json::json!({});
            Ok(())
        }
        serde_json::Value::Object(_) => {
            call.args = args;
            Ok(())
        }
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.is_empty() || text == "null" {
                call.args = serde_json::json!({});
                return Ok(());
            }

            let parsed = serde_json::from_str::<serde_json::Value>(text)
                .or_else(|_| json5::from_str::<serde_json::Value>(text))
                .map_err(|err| {
                    format!(
                        "args was a string but could not be parsed as JSON. Provide an object like `{{}}`.\nError: {err}"
                    )
                })?;
            match parsed {
                serde_json::Value::Null => {
                    call.args = serde_json::json!({});
                    Ok(())
                }
                serde_json::Value::Object(_) => {
                    call.args = parsed;
                    Ok(())
                }
                other => Err(format!(
                    "args string parsed, but was not an object (got {}). Provide an object like `{{}}`.",
                    match other {
                        serde_json::Value::Null => "null",
                        serde_json::Value::Bool(_) => "bool",
                        serde_json::Value::Number(_) => "number",
                        serde_json::Value::String(_) => "string",
                        serde_json::Value::Array(_) => "array",
                        serde_json::Value::Object(_) => "object",
                    }
                )),
            }
        }
        other => Err(format!(
            "args must be an object (or a JSON string encoding an object), got {}.",
            match other {
                serde_json::Value::Null => "null",
                serde_json::Value::Bool(_) => "bool",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::String(_) => "string",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
            }
        )),
    }
}

#[derive(Clone, Debug)]
struct Gen3dStepArtifactRef {
    attempt: u32,
    step: u32,
    path: PathBuf,
}

fn parse_prefixed_u32(name: &str, prefix: &str) -> Option<u32> {
    name.strip_prefix(prefix)?.trim().parse::<u32>().ok()
}

fn find_latest_gen3d_step_artifact(
    run_dir: &Path,
    filename: &str,
) -> Result<Gen3dStepArtifactRef, String> {
    let mut best: Option<Gen3dStepArtifactRef> = None;
    let attempts = std::fs::read_dir(run_dir).map_err(|err| {
        format!(
            "Failed to read Gen3D run_dir `{}`: {err}",
            run_dir.display()
        )
    })?;
    for attempt_entry in attempts {
        let Ok(attempt_entry) = attempt_entry else {
            continue;
        };
        let Ok(file_type) = attempt_entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        let name = attempt_entry.file_name();
        let name = name.to_string_lossy();
        let Some(attempt) = parse_prefixed_u32(name.as_ref(), "attempt_") else {
            continue;
        };
        let attempt_dir = attempt_entry.path();

        // New layout: <attempt_dir>/steps/step_####/<filename>
        let steps_dir = attempt_dir.join("steps");
        if let Ok(steps) = std::fs::read_dir(&steps_dir) {
            for step_entry in steps {
                let Ok(step_entry) = step_entry else {
                    continue;
                };
                let Ok(file_type) = step_entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = step_entry.file_name();
                let name = name.to_string_lossy();
                let Some(step) = parse_prefixed_u32(name.as_ref(), "step_") else {
                    continue;
                };
                let path = step_entry.path().join(filename);
                if !path.is_file() {
                    continue;
                }
                let is_better = match best.as_ref() {
                    None => true,
                    Some(b) => attempt > b.attempt || (attempt == b.attempt && step > b.step),
                };
                if is_better {
                    best = Some(Gen3dStepArtifactRef {
                        attempt,
                        step,
                        path,
                    });
                }
            }
        }

        // Legacy layout: <attempt_dir>/pass_####/<filename>
        if let Ok(passes) = std::fs::read_dir(&attempt_dir) {
            for pass_entry in passes {
                let Ok(pass_entry) = pass_entry else {
                    continue;
                };
                let Ok(file_type) = pass_entry.file_type() else {
                    continue;
                };
                if !file_type.is_dir() {
                    continue;
                }
                let name = pass_entry.file_name();
                let name = name.to_string_lossy();
                let Some(step) = parse_prefixed_u32(name.as_ref(), "pass_") else {
                    continue;
                };
                let path = pass_entry.path().join(filename);
                if !path.is_file() {
                    continue;
                }
                let is_better = match best.as_ref() {
                    None => true,
                    Some(b) => attempt > b.attempt || (attempt == b.attempt && step > b.step),
                };
                if is_better {
                    best = Some(Gen3dStepArtifactRef {
                        attempt,
                        step,
                        path,
                    });
                }
            }
        }
    }

    best.ok_or_else(|| {
        format!(
            "No `{filename}` artifact found under run_dir `{}`.",
            run_dir.display()
        )
    })
}

const MAX_PLAN_TEMPLATE_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlanTemplateMode {
    Auto,
    Full,
    Lean,
}

fn parse_plan_template_mode(raw: Option<&str>) -> Result<PlanTemplateMode, String> {
    let mode = raw.unwrap_or("auto").trim();
    match mode {
        "auto" => Ok(PlanTemplateMode::Auto),
        "full" => Ok(PlanTemplateMode::Full),
        "lean" => Ok(PlanTemplateMode::Lean),
        other => Err(format!(
            "Invalid mode={other:?}. Expected one of: \"auto\", \"full\", \"lean\"."
        )),
    }
}

fn plan_template_mode_label(mode: PlanTemplateMode) -> &'static str {
    match mode {
        PlanTemplateMode::Auto => "auto",
        PlanTemplateMode::Full => "full",
        PlanTemplateMode::Lean => "lean",
    }
}

fn json_compact_bytes(v: &serde_json::Value) -> usize {
    serde_json::to_vec(v).map(|v| v.len()).unwrap_or(0)
}

#[derive(Debug, Default)]
struct JsonByteLimitWriter {
    limit: usize,
    bytes: usize,
    exceeded: bool,
}

impl std::io::Write for JsonByteLimitWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.exceeded {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "byte limit exceeded",
            ));
        }

        if self.bytes.saturating_add(buf.len()) > self.limit {
            self.bytes = self.limit;
            self.exceeded = true;
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "byte limit exceeded",
            ));
        }

        self.bytes = self.bytes.saturating_add(buf.len());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn json_bytes_capped(value: &serde_json::Value, max_bytes: usize) -> Result<(usize, bool), String> {
    if max_bytes == 0 {
        return Ok((0, true));
    }

    let mut writer = JsonByteLimitWriter {
        limit: max_bytes,
        bytes: 0,
        exceeded: false,
    };
    match serde_json::to_writer(&mut writer, value) {
        Ok(()) => Ok((writer.bytes, false)),
        Err(err) => {
            if writer.exceeded {
                Ok((writer.bytes, true))
            } else {
                Err(format!("Failed to serialize JSON: {err}"))
            }
        }
    }
}

fn json_shape_preview(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Null => serde_json::Value::Null,
        serde_json::Value::Bool(v) => serde_json::Value::Bool(*v),
        serde_json::Value::Number(v) => serde_json::Value::Number(v.clone()),
        serde_json::Value::String(s) => serde_json::json!({
            "kind": "string",
            "len_bytes": s.as_bytes().len(),
        }),
        serde_json::Value::Array(arr) => serde_json::json!({
            "kind": "array",
            "len": arr.len(),
        }),
        serde_json::Value::Object(obj) => {
            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            keys.sort();
            let keys_sample: Vec<&str> = keys.into_iter().take(16).collect();
            serde_json::json!({
                "kind": "object",
                "keys_sample": keys_sample,
                "keys_total": obj.len(),
            })
        }
    }
}

fn json_pointer_escape_token(token: &str) -> String {
    let mut out = String::with_capacity(token.len());
    for ch in token.chars() {
        match ch {
            '~' => out.push_str("~0"),
            '/' => out.push_str("~1"),
            _ => out.push(ch),
        }
    }
    out
}

fn json_pointer_append(base: Option<&str>, token: &str) -> String {
    let escaped = json_pointer_escape_token(token);
    match base.map(str::trim).filter(|s| !s.is_empty()) {
        Some(prefix) => format!("{}/{}", prefix.trim_end_matches('/'), escaped),
        None => format!("/{escaped}"),
    }
}

fn info_kv_record_json(record: &super::info_store::InfoKvRecord) -> serde_json::Value {
    let mut record_json = serde_json::Map::new();
    record_json.insert("kv_rev".into(), serde_json::json!(record.kv_rev));
    record_json.insert(
        "written_at_ms".into(),
        serde_json::json!(record.written_at_ms),
    );
    record_json.insert("attempt".into(), serde_json::json!(record.attempt));
    record_json.insert("pass".into(), serde_json::json!(record.pass));
    record_json.insert(
        "assembly_rev".into(),
        serde_json::json!(record.assembly_rev),
    );
    record_json.insert(
        "workspace_id".into(),
        serde_json::Value::String(record.workspace_id.clone()),
    );
    record_json.insert(
        "key".into(),
        serde_json::json!({
            "namespace": record.key.namespace.as_str(),
            "key": record.key.key.as_str(),
        }),
    );
    record_json.insert(
        "summary".into(),
        serde_json::Value::String(record.summary.clone()),
    );
    record_json.insert("bytes".into(), serde_json::json!(record.bytes));
    if let Some(prov) = record.written_by.as_ref() {
        record_json.insert(
            "written_by".into(),
            serde_json::json!({
                "tool_id": prov.tool_id.as_str(),
                "call_id": prov.call_id.as_str(),
            }),
        );
    }
    serde_json::Value::Object(record_json)
}

fn build_info_kv_oversize_fixits(
    record: &super::info_store::InfoKvRecord,
    namespace: &str,
    key: &str,
    json_pointer: Option<&str>,
    max_bytes: usize,
    selected: &serde_json::Value,
) -> Vec<serde_json::Value> {
    const MAX_FIXITS: usize = 6;

    let mut fixits: Vec<serde_json::Value> = Vec::new();
    let selector = serde_json::json!({ "kind": "kv_rev", "kv_rev": record.kv_rev });

    match selected {
        serde_json::Value::Array(_) => {
            // Use paged reads for arrays.
            let mut args = serde_json::json!({
                "namespace": namespace,
                "key": key,
                "selector": selector,
                "page": { "limit": 50 },
                "max_item_bytes": 4096,
            });
            if let Some(ptr) = json_pointer.map(str::trim).filter(|s| !s.is_empty()) {
                if let Some(obj) = args.as_object_mut() {
                    obj.insert(
                        "json_pointer".into(),
                        serde_json::Value::String(ptr.to_string()),
                    );
                }
            }
            fixits.push(serde_json::json!({
                "tool_id": TOOL_ID_INFO_KV_GET_PAGED,
                "args": args,
                "note": "Page through the selected JSON array.",
            }));
        }
        serde_json::Value::Object(obj) => {
            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
            keys.sort();
            for k in keys.into_iter().take(4) {
                if fixits.len() >= MAX_FIXITS {
                    break;
                }
                let ptr = json_pointer_append(json_pointer, k);
                fixits.push(serde_json::json!({
                    "tool_id": TOOL_ID_INFO_KV_GET,
                    "args": {
                        "namespace": namespace,
                        "key": key,
                        "selector": selector,
                        "json_pointer": ptr,
                        "max_bytes": max_bytes,
                    },
                    "note": format!("Retry with json_pointer for top-level key `{k}`."),
                }));

                if fixits.len() >= MAX_FIXITS {
                    break;
                }
                if obj.get(k).is_some_and(|v| v.is_array()) {
                    let ptr = json_pointer_append(json_pointer, k);
                    fixits.push(serde_json::json!({
                        "tool_id": TOOL_ID_INFO_KV_GET_PAGED,
                        "args": {
                            "namespace": namespace,
                            "key": key,
                            "selector": selector,
                            "json_pointer": ptr,
                            "page": { "limit": 50 },
                            "max_item_bytes": 4096,
                        },
                        "note": format!("Page through array at json_pointer for key `{k}`."),
                    }));
                }
            }
        }
        serde_json::Value::String(_) => {
            if max_bytes < 512 * 1024 {
                fixits.push(serde_json::json!({
                    "tool_id": TOOL_ID_INFO_KV_GET,
                    "args": {
                        "namespace": namespace,
                        "key": key,
                        "selector": selector,
                        "max_bytes": 512 * 1024,
                    },
                    "note": "Retry with a larger max_bytes (clamped).",
                }));
            }
        }
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => {}
    }

    if fixits.len() > MAX_FIXITS {
        fixits.truncate(MAX_FIXITS);
    }
    fixits
}

fn strip_plan_template_modeling_notes(plan: &mut serde_json::Value) -> bool {
    let Some(components) = plan.get_mut("components").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for comp in components.iter_mut() {
        let Some(obj) = comp.as_object_mut() else {
            continue;
        };
        let notes = obj
            .entry("modeling_notes")
            .or_insert_with(|| serde_json::Value::String(String::new()));
        if notes.as_str().is_some_and(|s| !s.is_empty()) {
            changed = true;
        }
        *notes = serde_json::Value::String(String::new());
    }
    changed
}

fn strip_plan_template_contacts(plan: &mut serde_json::Value) -> bool {
    let Some(components) = plan.get_mut("components").and_then(|v| v.as_array_mut()) else {
        return false;
    };
    let mut changed = false;
    for comp in components.iter_mut() {
        let Some(obj) = comp.as_object_mut() else {
            continue;
        };
        let contacts = obj
            .entry("contacts")
            .or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if contacts.as_array().is_some_and(|arr| !arr.is_empty()) {
            changed = true;
        }
        *contacts = serde_json::Value::Array(Vec::new());
    }
    changed
}

fn strip_plan_template_assembly_notes(plan: &mut serde_json::Value) -> bool {
    let Some(obj) = plan.as_object_mut() else {
        return false;
    };
    let notes = obj
        .entry("assembly_notes")
        .or_insert_with(|| serde_json::Value::String(String::new()));
    let changed = notes.as_str().is_some_and(|s| !s.is_empty());
    *notes = serde_json::Value::String(String::new());
    changed
}

#[derive(Clone, Debug, Default)]
struct PlanTemplateScopeReport {
    scoped: bool,
    scope_components_total: usize,
    scope_components_sample: Vec<String>,
    anchors_total_full: usize,
    anchors_total: usize,
    anchors_dropped: usize,
    components_with_anchors_trimmed: usize,
}

fn is_origin_anchor_name(raw: &str) -> bool {
    let name = raw.trim();
    name.is_empty() || name == "origin"
}

fn scope_plan_template_anchors_to_components(
    plan: &mut serde_json::Value,
    scope_components: &[String],
) -> Result<PlanTemplateScopeReport, String> {
    let mut report = PlanTemplateScopeReport::default();
    let scope_components: Vec<String> = scope_components
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if scope_components.len() > 64 {
        return Err(format!(
            "scope_components is too large ({} > max 64)",
            scope_components.len()
        ));
    }

    let mut attack_muzzle: Option<(String, String)> = None;
    if let Some(attack) = plan.get("attack").and_then(|v| v.as_object()) {
        if attack.get("kind").and_then(|v| v.as_str()) == Some("ranged_projectile") {
            if let Some(muzzle) = attack.get("muzzle").and_then(|v| v.as_object()) {
                let component = muzzle
                    .get("component")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let anchor = muzzle
                    .get("anchor")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                if !component.is_empty() && !anchor.is_empty() {
                    attack_muzzle = Some((component.to_string(), anchor.to_string()));
                }
            }
        }
    }

    let Some(obj) = plan.as_object_mut() else {
        return Err("expected plan to be a JSON object".into());
    };
    let Some(components) = obj.get_mut("components").and_then(|v| v.as_array_mut()) else {
        return Err("expected plan.components to be an array".into());
    };

    let mut anchors_total_full: usize = 0;
    for comp in components.iter() {
        if let Some(arr) = comp.get("anchors").and_then(|v| v.as_array()) {
            anchors_total_full = anchors_total_full.saturating_add(arr.len());
        }
    }
    report.anchors_total_full = anchors_total_full;
    report.anchors_total = anchors_total_full;

    if scope_components.is_empty() {
        return Ok(report);
    }

    let mut available: std::collections::HashSet<String> = std::collections::HashSet::new();
    for comp in components.iter() {
        if let Some(name) = comp.get("name").and_then(|v| v.as_str()) {
            let name = name.trim();
            if !name.is_empty() {
                available.insert(name.to_string());
            }
        }
    }
    let mut missing: Vec<String> = scope_components
        .iter()
        .filter(|name| !available.contains(name.as_str()))
        .cloned()
        .collect();
    missing.sort();
    missing.dedup();
    if !missing.is_empty() {
        let mut sample: Vec<String> = available.into_iter().collect();
        sample.sort();
        sample.truncate(24);
        return Err(format!(
            "Unknown scope_components: {missing:?}. Available (sample): {sample:?}"
        ));
    }

    let scope_set: std::collections::HashSet<&str> =
        scope_components.iter().map(|s| s.as_str()).collect();

    // Compute required anchors for semantic validity of the existing plan graph.
    let mut required: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    let mut add_required = |component: &str, anchor: &str| {
        let component = component.trim();
        if component.is_empty() {
            return;
        }
        let anchor = anchor.trim();
        if is_origin_anchor_name(anchor) {
            return;
        }
        required
            .entry(component.to_string())
            .or_default()
            .insert(anchor.to_string());
    };

    for comp in components.iter() {
        let Some(comp_obj) = comp.as_object() else {
            continue;
        };
        let name = comp_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            continue;
        }

        if let Some(contacts) = comp_obj.get("contacts").and_then(|v| v.as_array()) {
            for c in contacts.iter() {
                let Some(anchor) = c.get("anchor").and_then(|v| v.as_str()) else {
                    continue;
                };
                add_required(name, anchor);
            }
        }

        let Some(att) = comp_obj.get("attach_to").and_then(|v| v.as_object()) else {
            continue;
        };
        let parent = att.get("parent").and_then(|v| v.as_str()).unwrap_or("");
        let parent_anchor = att
            .get("parent_anchor")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let child_anchor = att
            .get("child_anchor")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        add_required(name, child_anchor);
        add_required(parent, parent_anchor);
    }

    if let Some((component, anchor)) = attack_muzzle.as_ref() {
        add_required(component.as_str(), anchor.as_str());
    }

    // Count anchors before, then filter.
    let mut anchors_total: usize = 0;
    let mut components_trimmed: usize = 0;
    for comp in components.iter_mut() {
        let Some(comp_obj) = comp.as_object_mut() else {
            continue;
        };
        let name = comp_obj
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if name.is_empty() {
            continue;
        }
        if scope_set.contains(name) {
            if let Some(arr) = comp_obj.get("anchors").and_then(|v| v.as_array()) {
                anchors_total = anchors_total.saturating_add(arr.len());
            }
            continue;
        }

        let Some(arr) = comp_obj.get("anchors").and_then(|v| v.as_array()) else {
            continue;
        };
        let before = arr.len();
        let required_for_comp = required.get(name);
        let mut filtered: Vec<serde_json::Value> = Vec::new();
        for a in arr.iter() {
            let Some(anchor_name) = a.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(required_for_comp) = required_for_comp else {
                continue;
            };
            if required_for_comp.contains(anchor_name.trim()) {
                filtered.push(a.clone());
            }
        }
        let after = filtered.len();
        anchors_total = anchors_total.saturating_add(after);
        if before != after {
            components_trimmed = components_trimmed.saturating_add(1);
        }
        comp_obj.insert("anchors".into(), serde_json::Value::Array(filtered));
    }

    report.scoped = true;
    report.scope_components_total = scope_components.len();
    report.scope_components_sample = scope_components.iter().cloned().take(24).collect();
    report.anchors_total = anchors_total;
    report.anchors_dropped = anchors_total_full.saturating_sub(anchors_total);
    report.components_with_anchors_trimmed = components_trimmed;
    Ok(report)
}

#[derive(Clone, Debug)]
struct PlanTemplateFitReport {
    bytes_full: usize,
    bytes: usize,
    truncated: bool,
    omitted_fields: Vec<&'static str>,
}

fn fit_plan_template_to_budget(
    mut plan: serde_json::Value,
    mode: PlanTemplateMode,
    max_bytes: usize,
) -> Result<(serde_json::Value, PlanTemplateFitReport), String> {
    let bytes_full = json_compact_bytes(&plan);
    let mut bytes = bytes_full;
    let mut omitted_fields: Vec<&'static str> = Vec::new();

    if mode == PlanTemplateMode::Lean {
        if strip_plan_template_modeling_notes(&mut plan) {
            omitted_fields.push("components[].modeling_notes");
        }
        if strip_plan_template_contacts(&mut plan) {
            omitted_fields.push("components[].contacts");
        }
        bytes = json_compact_bytes(&plan);
    }

    if bytes > max_bytes {
        if mode == PlanTemplateMode::Full {
            return Err(format!(
                "output is too large ({bytes} bytes > max_bytes={max_bytes}). Retry with mode=\"auto\" or mode=\"lean\"."
            ));
        }

        if strip_plan_template_modeling_notes(&mut plan) {
            omitted_fields.push("components[].modeling_notes");
        }
        bytes = json_compact_bytes(&plan);
        if bytes > max_bytes {
            if strip_plan_template_contacts(&mut plan) {
                omitted_fields.push("components[].contacts");
            }
            if strip_plan_template_assembly_notes(&mut plan) {
                omitted_fields.push("assembly_notes");
            }
            bytes = json_compact_bytes(&plan);
        }
    }

    if bytes > max_bytes {
        return Err(format!(
            "output is too large ({bytes} bytes > max_bytes={max_bytes}) even after lean trimming."
        ));
    }

    omitted_fields.sort();
    omitted_fields.dedup();

    Ok((
        plan,
        PlanTemplateFitReport {
            bytes_full,
            bytes,
            truncated: !omitted_fields.is_empty(),
            omitted_fields,
        },
    ))
}

fn should_require_plan_template_kv_for_preserve_replan(
    preserve_existing_components: bool,
    planned_components_len: usize,
    plan_template_kv_present: bool,
) -> bool {
    preserve_existing_components && planned_components_len > 0 && !plan_template_kv_present
}

fn preserve_replan_missing_template_error(tool_id: &str) -> String {
    format!(
        "Preserve-mode replanning requires `plan_template_kv`. Call `{TOOL_ID_GET_PLAN_TEMPLATE}` first, then retry `{tool_id}` with the returned `plan_template_kv`."
    )
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InfoKvSelectorArgsV1 {
    kind: String,
    #[serde(default)]
    kv_rev: Option<u64>,
    #[serde(default)]
    assembly_rev: Option<u32>,
    #[serde(default)]
    pass: Option<u32>,
}

fn format_info_kv_get_many_args_error(args: &serde_json::Value, err: &serde_json::Error) -> String {
    fn selector_kind_from_value(value: &serde_json::Value) -> Option<&str> {
        value
            .as_object()?
            .get("kind")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }

    let base = err.to_string();
    let selector_kind = args
        .get("selector")
        .and_then(selector_kind_from_value)
        .or_else(|| {
            args.get("items")
                .and_then(|v| v.as_array())
                .and_then(|items| items.first())
                .and_then(|item| item.get("selector"))
                .and_then(selector_kind_from_value)
        })
        .unwrap_or("latest");

    let selector_in_items = args
        .get("items")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_object())
                .any(|item| item.contains_key("selector"))
        })
        .unwrap_or(false);

    if selector_in_items {
        let example = serde_json::json!({
            "selector": { "kind": selector_kind },
            "items": [
                { "namespace": "gen3d", "key": "ws.main.scene_graph_summary" }
            ]
        });
        return format!(
            "Invalid args for `{TOOL_ID_INFO_KV_GET_MANY}`: `selector` is shared and must be top-level (not inside `items[]`). Fix: move `selector` out of each item. Example: {}. Original error: {}",
            example,
            base
        );
    }

    format!("Invalid args for `{TOOL_ID_INFO_KV_GET_MANY}`: {base}")
}

fn select_kv_record<'a>(
    store: &'a super::info_store::Gen3dInfoStore,
    namespace: &str,
    key: &str,
    selector_kind: &str,
    selector: Option<&InfoKvSelectorArgsV1>,
) -> Result<Option<&'a super::info_store::InfoKvRecord>, String> {
    match selector_kind {
        "latest" => Ok(store.kv_latest_record(namespace, key)),
        "kv_rev" => {
            let rev = selector
                .and_then(|s| s.kv_rev)
                .ok_or_else(|| "selector.kv_rev is required when selector.kind=\"kv_rev\"".to_string())?;
            Ok(store
                .kv_record_by_rev(rev)
                .filter(|r| r.key.namespace == namespace && r.key.key == key))
        }
        "as_of_assembly_rev" => {
            let target = selector.and_then(|s| s.assembly_rev).ok_or_else(|| {
                "selector.assembly_rev is required when selector.kind=\"as_of_assembly_rev\""
                    .to_string()
            })?;
            let mut best: Option<&super::info_store::InfoKvRecord> = None;
            for rec in store.kv_records_for_key(namespace, key) {
                if rec.assembly_rev > target {
                    continue;
                }
                best = match best {
                    None => Some(rec),
                    Some(prev) => {
                        if rec.assembly_rev > prev.assembly_rev
                            || (rec.assembly_rev == prev.assembly_rev && rec.kv_rev > prev.kv_rev)
                        {
                            Some(rec)
                        } else {
                            Some(prev)
                        }
                    }
                };
            }
            Ok(best)
        }
        "as_of_pass" => {
            let target = selector.and_then(|s| s.pass).ok_or_else(|| {
                "selector.pass is required when selector.kind=\"as_of_pass\"".to_string()
            })?;
            let mut best: Option<&super::info_store::InfoKvRecord> = None;
            for rec in store.kv_records_for_key(namespace, key) {
                if rec.pass > target {
                    continue;
                }
                best = match best {
                    None => Some(rec),
                    Some(prev) => {
                        if rec.pass > prev.pass || (rec.pass == prev.pass && rec.kv_rev > prev.kv_rev) {
                            Some(rec)
                        } else {
                            Some(prev)
                        }
                    }
                };
            }
            Ok(best)
        }
        other => Err(format!(
            "Unknown selector.kind `{other}` (expected latest|kv_rev|as_of_assembly_rev|as_of_pass)."
        )),
    }
}

const INFO_KV_NAMESPACE_GEN3D: &str = "gen3d";

fn clamp_info_kv_summary(summary: String) -> String {
    const MAX_CHARS: usize = 160;
    let summary = summary.replace('\n', " ").trim().to_string();
    if summary.chars().count() <= MAX_CHARS {
        return summary;
    }
    let mut out = String::with_capacity(MAX_CHARS + 16);
    for ch in summary.chars().take(MAX_CHARS) {
        out.push(ch);
    }
    out.push_str("…");
    out
}

fn info_kv_ref_json(namespace: &str, key: &str, kv_rev: u64) -> serde_json::Value {
    serde_json::json!({
        "namespace": namespace,
        "key": key,
        "selector": { "kind": "kv_rev", "kv_rev": kv_rev },
    })
}

fn info_kv_put_for_tool(
    job: &mut Gen3dAiJob,
    workspace_id: &str,
    tool_id: &str,
    call_id: &str,
    key: &str,
    value: serde_json::Value,
    summary: String,
) -> Result<super::info_store::InfoKvRecord, String> {
    let attempt = job.attempt;
    let step = job.step;
    let assembly_rev = job.assembly_rev;
    let store = job.ensure_info_store()?;
    store.kv_put(
        attempt,
        step,
        assembly_rev,
        workspace_id,
        INFO_KV_NAMESPACE_GEN3D,
        key,
        value,
        clamp_info_kv_summary(summary),
        Some(super::info_store::InfoProvenance {
            tool_id: tool_id.to_string(),
            call_id: call_id.to_string(),
        }),
    )
}

fn run_validate_v1(job: &mut Gen3dAiJob, draft: &Gen3dDraft) -> serde_json::Value {
    let json = super::build_gen3d_validate_results(&job.planned_components, draft);
    if let Some(dir) = job.step_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "validate.json", &json);
    }
    job.agent.last_validate_ok = json.get("ok").and_then(|v| v.as_bool());
    job.agent.ever_validated = true;
    json
}

fn run_smoke_check_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) -> Result<serde_json::Value, String> {
    let json = super::build_gen3d_smoke_results(
        job.prompt_intent.as_ref().map(|i| i.requires_attack),
        !job.user_images.is_empty(),
        job.rig_move_cycle_m,
        &job.planned_components,
        draft,
    );

    job.agent.last_smoke_ok = json.get("ok").and_then(|v| v.as_bool());
    job.agent.last_motion_ok = json
        .get("motion_validation")
        .and_then(|v| v.get("ok"))
        .and_then(|v| v.as_bool());
    if let Some(dir) = job.step_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "smoke_results.json", &json);
    }
    job.agent.ever_smoke_checked = true;
    Ok(json)
}

fn build_capability_gaps_from_smoke_v1(
    job: &Gen3dAiJob,
    draft: &Gen3dDraft,
    smoke: &serde_json::Value,
) -> Vec<serde_json::Value> {
    use crate::object::registry::ColliderProfile;

    const MAX_GAPS: usize = 16;

    let mut gaps: Vec<serde_json::Value> = Vec::new();

    let root_def = draft.root_def();
    let movable = root_def.and_then(|def| def.mobility.as_ref()).is_some();

    let preserve_mode_plan_ops_fixits = |prompt_override: &str| -> Vec<serde_json::Value> {
        if prompt_override.trim().is_empty() {
            return Vec::new();
        }
        if job.planned_components.is_empty() {
            return vec![serde_json::json!({
                "tool_id": TOOL_ID_LLM_GENERATE_PLAN,
                "args": {},
            })];
        }

        let workspace_id = job.active_workspace_id().trim();
        let key = format!("ws.{workspace_id}.plan_template.preserve_mode.v1");

        vec![
            serde_json::json!({
                "tool_id": TOOL_ID_GET_PLAN_TEMPLATE,
                "args": { "version": 2, "mode": "auto" },
            }),
            serde_json::json!({
                "tool_id": TOOL_ID_LLM_GENERATE_PLAN_OPS,
                "args": {
                    "prompt": prompt_override,
                    "max_ops": 12,
                    "constraints": {
                        "preserve_existing_components": true,
                        "preserve_edit_policy": "additive",
                    },
                    "plan_template_kv": {
                        "namespace": "gen3d",
                        "key": key,
                        "selector": { "kind": "latest" },
                    },
                },
            }),
        ]
    };

    if movable {
        let has_move = job.planned_components.iter().any(|c| {
            c.attach_to.as_ref().is_some_and(|att| {
                att.animations
                    .iter()
                    .any(|slot| slot.channel.as_ref() == "move")
            })
        });
        if !has_move {
            gaps.push(serde_json::json!({
                "kind": "missing_motion_channel",
                "severity": "error",
                "message": "Movable unit has no \"move\" motion channel slots (required to finish the run).",
                "evidence": {
                    "channel": "move",
                },
                "fixits": [],
            }));
        }
    }

    if movable {
        if let Some(root_def) = root_def {
            if matches!(root_def.collider, ColliderProfile::None) {
                let fixits = preserve_mode_plan_ops_fixits(
                    "QA fix needed: movable unit has collider.kind=\"none\".\n\
Set plan.collider to a valid selection/click collider (circle_xz or aabb_xz) sized to the MAIN BODY footprint (do not inflate to cover tails/wings). Do not change components or attachment structure.",
                );
                gaps.push(serde_json::json!({
                    "kind": "missing_root_field",
                    "severity": "error",
                    "message": "Movable unit has collider.kind=\"none\" (selection/click hit area is required).",
                    "evidence": {
                        "field": "collider",
                    },
                    "fixits": fixits,
                }));
            }
        }
    }

    let attack_required_by_prompt = smoke
        .get("attack_required_by_prompt")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mobility_present = smoke
        .get("mobility_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let attack_present = smoke
        .get("attack_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if attack_required_by_prompt && (!mobility_present || !attack_present) {
        let missing_fields: Vec<&str> = [
            (!mobility_present).then_some("mobility"),
            (!attack_present).then_some("attack"),
        ]
        .into_iter()
        .flatten()
        .collect();
        let message = if missing_fields.as_slice() == ["attack"] {
            "Prompt implies the object should be attack-capable, but the draft has no root attack profile."
        } else if missing_fields.as_slice() == ["mobility"] {
            "Prompt implies the object should be attack-capable, but the draft is missing mobility."
        } else {
            "Prompt implies the object should be attack-capable, but the draft has no mobility/attack profile."
        };

        let fixits = preserve_mode_plan_ops_fixits(
            "QA fix needed: prompt requires attack capability but the root mobility/attack profile is missing.\n\
Set plan.mobility (ground/air) and plan.attack (melee or ranged_projectile) so the unit can actually attack. Do not change components or attachment structure.",
        );
        gaps.push(serde_json::json!({
            "kind": "missing_root_field",
            "severity": "error",
            "message": message,
            "evidence": {
                "attack_required_by_prompt": true,
                "mobility_present": mobility_present,
                "attack_present": attack_present,
                "missing_fields": missing_fields,
            },
            "fixits": fixits,
        }));
    }

    if attack_present && !mobility_present {
        let fixits = preserve_mode_plan_ops_fixits(
            "QA fix needed: root has an attack profile but is missing mobility.\n\
Set plan.mobility to a movable mode (ground/air) with a reasonable max_speed so the unit is movable. Do not change components or attachment structure.",
        );
        gaps.push(serde_json::json!({
            "kind": "inconsistent_root_fields",
            "severity": "error",
            "message": "Draft has an attack profile but is not movable (missing mobility).",
            "evidence": {
                "mobility_present": mobility_present,
                "attack_present": attack_present,
                "field": "mobility",
            },
            "fixits": fixits,
        }));
    }

    if let Some(motion_issues) = smoke
        .get("motion_validation")
        .and_then(|v| v.get("issues"))
        .and_then(|v| v.as_array())
    {
        for issue in motion_issues.iter() {
            if gaps.len() >= MAX_GAPS {
                break;
            }

            let issue_kind = issue
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            if issue_kind.is_empty() {
                continue;
            }

            let severity = issue
                .get("severity")
                .and_then(|v| v.as_str())
                .unwrap_or("warn")
                .trim();
            if severity != "error" {
                continue;
            }
            let message = issue
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let component_name = issue
                .get("component_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let component_id = issue
                .get("component_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let channel = issue
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let evidence = issue
                .get("evidence")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            let fixits: Vec<serde_json::Value> = Vec::new();

            gaps.push(serde_json::json!({
                "kind": "motion_validation_error",
                "severity": severity,
                "message": if message.is_empty() { issue_kind } else { message },
                "evidence": {
                    "issue_kind": issue_kind,
                    "component_name": component_name,
                    "component_id": component_id,
                    "channel": channel,
                    "evidence": evidence,
                },
                "fixits": fixits,
            }));
        }
    }

    if gaps.len() > MAX_GAPS {
        gaps.truncate(MAX_GAPS);
    }
    gaps
}

fn execute_qa_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    tool_id: &str,
    call_id: &str,
    args_value: serde_json::Value,
) -> Gen3dToolResultJsonV1 {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct QaArgsV1 {
        #[serde(default)]
        force: bool,
    }

    let args: QaArgsV1 = match serde_json::from_value(args_value) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(
                call_id.to_string(),
                tool_id.to_string(),
                format!("Invalid args for `{TOOL_ID_QA}`: {err}"),
            );
        }
    };

    let workspace_id = job.active_workspace_id().trim().to_string();
    let state_hash = compute_agent_state_hash(job, draft);
    let basis = serde_json::json!({
        "workspace_id": workspace_id.as_str(),
        "state_hash": state_hash.as_str(),
        "plan_hash": job.plan_hash.as_str(),
        "assembly_rev": job.assembly_rev,
    });

    let same_basis = job.agent.last_qa_basis_workspace_id.as_deref() == Some(workspace_id.as_str())
        && job.agent.last_qa_basis_state_hash.as_deref() == Some(state_hash.as_str());

    if !args.force && same_basis {
        if let Some(prev_json) = job.agent.last_qa_result_json.clone() {
            let mut json = prev_json;
            if let Some(obj) = json.as_object_mut() {
                obj.insert("cached".into(), serde_json::Value::Bool(true));
                obj.insert("no_new_information".into(), serde_json::Value::Bool(true));
                obj.insert("basis".into(), basis.clone());
                obj.insert(
                    "no_new_information_message".into(),
                    serde_json::Value::String(
                        "No new information: QA basis unchanged. Mutate draft/plan before retrying `qa_v1` (ex: `apply_draft_ops_v1`, `apply_plan_ops_v1`, `llm_generate_plan_v1`, `llm_generate_motions_v1`). Use force=true to bypass caching."
                            .into(),
                    ),
                );
            }
            if let Some(dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "qa.json", &json);
                let validate_json = json
                    .get("validate")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if validate_json != serde_json::Value::Null {
                    write_gen3d_json_artifact(Some(dir), "validate.json", &validate_json);
                }
                let smoke_json = json
                    .get("smoke")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null);
                if smoke_json != serde_json::Value::Null {
                    write_gen3d_json_artifact(Some(dir), "smoke_results.json", &smoke_json);
                }
            }
            return Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json);
        }
    }

    let validate = run_validate_v1(job, draft);
    let mut smoke = match run_smoke_check_v1(job, draft) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let capability_gaps = build_capability_gaps_from_smoke_v1(job, draft, &smoke);
    if let Some(obj) = smoke.as_object_mut() {
        obj.insert(
            "capability_gaps".into(),
            serde_json::Value::Array(capability_gaps.clone()),
        );
    }

    let validate_ok = validate
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let smoke_ok = smoke.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

    let mut errors: Vec<serde_json::Value> = Vec::new();
    let mut warnings: Vec<serde_json::Value> = Vec::new();
    let mut complaints: Vec<serde_json::Value> = Vec::new();

    fn push_issue(
        out: &mut Vec<serde_json::Value>,
        source: &'static str,
        issue: &serde_json::Value,
    ) {
        if let serde_json::Value::Object(map) = issue {
            let mut merged = map.clone();
            merged.insert(
                "source".to_string(),
                serde_json::Value::String(source.into()),
            );
            out.push(serde_json::Value::Object(merged));
        } else {
            out.push(serde_json::json!({ "source": source, "issue": issue }));
        }
    }

    fn collect(
        source: &'static str,
        issues: Option<&serde_json::Value>,
        errors: &mut Vec<serde_json::Value>,
        warnings: &mut Vec<serde_json::Value>,
        complaints: &mut Vec<serde_json::Value>,
    ) {
        let Some(issues) = issues else {
            return;
        };
        let Some(items) = issues.as_array() else {
            return;
        };
        for item in items {
            match item.get("severity").and_then(|v| v.as_str()) {
                Some("error") => push_issue(errors, source, item),
                Some("complaint") => {
                    push_issue(complaints, source, item);
                    push_issue(warnings, source, item);
                }
                Some("warn" | "warning") => push_issue(warnings, source, item),
                Some(_) | None => push_issue(warnings, source, item),
            }
        }
    }

    collect(
        "validate",
        validate.get("issues"),
        &mut errors,
        &mut warnings,
        &mut complaints,
    );
    collect(
        "smoke",
        smoke.get("issues"),
        &mut errors,
        &mut warnings,
        &mut complaints,
    );
    collect(
        "motion_validation",
        smoke.get("motion_validation").and_then(|v| v.get("issues")),
        &mut errors,
        &mut warnings,
        &mut complaints,
    );

    fn push_qa_complaint(
        complaints: &mut Vec<serde_json::Value>,
        warnings: &mut Vec<serde_json::Value>,
        issue: serde_json::Value,
    ) {
        let mut issue = issue;
        if let serde_json::Value::Object(map) = &mut issue {
            map.insert("source".into(), serde_json::Value::String("qa".to_string()));
        }
        complaints.push(issue.clone());
        warnings.push(issue);
    }

    // Complaint: reuse-groups missing when component naming strongly suggests repetition/symmetry.
    if job.reuse_groups.is_empty() && !job.planned_components.is_empty() {
        let mut bases: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for comp in job.planned_components.iter() {
            let name = comp.name.trim();
            if name.is_empty() {
                continue;
            }
            let mut base = name;
            if let Some(rest) = base.strip_prefix("left_") {
                base = rest;
            } else if let Some(rest) = base.strip_prefix("right_") {
                base = rest;
            } else if let Some(rest) = base.strip_suffix("_left") {
                base = rest;
            } else if let Some(rest) = base.strip_suffix("_right") {
                base = rest;
            } else if let Some((prefix, suffix)) = base.rsplit_once('_') {
                if !prefix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) {
                    base = prefix;
                }
            }

            let base = base.trim();
            if base.is_empty() || base == name {
                continue;
            }
            *bases.entry(base.to_string()).or_insert(0) += 1;
        }
        let mut candidates: Vec<(String, usize)> = bases.into_iter().collect();
        candidates.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        let candidate_bases: Vec<String> = candidates
            .iter()
            .filter(|(_, count)| *count >= 2)
            .map(|(base, _)| base.clone())
            .take(6)
            .collect();
        if !candidate_bases.is_empty() {
            push_qa_complaint(
                &mut complaints,
                &mut warnings,
                serde_json::json!({
                    "severity":"complaint",
                    "kind":"missing_reuse_groups",
                    "fix_step":"plan",
                    "message":"Plan has no `reuse_groups`, but component names suggest repetition/symmetry. Add `reuse_groups` when parts share identical/mirrored geometry (L/R limbs, repeated legs/wheels) to improve consistency and reduce weird asymmetry.",
                    "evidence": {
                        "candidate_bases": candidate_bases,
                        "reuse_groups_total": 0,
                        "components_total": job.planned_components.len(),
                    },
                }),
            );
        }
    }

    // Warning: reuse group validation produced warnings (some groups may have been ignored or normalized).
    if !job.reuse_group_warnings.is_empty() {
        let sample: Vec<String> = job.reuse_group_warnings.iter().cloned().take(6).collect();
        push_issue(
            &mut warnings,
            "qa",
            &serde_json::json!({
                "severity":"warn",
                "kind":"reuse_group_warnings",
                "fix_step":"plan",
                "message":"Plan reuse_groups produced warnings during validation; some reuse groups may be ignored or normalized.",
                "evidence": {
                    "reuse_groups_total": job.reuse_groups.len(),
                    "warnings_total": job.reuse_group_warnings.len(),
                    "warnings_sample": sample,
                },
            }),
        );
    }

    // Complaint: motion validation produced warnings after motion authoring; allow one retry to improve.
    let mobility_present = smoke
        .get("mobility_present")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let has_any_motion_slots = job.planned_components.iter().any(|c| {
        c.attach_to
            .as_ref()
            .is_some_and(|a| !a.animations.is_empty())
    });
    if mobility_present && has_any_motion_slots {
        let warn_issues: Vec<&serde_json::Value> = smoke
            .get("motion_validation")
            .and_then(|v| v.get("issues"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter(|issue| issue.get("severity").and_then(|v| v.as_str()) == Some("warn"))
                    .collect()
            })
            .unwrap_or_default();
        if !warn_issues.is_empty() {
            let mut kinds: Vec<String> = warn_issues
                .iter()
                .filter_map(|issue| issue.get("kind").and_then(|v| v.as_str()))
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            kinds.sort();
            kinds.dedup();
            kinds.truncate(6);
            push_qa_complaint(
                &mut complaints,
                &mut warnings,
                serde_json::json!({
                    "severity":"complaint",
                    "kind":"motion_quality_warnings",
                    "fix_step":"motion",
                    "message":"Motion validation produced warnings. Try one more motion-authoring pass to improve motion quality (or keep the current motion if you believe it is acceptable).",
                    "evidence": {
                        "warning_kinds": kinds,
                        "warnings_total": warn_issues.len(),
                    },
                }),
            );
        }
    }

    job.agent.last_qa_warnings_count = Some(warnings.len().min(u32::MAX as usize) as u32);
    job.agent.last_qa_warning_example = warnings.first().and_then(|issue| {
        let source = issue.get("source").and_then(|v| v.as_str()).unwrap_or("");
        let component_name = issue
            .get("component_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let kind = issue.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let message = issue.get("message").and_then(|v| v.as_str()).unwrap_or("");

        let mut example = String::new();
        if !source.trim().is_empty() {
            example.push_str(source.trim());
            example.push(' ');
        }
        if !component_name.trim().is_empty() {
            example.push_str(component_name.trim());
            example.push(' ');
        }
        if !kind.trim().is_empty() {
            example.push_str(kind.trim());
            example.push_str(": ");
        }
        example.push_str(message.trim());

        let example = example.trim();
        if example.is_empty() {
            None
        } else {
            Some(example.replace('\n', " "))
        }
    });

    let ok = validate_ok && smoke_ok;
    let mut json = serde_json::json!({
        "ok": ok,
        "validate": validate,
        "smoke": smoke,
        "errors": errors,
        "warnings": warnings,
        "complaints": complaints,
        "cached": false,
        "no_new_information": false,
        "basis": basis,
        "capability_gaps": capability_gaps,
    });

    if let Some(dir) = job.step_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "qa.json", &json);
    }

    // Also persist validate/smoke individually so agents can fetch them via stable keys even
    // when they only run `qa_v1`.
    let validate_json = json
        .get("validate")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let smoke_json = json
        .get("smoke")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let validate_key = format!("ws.{workspace_id}.validate");
    let smoke_key = format!("ws.{workspace_id}.smoke");
    let qa_key = format!("ws.{workspace_id}.qa");

    if validate_json != serde_json::Value::Null {
        let validate_ok = validate_json
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let validate_issues = validate_json
            .get("issues")
            .and_then(|v| v.as_array())
            .map(|v| v.len())
            .unwrap_or(0);
        if let Err(err) = info_kv_put_for_tool(
            job,
            workspace_id.as_str(),
            tool_id,
            call_id,
            validate_key.as_str(),
            validate_json,
            format!("validate (ok={validate_ok} issues={validate_issues})"),
        ) {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    }
    if smoke_json != serde_json::Value::Null {
        let smoke_ok = smoke_json
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let smoke_issues = smoke_json
            .get("issues")
            .and_then(|v| v.as_array())
            .map(|v| v.len())
            .unwrap_or(0);
        if let Err(err) = info_kv_put_for_tool(
            job,
            workspace_id.as_str(),
            tool_id,
            call_id,
            smoke_key.as_str(),
            smoke_json,
            format!("smoke (ok={smoke_ok} issues={smoke_issues})"),
        ) {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    }

    let error_count = json
        .get("errors")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    let warning_count = json
        .get("warnings")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    let complaint_count = json
        .get("complaints")
        .and_then(|v| v.as_array())
        .map(|v| v.len())
        .unwrap_or(0);
    let qa_record = match info_kv_put_for_tool(
        job,
        workspace_id.as_str(),
        tool_id,
        call_id,
        qa_key.as_str(),
        json.clone(),
        format!(
            "qa (ok={ok} errors={error_count} warnings={warning_count} complaints={complaint_count})"
        ),
    ) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };
    if let Some(obj) = json.as_object_mut() {
        obj.insert(
            "info_kv".into(),
            info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, qa_key.as_str(), qa_record.kv_rev),
        );
    }

    job.agent.last_qa_basis_workspace_id = Some(workspace_id);
    job.agent.last_qa_basis_state_hash = Some(state_hash);
    job.agent.last_qa_result_json = Some(json.clone());

    Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json)
}

fn execute_info_kv_get_v1(
    job: &mut Gen3dAiJob,
    tool_id: &str,
    call_id: &str,
    args_value: serde_json::Value,
) -> Gen3dToolResultJsonV1 {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InfoKvGetArgsV1 {
        namespace: String,
        key: String,
        #[serde(default)]
        selector: Option<InfoKvSelectorArgsV1>,
        #[serde(default)]
        json_pointer: Option<String>,
        #[serde(default)]
        max_bytes: Option<u64>,
    }

    let args: InfoKvGetArgsV1 = match serde_json::from_value(args_value) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(
                call_id.to_string(),
                tool_id.to_string(),
                format!("Invalid args for `{TOOL_ID_INFO_KV_GET}`: {err}"),
            );
        }
    };

    let namespace = args.namespace.trim();
    let key = args.key.trim();
    if namespace.is_empty() {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Missing args.namespace".into(),
        );
    }
    if key.is_empty() {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Missing args.key".into(),
        );
    }

    let max_bytes = args.max_bytes.unwrap_or(64 * 1024).clamp(1024, 512 * 1024) as usize;

    let selector_kind = args
        .selector
        .as_ref()
        .map(|s| s.kind.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "latest".into());

    if let Err(err) = job.ensure_info_store() {
        return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
    }
    let Some(store) = job.info_store.as_ref() else {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Internal error: missing info_store after ensure_info_store.".into(),
        );
    };

    let record = match select_kv_record(
        store,
        namespace,
        key,
        selector_kind.as_str(),
        args.selector.as_ref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let Some(record) = record else {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            format!(
                "KV not found: namespace={namespace:?} key={key:?}. Use `{TOOL_ID_INFO_KV_LIST_KEYS}`."
            ),
        );
    };

    let json_pointer = args
        .json_pointer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    // Cache key is based on the resolved KV record (kv_rev) so selector.kind="latest" is safe.
    let cache_key = {
        let params_sig = store.stable_params_sig(&serde_json::json!({
            "namespace": namespace,
            "key": key,
            "kv_rev": record.kv_rev,
            "json_pointer": json_pointer.as_deref().unwrap_or(""),
            "max_bytes": max_bytes,
        }));
        format!("{TOOL_ID_INFO_KV_GET}|{params_sig}")
    };

    if let Some(prev_json) = job
        .agent
        .info_store_inspection_cache
        .get(&cache_key)
        .cloned()
    {
        let mut json = prev_json;
        if let Some(obj) = json.as_object_mut() {
            obj.insert("cached".into(), serde_json::Value::Bool(true));
            obj.insert("no_new_information".into(), serde_json::Value::Bool(true));
            obj.insert(
                "no_new_information_message".into(),
                serde_json::Value::String(
                    "No new information: same KV record (kv_rev) and json_pointer in this pass. Use `info_kv_list_history_v1` to browse revisions, or wait for a mutation to write a new revision."
                        .into(),
                ),
            );
        }
        return Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json);
    }

    let selected = if let Some(ptr) = json_pointer.as_deref() {
        record
            .value
            .pointer(ptr)
            .ok_or_else(|| format!("JSON pointer not found in KV value: {ptr}"))
    } else {
        Ok(&record.value)
    };
    let selected = match selected {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let size_limit = max_bytes.saturating_add(1);
    let (selected_bytes, _) = match json_bytes_capped(selected, size_limit) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };
    if selected_bytes > max_bytes {
        let shape_preview = json_shape_preview(selected);
        let fixits = build_info_kv_oversize_fixits(
            record,
            namespace,
            key,
            json_pointer.as_deref(),
            max_bytes,
            selected,
        );
        let mut diag = serde_json::Map::new();
        diag.insert(
            "kind".into(),
            serde_json::Value::String("kv_value_too_large".into()),
        );
        diag.insert("record".into(), info_kv_record_json(record));
        diag.insert("max_bytes".into(), serde_json::json!(max_bytes));
        diag.insert(
            "selected_bytes_capped".into(),
            serde_json::json!(selected_bytes),
        );
        if let Some(ptr) = json_pointer.as_deref() {
            diag.insert(
                "json_pointer".into(),
                serde_json::Value::String(ptr.to_string()),
            );
        }
        diag.insert("shape_preview".into(), shape_preview.clone());
        diag.insert("fixits".into(), serde_json::Value::Array(fixits));

        let mut msg = format!(
            "KV value is too large (selected_bytes > max_bytes={max_bytes}). Use `json_pointer` to select a smaller subset."
        );
        if !shape_preview.is_null() {
            msg.push_str(" shape_preview=");
            msg.push_str(&shape_preview.to_string());
        }

        return Gen3dToolResultJsonV1::err_with_result(
            call_id.to_string(),
            tool_id.to_string(),
            msg,
            serde_json::Value::Object(diag),
        );
    }

    let mut out = serde_json::Map::new();
    out.insert("ok".into(), serde_json::Value::Bool(true));
    out.insert("record".into(), info_kv_record_json(record));
    out.insert("value".into(), selected.clone());
    out.insert("truncated".into(), serde_json::Value::Bool(false));
    out.insert("cached".into(), serde_json::Value::Bool(false));
    out.insert("no_new_information".into(), serde_json::Value::Bool(false));
    if let Some(ptr) = json_pointer {
        out.insert("json_pointer".into(), serde_json::Value::String(ptr));
    }
    let json = serde_json::Value::Object(out);
    job.agent
        .info_store_inspection_cache
        .insert(cache_key, json.clone());
    Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json)
}

fn execute_info_kv_get_many_v1(
    job: &mut Gen3dAiJob,
    tool_id: &str,
    call_id: &str,
    args_value: serde_json::Value,
) -> Gen3dToolResultJsonV1 {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InfoKvGetManyItemArgsV1 {
        namespace: String,
        key: String,
        #[serde(default)]
        json_pointer: Option<String>,
        #[serde(default)]
        max_bytes: Option<u64>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InfoKvGetManyArgsV1 {
        items: Vec<InfoKvGetManyItemArgsV1>,
        #[serde(default)]
        selector: Option<InfoKvSelectorArgsV1>,
        #[serde(default)]
        max_items: Option<u32>,
    }

    let args_value_for_error = args_value.clone();
    let args: InfoKvGetManyArgsV1 = match serde_json::from_value(args_value.clone()) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(
                call_id.to_string(),
                tool_id.to_string(),
                format_info_kv_get_many_args_error(&args_value_for_error, &err),
            );
        }
    };

    let max_items = args.max_items.unwrap_or(20).clamp(1, 50) as usize;
    let mut truncated = false;
    let mut requested = args.items;
    if requested.len() > max_items {
        requested.truncate(max_items);
        truncated = true;
    }

    let selector_kind = args
        .selector
        .as_ref()
        .map(|s| s.kind.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "latest".into());

    if let Err(err) = job.ensure_info_store() {
        return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
    }
    let Some(store) = job.info_store.as_ref() else {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Internal error: missing info_store after ensure_info_store.".into(),
        );
    };

    struct ResolvedItem<'a> {
        namespace: String,
        key: String,
        json_pointer: Option<String>,
        max_bytes: usize,
        record: Option<&'a super::info_store::InfoKvRecord>,
    }

    let selector_ref = args.selector.as_ref();
    let mut resolved: Vec<ResolvedItem<'_>> = Vec::with_capacity(requested.len());
    for item in requested {
        let namespace = item.namespace.trim().to_string();
        let key = item.key.trim().to_string();
        let json_pointer = item
            .json_pointer
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let max_bytes = item.max_bytes.unwrap_or(64 * 1024).clamp(1024, 512 * 1024) as usize;

        let record = if namespace.is_empty() || key.is_empty() {
            None
        } else {
            match select_kv_record(
                store,
                namespace.as_str(),
                key.as_str(),
                selector_kind.as_str(),
                selector_ref,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return Gen3dToolResultJsonV1::err(
                        call_id.to_string(),
                        tool_id.to_string(),
                        err,
                    );
                }
            }
        };

        resolved.push(ResolvedItem {
            namespace,
            key,
            json_pointer,
            max_bytes,
            record,
        });
    }

    let cache_key = {
        let items_sig: Vec<serde_json::Value> = resolved
            .iter()
            .map(|item| {
                serde_json::json!({
                    "namespace": item.namespace.as_str(),
                    "key": item.key.as_str(),
                    "kv_rev": item.record.map(|r| r.kv_rev),
                    "json_pointer": item.json_pointer.as_deref().unwrap_or(""),
                    "max_bytes": item.max_bytes,
                })
            })
            .collect();
        let params_sig = store.stable_params_sig(&serde_json::json!({
            "items": items_sig,
            "truncated": truncated,
        }));
        format!("{TOOL_ID_INFO_KV_GET_MANY}|{params_sig}")
    };

    if let Some(prev_json) = job
        .agent
        .info_store_inspection_cache
        .get(&cache_key)
        .cloned()
    {
        let mut json = prev_json;
        if let Some(obj) = json.as_object_mut() {
            obj.insert("cached".into(), serde_json::Value::Bool(true));
            obj.insert("no_new_information".into(), serde_json::Value::Bool(true));
            obj.insert(
                "no_new_information_message".into(),
                serde_json::Value::String(
                    "No new information: identical KV reads (resolved kv_rev + json_pointer) already returned in this pass."
                        .into(),
                ),
            );
        }
        return Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json);
    }

    let mut out_items: Vec<serde_json::Value> = Vec::with_capacity(resolved.len());
    for item in resolved {
        if item.namespace.is_empty() || item.key.is_empty() {
            out_items.push(serde_json::json!({
                "namespace": item.namespace,
                "key": item.key,
                "ok": false,
                "error": "Missing namespace/key.",
            }));
            continue;
        }

        let Some(record) = item.record else {
            out_items.push(serde_json::json!({
                "namespace": item.namespace,
                "key": item.key,
                "ok": false,
                "error": "KV not found for selector.",
            }));
            continue;
        };

        let selected = if let Some(ptr) = item.json_pointer.as_deref() {
            match record.value.pointer(ptr) {
                Some(v) => Ok(v),
                None => Err(format!("JSON pointer not found: {ptr}")),
            }
        } else {
            Ok(&record.value)
        };
        let selected = match selected {
            Ok(v) => v,
            Err(err) => {
                out_items.push(serde_json::json!({
                    "namespace": item.namespace,
                    "key": item.key,
                    "ok": false,
                    "record": info_kv_record_json(record),
                    "error": err,
                }));
                continue;
            }
        };

        let size_limit = item.max_bytes.saturating_add(1);
        let (selected_bytes, _) = match json_bytes_capped(selected, size_limit) {
            Ok(v) => v,
            Err(err) => {
                out_items.push(serde_json::json!({
                    "namespace": item.namespace,
                    "key": item.key,
                    "ok": false,
                    "record": info_kv_record_json(record),
                    "error": err,
                }));
                continue;
            }
        };

        if selected_bytes > item.max_bytes {
            let shape_preview = json_shape_preview(selected);
            let fixits = build_info_kv_oversize_fixits(
                record,
                item.namespace.as_str(),
                item.key.as_str(),
                item.json_pointer.as_deref(),
                item.max_bytes,
                selected,
            );
            out_items.push(serde_json::json!({
                "namespace": item.namespace,
                "key": item.key,
                "ok": false,
                "record": info_kv_record_json(record),
                "json_pointer": item.json_pointer,
                "error_kind": "kv_value_too_large",
                "error": format!("KV value is too large (selected_bytes > max_bytes={}). Use json_pointer.", item.max_bytes),
                "max_bytes": item.max_bytes,
                "selected_bytes_capped": selected_bytes,
                "shape_preview": shape_preview,
                "fixits": fixits,
            }));
            continue;
        }

        let mut out = serde_json::Map::new();
        out.insert(
            "namespace".into(),
            serde_json::Value::String(item.namespace),
        );
        out.insert("key".into(), serde_json::Value::String(item.key));
        out.insert("ok".into(), serde_json::Value::Bool(true));
        out.insert("record".into(), info_kv_record_json(record));
        out.insert("value".into(), selected.clone());
        out.insert("truncated".into(), serde_json::Value::Bool(false));
        if let Some(ptr) = item.json_pointer {
            out.insert("json_pointer".into(), serde_json::Value::String(ptr));
        }
        out_items.push(serde_json::Value::Object(out));
    }

    let json = serde_json::json!({
        "ok": true,
        "items": out_items,
        "truncated": truncated,
        "cached": false,
        "no_new_information": false,
    });
    job.agent
        .info_store_inspection_cache
        .insert(cache_key, json.clone());
    Gen3dToolResultJsonV1::ok(call_id.to_string(), tool_id.to_string(), json)
}

fn execute_info_kv_get_paged_v1(
    job: &mut Gen3dAiJob,
    tool_id: &str,
    call_id: &str,
    args_value: serde_json::Value,
) -> Gen3dToolResultJsonV1 {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct InfoKvGetPagedArgsV1 {
        namespace: String,
        key: String,
        #[serde(default)]
        selector: Option<InfoKvSelectorArgsV1>,
        #[serde(default)]
        json_pointer: Option<String>,
        #[serde(default)]
        page: Option<InfoPage>,
        #[serde(default)]
        max_item_bytes: Option<u64>,
    }

    let args: InfoKvGetPagedArgsV1 = match serde_json::from_value(args_value) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(
                call_id.to_string(),
                tool_id.to_string(),
                format!("Invalid args for `{TOOL_ID_INFO_KV_GET_PAGED}`: {err}"),
            );
        }
    };

    let namespace = args.namespace.trim();
    let key = args.key.trim();
    if namespace.is_empty() {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Missing args.namespace".into(),
        );
    }
    if key.is_empty() {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Missing args.key".into(),
        );
    }

    let max_item_bytes = args.max_item_bytes.unwrap_or(4096).clamp(256, 64 * 1024) as usize;

    let selector_kind = args
        .selector
        .as_ref()
        .map(|s| s.kind.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "latest".into());

    let store = match job.ensure_info_store() {
        Ok(s) => s,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let record = match select_kv_record(
        store,
        namespace,
        key,
        selector_kind.as_str(),
        args.selector.as_ref(),
    ) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let Some(record) = record else {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            format!(
                "KV not found: namespace={namespace:?} key={key:?}. Use `{TOOL_ID_INFO_KV_LIST_KEYS}`."
            ),
        );
    };

    let json_pointer = args
        .json_pointer
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let selected = if let Some(ptr) = json_pointer.as_deref() {
        record
            .value
            .pointer(ptr)
            .ok_or_else(|| format!("JSON pointer not found in KV value: {ptr}"))
    } else {
        Ok(&record.value)
    };
    let selected: &serde_json::Value = match selected {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let Some(items) = selected.as_array() else {
        return Gen3dToolResultJsonV1::err(
            call_id.to_string(),
            tool_id.to_string(),
            "Selected JSON value is not an array. Provide json_pointer to an array (example: \"/errors\" or \"/items\")."
                .into(),
        );
    };

    let params_sig = store.stable_params_sig(&serde_json::json!({
        "tool_id": TOOL_ID_INFO_KV_GET_PAGED,
        "namespace": namespace,
        "key": key,
        "kv_rev": record.kv_rev,
        "json_pointer": json_pointer.clone().unwrap_or_default(),
        "max_item_bytes": max_item_bytes,
    }));
    let (limit, offset) = match store.page_from_args(
        TOOL_ID_INFO_KV_GET_PAGED,
        params_sig.as_str(),
        args.page.as_ref(),
        50,
        200,
    ) {
        Ok(v) => v,
        Err(err) => {
            return Gen3dToolResultJsonV1::err(call_id.to_string(), tool_id.to_string(), err);
        }
    };

    let array_len = items.len();
    let end = (offset + limit).min(array_len);
    let truncated = end < array_len;
    let next_cursor =
        truncated.then(|| store.offset_cursor(TOOL_ID_INFO_KV_GET_PAGED, params_sig.as_str(), end));

    let mut out_items: Vec<serde_json::Value> = Vec::new();
    if offset < array_len {
        out_items.reserve(end.saturating_sub(offset));
        for idx in offset..end {
            let item = &items[idx];
            let (bytes, _) = match json_bytes_capped(item, max_item_bytes.saturating_add(1)) {
                Ok(v) => v,
                Err(err) => {
                    return Gen3dToolResultJsonV1::err(
                        call_id.to_string(),
                        tool_id.to_string(),
                        err,
                    );
                }
            };
            let item_truncated = bytes > max_item_bytes;
            let value_preview = if item_truncated {
                json_shape_preview(item)
            } else {
                item.clone()
            };
            out_items.push(serde_json::json!({
                "index": idx,
                "bytes": bytes,
                "truncated": item_truncated,
                "value_preview": value_preview,
            }));
        }
    }

    let mut out = serde_json::Map::new();
    out.insert("ok".into(), serde_json::Value::Bool(true));
    let mut record_json = serde_json::Map::new();
    record_json.insert("kv_rev".into(), serde_json::json!(record.kv_rev));
    record_json.insert(
        "written_at_ms".into(),
        serde_json::json!(record.written_at_ms),
    );
    record_json.insert("attempt".into(), serde_json::json!(record.attempt));
    record_json.insert("pass".into(), serde_json::json!(record.pass));
    record_json.insert(
        "assembly_rev".into(),
        serde_json::json!(record.assembly_rev),
    );
    record_json.insert(
        "workspace_id".into(),
        serde_json::Value::String(record.workspace_id.clone()),
    );
    record_json.insert(
        "key".into(),
        serde_json::json!({
            "namespace": record.key.namespace.as_str(),
            "key": record.key.key.as_str(),
        }),
    );
    record_json.insert(
        "summary".into(),
        serde_json::Value::String(record.summary.clone()),
    );
    record_json.insert("bytes".into(), serde_json::json!(record.bytes));
    if let Some(prov) = record.written_by.as_ref() {
        record_json.insert(
            "written_by".into(),
            serde_json::json!({
                "tool_id": prov.tool_id.as_str(),
                "call_id": prov.call_id.as_str(),
            }),
        );
    }
    out.insert("record".into(), serde_json::Value::Object(record_json));
    out.insert("array_len".into(), serde_json::json!(array_len));
    out.insert("items".into(), serde_json::Value::Array(out_items));
    out.insert("truncated".into(), serde_json::Value::Bool(truncated));
    if let Some(cursor) = next_cursor {
        out.insert("next_cursor".into(), serde_json::Value::String(cursor));
    }
    if let Some(ptr) = json_pointer {
        out.insert("json_pointer".into(), serde_json::Value::String(ptr));
    }

    Gen3dToolResultJsonV1::ok(
        call_id.to_string(),
        tool_id.to_string(),
        serde_json::Value::Object(out),
    )
}

pub(super) fn execute_tool_call(
    config: &AppConfig,
    _time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    _preview: &mut Gen3dPreview,
    _preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
    call: Gen3dToolCallJsonV1,
) -> ToolCallOutcome {
    let mut call = call;
    if let Err(err) = normalize_tool_call_args(&mut call) {
        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
            call.call_id,
            call.tool_id,
            err,
        ));
    }

    match call.tool_id.as_str() {
        TOOL_ID_BASIS_FROM_UP_FORWARD => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct BasisFromUpForwardArgsV1 {
                #[serde(default)]
                version: u32,
                up: [f32; 3],
                #[serde(default)]
                forward_hint: Option<[f32; 3]>,
            }

            let mut args: BasisFromUpForwardArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Invalid args for `{TOOL_ID_BASIS_FROM_UP_FORWARD}`: {err}. Expected: {{ up:[x,y,z], forward_hint?:[x,y,z] }}."
                        ),
                    ));
                }
            };
            if args.version == 0 {
                args.version = 1;
            }
            if args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_BASIS_FROM_UP_FORWARD}` version {}.",
                        args.version
                    ),
                ));
            }

            let up_in = Vec3::new(args.up[0], args.up[1], args.up[2]);
            let forward_hint = args
                .forward_hint
                .map(|arr| Vec3::new(arr[0], arr[1], arr[2]));
            let basis = match basis_from_up_forward_v1(up_in, forward_hint) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("`{TOOL_ID_BASIS_FROM_UP_FORWARD}`: {err}"),
                    ));
                }
            };

            let json = serde_json::json!({
                "version": 1,
                "input": {
                    "up": args.up,
                    "forward_hint": args.forward_hint,
                },
                "forward": [basis.forward.x, basis.forward.y, basis.forward.z],
                "up": [basis.up.x, basis.up.y, basis.up.z],
                "right": [basis.right.x, basis.right.y, basis.right.z],
                "forward_source": basis.forward_source,
                "fallback_axis": basis.fallback_axis,
                "notes": basis.notes,
            });

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_GET_USER_INPUTS => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "prompt": job.user_prompt_raw,
                "reference_images_count": job.user_images.len(),
                "image_object_summary": job.user_image_object_summary.as_ref().map(|s| s.text.clone()),
                "image_object_summary_word_count": job.user_image_object_summary.as_ref().map(|s| s.word_count),
                "image_object_summary_truncated": job.user_image_object_summary.as_ref().map(|s| s.truncated),
            }),
        )),
        TOOL_ID_SET_DESCRIPTOR_META => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct SetDescriptorMetaArgsV1 {
                #[serde(default)]
                version: u32,
                #[serde(default)]
                name: Option<String>,
                #[serde(default)]
                short: Option<String>,
                #[serde(default)]
                tags: Option<Vec<String>>,
            }

            fn canonicalize_name(name: String) -> String {
                name.trim()
                    .split_whitespace()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ")
            }

            fn canonicalize_tags(mut tags: Vec<String>) -> Vec<String> {
                tags = tags
                    .into_iter()
                    .map(|v| v.trim().to_string())
                    .filter(|v| !v.is_empty())
                    .collect();
                tags.sort();
                tags.dedup();
                tags
            }

            let args: SetDescriptorMetaArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_SET_DESCRIPTOR_META}`: {err}"),
                    ));
                }
            };

            if args.version != 0 && args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_SET_DESCRIPTOR_META}` version {}.",
                        args.version
                    ),
                ));
            }

            let mut meta = job
                .descriptor_meta_for_save()
                .map(|(_, meta)| meta.clone())
                .unwrap_or_else(|| super::schema::AiDescriptorMetaJsonV1 {
                    version: 1,
                    name: String::new(),
                    short: String::new(),
                    tags: Vec::new(),
                });

            if let Some(name) = args.name {
                meta.name = canonicalize_name(name);
            }
            if let Some(short) = args.short {
                meta.short = short.trim().to_string();
            }
            if let Some(tags) = args.tags {
                meta.tags = canonicalize_tags(tags);
            }
            meta.version = 1;

            job.descriptor_meta_override = Some(meta.clone());

            if let Some(dir) = job.step_dir.as_deref() {
                let name = meta.name.clone();
                let short = meta.short.clone();
                let tags = meta.tags.clone();
                write_gen3d_json_artifact(
                    Some(dir),
                    "descriptor_meta_override.json",
                    &serde_json::json!({
                        "version": 1,
                        "name": name,
                        "short": short,
                        "tags": tags,
                    }),
                );
            }
            append_gen3d_run_log(
                job.step_dir.as_deref(),
                format!(
                    "descriptor_meta_override_set name_chars={} short_chars={} tags={}",
                    meta.name.chars().count(),
                    meta.short.chars().count(),
                    meta.tags.len()
                ),
            );

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "version": 1,
                    "name": meta.name,
                    "short": meta.short,
                    "tags": meta.tags,
                }),
            ))
        }
        TOOL_ID_GET_SCENE_GRAPH_SUMMARY => {
            let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
            let mut json = super::build_gen3d_scene_graph_summary(
                &run_id,
                job.attempt,
                job.step,
                &job.plan_hash,
                job.assembly_rev,
                &job.planned_components,
                draft,
            );
            if let Some(dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "scene_graph_summary.json", &json);
            }

            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.scene_graph_summary");
            let components_total = json
                .get("components_total")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("scene graph summary (components={components_total})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_INSPECT_PLAN => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InspectPlanArgsV1 {
                #[serde(default)]
                version: u32,
            }

            let args: InspectPlanArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INSPECT_PLAN}`: {err}"),
                    ));
                }
            };
            if args.version != 0 && args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_INSPECT_PLAN}` version {}.",
                        args.version
                    ),
                ));
            }

            let json = super::plan_tools::inspect_pending_plan_attempt_v1(
                job.pending_plan_attempt.as_ref(),
                &job.planned_components,
                job.preserve_existing_components_mode,
            );
            if let Some(dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "plan_inspect.json", &json);
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_GET_PLAN_TEMPLATE => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct GetPlanTemplateArgsV1 {
                #[serde(default)]
                version: u32,
                #[serde(default)]
                mode: Option<String>,
                #[serde(default)]
                max_bytes: Option<u32>,
                #[serde(default)]
                scope_components: Vec<String>,
            }

            let args: GetPlanTemplateArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_GET_PLAN_TEMPLATE}`: {err}"),
                    ));
                }
            };
            if args.version != 0 && args.version != 1 && args.version != 2 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_GET_PLAN_TEMPLATE}` version {}.",
                        args.version
                    ),
                ));
            }

            let mode = match parse_plan_template_mode(args.mode.as_deref()) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid `{TOOL_ID_GET_PLAN_TEMPLATE}` args: {err}"),
                    ));
                }
            };

            let max_bytes = args
                .max_bytes
                .unwrap_or(MAX_PLAN_TEMPLATE_BYTES as u32)
                .clamp(1024, MAX_PLAN_TEMPLATE_BYTES as u32) as usize;

            let Some(step_dir) = job.step_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir.".into(),
                ));
            };

            let plan = match super::plan_tools::build_preserve_mode_plan_template_json_v8(
                draft,
                &job.planned_components,
                &job.assembly_notes,
                job.plan_collider.as_ref(),
                job.rig_move_cycle_m,
                &job.reuse_groups,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let mut plan = plan;
            let scope_report = match scope_plan_template_anchors_to_components(
                &mut plan,
                args.scope_components.as_slice(),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid `{TOOL_ID_GET_PLAN_TEMPLATE}` args: {err}"),
                    ));
                }
            };

            let (plan, fit) = match fit_plan_template_to_budget(plan, mode, max_bytes) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("`{TOOL_ID_GET_PLAN_TEMPLATE}` {err} Disable preserve mode for replanning (constraints.preserve_existing_components=false) or simplify the assembly before templating."),
                    ));
                }
            };

            let bytes_full = fit.bytes_full;
            let bytes = fit.bytes;
            let truncated = fit.truncated;
            let omitted_fields = fit.omitted_fields;

            let filename = format!("plan_template_{}.json", sanitize_prefix(&call.call_id));
            write_gen3d_json_artifact(Some(step_dir), &filename, &plan);

            let attempt = job.attempt;
            let step = job.step;
            let assembly_rev = job.assembly_rev;
            let workspace_id = job.active_workspace_id().trim().to_string();

            let components_total = plan
                .get("components")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            let namespace = "gen3d";
            let key = format!("ws.{workspace_id}.plan_template.preserve_mode.v1");
            let record = match job.ensure_info_store() {
                Ok(store) => store.kv_put(
                    attempt,
                    step,
                    assembly_rev,
                    workspace_id.as_str(),
                    namespace,
                    key.as_str(),
                    plan.clone(),
                    format!(
                        "plan template preserve_mode v1 (components={components_total}, bytes={bytes}, truncated={truncated})"
                    ),
                    Some(super::info_store::InfoProvenance {
                        tool_id: call.tool_id.clone(),
                        call_id: call.call_id.clone(),
                    }),
                ),
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Failed to open Info Store: {err}"),
                    ))
                }
            };
            let record = match record {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ))
                }
            };

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                        "version": 2,
                        "plan_template_kv": {
                            "namespace": namespace,
                            "key": key,
                            "selector": { "kind": "kv_rev", "kv_rev": record.kv_rev },
                        },
                        "mode": plan_template_mode_label(mode),
                        "max_bytes": max_bytes,
                        "bytes": record.bytes,
                        "bytes_full": bytes_full,
                        "truncated": truncated,
                    "omitted_fields": omitted_fields,
                    "components_total": components_total,
                    "scoped": scope_report.scoped,
                    "scope_components_total": scope_report.scope_components_total,
                    "scope_components_sample": scope_report.scope_components_sample,
                    "anchors_total_full": scope_report.anchors_total_full,
                    "anchors_total": scope_report.anchors_total,
                    "anchors_dropped": scope_report.anchors_dropped,
                    "components_with_anchors_trimmed": scope_report.components_with_anchors_trimmed,
                }),
            ))
        }
        TOOL_ID_QUERY_COMPONENT_PARTS => {
            let mut json = match super::draft_ops::query_component_parts_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let workspace_id = job.active_workspace_id().trim().to_string();
            let component_name = json
                .get("component")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let component_seg = normalize_identifier_for_match(component_name);
            let component_seg = if component_seg.is_empty() {
                "unknown".to_string()
            } else {
                component_seg
            };
            let key = format!("ws.{workspace_id}.component_parts.{component_seg}");
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("component parts (component={component_seg})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_VALIDATE => {
            let mut json = run_validate_v1(job, draft);
            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.validate");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let issues = json
                .get("issues")
                .and_then(|v| v.as_array())
                .map(|v| v.len())
                .unwrap_or(0);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("validate (ok={ok} issues={issues})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SMOKE_CHECK => {
            let mut json = match run_smoke_check_v1(job, draft) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let gaps = build_capability_gaps_from_smoke_v1(job, draft, &json);
            if let Some(obj) = json.as_object_mut() {
                obj.insert("capability_gaps".into(), serde_json::Value::Array(gaps));
            }
            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.smoke");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let issues = json
                .get("issues")
                .and_then(|v| v.as_array())
                .map(|v| v.len())
                .unwrap_or(0);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("smoke (ok={ok} issues={issues})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_MOTION_METRICS => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct MotionMetricsArgsV1 {
                #[serde(default)]
                version: u32,
                #[serde(default)]
                sample_count: Option<usize>,
            }

            let args: MotionMetricsArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_MOTION_METRICS}`: {err}"),
                    ));
                }
            };

            if args.version != 0 && args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_MOTION_METRICS}` version {}.",
                        args.version
                    ),
                ));
            }

            let sample_count = args.sample_count.unwrap_or(24).clamp(8, 256);
            let json = super::motion_validation::build_motion_metrics_report_v1(
                job.rig_move_cycle_m,
                &job.planned_components,
                sample_count,
            );
            if let Some(dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "motion_metrics.json", &json);
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_QA => {
            let Gen3dToolCallJsonV1 {
                call_id,
                tool_id,
                args,
            } = call;
            ToolCallOutcome::Immediate(execute_qa_v1(
                job,
                draft,
                tool_id.as_str(),
                call_id.as_str(),
                args,
            ))
        }
        TOOL_ID_INFO_KV_LIST_KEYS => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoKvListKeysArgsV1 {
                #[serde(default)]
                namespace: Option<String>,
                #[serde(default)]
                key_prefix: Option<String>,
                #[serde(default)]
                sort: Option<String>,
                #[serde(default)]
                page: Option<InfoPage>,
            }

            let args: InfoKvListKeysArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_KV_LIST_KEYS}`: {err}"),
                    ));
                }
            };

            let namespace = args
                .namespace
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let key_prefix = args
                .key_prefix
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let sort = args
                .sort
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("last_written_desc");
            if sort != "key_asc" && sort != "last_written_desc" {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Invalid sort `{sort}` (expected `key_asc` or `last_written_desc`)."),
                ));
            }

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let mut items: Vec<serde_json::Value> = Vec::new();
            for (k, rec) in store.kv_latest_entries() {
                if let Some(ns) = namespace.as_deref() {
                    if k.namespace.trim() != ns {
                        continue;
                    }
                }
                if let Some(prefix) = key_prefix.as_deref() {
                    if !k.key.starts_with(prefix) {
                        continue;
                    }
                }

                let mut latest = serde_json::Map::new();
                latest.insert("kv_rev".into(), serde_json::json!(rec.kv_rev));
                latest.insert("written_at_ms".into(), serde_json::json!(rec.written_at_ms));
                latest.insert("attempt".into(), serde_json::json!(rec.attempt));
                latest.insert("pass".into(), serde_json::json!(rec.pass));
                latest.insert("assembly_rev".into(), serde_json::json!(rec.assembly_rev));
                latest.insert(
                    "workspace_id".into(),
                    serde_json::Value::String(rec.workspace_id.clone()),
                );
                latest.insert(
                    "summary".into(),
                    serde_json::Value::String(rec.summary.clone()),
                );
                latest.insert("bytes".into(), serde_json::json!(rec.bytes));
                if let Some(prov) = rec.written_by.as_ref() {
                    latest.insert(
                        "written_by".into(),
                        serde_json::json!({
                            "tool_id": prov.tool_id.as_str(),
                            "call_id": prov.call_id.as_str(),
                        }),
                    );
                }

                let mut item = serde_json::Map::new();
                item.insert(
                    "namespace".into(),
                    serde_json::Value::String(k.namespace.clone()),
                );
                item.insert("key".into(), serde_json::Value::String(k.key.clone()));
                item.insert("latest".into(), serde_json::Value::Object(latest));
                items.push(serde_json::Value::Object(item));
            }

            if sort == "key_asc" {
                items.sort_by(|a, b| {
                    let a_ns = a.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                    let b_ns = b.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                    let a_key = a.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    let b_key = b.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    (a_ns, a_key).cmp(&(b_ns, b_key))
                });
            } else {
                items.sort_by(|a, b| {
                    let a_latest = a.get("latest").unwrap_or(&serde_json::Value::Null);
                    let b_latest = b.get("latest").unwrap_or(&serde_json::Value::Null);
                    let a_ts = a_latest
                        .get("written_at_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let b_ts = b_latest
                        .get("written_at_ms")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let a_rev = a_latest.get("kv_rev").and_then(|v| v.as_u64()).unwrap_or(0);
                    let b_rev = b_latest.get("kv_rev").and_then(|v| v.as_u64()).unwrap_or(0);
                    let a_ns = a.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                    let b_ns = b.get("namespace").and_then(|v| v.as_str()).unwrap_or("");
                    let a_key = a.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    let b_key = b.get("key").and_then(|v| v.as_str()).unwrap_or("");
                    b_ts.cmp(&a_ts)
                        .then_with(|| b_rev.cmp(&a_rev))
                        .then_with(|| (a_ns, a_key).cmp(&(b_ns, b_key)))
                });
            }

            let params_sig = store.stable_params_sig(&serde_json::json!({
                "namespace": namespace,
                "key_prefix": key_prefix,
                "sort": sort,
            }));
            let (limit, offset) = match store.page_from_args(
                TOOL_ID_INFO_KV_LIST_KEYS,
                params_sig.as_str(),
                args.page.as_ref(),
                50,
                200,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let page = store.page_out(
                &items,
                TOOL_ID_INFO_KV_LIST_KEYS,
                params_sig.as_str(),
                limit,
                offset,
            );

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("items".into(), serde_json::Value::Array(page.items));
            out.insert("truncated".into(), serde_json::Value::Bool(page.truncated));
            if let Some(cursor) = page.next_cursor {
                out.insert("next_cursor".into(), serde_json::Value::String(cursor));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_KV_LIST_HISTORY => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoKvListHistoryArgsV1 {
                namespace: String,
                key: String,
                #[serde(default)]
                sort: Option<String>,
                #[serde(default)]
                page: Option<InfoPage>,
            }

            let args: InfoKvListHistoryArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_KV_LIST_HISTORY}`: {err}"),
                    ));
                }
            };

            let namespace = args.namespace.trim();
            let key = args.key.trim();
            if namespace.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.namespace".into(),
                ));
            }
            if key.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.key".into(),
                ));
            }
            let sort = args
                .sort
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("rev_desc");
            if sort != "rev_desc" && sort != "rev_asc" {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Invalid sort `{sort}` (expected `rev_desc` or `rev_asc`)."),
                ));
            }

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let mut records = store.kv_records_for_key(namespace, key);
            if sort == "rev_asc" {
                records.sort_by_key(|r| r.kv_rev);
            } else {
                records.sort_by_key(|r| std::cmp::Reverse(r.kv_rev));
            }

            let mut items: Vec<serde_json::Value> = Vec::with_capacity(records.len());
            for rec in records {
                let mut item = serde_json::Map::new();
                item.insert("kv_rev".into(), serde_json::json!(rec.kv_rev));
                item.insert("written_at_ms".into(), serde_json::json!(rec.written_at_ms));
                item.insert("attempt".into(), serde_json::json!(rec.attempt));
                item.insert("pass".into(), serde_json::json!(rec.pass));
                item.insert("assembly_rev".into(), serde_json::json!(rec.assembly_rev));
                item.insert(
                    "workspace_id".into(),
                    serde_json::Value::String(rec.workspace_id.clone()),
                );
                item.insert(
                    "summary".into(),
                    serde_json::Value::String(rec.summary.clone()),
                );
                item.insert("bytes".into(), serde_json::json!(rec.bytes));
                if let Some(prov) = rec.written_by.as_ref() {
                    item.insert(
                        "written_by".into(),
                        serde_json::json!({
                            "tool_id": prov.tool_id.as_str(),
                            "call_id": prov.call_id.as_str(),
                        }),
                    );
                }
                items.push(serde_json::Value::Object(item));
            }

            let params_sig = store.stable_params_sig(&serde_json::json!({
                "namespace": namespace,
                "key": key,
                "sort": sort,
            }));
            let (limit, offset) = match store.page_from_args(
                TOOL_ID_INFO_KV_LIST_HISTORY,
                params_sig.as_str(),
                args.page.as_ref(),
                50,
                200,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let page = store.page_out(
                &items,
                TOOL_ID_INFO_KV_LIST_HISTORY,
                params_sig.as_str(),
                limit,
                offset,
            );

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("items".into(), serde_json::Value::Array(page.items));
            out.insert("truncated".into(), serde_json::Value::Bool(page.truncated));
            if let Some(cursor) = page.next_cursor {
                out.insert("next_cursor".into(), serde_json::Value::String(cursor));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_KV_GET => {
            let Gen3dToolCallJsonV1 {
                call_id,
                tool_id,
                args,
            } = call;
            ToolCallOutcome::Immediate(execute_info_kv_get_v1(
                job,
                tool_id.as_str(),
                call_id.as_str(),
                args,
            ))
        }
        TOOL_ID_INFO_KV_GET_PAGED => {
            let Gen3dToolCallJsonV1 {
                call_id,
                tool_id,
                args,
            } = call;
            ToolCallOutcome::Immediate(execute_info_kv_get_paged_v1(
                job,
                tool_id.as_str(),
                call_id.as_str(),
                args,
            ))
        }
        TOOL_ID_INFO_KV_GET_MANY => {
            let Gen3dToolCallJsonV1 {
                call_id,
                tool_id,
                args,
            } = call;
            ToolCallOutcome::Immediate(execute_info_kv_get_many_v1(
                job,
                tool_id.as_str(),
                call_id.as_str(),
                args,
            ))
        }
        TOOL_ID_INFO_EVENTS_LIST => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoEventsFiltersV1 {
                #[serde(default)]
                kind: Option<String>,
                #[serde(default)]
                tool_id: Option<String>,
                #[serde(default)]
                call_id: Option<String>,
                #[serde(default)]
                min_ts_ms: Option<u64>,
                #[serde(default)]
                max_ts_ms: Option<u64>,
                #[serde(default)]
                attempt: Option<u32>,
                #[serde(default)]
                pass: Option<u32>,
            }

            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoEventsListArgsV1 {
                #[serde(default)]
                filters: Option<InfoEventsFiltersV1>,
                #[serde(default)]
                sort: Option<String>,
                #[serde(default)]
                page: Option<InfoPage>,
            }

            fn kind_from_str(kind: &str) -> Option<super::info_store::InfoEventKindV1> {
                match kind.trim() {
                    "tool_call_start" => Some(super::info_store::InfoEventKindV1::ToolCallStart),
                    "tool_call_result" => Some(super::info_store::InfoEventKindV1::ToolCallResult),
                    "engine_log" => Some(super::info_store::InfoEventKindV1::EngineLog),
                    "budget_stop" => Some(super::info_store::InfoEventKindV1::BudgetStop),
                    "warning" => Some(super::info_store::InfoEventKindV1::Warning),
                    "error" => Some(super::info_store::InfoEventKindV1::Error),
                    _ => None,
                }
            }

            fn truncate_chars(text: &str, max_chars: usize) -> String {
                if text.chars().count() <= max_chars {
                    return text.to_string();
                }
                let mut out = String::with_capacity(max_chars + 24);
                for ch in text.chars().take(max_chars) {
                    out.push(ch);
                }
                out.push_str("…(truncated)");
                out
            }

            let args: InfoEventsListArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_EVENTS_LIST}`: {err}"),
                    ));
                }
            };

            let sort = args
                .sort
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("ts_desc");
            if sort != "ts_desc" && sort != "ts_asc" {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Invalid sort `{sort}` (expected `ts_desc` or `ts_asc`)."),
                ));
            }

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let filters = args.filters.as_ref();
            let kind_filter = filters
                .and_then(|f| f.kind.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let kind_filter_parsed = match kind_filter.as_deref() {
                Some(k) => match kind_from_str(k) {
                    Some(v) => Some(v),
                    None => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!(
                                "Unknown filters.kind `{}` (expected tool_call_start|tool_call_result|engine_log|budget_stop|warning|error).",
                                k
                            ),
                        ));
                    }
                },
                None => None,
            };
            let tool_id_filter = filters
                .and_then(|f| f.tool_id.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let call_id_filter = filters
                .and_then(|f| f.call_id.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let min_ts_ms = filters.and_then(|f| f.min_ts_ms);
            let max_ts_ms = filters.and_then(|f| f.max_ts_ms);
            let attempt_filter = filters.and_then(|f| f.attempt);
            let pass_filter = filters.and_then(|f| f.pass);

            let mut events: Vec<&super::info_store::InfoEvent> = Vec::new();
            for ev in store.events() {
                if let Some(kind) = kind_filter_parsed {
                    if ev.kind != kind {
                        continue;
                    }
                }
                if let Some(tool_id) = tool_id_filter {
                    if ev.tool_id.as_deref().map(str::trim) != Some(tool_id) {
                        continue;
                    }
                }
                if let Some(call_id) = call_id_filter {
                    if ev.call_id.as_deref().map(str::trim) != Some(call_id) {
                        continue;
                    }
                }
                if let Some(min_ts_ms) = min_ts_ms {
                    if ev.ts_ms < min_ts_ms {
                        continue;
                    }
                }
                if let Some(max_ts_ms) = max_ts_ms {
                    if ev.ts_ms > max_ts_ms {
                        continue;
                    }
                }
                if let Some(attempt) = attempt_filter {
                    if ev.attempt != attempt {
                        continue;
                    }
                }
                if let Some(pass) = pass_filter {
                    if ev.pass != pass {
                        continue;
                    }
                }
                events.push(ev);
            }

            if sort == "ts_asc" {
                events.sort_by(|a, b| {
                    a.ts_ms
                        .cmp(&b.ts_ms)
                        .then_with(|| a.event_id.cmp(&b.event_id))
                });
            } else {
                events.sort_by(|a, b| {
                    b.ts_ms
                        .cmp(&a.ts_ms)
                        .then_with(|| b.event_id.cmp(&a.event_id))
                });
            }

            let params_sig = store.stable_params_sig(&serde_json::json!({
                "filters": {
                    "kind": kind_filter,
                    "tool_id": tool_id_filter,
                    "call_id": call_id_filter,
                    "min_ts_ms": min_ts_ms,
                    "max_ts_ms": max_ts_ms,
                    "attempt": attempt_filter,
                    "pass": pass_filter,
                },
                "sort": sort,
            }));
            let (limit, offset) = match store.page_from_args(
                TOOL_ID_INFO_EVENTS_LIST,
                params_sig.as_str(),
                args.page.as_ref(),
                100,
                500,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let page = store.page_out(
                events.as_slice(),
                TOOL_ID_INFO_EVENTS_LIST,
                params_sig.as_str(),
                limit,
                offset,
            );

            let mut items: Vec<serde_json::Value> = Vec::with_capacity(page.items.len());
            for ev in page.items {
                let data_preview = if ev.data.is_null() {
                    serde_json::Value::Null
                } else {
                    let json =
                        serde_json::to_string(&ev.data).unwrap_or_else(|_| ev.data.to_string());
                    serde_json::Value::String(truncate_chars(&json, 2000))
                };
                let mut item = serde_json::Map::new();
                item.insert("event_id".into(), serde_json::json!(ev.event_id));
                item.insert("ts_ms".into(), serde_json::json!(ev.ts_ms));
                item.insert("attempt".into(), serde_json::json!(ev.attempt));
                item.insert("pass".into(), serde_json::json!(ev.pass));
                item.insert("assembly_rev".into(), serde_json::json!(ev.assembly_rev));
                item.insert("kind".into(), serde_json::json!(ev.kind));
                if let Some(tool_id) = ev.tool_id.as_deref() {
                    item.insert(
                        "tool_id".into(),
                        serde_json::Value::String(tool_id.to_string()),
                    );
                }
                if let Some(call_id) = ev.call_id.as_deref() {
                    item.insert(
                        "call_id".into(),
                        serde_json::Value::String(call_id.to_string()),
                    );
                }
                item.insert(
                    "message".into(),
                    serde_json::Value::String(truncate_chars(&ev.message, 400)),
                );
                item.insert("data_preview".into(), data_preview);
                items.push(serde_json::Value::Object(item));
            }

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("items".into(), serde_json::Value::Array(items));
            out.insert("truncated".into(), serde_json::Value::Bool(page.truncated));
            if let Some(cursor) = page.next_cursor {
                out.insert("next_cursor".into(), serde_json::Value::String(cursor));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_EVENTS_GET => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoEventsGetArgsV1 {
                event_id: u64,
                #[serde(default)]
                json_pointer: Option<String>,
                #[serde(default)]
                max_bytes: Option<u64>,
            }

            let args: InfoEventsGetArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_EVENTS_GET}`: {err}"),
                    ));
                }
            };

            let max_bytes = args.max_bytes.unwrap_or(64 * 1024).clamp(1024, 512 * 1024) as usize;

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let Some(event) = store.event_by_id(args.event_id) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown event_id {}", args.event_id),
                ));
            };

            let json_pointer = args
                .json_pointer
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let data = if let Some(ptr) = json_pointer.as_deref() {
                event
                    .data
                    .pointer(ptr)
                    .cloned()
                    .ok_or_else(|| format!("JSON pointer not found: {ptr}"))
            } else {
                Ok(event.data.clone())
            };
            let data = match data {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let bytes = match serde_json::to_vec(&data) {
                Ok(v) => v.len(),
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Failed to serialize event data: {err}"),
                    ));
                }
            };
            if bytes > max_bytes {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Event data is too large ({bytes} bytes > max_bytes={max_bytes}). Use `json_pointer`."
                    ),
                ));
            }

            let mut out_event = serde_json::Map::new();
            out_event.insert("event_id".into(), serde_json::json!(event.event_id));
            out_event.insert("ts_ms".into(), serde_json::json!(event.ts_ms));
            out_event.insert("attempt".into(), serde_json::json!(event.attempt));
            out_event.insert("pass".into(), serde_json::json!(event.pass));
            out_event.insert("assembly_rev".into(), serde_json::json!(event.assembly_rev));
            out_event.insert("kind".into(), serde_json::json!(event.kind));
            if let Some(tool_id) = event.tool_id.as_deref() {
                out_event.insert(
                    "tool_id".into(),
                    serde_json::Value::String(tool_id.to_string()),
                );
            }
            if let Some(call_id) = event.call_id.as_deref() {
                out_event.insert(
                    "call_id".into(),
                    serde_json::Value::String(call_id.to_string()),
                );
            }
            out_event.insert(
                "message".into(),
                serde_json::Value::String(event.message.clone()),
            );
            out_event.insert("data".into(), data);

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("event".into(), serde_json::Value::Object(out_event));
            out.insert("truncated".into(), serde_json::Value::Bool(false));
            if let Some(ptr) = json_pointer {
                out.insert("json_pointer".into(), serde_json::Value::String(ptr));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_EVENTS_SEARCH => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoEventsSearchFiltersV1 {
                #[serde(default)]
                kind: Option<String>,
                #[serde(default)]
                attempt: Option<u32>,
                #[serde(default)]
                pass: Option<u32>,
            }

            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoEventsSearchArgsV1 {
                query: String,
                #[serde(default)]
                filters: Option<InfoEventsSearchFiltersV1>,
                #[serde(default)]
                page: Option<InfoPage>,
            }

            fn kind_from_str(kind: &str) -> Option<super::info_store::InfoEventKindV1> {
                match kind.trim() {
                    "tool_call_start" => Some(super::info_store::InfoEventKindV1::ToolCallStart),
                    "tool_call_result" => Some(super::info_store::InfoEventKindV1::ToolCallResult),
                    "engine_log" => Some(super::info_store::InfoEventKindV1::EngineLog),
                    "budget_stop" => Some(super::info_store::InfoEventKindV1::BudgetStop),
                    "warning" => Some(super::info_store::InfoEventKindV1::Warning),
                    "error" => Some(super::info_store::InfoEventKindV1::Error),
                    _ => None,
                }
            }

            fn truncate_chars(text: &str, max_chars: usize) -> String {
                if text.chars().count() <= max_chars {
                    return text.to_string();
                }
                let mut out = String::with_capacity(max_chars + 24);
                for ch in text.chars().take(max_chars) {
                    out.push(ch);
                }
                out.push_str("…(truncated)");
                out
            }

            let args: InfoEventsSearchArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_EVENTS_SEARCH}`: {err}"),
                    ));
                }
            };

            let query = args.query.trim();
            if query.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.query".into(),
                ));
            }
            if query.as_bytes().len() > 256 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "args.query is too long (max 256 bytes).".into(),
                ));
            }

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let kind_filter = args
                .filters
                .as_ref()
                .and_then(|f| f.kind.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());
            let kind_filter_parsed = match kind_filter.as_deref() {
                Some(k) => match kind_from_str(k) {
                    Some(v) => Some(v),
                    None => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!(
                                "Unknown filters.kind `{}` (expected tool_call_start|tool_call_result|engine_log|budget_stop|warning|error).",
                                k
                            ),
                        ));
                    }
                },
                None => None,
            };
            let attempt_filter = args.filters.as_ref().and_then(|f| f.attempt);
            let pass_filter = args.filters.as_ref().and_then(|f| f.pass);

            let mut matches_out: Vec<serde_json::Value> = Vec::new();
            for ev in store.events() {
                if let Some(kind) = kind_filter_parsed {
                    if ev.kind != kind {
                        continue;
                    }
                }
                if let Some(attempt) = attempt_filter {
                    if ev.attempt != attempt {
                        continue;
                    }
                }
                if let Some(pass) = pass_filter {
                    if ev.pass != pass {
                        continue;
                    }
                }

                let mut matched = ev.message.contains(query);
                if !matched && !ev.data.is_null() {
                    let data =
                        serde_json::to_string(&ev.data).unwrap_or_else(|_| ev.data.to_string());
                    matched = data.contains(query);
                }
                if !matched {
                    continue;
                }

                matches_out.push(serde_json::json!({
                    "event_id": ev.event_id,
                    "ts_ms": ev.ts_ms,
                    "kind": ev.kind,
                    "message_excerpt": truncate_chars(&ev.message, 240),
                }));
            }

            matches_out.sort_by(|a, b| {
                let a_ts = a.get("ts_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let b_ts = b.get("ts_ms").and_then(|v| v.as_u64()).unwrap_or(0);
                let a_id = a.get("event_id").and_then(|v| v.as_u64()).unwrap_or(0);
                let b_id = b.get("event_id").and_then(|v| v.as_u64()).unwrap_or(0);
                b_ts.cmp(&a_ts).then_with(|| b_id.cmp(&a_id))
            });

            let params_sig = store.stable_params_sig(&serde_json::json!({
                "query": query,
                "filters": {
                    "kind": kind_filter,
                    "attempt": attempt_filter,
                    "pass": pass_filter,
                }
            }));
            let (limit, offset) = match store.page_from_args(
                TOOL_ID_INFO_EVENTS_SEARCH,
                params_sig.as_str(),
                args.page.as_ref(),
                100,
                500,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let page = store.page_out(
                &matches_out,
                TOOL_ID_INFO_EVENTS_SEARCH,
                params_sig.as_str(),
                limit,
                offset,
            );

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("matches".into(), serde_json::Value::Array(page.items));
            out.insert("truncated".into(), serde_json::Value::Bool(page.truncated));
            if let Some(cursor) = page.next_cursor {
                out.insert("next_cursor".into(), serde_json::Value::String(cursor));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_BLOBS_LIST => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoBlobsListFiltersV1 {
                #[serde(default)]
                label_prefix: Option<String>,
                #[serde(default)]
                labels_any: Option<Vec<String>>,
                #[serde(default)]
                labels_all: Option<Vec<String>>,
                #[serde(default)]
                content_type_prefix: Option<String>,
                #[serde(default)]
                attempt: Option<u32>,
                #[serde(default)]
                pass: Option<u32>,
            }

            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoBlobsListArgsV1 {
                #[serde(default)]
                filters: Option<InfoBlobsListFiltersV1>,
                #[serde(default)]
                sort: Option<String>,
                #[serde(default)]
                page: Option<InfoPage>,
            }

            let args: InfoBlobsListArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_BLOBS_LIST}`: {err}"),
                    ));
                }
            };

            let sort = args
                .sort
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or("created_desc");
            if sort != "created_desc" && sort != "created_asc" {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Invalid sort `{sort}` (expected `created_desc` or `created_asc`)."),
                ));
            }

            let filters = args.filters.as_ref();
            let label_prefix = filters
                .and_then(|f| f.label_prefix.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let mut labels_any = filters
                .and_then(|f| f.labels_any.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            let mut labels_all = filters
                .and_then(|f| f.labels_all.clone())
                .unwrap_or_default()
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>();
            labels_any.sort();
            labels_any.dedup();
            labels_all.sort();
            labels_all.dedup();
            if labels_any.len() > 8 || labels_all.len() > 8 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "labels_any/labels_all can contain at most 8 labels.".into(),
                ));
            }
            if labels_any.iter().any(|l| l.as_bytes().len() > 64)
                || labels_all.iter().any(|l| l.as_bytes().len() > 64)
            {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Blob labels must be <= 64 bytes each.".into(),
                ));
            }
            let content_type_prefix = filters
                .and_then(|f| f.content_type_prefix.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);
            let attempt_filter = filters.and_then(|f| f.attempt);
            let pass_filter = filters.and_then(|f| f.pass);

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let mut blobs: Vec<&super::info_store::InfoBlob> = Vec::new();
            for blob in store.blobs() {
                if let Some(attempt) = attempt_filter {
                    if blob.attempt != attempt {
                        continue;
                    }
                }
                if let Some(pass) = pass_filter {
                    if blob.pass != pass {
                        continue;
                    }
                }
                if let Some(prefix) = content_type_prefix.as_deref() {
                    if !blob.content_type.starts_with(prefix) {
                        continue;
                    }
                }
                if let Some(prefix) = label_prefix.as_deref() {
                    if !blob.labels.iter().any(|l| l.starts_with(prefix)) {
                        continue;
                    }
                }
                if !labels_any.is_empty()
                    && !blob
                        .labels
                        .iter()
                        .any(|l| labels_any.iter().any(|q| q == l))
                {
                    continue;
                }
                if !labels_all.is_empty()
                    && !labels_all
                        .iter()
                        .all(|q| blob.labels.iter().any(|l| l == q))
                {
                    continue;
                }
                blobs.push(blob);
            }

            if sort == "created_asc" {
                blobs.sort_by(|a, b| {
                    a.created_at_ms
                        .cmp(&b.created_at_ms)
                        .then_with(|| a.blob_id.cmp(&b.blob_id))
                });
            } else {
                blobs.sort_by(|a, b| {
                    b.created_at_ms
                        .cmp(&a.created_at_ms)
                        .then_with(|| b.blob_id.cmp(&a.blob_id))
                });
            }

            let params_sig = store.stable_params_sig(&serde_json::json!({
                "filters": {
                    "label_prefix": label_prefix,
                    "labels_any": labels_any,
                    "labels_all": labels_all,
                    "content_type_prefix": content_type_prefix,
                    "attempt": attempt_filter,
                    "pass": pass_filter,
                },
                "sort": sort,
            }));
            let (limit, offset) = match store.page_from_args(
                TOOL_ID_INFO_BLOBS_LIST,
                params_sig.as_str(),
                args.page.as_ref(),
                50,
                200,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let page = store.page_out(
                blobs.as_slice(),
                TOOL_ID_INFO_BLOBS_LIST,
                params_sig.as_str(),
                limit,
                offset,
            );

            let mut items: Vec<serde_json::Value> = Vec::with_capacity(page.items.len());
            for blob in page.items {
                items.push(serde_json::json!({
                    "blob_id": blob.blob_id.as_str(),
                    "created_at_ms": blob.created_at_ms,
                    "attempt": blob.attempt,
                    "pass": blob.pass,
                    "assembly_rev": blob.assembly_rev,
                    "content_type": blob.content_type.as_str(),
                    "bytes": blob.bytes,
                    "labels": blob.labels.clone(),
                }));
            }

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            out.insert("items".into(), serde_json::Value::Array(items));
            out.insert("truncated".into(), serde_json::Value::Bool(page.truncated));
            if let Some(cursor) = page.next_cursor {
                out.insert("next_cursor".into(), serde_json::Value::String(cursor));
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::Value::Object(out),
            ))
        }
        TOOL_ID_INFO_BLOBS_GET => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct InfoBlobsGetArgsV1 {
                blob_id: String,
            }

            let args: InfoBlobsGetArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_BLOBS_GET}`: {err}"),
                    ));
                }
            };
            let blob_id = args.blob_id.trim();
            if blob_id.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.blob_id".into(),
                ));
            }

            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            let Some(blob) = store.blob_by_id(blob_id) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown blob_id `{blob_id}`."),
                ));
            };

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "ok": true,
                    "blob": {
                        "blob_id": blob.blob_id.as_str(),
                        "created_at_ms": blob.created_at_ms,
                        "attempt": blob.attempt,
                        "pass": blob.pass,
                        "assembly_rev": blob.assembly_rev,
                        "content_type": blob.content_type.as_str(),
                        "bytes": blob.bytes,
                        "labels": blob.labels.clone(),
                    }
                }),
            ))
        }
        TOOL_ID_APPLY_DRAFT_OPS => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let mut json = match super::draft_ops::apply_draft_ops_v1(
                job,
                draft,
                Some(call_id.as_str()),
                call.args,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.apply_draft_ops_last");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let committed = json
                .get("committed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                tool_id.as_str(),
                call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("apply draft ops (ok={ok} committed={committed})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_APPLY_LAST_DRAFT_OPS => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ApplyLastDraftOpsArgsV1 {}

            let _: ApplyLastDraftOpsArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_APPLY_LAST_DRAFT_OPS}`: {err}"),
                    ));
                }
            };

            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();

            let run_dir = match job.run_dir_path() {
                Some(v) => v.to_path_buf(),
                None => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        "No active Gen3D run (missing run_dir).".into(),
                    ));
                }
            };

            let artifact = match find_latest_gen3d_step_artifact(
                run_dir.as_path(),
                "draft_ops_suggested_last.json",
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "{err}\nHint: Call `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}` first to generate DraftOps suggestions."
                        ),
                    ));
                }
            };

            let text = match std::fs::read_to_string(&artifact.path) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!("Failed to read `{}`: {err}", artifact.path.display()),
                    ));
                }
            };
            let suggested: serde_json::Value =
                match serde_json::from_str(&text).or_else(|_| json5::from_str(&text)) {
                    Ok(v) => v,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call_id,
                            tool_id,
                            format!("Invalid JSON in `{}`: {err}", artifact.path.display()),
                        ));
                    }
                };

            let ops = match suggested.get("ops") {
                Some(serde_json::Value::Array(_)) => suggested.get("ops").cloned().unwrap(),
                Some(_) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "Invalid `{}` payload: expected `ops` array.",
                            artifact.path.display()
                        ),
                    ));
                }
                None => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "Invalid `{}` payload: missing `ops`.",
                            artifact.path.display()
                        ),
                    ));
                }
            };

            if let Some(ws) = suggested
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                let current = job.active_workspace_id().trim();
                if ws != current {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "DraftOps suggestions target workspace_id={ws:?}, but active_workspace_id is {current:?}. Switch workspaces via `{TOOL_ID_SET_ACTIVE_WORKSPACE}` or re-run `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`."
                        ),
                    ));
                }
            }

            let suggested_if_assembly_rev = suggested
                .get("if_assembly_rev")
                .and_then(|v| v.as_u64())
                .or_else(|| suggested.get("assembly_rev").and_then(|v| v.as_u64()))
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or_else(|| job.assembly_rev());

            let apply_args = serde_json::json!({
                "version": 1,
                "atomic": true,
                "if_assembly_rev": suggested_if_assembly_rev,
                "ops": ops,
            });
            let mut json = match super::draft_ops::apply_draft_ops_v1(
                job,
                draft,
                Some(call_id.as_str()),
                apply_args,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "{err}\nHint: If this is an `if_assembly_rev` mismatch, re-run `{TOOL_ID_QUERY_COMPONENT_PARTS}` + `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}` to get fresh DraftOps for the current draft."
                        ),
                    ));
                }
            };

            let relative_path = artifact
                .path
                .strip_prefix(run_dir.as_path())
                .ok()
                .and_then(|p| p.to_str())
                .map(|s| s.replace('\\', "/"))
                .unwrap_or_else(|| artifact.path.display().to_string());
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "applied_from".into(),
                    serde_json::json!({
                        "kind": "draft_ops_suggested_last_artifact",
                        "relative_path": relative_path,
                        "attempt": artifact.attempt,
                        "step": artifact.step,
                        "tool_id": TOOL_ID_LLM_GENERATE_DRAFT_OPS,
                        "if_assembly_rev": suggested_if_assembly_rev,
                    }),
                );
            }

            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.apply_draft_ops_last");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let committed = json
                .get("committed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                tool_id.as_str(),
                call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("apply last draft ops (ok={ok} committed={committed})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ApplyDraftOpsFromEventArgsV1 {
                event_id: u64,
            }

            let args: ApplyDraftOpsFromEventArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_APPLY_DRAFT_OPS_FROM_EVENT}`: {err}"),
                    ));
                }
            };
            if args.event_id == 0 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "args.event_id must be > 0.".into(),
                ));
            }

            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();

            let (event_attempt, event_pass, event_assembly_rev, event_tool_id, event_call_id, data) =
                match job.ensure_info_store() {
                    Ok(store) => {
                        let Some(event) = store.event_by_id(args.event_id) else {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                call_id,
                                tool_id,
                                format!("Unknown event_id {}.", args.event_id),
                            ));
                        };
                        if event.kind != super::info_store::InfoEventKindV1::ToolCallResult {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                call_id,
                                tool_id,
                                format!(
                                    "event_id {} is kind={:?} (expected tool_call_result).",
                                    args.event_id, event.kind
                                ),
                            ));
                        }
                        (
                            event.attempt,
                            event.pass,
                            event.assembly_rev,
                            event.tool_id.clone(),
                            event.call_id.clone(),
                            event.data.clone(),
                        )
                    }
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call_id, tool_id, err,
                        ));
                    }
                };

            if event_tool_id.as_deref() != Some(TOOL_ID_LLM_GENERATE_DRAFT_OPS) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call_id,
                    tool_id,
                    format!(
                        "event_id {} tool_id={:?} (expected `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`).",
                        args.event_id, event_tool_id
                    ),
                ));
            }

            let data_obj = match data.as_object() {
                Some(v) => v,
                None => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "event_id {} data is not an object (expected a tool result record).",
                            args.event_id
                        ),
                    ));
                }
            };
            let ok = data_obj
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let tool_result_tool_id = data_obj
                .get("tool_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let tool_error = data_obj
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim();
            let suggested = data_obj
                .get("result")
                .cloned()
                .unwrap_or(serde_json::Value::Null);

            if !ok {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call_id,
                    tool_id,
                    format!(
                        "event_id {} tool result is ok=false (tool_error={}): re-run `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`.",
                        args.event_id,
                        if tool_error.is_empty() { "<none>" } else { tool_error }
                    ),
                ));
            }
            if tool_result_tool_id != TOOL_ID_LLM_GENERATE_DRAFT_OPS {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call_id,
                    tool_id,
                    format!(
                        "event_id {} tool result tool_id={} (expected `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`).",
                        args.event_id, tool_result_tool_id
                    ),
                ));
            }
            let ops = match suggested.get("ops") {
                Some(serde_json::Value::Array(_)) => suggested.get("ops").cloned().unwrap(),
                Some(_) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "event_id {} payload invalid: expected `result.ops` array.",
                            args.event_id
                        ),
                    ));
                }
                None => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "event_id {} payload invalid: missing `result.ops`.",
                            args.event_id
                        ),
                    ));
                }
            };

            if let Some(ws) = suggested
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                let current = job.active_workspace_id().trim();
                if ws != current {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "DraftOps suggestions target workspace_id={ws:?}, but active_workspace_id is {current:?}. Switch workspaces via `{TOOL_ID_SET_ACTIVE_WORKSPACE}` or re-run `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`."
                        ),
                    ));
                }
            }

            if let Some(if_rev) = suggested
                .get("if_assembly_rev")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok())
            {
                if if_rev != event_assembly_rev {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "event_id {} payload if_assembly_rev={} does not match event.assembly_rev={event_assembly_rev}.",
                            args.event_id, if_rev
                        ),
                    ));
                }
            }

            let apply_args = serde_json::json!({
                "version": 1,
                "atomic": true,
                "if_assembly_rev": event_assembly_rev,
                "ops": ops,
            });
            let mut json = match super::draft_ops::apply_draft_ops_v1(
                job,
                draft,
                Some(call_id.as_str()),
                apply_args,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!(
                            "{err}\nHint: If this is an `if_assembly_rev` mismatch, re-run `{TOOL_ID_QUERY_COMPONENT_PARTS}` + `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}` to get fresh DraftOps for the current draft."
                        ),
                    ));
                }
            };

            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "applied_from".into(),
                    serde_json::json!({
                        "kind": "info_event",
                        "event_id": args.event_id,
                        "attempt": event_attempt,
                        "pass": event_pass,
                        "tool_id": TOOL_ID_LLM_GENERATE_DRAFT_OPS,
                        "call_id": event_call_id,
                        "assembly_rev": event_assembly_rev,
                    }),
                );
            }

            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.apply_draft_ops_last");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let committed = json
                .get("committed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                tool_id.as_str(),
                call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!(
                    "apply draft ops from event (ok={ok} committed={committed} event_id={})",
                    args.event_id
                ),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_APPLY_PLAN_OPS => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let mut json = match super::plan_ops::apply_plan_ops_v1(
                job,
                draft,
                Some(call_id.as_str()),
                call.args,
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };

            let accepted = json
                .get("accepted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if accepted {
                if let Some(def) = draft.root_def() {
                    let max_dim = def.size.x.max(def.size.y).max(def.size.z).max(0.5);
                    _preview.distance = (max_dim * 2.8 + 0.8).clamp(2.0, 250.0);
                    _preview.pitch = super::super::GEN3D_PREVIEW_DEFAULT_PITCH;
                    _preview.yaw = super::super::GEN3D_PREVIEW_DEFAULT_YAW;
                    _preview.last_cursor = None;
                }
            }

            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.apply_plan_ops_last");
            let ok = json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            let committed = json
                .get("committed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                tool_id.as_str(),
                call_id.as_str(),
                key.as_str(),
                json.clone(),
                format!("apply plan ops (ok={ok} committed={committed} accepted={accepted})"),
            ) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            if let Some(obj) = json.as_object_mut() {
                obj.insert(
                    "info_kv".into(),
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, key.as_str(), record.kv_rev),
                );
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_APPLY_REUSE_GROUPS => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct ApplyReuseGroupsArgsV1 {
                #[serde(default)]
                version: u32,
            }

            let mut args: ApplyReuseGroupsArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_APPLY_REUSE_GROUPS}`: {err}"),
                    ));
                }
            };
            if args.version == 0 {
                args.version = 1;
            }
            if args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_APPLY_REUSE_GROUPS}` version {}.",
                        args.version
                    ),
                ));
            }

            let reuse_groups_total = job.reuse_groups.len();

            let mut report = super::reuse_groups::apply_auto_copy(
                &mut job.planned_components,
                draft,
                &job.reuse_groups,
            );

            if report.component_copies_applied > 0 {
                if let Some(root_idx) = job
                    .planned_components
                    .iter()
                    .position(|c| c.attach_to.is_none())
                {
                    if let Err(err) = super::convert::resolve_planned_component_transforms(
                        &mut job.planned_components,
                        root_idx,
                    ) {
                        report.errors.push(format!(
                            "apply_reuse_groups: resolve transforms failed: {err}"
                        ));
                    }
                }
                super::convert::update_root_def_from_planned_components(
                    &job.planned_components,
                    &job.plan_collider,
                    draft,
                );
                write_gen3d_assembly_snapshot(job.step_dir.as_deref(), &job.planned_components);
                job.assembly_rev = job.assembly_rev.saturating_add(1);
            }

            let fallback_component_indices: Vec<usize> = report
                .fallback_component_indices
                .iter()
                .copied()
                .filter(|idx| *idx < job.planned_components.len())
                .filter(|idx| {
                    job.planned_components
                        .get(*idx)
                        .is_some_and(|c| c.actual_size.is_none())
                })
                .collect();
            if !fallback_component_indices.is_empty() {
                let mut pending_set: std::collections::HashSet<usize> = job
                    .agent
                    .pending_regen_component_indices
                    .iter()
                    .copied()
                    .collect();
                for idx in fallback_component_indices.iter().copied() {
                    pending_set.insert(idx);
                }
                let mut pending: Vec<usize> = pending_set.into_iter().collect();
                pending.sort_unstable();
                job.agent.pending_regen_component_indices = pending;
                job.agent
                    .pending_regen_component_indices_skipped_due_to_budget
                    .clear();
            }

            let fallback_components_json: Vec<serde_json::Value> = fallback_component_indices
                .iter()
                .copied()
                .filter(|idx| *idx < job.planned_components.len())
                .map(|idx| {
                    serde_json::json!({
                        "index": idx,
                        "name": job.planned_components[idx].name.as_str(),
                    })
                })
                .collect();

            let mut outcomes_json: Vec<serde_json::Value> = Vec::new();
            const MAX_OUTCOMES: usize = 24;
            let copied_targets = report
                .outcomes
                .iter()
                .filter(|outcome| {
                    outcome.alignment_used
                        != Some(super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX)
                })
                .count();
            let mirrored_targets = report
                .outcomes
                .iter()
                .filter(|outcome| {
                    outcome.alignment_used
                        == Some(super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX)
                })
                .count();
            for outcome in report.outcomes.iter().take(MAX_OUTCOMES) {
                let mode = match outcome.mode_used {
                    super::copy_component::Gen3dCopyMode::Detached => "detached",
                    super::copy_component::Gen3dCopyMode::Linked => "linked",
                };
                let alignment = match outcome
                    .alignment_used
                    .unwrap_or(super::copy_component::Gen3dCopyAlignmentMode::Rotation)
                {
                    super::copy_component::Gen3dCopyAlignmentMode::Rotation => "rotation",
                    super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX => "mirror_mount_x",
                };
                outcomes_json.push(serde_json::json!({
                    "source": outcome.source_component_name.as_str(),
                    "target": outcome.target_component_name.as_str(),
                    "mode": mode,
                    "alignment": alignment,
                }));
            }
            let outcomes_omitted = report.outcomes.len().saturating_sub(MAX_OUTCOMES);

            let json = serde_json::json!({
                "ok": true,
                "version": 1,
                "enabled": report.enabled,
                "reuse_groups_total": reuse_groups_total,
                "component_copies_applied": report.component_copies_applied,
                "subtree_copies_applied": report.subtree_copies_applied,
                "copied_targets": copied_targets,
                "mirrored_targets": mirrored_targets,
                "targets_skipped_already_generated": report.targets_skipped_already_generated,
                "subtrees_skipped_partially_generated": report.subtrees_skipped_partially_generated,
                "preflight_mismatches": report.preflight_mismatches,
                "fallback_component_indices": fallback_component_indices,
                "fallback_components": fallback_components_json,
                "errors": report.errors,
                "outcomes": outcomes_json,
                "outcomes_omitted": outcomes_omitted,
            });

            if let Some(step_dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(Some(step_dir), "apply_reuse_groups_last.json", &json);
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SNAPSHOT => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::snapshots::snapshot_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_LIST_SNAPSHOTS => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::snapshots::list_snapshots_v1(job, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_DIFF_SNAPSHOTS => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::snapshots::diff_snapshots_v1(job, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_RESTORE_SNAPSHOT => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::snapshots::restore_snapshot_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_COPY_COMPONENT | TOOL_ID_MIRROR_COMPONENT => {
            let source_name = call
                .args
                .get("source_component")
                .or_else(|| call.args.get("source_component_name"))
                .or_else(|| call.args.get("source_component_id"))
                .or_else(|| call.args.get("source"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let source_idx = call
                .args
                .get("source_component_index")
                .or_else(|| call.args.get("source_index"))
                .or_else(|| call.args.get("source_idx"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or_else(|| {
                    source_name.as_deref().and_then(|name| {
                        job.planned_components
                            .iter()
                            .position(|c| c.name == name)
                            .or_else(|| {
                                resolve_component_index_by_name_hint(&job.planned_components, name)
                            })
                    })
                });
            let Some(source_idx) = source_idx else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.source_component (name) or args.source_component_index (index)."
                        .into(),
                ));
            };
            if source_idx >= job.planned_components.len() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("source_component_index out of range: {source_idx}"),
                ));
            }

            let mode = call
                .args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("detached")
                .trim()
                .to_ascii_lowercase();
            let mode = match mode.as_str() {
                "" | "detached" | "copy" | "duplicate" => {
                    super::copy_component::Gen3dCopyMode::Detached
                }
                "linked" | "link" | "shared" | "instance" => {
                    super::copy_component::Gen3dCopyMode::Linked
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Unknown mode `{other}` (expected `detached` or `linked`)"),
                    ));
                }
            };
            let anchors_mode = call
                .args
                .get("anchors")
                .or_else(|| call.args.get("anchors_mode"))
                .or_else(|| call.args.get("anchor_mode"))
                .and_then(|v| v.as_str())
                .unwrap_or("preserve_interfaces")
                .trim()
                .to_ascii_lowercase();
            let anchors_mode = match anchors_mode.as_str() {
                "" | "preserve_interfaces" | "preserve_interface" | "interfaces" | "interface" => {
                    super::copy_component::Gen3dCopyAnchorsMode::PreserveInterfaceAnchors
                }
                "preserve_target" | "preserve" | "target" => {
                    super::copy_component::Gen3dCopyAnchorsMode::PreserveTargetAnchors
                }
                "copy_source" | "copy" | "source" => {
                    super::copy_component::Gen3dCopyAnchorsMode::CopySourceAnchors
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Unknown anchors `{other}` (expected `preserve_interfaces`, `preserve_target`, or `copy_source`)"
                        ),
                    ));
                }
            };
            let delta = parse_delta_transform(call.args.get("transform"));
            let alignment = if call.tool_id.as_str() == TOOL_ID_MIRROR_COMPONENT {
                super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX
            } else {
                super::copy_component::Gen3dCopyAlignmentMode::Rotation
            };
            let alignment_frame = call
                .args
                .get("alignment_frame")
                .or_else(|| call.args.get("frame"))
                .and_then(|v| v.as_str())
                .unwrap_or("join")
                .trim()
                .to_ascii_lowercase();
            let alignment_frame = match alignment_frame.as_str() {
                "" | "join" | "join_frame" => super::copy_component::Gen3dCopyAlignmentFrame::Join,
                "child_anchor" | "child" | "child_frame" => {
                    super::copy_component::Gen3dCopyAlignmentFrame::ChildAnchor
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Unknown alignment_frame `{other}` (expected `join` or `child_anchor`)"
                        ),
                    ));
                }
            };
            if alignment == super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX
                && alignment_frame != super::copy_component::Gen3dCopyAlignmentFrame::Join
            {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "mirror_component_v1 does not support alignment_frame=child_anchor (use join)."
                        .into(),
                ));
            }

            let mut targets: Vec<usize> = Vec::new();
            let target_list = call
                .args
                .get("targets")
                .or_else(|| call.args.get("target_component_indices"))
                .or_else(|| call.args.get("target_indices"))
                .or_else(|| call.args.get("target_idxs"))
                .or_else(|| call.args.get("target_component_names"))
                .or_else(|| call.args.get("target_names"));

            if let Some(arr) = target_list.and_then(|v| v.as_array()) {
                for item in arr.iter() {
                    if let Some(idx) = item.as_u64().map(|v| v as usize) {
                        targets.push(idx);
                    } else if let Some(name) =
                        item.as_str().map(|s| s.trim()).filter(|s| !s.is_empty())
                    {
                        let idx = job
                            .planned_components
                            .iter()
                            .position(|c| c.name == name)
                            .or_else(|| {
                                resolve_component_index_by_name_hint(&job.planned_components, name)
                            });
                        let Some(idx) = idx else {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                call.call_id,
                                call.tool_id,
                                format!("Unknown target component `{name}`"),
                            ));
                        };
                        targets.push(idx);
                    } else {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            "targets must contain component indices or names".into(),
                        ));
                    }
                }
            } else {
                let target_name = call
                    .args
                    .get("target_component")
                    .or_else(|| call.args.get("target_component_name"))
                    .or_else(|| call.args.get("target_component_id"))
                    .or_else(|| call.args.get("target"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let target_idx = call
                    .args
                    .get("target_component_index")
                    .or_else(|| call.args.get("target_index"))
                    .or_else(|| call.args.get("target_idx"))
                    .and_then(|v| v.as_u64())
                    .map(|v| v as usize)
                    .or_else(|| {
                        target_name.as_deref().and_then(|name| {
                            job.planned_components
                                .iter()
                                .position(|c| c.name == name)
                                .or_else(|| {
                                    resolve_component_index_by_name_hint(
                                        &job.planned_components,
                                        name,
                                    )
                                })
                        })
                    });
                let Some(target_idx) = target_idx else {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "Missing target component (use args.targets / args.target_component, or args.target_component_indices)."
                            .into(),
                    ));
                };
                targets.push(target_idx);
            }

            targets.sort_unstable();
            targets.dedup();
            if targets.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No targets provided".into(),
                ));
            }
            if targets.iter().any(|&t| t >= job.planned_components.len()) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "One or more target indices are out of range".into(),
                ));
            }
            if targets.iter().any(|&t| t == source_idx) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Targets must not include the source component".into(),
                ));
            }

            let mut copies_json: Vec<serde_json::Value> = Vec::new();
            for target_idx in targets.iter().copied() {
                let outcome = match super::copy_component::copy_component_into(
                    &mut job.planned_components,
                    draft,
                    source_idx,
                    target_idx,
                    mode,
                    anchors_mode,
                    alignment,
                    alignment_frame,
                    delta,
                    None,
                ) {
                    Ok(outcome) => outcome,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                copies_json.push(serde_json::json!({
                    "source": outcome.source_component_name,
                    "target": outcome.target_component_name,
                    "mode": match outcome.mode_used {
                        super::copy_component::Gen3dCopyMode::Detached => "detached",
                        super::copy_component::Gen3dCopyMode::Linked => "linked",
                    },
                    "alignment": match alignment {
                        super::copy_component::Gen3dCopyAlignmentMode::Rotation => "rotation",
                        super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX => "mirror_mount_x",
                    },
                }));
            }

            if let Some(root_idx) = job
                .planned_components
                .iter()
                .position(|c| c.attach_to.is_none())
            {
                let _ = super::convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                );
            }
            super::convert::update_root_def_from_planned_components(
                &job.planned_components,
                &job.plan_collider,
                draft,
            );
            write_gen3d_assembly_snapshot(job.step_dir.as_deref(), &job.planned_components);
            job.assembly_rev = job.assembly_rev.saturating_add(1);

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "ok": true,
                    "copies": copies_json,
                }),
            ))
        }
        TOOL_ID_COPY_COMPONENT_SUBTREE | TOOL_ID_MIRROR_COMPONENT_SUBTREE => {
            let source_name = call
                .args
                .get("source_root")
                .or_else(|| call.args.get("source_root_component"))
                .or_else(|| call.args.get("source_component"))
                .or_else(|| call.args.get("source"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let source_idx = call
                .args
                .get("source_root_index")
                .or_else(|| call.args.get("source_root_idx"))
                .or_else(|| call.args.get("source_index"))
                .or_else(|| call.args.get("source_idx"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or_else(|| {
                    source_name.as_deref().and_then(|name| {
                        job.planned_components
                            .iter()
                            .position(|c| c.name == name)
                            .or_else(|| {
                                resolve_component_index_by_name_hint(&job.planned_components, name)
                            })
                    })
                });
            let Some(source_idx) = source_idx else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.source_root (name) or args.source_root_index (index).".into(),
                ));
            };
            if source_idx >= job.planned_components.len() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("source_root_index out of range: {source_idx}"),
                ));
            }

            let mode = call
                .args
                .get("mode")
                .and_then(|v| v.as_str())
                .unwrap_or("detached")
                .trim()
                .to_ascii_lowercase();
            let mode = match mode.as_str() {
                "" | "detached" | "copy" | "duplicate" => {
                    super::copy_component::Gen3dCopyMode::Detached
                }
                "linked" | "link" | "shared" | "instance" => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "copy_component_subtree_v1 does not support mode=linked (use detached)."
                            .into(),
                    ));
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Unknown mode `{other}` (expected `detached`)"),
                    ));
                }
            };

            let anchors_mode = call
                .args
                .get("anchors")
                .or_else(|| call.args.get("anchors_mode"))
                .or_else(|| call.args.get("anchor_mode"))
                .and_then(|v| v.as_str())
                .unwrap_or("preserve_interfaces")
                .trim()
                .to_ascii_lowercase();
            let anchors_mode = match anchors_mode.as_str() {
                "" | "preserve_interfaces" | "preserve_interface" | "interfaces" | "interface" => {
                    super::copy_component::Gen3dCopyAnchorsMode::PreserveInterfaceAnchors
                }
                "preserve_target" | "preserve" | "target" => {
                    super::copy_component::Gen3dCopyAnchorsMode::PreserveTargetAnchors
                }
                "copy_source" | "copy" | "source" => {
                    super::copy_component::Gen3dCopyAnchorsMode::CopySourceAnchors
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Unknown anchors `{other}` (expected `preserve_interfaces`, `preserve_target`, or `copy_source`)"
                        ),
                    ));
                }
            };

            let delta = parse_delta_transform(call.args.get("transform"));
            let alignment = if call.tool_id.as_str() == TOOL_ID_MIRROR_COMPONENT_SUBTREE {
                super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX
            } else {
                super::copy_component::Gen3dCopyAlignmentMode::Rotation
            };
            let alignment_frame = call
                .args
                .get("alignment_frame")
                .or_else(|| call.args.get("frame"))
                .and_then(|v| v.as_str())
                .unwrap_or("join")
                .trim()
                .to_ascii_lowercase();
            let alignment_frame = match alignment_frame.as_str() {
                "" | "join" | "join_frame" => super::copy_component::Gen3dCopyAlignmentFrame::Join,
                "child_anchor" | "child" | "child_frame" => {
                    super::copy_component::Gen3dCopyAlignmentFrame::ChildAnchor
                }
                other => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Unknown alignment_frame `{other}` (expected `join` or `child_anchor`)"
                        ),
                    ));
                }
            };
            if alignment == super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX
                && alignment_frame != super::copy_component::Gen3dCopyAlignmentFrame::Join
            {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "mirror_component_subtree_v1 does not support alignment_frame=child_anchor (use join)."
                        .into(),
                ));
            }

            let Some(arr) = call.args.get("targets").and_then(|v| v.as_array()) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.targets (array of target root component names/indices)".into(),
                ));
            };
            let mut target_roots: Vec<usize> = Vec::new();
            for item in arr.iter() {
                if let Some(idx) = item.as_u64().map(|v| v as usize) {
                    target_roots.push(idx);
                } else if let Some(name) = item.as_str().map(|s| s.trim()).filter(|s| !s.is_empty())
                {
                    let idx = job
                        .planned_components
                        .iter()
                        .position(|c| c.name == name)
                        .or_else(|| {
                            resolve_component_index_by_name_hint(&job.planned_components, name)
                        });
                    let Some(idx) = idx else {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!("Unknown target root component `{name}`"),
                        ));
                    };
                    target_roots.push(idx);
                } else {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "targets must contain component indices or names".into(),
                    ));
                }
            }

            target_roots.sort_unstable();
            target_roots.dedup();
            if target_roots.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No targets provided".into(),
                ));
            }
            if target_roots
                .iter()
                .any(|&t| t >= job.planned_components.len())
            {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "One or more target indices are out of range".into(),
                ));
            }
            if target_roots.iter().any(|&t| t == source_idx) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Targets must not include the source_root component".into(),
                ));
            }

            let mut copies_json: Vec<serde_json::Value> = Vec::new();
            for target_root_idx in target_roots.iter().copied() {
                let outcomes = super::copy_component::copy_component_subtree_into(
                    &mut job.planned_components,
                    draft,
                    source_idx,
                    target_root_idx,
                    mode,
                    anchors_mode,
                    alignment,
                    alignment_frame,
                    delta,
                    super::copy_component::Gen3dSubtreeCopyMissingBranchPolicy::CloneAllMissing,
                );
                let outcomes = match outcomes {
                    Ok(v) => v,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                for outcome in outcomes {
                    copies_json.push(serde_json::json!({
                        "source": outcome.source_component_name,
                        "target": outcome.target_component_name,
                        "mode": match outcome.mode_used {
                            super::copy_component::Gen3dCopyMode::Detached => "detached",
                            super::copy_component::Gen3dCopyMode::Linked => "linked",
                        },
                        "alignment": match alignment {
                            super::copy_component::Gen3dCopyAlignmentMode::Rotation => "rotation",
                            super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX => "mirror_mount_x",
                        },
                    }));
                }
            }

            if let Some(root_idx) = job
                .planned_components
                .iter()
                .position(|c| c.attach_to.is_none())
            {
                let _ = super::convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                );
            }
            super::convert::update_root_def_from_planned_components(
                &job.planned_components,
                &job.plan_collider,
                draft,
            );
            write_gen3d_assembly_snapshot(job.step_dir.as_deref(), &job.planned_components);
            job.assembly_rev = job.assembly_rev.saturating_add(1);

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "ok": true,
                    "copies": copies_json,
                }),
            ))
        }
        TOOL_ID_DETACH_COMPONENT => {
            let target_name = call
                .args
                .get("component_name")
                .or_else(|| call.args.get("component_id"))
                .or_else(|| call.args.get("component"))
                .or_else(|| call.args.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let target_idx = call
                .args
                .get("component_index")
                .or_else(|| call.args.get("component_idx"))
                .or_else(|| call.args.get("index"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .or_else(|| {
                    target_name.as_deref().and_then(|name| {
                        job.planned_components
                            .iter()
                            .position(|c| c.name == name)
                            .or_else(|| {
                                resolve_component_index_by_name_hint(&job.planned_components, name)
                            })
                    })
                });
            let Some(target_idx) = target_idx else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.component (name) or args.component_index (index).".into(),
                ));
            };
            if target_idx >= job.planned_components.len() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("component_index out of range: {target_idx}"),
                ));
            }

            let outcome = match super::copy_component::detach_component_copy(
                &mut job.planned_components,
                draft,
                target_idx,
            ) {
                Ok(outcome) => outcome,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            if let Some(root_idx) = job
                .planned_components
                .iter()
                .position(|c| c.attach_to.is_none())
            {
                let _ = super::convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                );
            }
            super::convert::update_root_def_from_planned_components(
                &job.planned_components,
                &job.plan_collider,
                draft,
            );
            write_gen3d_assembly_snapshot(job.step_dir.as_deref(), &job.planned_components);
            job.assembly_rev = job.assembly_rev.saturating_add(1);

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "ok": true,
                    "component": outcome.target_component_name,
                    "mode": "detached",
                }),
            ))
        }
        TOOL_ID_LLM_SELECT_EDIT_STRATEGY => {
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            if job.planned_components.is_empty() || job.plan_hash.trim().is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "{TOOL_ID_LLM_SELECT_EDIT_STRATEGY} requires an existing accepted plan (planned_components + plan_hash)."
                    ),
                ));
            }

            let prompt_override = call.args.get("prompt").and_then(|v| v.as_str());
            let prompt_text = prompt_override
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(job.user_prompt_raw.as_str());

            let system = super::prompts::build_gen3d_edit_strategy_system_instructions();
            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());
            let user_text = super::prompts::build_gen3d_edit_strategy_user_text(
                prompt_text,
                image_object_summary,
                job.preserve_existing_components_mode,
                &job.planned_components,
            );

            let reasoning_effort = ai.model_reasoning_effort().to_string();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Selecting edit strategy…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for edit strategy…");
            job.agent.pending_llm_repair_attempt = 0;

            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::EditStrategyV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                step_dir,
                sanitize_prefix(&format!("tool_edit_strategy_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::SelectEditStrategy);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = "Selecting edit strategy…".into();
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_PLAN => {
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            let system = super::prompts::build_gen3d_plan_system_instructions();
            let prompt_override = call.args.get("prompt").and_then(|v| v.as_str());
            let style_hint = call.args.get("style").and_then(|v| v.as_str());
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PlanTemplateKvRefArgsV1 {
                namespace: String,
                key: String,
                #[serde(default)]
                selector: Option<InfoKvSelectorArgsV1>,
            }

            let plan_template_kv: Option<PlanTemplateKvRefArgsV1> =
                match call.args.get("plan_template_kv").cloned() {
                    Some(value) => match serde_json::from_value(value) {
                        Ok(v) => Some(v),
                        Err(err) => {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                call.call_id,
                                call.tool_id,
                                format!("Invalid args.plan_template_kv: {err}"),
                            ));
                        }
                    },
                    None => None,
                };
            let preserve_existing_components = call
                .args
                .get("constraints")
                .and_then(|v| v.get("preserve_existing_components"))
                .and_then(|v| v.as_bool())
                .unwrap_or(job.preserve_existing_components_mode);
            if should_require_plan_template_kv_for_preserve_replan(
                preserve_existing_components,
                job.planned_components.len(),
                plan_template_kv.is_some(),
            ) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    preserve_replan_missing_template_error(TOOL_ID_LLM_GENERATE_PLAN),
                ));
            }
            let preserve_edit_policy = call
                .args
                .get("constraints")
                .and_then(|v| v.get("preserve_edit_policy"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("additive");
            let preserve_edit_policy_is_valid = matches!(
                preserve_edit_policy,
                "additive" | "allow_offsets" | "allow_rewire"
            );
            if preserve_existing_components && !preserve_edit_policy_is_valid {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Invalid constraints.preserve_edit_policy={preserve_edit_policy:?}. Expected one of: \"additive\", \"allow_offsets\", \"allow_rewire\"."
                    ),
                ));
            }
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
            if required_component_names.len() > super::max_components_for_speed(workshop.speed_mode)
            {
                required_component_names
                    .truncate(super::max_components_for_speed(workshop.speed_mode));
            }

            let plan_template_json: Option<serde_json::Value> = if let Some(kv_ref) =
                plan_template_kv.as_ref()
            {
                if !preserve_existing_components || job.planned_components.is_empty() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "`plan_template_kv` requires preserve mode (constraints.preserve_existing_components=true) and an existing plan."
                            .into(),
                    ));
                }

                let namespace = kv_ref.namespace.trim();
                let key = kv_ref.key.trim();
                if namespace.is_empty() || key.is_empty() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "plan_template_kv.namespace and plan_template_kv.key are required.".into(),
                    ));
                }

                let selector_kind = kv_ref
                    .selector
                    .as_ref()
                    .map(|s| s.kind.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("latest");

                let store = match job.ensure_info_store() {
                    Ok(s) => s,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                let record = match select_kv_record(
                    store,
                    namespace,
                    key,
                    selector_kind,
                    kv_ref.selector.as_ref(),
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                let Some(record) = record else {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "plan_template_kv not found: namespace={namespace:?} key={key:?}. Call `{TOOL_ID_GET_PLAN_TEMPLATE}` first."
                        ),
                    ));
                };
                if record.bytes > 64 * 1024 {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "plan_template_kv is too large ({} bytes). Re-generate a smaller template via `{TOOL_ID_GET_PLAN_TEMPLATE}` (mode=\"auto\" or mode=\"lean\") and retry.",
                            record.bytes
                        ),
                    ));
                }

                Some(record.value.clone())
            } else {
                None
            };

            let prompt_text = prompt_override
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(job.user_prompt_raw.as_str());

            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());
            let user_text = if preserve_existing_components && !job.planned_components.is_empty() {
                super::prompts::build_gen3d_plan_user_text_preserve_existing_components(
                    prompt_text,
                    image_object_summary,
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
                    image_object_summary,
                    workshop.speed_mode,
                    style_hint,
                    &required_component_names,
                )
            };
            let mut user_text = user_text;
            if let Some(feedback) = call
                .args
                .get("qa_feedback")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                user_text.push_str("\nQA feedback:\n");
                user_text.push_str(feedback);
                user_text.push('\n');
            }
            let reasoning_effort = ai.model_reasoning_effort().to_string();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating plan…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for plan…");
            job.agent.pending_llm_repair_attempt = 0;

            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                job.user_images_component.clone(),
                step_dir,
                sanitize_prefix(&format!("tool_plan_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GeneratePlan);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_PLAN_OPS => {
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            if job.planned_components.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "{TOOL_ID_LLM_GENERATE_PLAN_OPS} requires an existing accepted plan. Run `{TOOL_ID_LLM_GENERATE_PLAN}` first."
                    ),
                ));
            }

            let system = super::prompts::build_gen3d_plan_ops_system_instructions();
            let prompt_override = call.args.get("prompt").and_then(|v| v.as_str());
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct PlanTemplateKvRefArgsV1 {
                namespace: String,
                key: String,
                #[serde(default)]
                selector: Option<InfoKvSelectorArgsV1>,
            }

            let plan_template_kv: Option<PlanTemplateKvRefArgsV1> =
                match call.args.get("plan_template_kv").cloned() {
                    Some(value) => match serde_json::from_value(value) {
                        Ok(v) => Some(v),
                        Err(err) => {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                call.call_id,
                                call.tool_id,
                                format!("Invalid args.plan_template_kv: {err}"),
                            ));
                        }
                    },
                    None => None,
                };

            let preserve_existing_components = call
                .args
                .get("constraints")
                .and_then(|v| v.get("preserve_existing_components"))
                .and_then(|v| v.as_bool())
                .unwrap_or(job.preserve_existing_components_mode);
            if !preserve_existing_components {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "{TOOL_ID_LLM_GENERATE_PLAN_OPS} is intended for preserve-mode diff-first replanning. Set constraints.preserve_existing_components=true, or use `{TOOL_ID_LLM_GENERATE_PLAN}` for a full replan."
                    ),
                ));
            }
            if should_require_plan_template_kv_for_preserve_replan(
                preserve_existing_components,
                job.planned_components.len(),
                plan_template_kv.is_some(),
            ) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    preserve_replan_missing_template_error(TOOL_ID_LLM_GENERATE_PLAN_OPS),
                ));
            }

            let preserve_edit_policy = call
                .args
                .get("constraints")
                .and_then(|v| v.get("preserve_edit_policy"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("additive");
            let preserve_edit_policy_is_valid = matches!(
                preserve_edit_policy,
                "additive" | "allow_offsets" | "allow_rewire"
            );
            if preserve_existing_components && !preserve_edit_policy_is_valid {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Invalid constraints.preserve_edit_policy={preserve_edit_policy:?}. Expected one of: \"additive\", \"allow_offsets\", \"allow_rewire\"."
                    ),
                ));
            }
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

            let scope_components: Vec<String> = call
                .args
                .get("scope_components")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if scope_components.len() > 64 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "scope_components is too large ({} > max 64)",
                        scope_components.len()
                    ),
                ));
            }

            let max_ops = call
                .args
                .get("max_ops")
                .and_then(|v| v.as_u64())
                .unwrap_or(32)
                .clamp(1, 64) as usize;

            let plan_template_json: Option<serde_json::Value> = if let Some(kv_ref) =
                plan_template_kv.as_ref()
            {
                if !preserve_existing_components || job.planned_components.is_empty() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "`plan_template_kv` requires preserve mode (constraints.preserve_existing_components=true) and an existing plan."
                            .into(),
                    ));
                }

                let namespace = kv_ref.namespace.trim();
                let key = kv_ref.key.trim();
                if namespace.is_empty() || key.is_empty() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "plan_template_kv.namespace and plan_template_kv.key are required.".into(),
                    ));
                }

                let selector_kind = kv_ref
                    .selector
                    .as_ref()
                    .map(|s| s.kind.trim())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("latest");

                let store = match job.ensure_info_store() {
                    Ok(s) => s,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                let record = match select_kv_record(
                    store,
                    namespace,
                    key,
                    selector_kind,
                    kv_ref.selector.as_ref(),
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };
                let Some(record) = record else {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "plan_template_kv not found: namespace={namespace:?} key={key:?}. Call `{TOOL_ID_GET_PLAN_TEMPLATE}` first."
                        ),
                    ));
                };
                if record.bytes > 64 * 1024 {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "plan_template_kv is too large ({} bytes). Re-generate a smaller template via `{TOOL_ID_GET_PLAN_TEMPLATE}` (mode=\"auto\" or mode=\"lean\") and retry.",
                            record.bytes
                        ),
                    ));
                }

                Some(record.value.clone())
            } else {
                None
            };

            let prompt_text = prompt_override
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(job.user_prompt_raw.as_str());

            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());

            let user_text =
                super::prompts::build_gen3d_plan_ops_user_text_preserve_existing_components(
                    prompt_text,
                    image_object_summary,
                    workshop.speed_mode,
                    None,
                    &job.planned_components,
                    &job.assembly_notes,
                    preserve_edit_policy,
                    &rewire_components,
                    plan_template_json.as_ref(),
                    &scope_components,
                    max_ops,
                );

            let reasoning_effort = ai.model_reasoning_effort().to_string();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating plan ops…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for plan ops…");
            job.agent.pending_llm_repair_attempt = 0;

            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanOpsV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                job.user_images_component.clone(),
                step_dir,
                sanitize_prefix(&format!("tool_plan_ops_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GeneratePlanOps);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_DRAFT_OPS => {
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };
            if job.planned_components.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "{TOOL_ID_LLM_GENERATE_DRAFT_OPS} requires an existing accepted plan. Run `{TOOL_ID_LLM_GENERATE_PLAN}` first."
                    ),
                ));
            }

            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct GenerateDraftOpsArgsV1 {
                prompt: String,
                #[serde(default)]
                scope_components: Vec<String>,
                #[serde(default)]
                max_ops: Option<u32>,
                #[serde(default)]
                strategy: Option<String>,
            }

            let args: GenerateDraftOpsArgsV1 = match serde_json::from_value(call.args.clone()) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`: {err}"),
                    ));
                }
            };

            let prompt_text = args.prompt.trim();
            if prompt_text.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("`{TOOL_ID_LLM_GENERATE_DRAFT_OPS}` requires a non-empty args.prompt string."),
                ));
            }

            let max_ops = args.max_ops.unwrap_or(24).clamp(1, 64) as usize;
            let strategy = args
                .strategy
                .as_deref()
                .unwrap_or("balanced")
                .trim()
                .to_string();
            if strategy != "conservative" && strategy != "balanced" {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Invalid strategy={strategy:?}. Expected one of: \"conservative\", \"balanced\"."
                    ),
                ));
            }

            let mut scope_components: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::<String>::new();
            for name in args.scope_components {
                let name = name.trim().to_string();
                if name.is_empty() {
                    continue;
                }
                let idx = job
                    .planned_components
                    .iter()
                    .position(|c| c.name == name)
                    .or_else(|| {
                        resolve_component_index_by_name_hint(&job.planned_components, &name)
                    })
                    .unwrap_or(usize::MAX);
                if idx == usize::MAX || idx >= job.planned_components.len() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Unknown scope_components entry: {name:?}. Hint: use component names from the plan."
                        ),
                    ));
                }
                let canonical = job.planned_components[idx].name.clone();
                if seen.insert(canonical.clone()) {
                    scope_components.push(canonical);
                }
            }

            let include_components: Vec<String> = if scope_components.is_empty() {
                job.planned_components
                    .iter()
                    .map(|c| c.name.clone())
                    .collect()
            } else {
                scope_components.clone()
            };

            let workspace_id = job.active_workspace_id().trim().to_string();
            let (snapshots, mut missing) = {
                let store = match job.ensure_info_store() {
                    Ok(s) => s,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };

                let mut snapshots: Vec<serde_json::Value> = Vec::new();
                let mut missing: Vec<String> = Vec::new();
                for component in &include_components {
                    let component_seg = normalize_identifier_for_match(component.as_str());
                    let component_seg = if component_seg.is_empty() {
                        "unknown".to_string()
                    } else {
                        component_seg
                    };
                    let key = format!("ws.{workspace_id}.component_parts.{component_seg}");
                    if let Some(record) =
                        store.kv_latest_record(INFO_KV_NAMESPACE_GEN3D, key.as_str())
                    {
                        snapshots.push(record.value.clone());
                    } else {
                        missing.push(component.clone());
                    }
                }

                (snapshots, missing)
            };

            if !missing.is_empty() {
                missing.sort();
                missing.dedup();
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Missing component parts snapshots for: {missing:?}.\n\
Hint: Call `{TOOL_ID_QUERY_COMPONENT_PARTS}` for these component(s) first, then retry `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`.\n\
Example: {{\"component\":\"cannon\",\"max_parts\":128}}"
                    ),
                ));
            }
            if snapshots.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "No component parts snapshots available.\n\
Hint: Call `{TOOL_ID_QUERY_COMPONENT_PARTS}` first, then retry `{TOOL_ID_LLM_GENERATE_DRAFT_OPS}`."
                    ),
                ));
            }

            let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
            let scene_graph_summary = super::build_gen3d_scene_graph_summary(
                &run_id,
                job.attempt,
                job.step,
                &job.plan_hash,
                job.assembly_rev,
                &job.planned_components,
                draft,
            );
            if let Some(dir) = job.step_dir.as_deref() {
                write_gen3d_json_artifact(
                    Some(dir),
                    "scene_graph_summary.json",
                    &scene_graph_summary,
                );
            }

            let system = super::prompts::build_gen3d_draft_ops_system_instructions();
            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());
            let user_text = super::prompts::build_gen3d_draft_ops_user_text(
                prompt_text,
                image_object_summary,
                &run_id,
                job.attempt,
                job.step,
                &job.plan_hash,
                job.assembly_rev,
                strategy.as_str(),
                max_ops,
                &scene_graph_summary,
                &snapshots,
                scope_components.as_slice(),
            );

            let reasoning_effort = ai.model_reasoning_effort().to_string();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating DraftOps…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for DraftOps…");
            job.agent.pending_llm_repair_attempt = 0;

            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::DraftOpsV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                step_dir,
                sanitize_prefix(&format!("tool_draft_ops_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GenerateDraftOps);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = "Generating DraftOps…".into();
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_COMPONENT => {
            let component_name = call
                .args
                .get("component_name")
                .or_else(|| call.args.get("name_hint"))
                .or_else(|| call.args.get("component_id"))
                .or_else(|| call.args.get("component"))
                .or_else(|| call.args.get("name"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let component_idx = call
                .args
                .get("component_index")
                .or_else(|| call.args.get("component_idx"))
                .or_else(|| call.args.get("index"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let idx = if let Some(idx) = component_idx {
                idx
            } else if let Some(name) = component_name.as_deref() {
                job.planned_components
                    .iter()
                    .position(|c| c.name == name)
                    .or_else(|| resolve_component_index_by_name_hint(&job.planned_components, name))
                    .unwrap_or(usize::MAX)
            } else {
                usize::MAX
            };
            if idx == usize::MAX || idx >= job.planned_components.len() {
                let available: Vec<String> = job
                    .planned_components
                    .iter()
                    .take(24)
                    .map(|c| c.name.clone())
                    .collect();
                let hint = component_name.unwrap_or_else(|| "<none>".into());
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Invalid component_name/component_index. Hint={hint:?}. Available (first {}): {available:?}",
                        available.len()
                    ),
                ));
            }

            job.agent
                .pending_regen_component_indices
                .retain(|pending| *pending != idx);
            job.agent
                .pending_regen_component_indices_skipped_due_to_budget
                .retain(|pending| *pending != idx);

            let is_regen = job
                .planned_components
                .get(idx)
                .map(|c| c.actual_size.is_some())
                .unwrap_or(false);
            let force = call
                .args
                .get("force")
                .or_else(|| call.args.get("regen"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if job.preserve_existing_components_mode && is_regen && !force {
                let name = job
                    .planned_components
                    .get(idx)
                    .map(|c| c.name.as_str())
                    .unwrap_or("<unknown>");
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "ok": true,
                        "skipped_due_to_preserve_existing_components": true,
                        "note": "This run is in preserve-existing-components mode. Already-generated components are not regenerated unless {\"force\":true}. NOTE: force regeneration is QA-gated and only allowed when qa_v1 reports errors (last_validate_ok=false or last_smoke_ok=false).",
                        "component_index": idx,
                        "component_name": name,
                    }),
                ));
            }
            if is_regen && force {
                let validate_ok = job.agent.last_validate_ok;
                let smoke_ok = job.agent.last_smoke_ok;
                let has_errors = validate_ok == Some(false) || smoke_ok == Some(false);
                if !has_errors {
                    let name = job
                        .planned_components
                        .get(idx)
                        .map(|c| c.name.as_str())
                        .unwrap_or("<unknown>");
                    let reason = if validate_ok.is_none() || smoke_ok.is_none() {
                        "qa_v1 has not been run (or is incomplete)"
                    } else {
                        "qa_v1 reports no errors"
                    };
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!(
                            "Refusing force:true regeneration for component `{name}` because {reason}. validate_ok={validate_ok:?} smoke_ok={smoke_ok:?}. Run `qa_v1` and only use force regen when there are errors. For placement/assembly fixes, prefer `llm_review_delta_v1` / `apply_draft_ops_v1` instead of regenerating geometry. If you intend a style/geometry rebuild in a seeded edit, disable preserve mode via `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`, then regenerate without `force`."
                        ),
                    ));
                }
            }
            if is_regen && !consume_regen_budget(config, job, idx) {
                let name = job
                    .planned_components
                    .get(idx)
                    .map(|c| c.name.as_str())
                    .unwrap_or("<unknown>");
                append_gen3d_run_log(
                    job.step_dir.as_deref(),
                    format!(
                        "regen_budget_skip idx={} name={} max_total={} max_per_component={}",
                        idx,
                        name,
                        config.gen3d_max_regen_total,
                        config.gen3d_max_regen_per_component
                    ),
                );
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "ok": true,
                        "skipped_due_to_regen_budget": true,
                        "component_index": idx,
                        "component_name": name,
                        "max_regen_total": config.gen3d_max_regen_total,
                        "max_regen_per_component": config.gen3d_max_regen_per_component,
                        "regen_total": job.regen_total,
                        "regen_count": job.regen_per_component.get(idx).copied().unwrap_or(0),
                    }),
                ));
            }

            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating component…".into(),
            }));
            job.shared_progress = Some(progress.clone());

            let system = super::prompts::build_gen3d_component_system_instructions();
            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());
            let user_text = super::prompts::build_gen3d_component_user_text(
                &job.user_prompt_raw,
                image_object_summary,
                workshop.speed_mode,
                &job.assembly_notes,
                &job.planned_components,
                idx,
            );
            job.agent.pending_llm_repair_attempt = 0;
            let reasoning_effort = ai.model_reasoning_effort().to_string();
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                job.user_images_component.clone(),
                step_dir,
                sanitize_prefix(&format!("tool_component{}_{}", idx + 1, &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool =
                Some(super::Gen3dAgentLlmToolKind::GenerateComponent { component_idx: idx });
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_COMPONENTS => {
            let Some(_ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(_step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            let force = call
                .args
                .get("force")
                .or_else(|| call.args.get("regen"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            let mut requested_indices: Vec<usize> = Vec::new();
            let mut seen = std::collections::HashSet::<usize>::new();

            let indices_value = call
                .args
                .get("component_indices")
                .or_else(|| call.args.get("indices"))
                .or_else(|| call.args.get("component_idx"))
                .or_else(|| call.args.get("component_indexes"));
            if let Some(arr) = indices_value.and_then(|v| v.as_array()) {
                for v in arr {
                    let Some(raw) = v
                        .as_u64()
                        .or_else(|| v.as_i64().and_then(|i| (i >= 0).then_some(i as u64)))
                    else {
                        continue;
                    };
                    let idx = raw as usize;
                    if idx < job.planned_components.len() && seen.insert(idx) {
                        requested_indices.push(idx);
                    }
                }
            }

            let names_value = call
                .args
                .get("component_names")
                .or_else(|| call.args.get("names"))
                .or_else(|| call.args.get("components"));
            if let Some(arr) = names_value.and_then(|v| v.as_array()) {
                for v in arr {
                    let Some(name) = v.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                        continue;
                    };
                    let idx = job
                        .planned_components
                        .iter()
                        .position(|c| c.name == name)
                        .or_else(|| {
                            resolve_component_index_by_name_hint(&job.planned_components, name)
                        });
                    let Some(idx) = idx else {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!("Unknown component name hint: {name:?}"),
                        ));
                    };
                    if seen.insert(idx) {
                        requested_indices.push(idx);
                    }
                }
            }

            let missing_only_arg = call.args.get("missing_only").and_then(|v| v.as_bool());
            let mut missing_only = missing_only_arg.unwrap_or(requested_indices.is_empty());
            if job.preserve_existing_components_mode && !force {
                missing_only = true;
            }
            let mut optimized_by_reuse_groups = false;
            let mut skipped_due_to_reuse_groups: Vec<usize> = Vec::new();
            let mut skipped_due_to_preserve_existing_components: Vec<usize> = Vec::new();

            if requested_indices.is_empty() {
                if missing_only {
                    if !job.reuse_groups.is_empty() {
                        optimized_by_reuse_groups = true;
                        let optimized = super::reuse_groups::missing_only_generation_indices(
                            &job.planned_components,
                            &job.reuse_groups,
                        );
                        let mut included = vec![false; job.planned_components.len()];
                        for idx in optimized.iter().copied() {
                            if idx < included.len() {
                                included[idx] = true;
                            }
                        }
                        for (idx, comp) in job.planned_components.iter().enumerate() {
                            if comp.actual_size.is_some() {
                                continue;
                            }
                            if !included[idx] {
                                skipped_due_to_reuse_groups.push(idx);
                            }
                        }
                        requested_indices = optimized;
                    } else {
                        for (idx, comp) in job.planned_components.iter().enumerate() {
                            if comp.actual_size.is_none() {
                                requested_indices.push(idx);
                            }
                        }
                    }
                } else {
                    requested_indices.extend(0..job.planned_components.len());
                }
            }

            if missing_only && !force {
                for idx in requested_indices.iter().copied() {
                    let is_generated = job
                        .planned_components
                        .get(idx)
                        .map(|c| c.actual_size.is_some())
                        .unwrap_or(false);
                    if is_generated {
                        skipped_due_to_preserve_existing_components.push(idx);
                    }
                }
                requested_indices.retain(|&idx| {
                    job.planned_components
                        .get(idx)
                        .map(|c| c.actual_size.is_none())
                        .unwrap_or(false)
                });
            }

            if force {
                let wants_regen = requested_indices.iter().any(|idx| {
                    job.planned_components
                        .get(*idx)
                        .map(|c| c.actual_size.is_some())
                        .unwrap_or(false)
                });
                if wants_regen {
                    let validate_ok = job.agent.last_validate_ok;
                    let smoke_ok = job.agent.last_smoke_ok;
                    let has_errors = validate_ok == Some(false) || smoke_ok == Some(false);
                    if !has_errors {
                        // Safety net: if the agent tries a QA-gated force regen, clear any
                        // requested regen indices out of the pending queue so the run cannot
                        // deadlock on an un-executable "pending regen" list.
                        let mut blocked_regen_indices: Vec<usize> = requested_indices
                            .iter()
                            .copied()
                            .filter(|idx| {
                                job.planned_components
                                    .get(*idx)
                                    .map(|c| c.actual_size.is_some())
                                    .unwrap_or(false)
                            })
                            .collect();
                        blocked_regen_indices.sort_unstable();
                        blocked_regen_indices.dedup();
                        if !blocked_regen_indices.is_empty() {
                            let blocked_set: std::collections::HashSet<usize> =
                                blocked_regen_indices.iter().copied().collect();
                            job.agent
                                .pending_regen_component_indices
                                .retain(|pending| !blocked_set.contains(pending));
                            job.agent
                                .pending_regen_component_indices_skipped_due_to_budget
                                .retain(|pending| !blocked_set.contains(pending));

                            let mut merged: std::collections::HashSet<usize> = job
                                .agent
                                .pending_regen_component_indices_blocked_due_to_qa_gate
                                .iter()
                                .copied()
                                .collect();
                            for idx in blocked_regen_indices {
                                merged.insert(idx);
                            }
                            let mut merged: Vec<usize> = merged.into_iter().collect();
                            merged.sort_unstable();
                            job.agent
                                .pending_regen_component_indices_blocked_due_to_qa_gate = merged;
                        }

                        let reason = if validate_ok.is_none() || smoke_ok.is_none() {
                            "qa_v1 has not been run (or is incomplete)"
                        } else {
                            "qa_v1 reports no errors"
                        };
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
	                            call.call_id,
	                            call.tool_id,
	                            format!(
	                                "Refusing force:true regeneration because {reason}. validate_ok={validate_ok:?} smoke_ok={smoke_ok:?}. Run `qa_v1` and only use force regen when there are errors. For placement/assembly fixes, prefer `llm_review_delta_v1` / `apply_draft_ops_v1` instead of regenerating geometry. If you intend a style/geometry rebuild in a seeded edit, disable preserve mode via `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`, then regenerate without `force`."
	                            ),
	                        ));
                    }
                }
            }

            if requested_indices.is_empty() {
                let skipped_due_to_preserve_existing_components_json: Vec<serde_json::Value> =
                    skipped_due_to_preserve_existing_components
                        .iter()
                        .copied()
                        .filter(|idx| *idx < job.planned_components.len())
                        .map(|idx| {
                            serde_json::json!({
                                "index": idx,
                                "name": job.planned_components[idx].name.as_str(),
                            })
                        })
                        .collect();
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "requested": 0,
                        "succeeded": 0,
                        "failed": [],
                        "skipped_due_to_preserve_existing_components": skipped_due_to_preserve_existing_components_json,
                    }),
                ));
            }

            let request_set: std::collections::HashSet<usize> =
                requested_indices.iter().copied().collect();
            job.agent
                .pending_regen_component_indices
                .retain(|pending| !request_set.contains(pending));
            job.agent
                .pending_regen_component_indices_skipped_due_to_budget
                .retain(|pending| !request_set.contains(pending));

            // Enforce regen budgets for any components that are already generated (regen attempts).
            // Missing components are always allowed.
            let mut skipped_due_to_regen_budget: Vec<usize> = Vec::new();
            let mut filtered_indices: Vec<usize> = Vec::with_capacity(requested_indices.len());
            for idx in requested_indices {
                let is_regen = job
                    .planned_components
                    .get(idx)
                    .map(|c| c.actual_size.is_some())
                    .unwrap_or(false);
                if !is_regen {
                    filtered_indices.push(idx);
                    continue;
                }
                if consume_regen_budget(config, job, idx) {
                    filtered_indices.push(idx);
                    continue;
                }
                skipped_due_to_regen_budget.push(idx);
            }
            if !skipped_due_to_regen_budget.is_empty() {
                append_gen3d_run_log(
                    job.step_dir.as_deref(),
                    format!(
                        "regen_budget_skip_batch skipped={} max_total={} max_per_component={}",
                        skipped_due_to_regen_budget.len(),
                        config.gen3d_max_regen_total,
                        config.gen3d_max_regen_per_component
                    ),
                );
            }

            let requested_indices = filtered_indices;
            if requested_indices.is_empty() {
                let skipped_due_to_preserve_existing_components_json: Vec<serde_json::Value> =
                    skipped_due_to_preserve_existing_components
                        .iter()
                        .copied()
                        .filter(|idx| *idx < job.planned_components.len())
                        .map(|idx| {
                            serde_json::json!({
                                "index": idx,
                                "name": job.planned_components[idx].name.as_str(),
                            })
                        })
                        .collect();
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "requested": 0,
                        "succeeded": 0,
                        "failed": [],
                        "skipped_due_to_preserve_existing_components": skipped_due_to_preserve_existing_components_json,
                        "skipped_due_to_regen_budget": skipped_due_to_regen_budget,
                        "max_regen_total": config.gen3d_max_regen_total,
                        "max_regen_per_component": config.gen3d_max_regen_per_component,
                        "regen_total": job.regen_total,
                    }),
                ));
            }

            job.component_queue = requested_indices.clone();
            job.component_in_flight.clear();
            if job.component_attempts.len() < job.planned_components.len() {
                job.component_attempts
                    .resize(job.planned_components.len(), 0);
            }
            if job.component_last_errors.len() < job.planned_components.len() {
                job.component_last_errors
                    .resize(job.planned_components.len(), None);
            }
            for idx in &requested_indices {
                if *idx < job.component_attempts.len() {
                    job.component_attempts[*idx] = 0;
                }
                if *idx < job.component_last_errors.len() {
                    job.component_last_errors[*idx] = None;
                }
            }

            job.agent.pending_component_batch = Some(super::Gen3dPendingComponentBatch {
                requested_indices,
                optimized_by_reuse_groups,
                skipped_due_to_reuse_groups,
                skipped_due_to_preserve_existing_components,
                skipped_due_to_regen_budget,
                completed_indices: std::collections::HashSet::new(),
                failed: Vec::new(),
            });

            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating components (batch)…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Generating components (batch)…");

            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool =
                Some(super::Gen3dAgentLlmToolKind::GenerateComponentsBatch);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = "Generating components (batch)…".into();
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_MOTION => {
            if job.planned_components.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No planned components yet. Generate a plan first.".into(),
                ));
            }
            let channel = call
                .args
                .get("channel")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_ascii_lowercase());
            let Some(channel) = channel else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing required arg: channel".into(),
                ));
            };
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };
            let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: format!("Authoring motion ({})…", channel.as_str()),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(
                &progress,
                format!("Calling model for motion authoring ({})…", channel.as_str()),
            );
            job.agent.pending_llm_repair_attempt = 0;

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

            let system = super::prompts::build_gen3d_motion_authoring_system_instructions();
            let image_object_summary = job
                .user_image_object_summary
                .as_ref()
                .map(|s| s.text.as_str());
            let explicit_motion_channels = job
                .prompt_intent
                .as_ref()
                .map(|intent| intent.explicit_motion_channels.as_slice())
                .unwrap_or(&[]);
            let user_text = super::prompts::build_gen3d_motion_authoring_user_text(
                &job.user_prompt_raw,
                image_object_summary,
                &run_id,
                job.attempt,
                &job.plan_hash,
                job.assembly_rev,
                &channel,
                explicit_motion_channels,
                job.rig_move_cycle_m,
                has_idle_slot,
                has_move_slot,
                &job.planned_components,
                draft,
            );
            let mut user_text = user_text;
            if let Some(feedback) = call
                .args
                .get("qa_feedback")
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
            {
                user_text.push_str("\nQA feedback:\n");
                user_text.push_str(feedback);
                user_text.push('\n');
            }
            let reasoning_effort = ai.model_reasoning_effort().to_string();
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.ai_request_timeout(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::MotionAuthoringV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                step_dir,
                sanitize_prefix(&format!(
                    "tool_motion_{}_{}",
                    channel.as_str(),
                    &call.call_id
                )),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GenerateMotion);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = format!("Authoring motion ({})…", channel.as_str());
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_LLM_GENERATE_MOTIONS => {
            if job.planned_components.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No planned components yet. Generate a plan first.".into(),
                ));
            }
            let Some(_ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(_step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            let Some(arr) = call.args.get("channels").and_then(|v| v.as_array()) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing required arg: channels".into(),
                ));
            };
            let mut channels: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::<String>::new();
            for v in arr {
                let Some(raw) = v.as_str() else {
                    continue;
                };
                let ch = raw.trim().to_ascii_lowercase();
                if ch.is_empty() {
                    continue;
                }
                if seen.insert(ch.clone()) {
                    channels.push(ch);
                }
            }
            if channels.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No valid channels provided. Example: {\"channels\":[\"move\",\"action\"]}"
                        .into(),
                ));
            }

            job.motion_queue = channels.clone();
            job.motion_in_flight.clear();
            job.motion_attempts.clear();
            job.motion_last_errors.clear();

            job.agent.pending_motion_batch = Some(super::Gen3dPendingMotionBatch {
                requested_channels: channels,
                completed_channels: std::collections::HashSet::new(),
                failed: Vec::new(),
            });

            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Authoring motion channels (batch)…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Authoring motion channels (batch)…");

            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GenerateMotionsBatch);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = "Authoring motion channels (batch)…".into();
            ToolCallOutcome::StartedAsync
        }
        TOOL_ID_RENDER_PREVIEW => {
            if draft.total_non_projectile_primitive_parts() == 0 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Nothing to render yet (0 non-projectile primitive parts). Generate components first."
                        .to_string(),
                ));
            }
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };
            let views = call
                .args
                .get("views")
                .or_else(|| call.args.get("angles"))
                .or_else(|| call.args.get("view"))
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let overlay = call
                .args
                .get("overlay")
                .and_then(|v| v.as_str())
                .unwrap_or("none");
            let include_overlay = matches!(overlay, "axes_grid");
            let prefix = call
                .args
                .get("prefix")
                .and_then(|v| v.as_str())
                .unwrap_or("render");
            let prefix = sanitize_prefix(prefix);
            let include_motion_sheets = call
                .args
                .get("include_motion_sheets")
                .or_else(|| call.args.get("motion_sheets"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let resolution_px = call
                .args
                .get("resolution")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok());
            let width_arg = call
                .args
                .get("width")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok());
            let height_arg = call
                .args
                .get("height")
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok());
            let image_size_px = call
                .args
                .get("image_size")
                .or_else(|| call.args.get("image_size_px"))
                .or_else(|| call.args.get("image_px"))
                .and_then(|v| v.as_u64())
                .and_then(|v| u32::try_from(v).ok());

            let (width_px, height_px) = if let Some(res) = resolution_px {
                (res, res)
            } else if width_arg.is_some() || height_arg.is_some() {
                (
                    width_arg.unwrap_or(super::super::GEN3D_REVIEW_CAPTURE_WIDTH_PX),
                    height_arg.unwrap_or(super::super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX),
                )
            } else if let Some(size) = image_size_px {
                // Back-compat/robustness: some agent steps use `image_size` even though the
                // tool schema prefers `resolution` or `width`+`height`. Interpret `image_size`
                // as the maximum dimension and scale the default 16:9 review capture size.
                let size = size.clamp(256, 4096) as f32;
                let base_w = super::super::GEN3D_REVIEW_CAPTURE_WIDTH_PX as f32;
                let base_h = super::super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX as f32;
                let base_max = base_w.max(base_h).max(1.0);
                let scale = (size / base_max).max(1e-3);
                let w = (base_w * scale).round().clamp(256.0, 4096.0) as u32;
                let h = (base_h * scale).round().clamp(256.0, 4096.0) as u32;
                (w, h)
            } else {
                (
                    super::super::GEN3D_REVIEW_CAPTURE_WIDTH_PX,
                    super::super::GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
                )
            };
            let width_px = width_px.clamp(256, 4096);
            let height_px = height_px.clamp(256, 4096);

            let _background = call
                .args
                .get("background")
                .and_then(|v| v.as_str())
                .unwrap_or("default");

            let parsed_views: Vec<super::Gen3dReviewView> = if views.is_empty() {
                vec![
                    super::Gen3dReviewView::Front,
                    super::Gen3dReviewView::FrontLeft,
                    super::Gen3dReviewView::LeftBack,
                    super::Gen3dReviewView::Back,
                    super::Gen3dReviewView::RightBack,
                    super::Gen3dReviewView::FrontRight,
                    super::Gen3dReviewView::Top,
                    super::Gen3dReviewView::Bottom,
                ]
            } else {
                let mut out = Vec::new();
                for v in views {
                    let Some(s) = v.as_str() else {
                        continue;
                    };
                    let view = match normalize_identifier_for_match(s).as_str() {
                        "front" => super::Gen3dReviewView::Front,
                        "front_3q" | "front_three_quarter" | "front_quarter" => {
                            super::Gen3dReviewView::FrontLeft
                        }
                        "front_left" => super::Gen3dReviewView::FrontLeft,
                        "left" | "side" | "profile" => super::Gen3dReviewView::FrontLeft,
                        "left_back" => super::Gen3dReviewView::LeftBack,
                        "rear_3q" | "rear_three_quarter" => super::Gen3dReviewView::LeftBack,
                        "back" => super::Gen3dReviewView::Back,
                        "right_back" => super::Gen3dReviewView::RightBack,
                        "front_right" => super::Gen3dReviewView::FrontRight,
                        "top" => super::Gen3dReviewView::Top,
                        "bottom" => super::Gen3dReviewView::Bottom,
                        _ => continue,
                    };
                    out.push(view);
                }
                if out.is_empty() {
                    vec![super::Gen3dReviewView::Front]
                } else {
                    out
                }
            };

            match super::start_gen3d_review_capture(
                commands,
                images,
                &step_dir,
                draft,
                include_overlay,
                &prefix,
                &parsed_views,
                width_px,
                height_px,
            ) {
                Ok(state) => {
                    job.agent.pending_render_include_motion_sheets = include_motion_sheets;
                    job.agent.pending_tool_call = Some(call);
                    job.agent.pending_render = Some(state);
                    job.phase = Gen3dAiPhase::AgentCapturingRender;
                    ToolCallOutcome::StartedAsync
                }
                Err(err) => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    err,
                )),
            }
        }
        TOOL_ID_LLM_REVIEW_DELTA => {
            if job.ai.is_none() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(step_dir) = job.step_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing step dir".into(),
                ));
            };

            let rounds_max = config.gen3d_review_delta_rounds_max;
            let rounds_used = job.review_delta_rounds_used;
            if rounds_max == 0 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err_with_result(
                    call.call_id,
                    call.tool_id,
                    "Review-delta is disabled by config (gen3d.review_delta_rounds_max=0).".into(),
                    serde_json::json!({
                        "kind": "review_delta_disabled",
                        "used": rounds_used,
                        "max": rounds_max,
                        "guidance": "Do not call llm_review_delta_v1. Use deterministic tools (apply_draft_ops_v1 / apply_plan_ops_v1) and finish best-effort after qa_v1.",
                        "fixits": [
                            { "tool_id": TOOL_ID_QA, "args": {} }
                        ],
                    }),
                ));
            }
            if rounds_used >= rounds_max {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err_with_result(
                    call.call_id,
                    call.tool_id,
                    format!("Review-delta budget exhausted (used={rounds_used} max={rounds_max})."),
                    serde_json::json!({
                        "kind": "review_delta_budget_exhausted",
                        "used": rounds_used,
                        "max": rounds_max,
                        "guidance": "Do not call llm_review_delta_v1 again in this run. Run qa_v1 if needed, then finish best-effort or use deterministic tools (apply_draft_ops_v1 / apply_plan_ops_v1). Start a new run if you need more review-delta iterations.",
                        "fixits": [
                            { "tool_id": TOOL_ID_QA, "args": {} }
                        ],
                    }),
                ));
            }

            if let Some(obj) = call.args.as_object() {
                if obj.contains_key("preview_images") || obj.contains_key("preview_paths") {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "Deprecated args.preview_images/args.preview_paths. Use args.preview_blob_ids (blob ids from render_preview_v1 / info_blobs_list_v1).".into(),
                    ));
                }
                if obj.contains_key("include_original_images") {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "Unsupported args.include_original_images. User reference photos are summarized into text and are not sent to the LLM; omit this key.".into(),
                    ));
                }
                for key in obj.keys() {
                    if key != "preview_blob_ids" && key != "blob_ids" {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!(
                                "Unknown args key `{key}`. Allowed keys: preview_blob_ids, blob_ids."
                            ),
                        ));
                    }
                }
            }

            let preview_blobs_were_explicit =
                !parse_review_preview_blob_ids_from_args(&call.args).is_empty();
            let last_render_fresh = !job.agent.last_render_blob_ids.is_empty()
                && job.agent.last_render_assembly_rev == Some(job.assembly_rev);
            let can_render = draft.total_non_projectile_primitive_parts() > 0;

            if job.review_appearance
                && !preview_blobs_were_explicit
                && !last_render_fresh
                && can_render
            {
                let smoke_results = super::build_gen3d_smoke_results(
                    job.prompt_intent.as_ref().map(|i| i.requires_attack),
                    !job.user_images.is_empty(),
                    job.rig_move_cycle_m,
                    &job.planned_components,
                    draft,
                );
                let (include_move_sheet, include_action_sheet, include_attack_sheet) =
                    motion_sheets_needed_from_smoke_results(&smoke_results);
                let include_motion_sheets =
                    include_move_sheet || include_action_sheet || include_attack_sheet;

                let prefix = sanitize_prefix(&format!("review_prerender_{}", call.call_id));
                let views = [
                    super::Gen3dReviewView::Front,
                    super::Gen3dReviewView::LeftBack,
                    super::Gen3dReviewView::RightBack,
                    super::Gen3dReviewView::Top,
                    super::Gen3dReviewView::Bottom,
                ];
                let (width_px, height_px) = review_capture_dimensions_for_max_dim(960);
                match super::start_gen3d_review_capture(
                    commands, images, &step_dir, draft, false, &prefix, &views, width_px, height_px,
                ) {
                    Ok(state) => {
                        job.agent.pending_render_include_motion_sheets = include_motion_sheets;
                        job.agent.pending_tool_call = Some(call);
                        job.agent.pending_render = Some(state);
                        job.phase = Gen3dAiPhase::AgentCapturingRender;
                        ToolCallOutcome::StartedAsync
                    }
                    Err(err) => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    )),
                }
            } else {
                let call_id = call.call_id.clone();
                let tool_id = call.tool_id.clone();
                match start_agent_llm_review_delta_call(config, job, draft, call) {
                    Ok(()) => ToolCallOutcome::StartedAsync,
                    Err(err) => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    )),
                }
            }
        }
        TOOL_ID_CREATE_WORKSPACE => {
            let from = call
                .args
                .get("from")
                .or_else(|| call.args.get("base"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| job.agent.active_workspace_id.as_str())
                .to_string();
            let name = call
                .args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let include_components: Vec<String> = call
                .args
                .get("include_components")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();

            let (
                source_defs,
                source_planned_components,
                source_plan_hash,
                source_assembly_rev,
                source_assembly_notes,
                source_plan_collider,
                source_rig_move_cycle_m,
                source_motion_authoring,
                source_reuse_groups,
                source_reuse_group_warnings,
            ) = if from == job.agent.active_workspace_id {
                (
                    draft.defs.clone(),
                    job.planned_components.clone(),
                    job.plan_hash.clone(),
                    job.assembly_rev,
                    job.assembly_notes.clone(),
                    job.plan_collider.clone(),
                    job.rig_move_cycle_m,
                    job.motion_authoring.clone(),
                    job.reuse_groups.clone(),
                    job.reuse_group_warnings.clone(),
                )
            } else if let Some(ws) = job.agent.workspaces.get(&from) {
                (
                    ws.defs.clone(),
                    ws.planned_components.clone(),
                    ws.plan_hash.clone(),
                    ws.assembly_rev,
                    ws.assembly_notes.clone(),
                    ws.plan_collider.clone(),
                    ws.rig_move_cycle_m,
                    ws.motion_authoring.clone(),
                    ws.reuse_groups.clone(),
                    ws.reuse_group_warnings.clone(),
                )
            } else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown workspace `{from}`"),
                ));
            };

            let new_defs = if include_components.is_empty() {
                source_defs
            } else {
                match build_component_subset_workspace_defs(&source_defs, &include_components) {
                    Ok(defs) => defs,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                }
            };

            let mut workspace_id = call
                .args
                .get("workspace_id")
                .or_else(|| call.args.get("id"))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                // Common agent behavior: provide only `name` and then try to `set_active_workspace`
                // using the same string. Treat `name` as the workspace_id in that case.
                .or_else(|| (!name.is_empty()).then_some(name.clone()))
                // Default: create a predictable workspace id so the agent can refer to it within
                // the same step without having to depend on tool return values.
                .unwrap_or_else(|| "preview".to_string());

            let normalized = normalize_identifier_for_match(&workspace_id);
            workspace_id = if normalized.is_empty() {
                format!("ws{}", job.agent.next_workspace_seq)
            } else {
                normalized
            };

            if workspace_id == job.agent.active_workspace_id
                || job.agent.workspaces.contains_key(&workspace_id)
            {
                workspace_id = format!("ws{}", job.agent.next_workspace_seq);
            }
            job.agent.next_workspace_seq = job.agent.next_workspace_seq.saturating_add(1);

            if workspace_id == job.agent.active_workspace_id {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "workspace_id must not be the active workspace".into(),
                ));
            }
            if job.agent.workspaces.contains_key(&workspace_id) {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("workspace_id already exists: `{workspace_id}`"),
                ));
            }

            job.agent.workspaces.insert(
                workspace_id.clone(),
                super::Gen3dAgentWorkspace {
                    name: if name.is_empty() {
                        workspace_id.clone()
                    } else {
                        name
                    },
                    defs: new_defs,
                    planned_components: source_planned_components,
                    plan_hash: source_plan_hash,
                    assembly_rev: source_assembly_rev,
                    assembly_notes: source_assembly_notes,
                    plan_collider: source_plan_collider,
                    rig_move_cycle_m: source_rig_move_cycle_m,
                    motion_authoring: source_motion_authoring,
                    reuse_groups: source_reuse_groups,
                    reuse_group_warnings: source_reuse_group_warnings,
                },
            );

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "workspace_id": workspace_id }),
            ))
        }
        TOOL_ID_DELETE_WORKSPACE => {
            let workspace_id = call
                .args
                .get("workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if workspace_id.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing workspace_id".into(),
                ));
            }
            if workspace_id == job.agent.active_workspace_id {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Cannot delete the active workspace".into(),
                ));
            }
            let removed = job.agent.workspaces.remove(&workspace_id).is_some();
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "ok": removed }),
            ))
        }
        TOOL_ID_SET_ACTIVE_WORKSPACE => {
            let workspace_id = call
                .args
                .get("workspace_id")
                .or_else(|| call.args.get("name"))
                .or_else(|| call.args.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if workspace_id.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing workspace_id".into(),
                ));
            }

            if workspace_id == job.agent.active_workspace_id {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({ "ok": true }),
                ));
            }

            // Save current active workspace back into the map.
            let prev = job.agent.active_workspace_id.clone();
            if prev != "main" || !draft.defs.is_empty() || !job.planned_components.is_empty() {
                job.agent.workspaces.insert(
                    prev.clone(),
                    super::Gen3dAgentWorkspace {
                        name: prev.clone(),
                        defs: draft.defs.clone(),
                        planned_components: job.planned_components.clone(),
                        plan_hash: job.plan_hash.clone(),
                        assembly_rev: job.assembly_rev,
                        assembly_notes: job.assembly_notes.clone(),
                        plan_collider: job.plan_collider.clone(),
                        rig_move_cycle_m: job.rig_move_cycle_m,
                        motion_authoring: job.motion_authoring.clone(),
                        reuse_groups: job.reuse_groups.clone(),
                        reuse_group_warnings: job.reuse_group_warnings.clone(),
                    },
                );
            }

            let next = if workspace_id == "main" {
                job.agent
                    .workspaces
                    .get("main")
                    .cloned()
                    .unwrap_or_else(|| super::Gen3dAgentWorkspace {
                        name: "main".into(),
                        defs: Vec::new(),
                        planned_components: Vec::new(),
                        plan_hash: String::new(),
                        assembly_rev: 0,
                        assembly_notes: String::new(),
                        plan_collider: None,
                        rig_move_cycle_m: None,
                        motion_authoring: None,
                        reuse_groups: Vec::new(),
                        reuse_group_warnings: Vec::new(),
                    })
            } else if let Some(ws) = job.agent.workspaces.get(&workspace_id) {
                ws.clone()
            } else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown workspace `{workspace_id}`"),
                ));
            };

            draft.defs = next.defs;
            job.planned_components = next.planned_components;
            job.plan_hash = next.plan_hash;
            job.assembly_rev = next.assembly_rev;
            job.assembly_notes = next.assembly_notes;
            job.plan_collider = next.plan_collider;
            job.rig_move_cycle_m = next.rig_move_cycle_m;
            job.motion_authoring = next.motion_authoring;
            job.reuse_groups = next.reuse_groups;
            job.reuse_group_warnings = next.reuse_group_warnings;

            let component_count = job.planned_components.len();
            job.regen_per_component.resize(component_count, 0);
            job.component_attempts.resize(component_count, 0);
            job.component_last_errors.resize(component_count, None);
            job.agent.active_workspace_id = workspace_id;

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "ok": true }),
            ))
        }
        TOOL_ID_DIFF_WORKSPACES => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::workspaces::diff_workspaces_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_COPY_FROM_WORKSPACE => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::workspaces::copy_from_workspace_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_MERGE_WORKSPACE => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::workspaces::merge_workspace_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call_id, tool_id, err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
        }
        TOOL_ID_SUBMIT_TOOLING_FEEDBACK => {
            const MAX_SUBMISSIONS_PER_RUN: u32 = 8;
            if job.agent.tooling_feedback_submissions >= MAX_SUBMISSIONS_PER_RUN {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Tool feedback submission limit reached ({MAX_SUBMISSIONS_PER_RUN} per run)"
                    ),
                ));
            }

            let parsed: Result<super::schema::AiToolingFeedbackJsonV1, _> =
                serde_json::from_value(call.args.clone());
            let mut feedback = match parsed {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid tooling feedback args JSON: {err}"),
                    ));
                }
            };
            if feedback.version == 0 {
                feedback.version = 1;
            }

            let before = feedback_history.entries.len();
            super::record_gen3d_tooling_feedback(
                config,
                workshop,
                feedback_history,
                job,
                &[feedback],
            );
            let entry_ids: Vec<String> = feedback_history
                .entries
                .iter()
                .skip(before)
                .map(|e| e.entry_id.clone())
                .collect();

            job.agent.tooling_feedback_submissions =
                job.agent.tooling_feedback_submissions.saturating_add(1);

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "ok": true, "entry_ids": entry_ids }),
            ))
        }
        _ => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
            call.call_id,
            call.tool_id,
            "Unknown tool_id".into(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::format_info_kv_get_many_args_error;
    use super::normalize_tool_call_args;
    use crate::gen3d::agent::Gen3dToolCallJsonV1;
    use bevy::prelude::*;
    use serde::Deserialize;
    use std::path::PathBuf;
    use uuid::Uuid;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{prefix}_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn normalizes_null_args_to_empty_object() {
        let mut call = Gen3dToolCallJsonV1 {
            call_id: "call_1".into(),
            tool_id: "qa_v1".into(),
            args: serde_json::Value::Null,
        };
        normalize_tool_call_args(&mut call).expect("normalize");
        assert!(call.args.is_object());
        assert_eq!(call.args.as_object().unwrap().len(), 0);
    }

    #[test]
    fn parses_args_from_json_string_object() {
        let mut call = Gen3dToolCallJsonV1 {
            call_id: "call_1".into(),
            tool_id: "get_tool_detail_v1".into(),
            args: serde_json::Value::String("{\"tool_id\":\"qa_v1\"}".into()),
        };
        normalize_tool_call_args(&mut call).expect("normalize");
        assert_eq!(
            call.args.get("tool_id").and_then(|v| v.as_str()),
            Some("qa_v1")
        );
    }

    #[test]
    fn rejects_args_string_that_is_not_object() {
        let mut call = Gen3dToolCallJsonV1 {
            call_id: "call_1".into(),
            tool_id: "qa_v1".into(),
            args: serde_json::Value::String("[1,2,3]".into()),
        };
        let err = normalize_tool_call_args(&mut call).expect_err("should reject");
        assert!(err.contains("not an object"), "{err}");
    }

    #[test]
    fn rejects_args_value_that_is_not_object() {
        let mut call = Gen3dToolCallJsonV1 {
            call_id: "call_1".into(),
            tool_id: "qa_v1".into(),
            args: serde_json::Value::Bool(true),
        };
        let err = normalize_tool_call_args(&mut call).expect_err("should reject");
        assert!(err.contains("args must be an object"), "{err}");
    }

    #[test]
    fn gen3d_info_kv_get_many_selector_misplaced_error_is_actionable() {
        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Item {
            namespace: String,
            key: String,
            #[serde(default)]
            json_pointer: Option<String>,
            #[serde(default)]
            max_bytes: Option<u64>,
        }

        #[derive(Debug, Deserialize)]
        #[serde(deny_unknown_fields)]
        #[allow(dead_code)]
        struct Args {
            items: Vec<Item>,
            #[serde(default)]
            selector: Option<super::InfoKvSelectorArgsV1>,
            #[serde(default)]
            max_items: Option<u32>,
        }

        let args_value = serde_json::json!({
            "items": [
                {
                    "namespace": "gen3d",
                    "key": "ws.main.scene_graph_summary",
                    "selector": { "kind": "latest" },
                }
            ],
        });
        let err = serde_json::from_value::<Args>(args_value.clone()).expect_err("should reject");

        let msg = format_info_kv_get_many_args_error(&args_value, &err);
        assert!(
            msg.contains("selector")
                && msg.contains("top-level")
                && msg.contains("items[]")
                && msg.contains("Example:")
                && msg.contains("ws.main.scene_graph_summary"),
            "{msg}"
        );
    }

    #[test]
    fn select_kv_record_selectors_work() {
        let run_dir = make_temp_dir("gravimera_select_kv_record_test");
        let mut store =
            super::super::info_store::Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let r1 = store
            .kv_put(
                0,
                1,
                10,
                "main",
                "gen3d",
                "ws.main.state_summary",
                serde_json::json!({"rev": 1}),
                "state summary".into(),
                None,
            )
            .expect("kv put r1");
        let r2 = store
            .kv_put(
                0,
                2,
                11,
                "main",
                "gen3d",
                "ws.main.state_summary",
                serde_json::json!({"rev": 2}),
                "state summary".into(),
                None,
            )
            .expect("kv put r2");
        let r3 = store
            .kv_put(
                0,
                5,
                20,
                "main",
                "gen3d",
                "ws.main.state_summary",
                serde_json::json!({"rev": 3}),
                "state summary".into(),
                None,
            )
            .expect("kv put r3");

        let latest =
            super::select_kv_record(&store, "gen3d", "ws.main.state_summary", "latest", None)
                .expect("latest selector should not error")
                .expect("expected record");
        assert_eq!(latest.kv_rev, r3.kv_rev);

        let by_rev_selector = super::InfoKvSelectorArgsV1 {
            kind: "kv_rev".into(),
            kv_rev: Some(r2.kv_rev),
            assembly_rev: None,
            pass: None,
        };
        let by_rev = super::select_kv_record(
            &store,
            "gen3d",
            "ws.main.state_summary",
            "kv_rev",
            Some(&by_rev_selector),
        )
        .expect("kv_rev selector should not error")
        .expect("expected record");
        assert_eq!(by_rev.kv_rev, r2.kv_rev);

        let as_of_asm_selector = super::InfoKvSelectorArgsV1 {
            kind: "as_of_assembly_rev".into(),
            kv_rev: None,
            assembly_rev: Some(19),
            pass: None,
        };
        let as_of_asm = super::select_kv_record(
            &store,
            "gen3d",
            "ws.main.state_summary",
            "as_of_assembly_rev",
            Some(&as_of_asm_selector),
        )
        .expect("as_of_assembly_rev selector should not error")
        .expect("expected record");
        assert_eq!(as_of_asm.kv_rev, r2.kv_rev);

        let as_of_pass_selector = super::InfoKvSelectorArgsV1 {
            kind: "as_of_pass".into(),
            kv_rev: None,
            assembly_rev: None,
            pass: Some(1),
        };
        let as_of_pass = super::select_kv_record(
            &store,
            "gen3d",
            "ws.main.state_summary",
            "as_of_pass",
            Some(&as_of_pass_selector),
        )
        .expect("as_of_pass selector should not error")
        .expect("expected record");
        assert_eq!(as_of_pass.kv_rev, r1.kv_rev);

        let bad_kind_selector = super::InfoKvSelectorArgsV1 {
            kind: "unknown".into(),
            kv_rev: None,
            assembly_rev: None,
            pass: None,
        };
        let err = super::select_kv_record(
            &store,
            "gen3d",
            "ws.main.state_summary",
            "unknown",
            Some(&bad_kind_selector),
        )
        .expect_err("unknown selector kind should error");
        assert!(err.contains("Unknown selector.kind"), "{err}");

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn info_kv_get_paged_pages_stably_and_deterministically() {
        let run_dir = make_temp_dir("gravimera_info_kv_get_paged_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        let record = {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.qa",
                    serde_json::json!({ "errors": [0, 1, 2, 3, 4] }),
                    "qa".into(),
                    None,
                )
                .expect("kv put")
        };

        let r1 = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_1",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.qa",
                "selector": { "kind": "latest" },
                "json_pointer": "/errors",
                "page": { "limit": 2 },
                "max_item_bytes": 4096,
            }),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        let j1 = r1.result.clone().expect("missing result");
        assert_eq!(
            j1.get("array_len").and_then(|v| v.as_u64()),
            Some(5),
            "expected array_len=5, got {j1:?}"
        );
        let items1 = j1.get("items").and_then(|v| v.as_array()).expect("items");
        assert_eq!(items1.len(), 2, "expected 2 items, got {j1:?}");
        assert_eq!(
            items1[0].get("index").and_then(|v| v.as_u64()),
            Some(0),
            "{j1:?}"
        );
        assert_eq!(
            items1[1].get("index").and_then(|v| v.as_u64()),
            Some(1),
            "{j1:?}"
        );
        assert_eq!(
            j1.get("truncated").and_then(|v| v.as_bool()),
            Some(true),
            "{j1:?}"
        );
        let cursor1 = j1
            .get("next_cursor")
            .and_then(|v| v.as_str())
            .expect("missing next_cursor")
            .to_string();

        // Simulate a newer KV revision appearing between pages. Paging must remain stable by
        // pinning `selector.kind="kv_rev"` to the record selected on page 1.
        {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    2,
                    3,
                    "main",
                    "gen3d",
                    "ws.main.qa",
                    serde_json::json!({ "errors": [999] }),
                    "qa newer".into(),
                    None,
                )
                .expect("kv put newer");
        }

        let args_page2 = serde_json::json!({
            "namespace": "gen3d",
            "key": "ws.main.qa",
            "selector": { "kind": "kv_rev", "kv_rev": record.kv_rev },
            "json_pointer": "/errors",
            "page": { "limit": 2, "cursor": cursor1 },
            "max_item_bytes": 4096,
        });

        // Using selector.kind="latest" after a new revision exists should reject the cursor.
        let r2_latest = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_2_latest",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.qa",
                "selector": { "kind": "latest" },
                "json_pointer": "/errors",
                "page": { "limit": 2, "cursor": j1.get("next_cursor").unwrap() },
                "max_item_bytes": 4096,
            }),
        );
        assert!(
            !r2_latest.ok
                && r2_latest
                    .error
                    .as_deref()
                    .is_some_and(|e| e.contains("Cursor does not match this request")),
            "expected cursor mismatch error, got {r2_latest:?}"
        );

        let r2 = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_2",
            args_page2.clone(),
        );
        assert!(r2.ok, "expected tool call ok, got {r2:?}");
        let j2 = r2.result.clone().expect("missing result");
        let items2 = j2.get("items").and_then(|v| v.as_array()).expect("items");
        assert_eq!(items2.len(), 2, "expected 2 items, got {j2:?}");
        assert_eq!(
            items2[0].get("index").and_then(|v| v.as_u64()),
            Some(2),
            "{j2:?}"
        );
        assert_eq!(
            items2[1].get("index").and_then(|v| v.as_u64()),
            Some(3),
            "{j2:?}"
        );
        let cursor2 = j2
            .get("next_cursor")
            .and_then(|v| v.as_str())
            .expect("missing next_cursor")
            .to_string();

        // Determinism: the same request must produce the same page output.
        let r2_again = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_2_again",
            args_page2,
        );
        assert!(r2_again.ok, "expected tool call ok, got {r2_again:?}");
        assert_eq!(
            r2_again.result.clone().expect("missing result"),
            j2,
            "expected deterministic paging output"
        );

        let r3 = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_3",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.qa",
                "selector": { "kind": "kv_rev", "kv_rev": record.kv_rev },
                "json_pointer": "/errors",
                "page": { "limit": 2, "cursor": cursor2 },
                "max_item_bytes": 4096,
            }),
        );
        assert!(r3.ok, "expected tool call ok, got {r3:?}");
        let j3 = r3.result.clone().expect("missing result");
        let items3 = j3.get("items").and_then(|v| v.as_array()).expect("items");
        assert_eq!(items3.len(), 1, "expected 1 item, got {j3:?}");
        assert_eq!(
            items3[0].get("index").and_then(|v| v.as_u64()),
            Some(4),
            "{j3:?}"
        );
        assert_eq!(
            j3.get("truncated").and_then(|v| v.as_bool()),
            Some(false),
            "{j3:?}"
        );
        assert!(
            j3.get("next_cursor").is_none(),
            "expected no next_cursor on last page, got {j3:?}"
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn info_kv_get_paged_item_truncation_uses_shape_preview() {
        let run_dir = make_temp_dir("gravimera_info_kv_get_paged_trunc_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        let string_len_bytes = 1024;
        {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.qa",
                    serde_json::json!({ "errors": ["x".repeat(string_len_bytes)] }),
                    "qa".into(),
                    None,
                )
                .expect("kv put");
        }

        let max_item_bytes = 256;
        let r1 = super::execute_info_kv_get_paged_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_PAGED,
            "call_1",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.qa",
                "selector": { "kind": "latest" },
                "json_pointer": "/errors",
                "page": { "limit": 1 },
                "max_item_bytes": max_item_bytes,
            }),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        let j1 = r1.result.clone().expect("missing result");

        let items = j1.get("items").and_then(|v| v.as_array()).expect("items");
        assert_eq!(items.len(), 1, "expected 1 item, got {j1:?}");
        let item = &items[0];
        assert_eq!(
            item.get("truncated").and_then(|v| v.as_bool()),
            Some(true),
            "expected item truncated=true, got {item:?}"
        );
        assert_eq!(
            item.get("bytes").and_then(|v| v.as_u64()),
            Some((max_item_bytes + 1) as u64),
            "expected capped bytes=max_item_bytes+1, got {item:?}"
        );

        let preview = item.get("value_preview").expect("missing value_preview");
        assert!(
            preview.is_object(),
            "expected shape preview object, got {preview:?}"
        );
        assert_eq!(
            preview.get("kind").and_then(|v| v.as_str()),
            Some("string"),
            "expected kind=string, got {preview:?}"
        );
        assert_eq!(
            preview.get("len_bytes").and_then(|v| v.as_u64()),
            Some(string_len_bytes as u64),
            "expected len_bytes={string_len_bytes}, got {preview:?}"
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn preserve_replan_template_gate_is_actionable() {
        assert!(super::should_require_plan_template_kv_for_preserve_replan(
            true, 1, false
        ));
        assert!(!super::should_require_plan_template_kv_for_preserve_replan(
            false, 1, false
        ));
        assert!(!super::should_require_plan_template_kv_for_preserve_replan(
            true, 0, false
        ));
        assert!(!super::should_require_plan_template_kv_for_preserve_replan(
            true, 3, true
        ));

        let msg = super::preserve_replan_missing_template_error(super::TOOL_ID_LLM_GENERATE_PLAN);
        assert!(
            msg.contains(super::TOOL_ID_GET_PLAN_TEMPLATE)
                && msg.contains(super::TOOL_ID_LLM_GENERATE_PLAN)
                && msg.contains("plan_template_kv"),
            "{msg}"
        );
    }

    fn make_test_root_def_movable(movable: bool) -> crate::object::registry::ObjectDef {
        use crate::object::registry::{
            ColliderProfile, MobilityDef, MobilityMode, ObjectDef, ObjectInteraction,
        };

        ObjectDef {
            object_id: crate::gen3d::gen3d_draft_object_id(),
            label: "gen3d_draft".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::AabbXZ {
                half_extents: Vec2::new(0.5, 0.5),
            },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: movable.then_some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    #[test]
    fn gen3d_capability_gaps_includes_missing_motion_channel_for_movable() {
        let job = super::Gen3dAiJob::default();
        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(true)],
        };

        let smoke = serde_json::json!({});
        let gaps = super::build_capability_gaps_from_smoke_v1(&job, &draft, &smoke);
        assert!(
            gaps.iter().any(|g| {
                g.get("kind").and_then(|v| v.as_str()) == Some("missing_motion_channel")
            }),
            "expected missing_motion_channel gap, got {gaps:?}"
        );
    }

    #[test]
    fn gen3d_capability_gaps_skips_warn_motion_validation_issues() {
        let job = super::Gen3dAiJob::default();
        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let smoke = serde_json::json!({
            "motion_validation": {
                "ok": true,
                "issues": [{
                    "severity": "warn",
                    "kind": "attack_self_intersection",
                    "component_id": "comp_1",
                    "component_name": "head_left",
                    "channel": "attack",
                    "message": "Self-intersection detected.",
                    "evidence": {},
                }]
            }
        });
        let gaps = super::build_capability_gaps_from_smoke_v1(&job, &draft, &smoke);
        assert!(
            gaps.iter()
                .all(|g| g.get("kind").and_then(|v| v.as_str()) != Some("motion_validation_error")),
            "expected warn motion issues not to produce capability gaps, got {gaps:?}"
        );
    }

    #[test]
    fn gen3d_smoke_results_attack_requirement_comes_from_prompt_intent() {
        let planned = vec![super::super::job::Gen3dPlannedComponent {
            display_name: "1. root".into(),
            name: "root".into(),
            purpose: "root".into(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: Some(Vec3::ONE),
            anchors: Vec::new(),
            contacts: Vec::new(),
            articulation_nodes: Vec::new(),
            root_animations: Vec::new(),
            attach_to: None,
        }];

        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let smoke_no_attack_required =
            super::super::build_gen3d_smoke_results(Some(false), false, None, &planned, &draft);
        assert_eq!(
            smoke_no_attack_required
                .get("attack_required_by_prompt")
                .and_then(|v| v.as_bool()),
            Some(false),
            "expected attack_required_by_prompt=false, got {smoke_no_attack_required:?}"
        );
        let has_error = smoke_no_attack_required
            .get("issues")
            .and_then(|v| v.as_array())
            .is_some_and(|issues| {
                issues
                    .iter()
                    .any(|i| i.get("severity").and_then(|v| v.as_str()) == Some("error"))
            });
        assert!(
            !has_error,
            "expected no smoke errors when attack is not required, got {smoke_no_attack_required:?}"
        );

        let smoke_attack_required =
            super::super::build_gen3d_smoke_results(Some(true), false, None, &planned, &draft);
        assert_eq!(
            smoke_attack_required
                .get("attack_required_by_prompt")
                .and_then(|v| v.as_bool()),
            Some(true),
            "expected attack_required_by_prompt=true, got {smoke_attack_required:?}"
        );
        let has_missing_root_error = smoke_attack_required
            .get("issues")
            .and_then(|v| v.as_array())
            .is_some_and(|issues| {
                issues.iter().any(|i| {
                    i.get("severity").and_then(|v| v.as_str()) == Some("error")
                        && i.get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .contains("attack-capable")
                })
            });
        assert!(
            has_missing_root_error,
            "expected missing root mobility/attack error when attack is required, got {smoke_attack_required:?}"
        );
    }

    #[test]
    fn gen3d_smoke_results_include_motion_quality_complaints_for_movable_ground() {
        use super::super::job::{Gen3dPlannedAttachment, Gen3dPlannedComponent};
        use crate::object::registry::AnchorDef;

        let planned = vec![
            Gen3dPlannedComponent {
                display_name: "1. root".into(),
                name: "root".into(),
                purpose: "root".into(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![AnchorDef {
                    name: "mount".into(),
                    transform: Transform::IDENTITY,
                }],
                contacts: Vec::new(),
                articulation_nodes: Vec::new(),
                root_animations: Vec::new(),
                attach_to: None,
            },
            Gen3dPlannedComponent {
                display_name: "2. child".into(),
                name: "child".into(),
                purpose: "child".into(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![],
                contacts: Vec::new(),
                articulation_nodes: Vec::new(),
                root_animations: Vec::new(),
                attach_to: Some(Gen3dPlannedAttachment {
                    parent: "root".into(),
                    parent_anchor: "mount".into(),
                    child_anchor: "origin".into(),
                    offset: Transform::IDENTITY,
                    fallback_basis: Transform::IDENTITY,
                    joint: None,
                    animations: Vec::new(),
                }),
            },
        ];

        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(true)],
        };

        let smoke =
            super::super::build_gen3d_smoke_results(Some(false), false, None, &planned, &draft);
        let kinds: Vec<&str> = smoke
            .get("issues")
            .and_then(|v| v.as_array())
            .map(|issues| {
                issues
                    .iter()
                    .filter_map(|i| {
                        if i.get("severity").and_then(|v| v.as_str()) != Some("complaint") {
                            return None;
                        }
                        i.get("kind").and_then(|v| v.as_str())
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        assert!(
            kinds.contains(&"missing_ground_contacts"),
            "expected missing_ground_contacts complaint, got {kinds:?} smoke={smoke:?}"
        );
        assert!(
            kinds.contains(&"missing_joint_metadata"),
            "expected missing_joint_metadata complaint, got {kinds:?} smoke={smoke:?}"
        );
    }

    #[test]
    fn gen3d_capability_gaps_hinge_limit_exceeded_includes_apply_draft_ops_fixit() {
        use crate::gen3d::ai::schema::{AiJointJson, AiJointKindJson};
        use crate::object::registry::{
            AnchorDef, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
            PartAnimationSlot, PartAnimationSpec,
        };

        let mut job = super::Gen3dAiJob::default();
        job.rig_move_cycle_m = Some(1.0);
        job.planned_components = vec![
            super::super::job::Gen3dPlannedComponent {
                display_name: "1. root".into(),
                name: "root".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![AnchorDef {
                    name: "mount".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                }],
                contacts: Vec::new(),
                articulation_nodes: Vec::new(),
                root_animations: Vec::new(),
                attach_to: None,
            },
            super::super::job::Gen3dPlannedComponent {
                display_name: "2. tongue".into(),
                name: "tongue".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![AnchorDef {
                    name: "mount".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                }],
                contacts: Vec::new(),
                articulation_nodes: Vec::new(),
                root_animations: Vec::new(),
                attach_to: Some(super::super::job::Gen3dPlannedAttachment {
                    parent: "root".into(),
                    parent_anchor: "origin".into(),
                    child_anchor: "origin".into(),
                    offset: Transform::IDENTITY,
                    fallback_basis: Transform::IDENTITY,
                    joint: Some(AiJointJson {
                        kind: AiJointKindJson::Hinge,
                        axis_join: Some([1.0, 0.0, 0.0]),
                        limits_degrees: Some([-30.0, 30.0]),
                        swing_limits_degrees: None,
                        twist_limits_degrees: None,
                    }),
                    animations: vec![PartAnimationSlot {
                        channel: "move".into(),
                        family: crate::object::registry::PartAnimationFamily::Base,
                        spec: PartAnimationSpec {
                            driver: PartAnimationDriver::MovePhase,
                            speed_scale: 1.0,
                            time_offset_units: 0.0,
                            basis: Transform::IDENTITY,
                            clip: PartAnimationDef::Loop {
                                duration_secs: 1.0,
                                keyframes: vec![
                                    PartAnimationKeyframeDef {
                                        time_secs: 0.0,
                                        delta: Transform::IDENTITY,
                                    },
                                    PartAnimationKeyframeDef {
                                        time_secs: 0.5,
                                        delta: Transform {
                                            rotation: Quat::from_axis_angle(
                                                Vec3::X,
                                                40.0_f32.to_radians(),
                                            ),
                                            ..Default::default()
                                        },
                                    },
                                    PartAnimationKeyframeDef {
                                        time_secs: 1.0,
                                        delta: Transform::IDENTITY,
                                    },
                                ],
                            },
                        },
                    }],
                }),
            },
        ];

        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let motion_report = super::super::motion_validation::build_motion_validation_report(
            Some(1.0),
            &job.planned_components,
        );
        assert!(
            motion_report
                .motion_validation
                .get("issues")
                .and_then(|v| v.as_array())
                .is_some_and(|issues| issues.iter().any(|i| {
                    i.get("kind").and_then(|v| v.as_str()) == Some("hinge_limit_exceeded")
                })),
            "expected hinge_limit_exceeded in motion_validation issues, got {:?}",
            motion_report.motion_validation
        );

        let smoke = serde_json::json!({
            "attack_required_by_prompt": false,
            "mobility_present": false,
            "attack_present": false,
            "motion_validation": motion_report.motion_validation,
        });
        let gaps = super::build_capability_gaps_from_smoke_v1(&job, &draft, &smoke);

        let hinge_gap = gaps
            .iter()
            .find(|g| {
                g.get("kind").and_then(|v| v.as_str()) == Some("motion_validation_error")
                    && g.get("evidence")
                        .and_then(|v| v.get("issue_kind"))
                        .and_then(|v| v.as_str())
                        == Some("hinge_limit_exceeded")
            })
            .expect("missing hinge_limit_exceeded capability gap");

        let fixits = hinge_gap
            .get("fixits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            fixits.is_empty(),
            "expected no deterministic fixits for hinge_limit_exceeded, got {fixits:?}"
        );
    }

    #[test]
    fn gen3d_capability_gaps_missing_root_interface_includes_actionable_fixits() {
        let job = super::Gen3dAiJob::default();
        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let smoke = serde_json::json!({
            "attack_required_by_prompt": true,
            "mobility_present": false,
            "attack_present": false,
        });

        let gaps = super::build_capability_gaps_from_smoke_v1(&job, &draft, &smoke);
        let gap = gaps
            .iter()
            .find(|g| g.get("kind").and_then(|v| v.as_str()) == Some("missing_root_field"))
            .expect("missing root gap");
        assert!(
            gap.get("blocked").is_none(),
            "expected no blocked marker, got {gap:?}"
        );
        let fixits = gap
            .get("fixits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            fixits.iter().any(|fixit| {
                fixit.get("tool_id").and_then(|v| v.as_str())
                    == Some(super::TOOL_ID_LLM_GENERATE_PLAN)
            }),
            "expected {}/fixit, got {gap:?}",
            super::TOOL_ID_LLM_GENERATE_PLAN
        );
    }

    #[test]
    fn gen3d_capability_gaps_missing_root_interface_prefers_plan_ops_fixits_when_plan_exists() {
        let mut job = super::Gen3dAiJob::default();
        job.planned_components = vec![super::super::job::Gen3dPlannedComponent {
            display_name: "1. root".into(),
            name: "root".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: Some(Vec3::ONE),
            anchors: Vec::new(),
            contacts: Vec::new(),
            articulation_nodes: Vec::new(),
            root_animations: Vec::new(),
            attach_to: None,
        }];

        let draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let smoke = serde_json::json!({
            "attack_required_by_prompt": true,
            "mobility_present": false,
            "attack_present": false,
        });

        let gaps = super::build_capability_gaps_from_smoke_v1(&job, &draft, &smoke);
        let gap = gaps
            .iter()
            .find(|g| g.get("kind").and_then(|v| v.as_str()) == Some("missing_root_field"))
            .expect("missing root gap");

        let fixits = gap
            .get("fixits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            fixits.iter().any(|fixit| {
                fixit.get("tool_id").and_then(|v| v.as_str())
                    == Some(super::TOOL_ID_GET_PLAN_TEMPLATE)
            }),
            "expected {}/fixit, got {gap:?}",
            super::TOOL_ID_GET_PLAN_TEMPLATE
        );
        assert!(
            fixits.iter().any(|fixit| {
                fixit.get("tool_id").and_then(|v| v.as_str())
                    == Some(super::TOOL_ID_LLM_GENERATE_PLAN_OPS)
            }),
            "expected {}/fixit, got {gap:?}",
            super::TOOL_ID_LLM_GENERATE_PLAN_OPS
        );
    }

    #[test]
    fn info_kv_get_v1_second_call_is_cached() {
        let run_dir = make_temp_dir("gravimera_info_kv_get_cache_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.small",
                    serde_json::json!({ "a": 1, "b": 2 }),
                    "small".into(),
                    None,
                )
                .expect("kv put");
        }

        let r1 = super::execute_info_kv_get_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET,
            "call_1",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.small",
                "selector": { "kind": "latest" },
                "json_pointer": "/a",
                "max_bytes": 64 * 1024,
            }),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        let j1 = r1.result.clone().expect("missing result");
        assert_eq!(
            j1.get("cached").and_then(|v| v.as_bool()),
            Some(false),
            "expected cached=false, got {j1:?}"
        );
        let kv_rev_1 = j1
            .get("record")
            .and_then(|v| v.get("kv_rev"))
            .and_then(|v| v.as_u64())
            .expect("missing record.kv_rev");

        let r2 = super::execute_info_kv_get_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET,
            "call_2",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.small",
                "selector": { "kind": "latest" },
                "json_pointer": "/a",
                "max_bytes": 64 * 1024,
            }),
        );
        assert!(r2.ok, "expected tool call ok, got {r2:?}");
        let j2 = r2.result.clone().expect("missing result");
        assert_eq!(
            j2.get("cached").and_then(|v| v.as_bool()),
            Some(true),
            "expected cached=true, got {j2:?}"
        );
        assert_eq!(
            j2.get("no_new_information").and_then(|v| v.as_bool()),
            Some(true),
            "expected no_new_information=true, got {j2:?}"
        );
        let kv_rev_2 = j2
            .get("record")
            .and_then(|v| v.get("kv_rev"))
            .and_then(|v| v.as_u64())
            .expect("missing record.kv_rev");
        assert_eq!(kv_rev_2, kv_rev_1, "expected cached kv_rev unchanged");

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn info_kv_get_v1_oversize_error_includes_shape_preview_and_fixits() {
        let run_dir = make_temp_dir("gravimera_info_kv_get_oversize_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.big",
                    serde_json::json!({
                        "arr": (0..128).collect::<Vec<_>>(),
                        "blob": "x".repeat(20_000),
                    }),
                    "big".into(),
                    None,
                )
                .expect("kv put");
        }

        let r = super::execute_info_kv_get_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET,
            "call_1",
            serde_json::json!({
                "namespace": "gen3d",
                "key": "ws.main.big",
                "selector": { "kind": "latest" },
                "max_bytes": 1024,
            }),
        );
        assert!(!r.ok, "expected tool call error, got {r:?}");
        let diag = r.result.clone().expect("expected error result payload");
        assert_eq!(
            diag.get("kind").and_then(|v| v.as_str()),
            Some("kv_value_too_large"),
            "expected kind, got {diag:?}"
        );
        let shape = diag
            .get("shape_preview")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        assert_eq!(
            shape.get("kind").and_then(|v| v.as_str()),
            Some("object"),
            "expected object shape preview, got {shape:?}"
        );
        let fixits = diag
            .get("fixits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            fixits.iter().any(|f| {
                f.get("tool_id").and_then(|v| v.as_str()) == Some(super::TOOL_ID_INFO_KV_GET)
            }),
            "expected info_kv_get_v1 fixit, got {fixits:?}"
        );
        assert!(
            fixits.iter().any(|f| {
                f.get("tool_id").and_then(|v| v.as_str()) == Some(super::TOOL_ID_INFO_KV_GET_PAGED)
            }),
            "expected info_kv_get_paged_v1 fixit, got {fixits:?}"
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn info_kv_get_many_v1_second_call_is_cached() {
        let run_dir = make_temp_dir("gravimera_info_kv_get_many_cache_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        {
            let store = job.ensure_info_store().expect("open store");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.small",
                    serde_json::json!({ "a": 1, "b": 2 }),
                    "small".into(),
                    None,
                )
                .expect("kv put");
            store
                .kv_put(
                    0,
                    1,
                    2,
                    "main",
                    "gen3d",
                    "ws.main.other",
                    serde_json::json!({ "k": "v" }),
                    "other".into(),
                    None,
                )
                .expect("kv put other");
        }

        let args = serde_json::json!({
            "selector": { "kind": "latest" },
            "items": [
                { "namespace": "gen3d", "key": "ws.main.small", "json_pointer": "/a" },
                { "namespace": "gen3d", "key": "ws.main.other" }
            ],
        });

        let r1 = super::execute_info_kv_get_many_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_MANY,
            "call_1",
            args.clone(),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        let j1 = r1.result.clone().expect("missing result");
        assert_eq!(
            j1.get("cached").and_then(|v| v.as_bool()),
            Some(false),
            "expected cached=false, got {j1:?}"
        );

        let r2 = super::execute_info_kv_get_many_v1(
            &mut job,
            super::TOOL_ID_INFO_KV_GET_MANY,
            "call_2",
            args,
        );
        assert!(r2.ok, "expected tool call ok, got {r2:?}");
        let j2 = r2.result.clone().expect("missing result");
        assert_eq!(
            j2.get("cached").and_then(|v| v.as_bool()),
            Some(true),
            "expected cached=true, got {j2:?}"
        );
        assert_eq!(
            j2.get("no_new_information").and_then(|v| v.as_bool()),
            Some(true),
            "expected no_new_information=true, got {j2:?}"
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn qa_v1_second_call_is_cached_and_preserves_info_kv() {
        let run_dir = make_temp_dir("gravimera_qa_cache_test");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());

        let mut draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let r1 = super::execute_qa_v1(
            &mut job,
            &mut draft,
            super::TOOL_ID_QA,
            "call_1",
            serde_json::json!({}),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        let j1 = r1.result.clone().expect("missing qa result");
        let kv1 = j1.get("info_kv").cloned().expect("missing info_kv");
        let kv_rev_1 = kv1
            .get("selector")
            .and_then(|v| v.get("kv_rev"))
            .and_then(|v| v.as_u64())
            .expect("missing info_kv.selector.kv_rev");

        let r2 = super::execute_qa_v1(
            &mut job,
            &mut draft,
            super::TOOL_ID_QA,
            "call_2",
            serde_json::json!({}),
        );
        assert!(r2.ok, "expected tool call ok, got {r2:?}");
        let j2 = r2.result.clone().expect("missing qa result");
        assert_eq!(
            j2.get("cached").and_then(|v| v.as_bool()),
            Some(true),
            "expected cached=true, got {j2:?}"
        );
        assert_eq!(
            j2.get("no_new_information").and_then(|v| v.as_bool()),
            Some(true),
            "expected no_new_information=true, got {j2:?}"
        );

        let kv2 = j2.get("info_kv").cloned().expect("missing info_kv");
        let kv_rev_2 = kv2
            .get("selector")
            .and_then(|v| v.get("kv_rev"))
            .and_then(|v| v.as_u64())
            .expect("missing info_kv.selector.kv_rev");
        assert_eq!(kv_rev_2, kv_rev_1, "expected cached QA kv_rev unchanged");

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn qa_v1_cached_call_writes_step_artifacts() {
        let run_dir = make_temp_dir("gravimera_qa_cache_artifacts_test");
        let step_1 = run_dir.join("step_1");
        let step_2 = run_dir.join("step_2");
        std::fs::create_dir_all(&step_1).expect("create step_1");
        std::fs::create_dir_all(&step_2).expect("create step_2");

        let mut job = super::Gen3dAiJob::default();
        job.run_dir = Some(run_dir.clone());
        job.step_dir = Some(step_1.clone());

        let mut draft = crate::gen3d::state::Gen3dDraft {
            defs: vec![make_test_root_def_movable(false)],
        };

        let r1 = super::execute_qa_v1(
            &mut job,
            &mut draft,
            super::TOOL_ID_QA,
            "call_1",
            serde_json::json!({}),
        );
        assert!(r1.ok, "expected tool call ok, got {r1:?}");
        assert!(step_1.join("qa.json").exists(), "expected step_1 qa.json");
        assert!(
            step_1.join("validate.json").exists(),
            "expected step_1 validate.json"
        );
        assert!(
            step_1.join("smoke_results.json").exists(),
            "expected step_1 smoke_results.json"
        );

        job.step_dir = Some(step_2.clone());
        let r2 = super::execute_qa_v1(
            &mut job,
            &mut draft,
            super::TOOL_ID_QA,
            "call_2",
            serde_json::json!({}),
        );
        assert!(r2.ok, "expected tool call ok, got {r2:?}");
        let j2 = r2.result.clone().expect("missing qa result");
        assert_eq!(
            j2.get("cached").and_then(|v| v.as_bool()),
            Some(true),
            "expected cached=true, got {j2:?}"
        );
        assert!(step_2.join("qa.json").exists(), "expected step_2 qa.json");
        assert!(
            step_2.join("validate.json").exists(),
            "expected step_2 validate.json"
        );
        assert!(
            step_2.join("smoke_results.json").exists(),
            "expected step_2 smoke_results.json"
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn plan_template_auto_trims_to_budget() {
        let long_notes = "x".repeat(2200);
        let long_assembly = "a".repeat(2200);
        let contacts = vec!["c"; 64];
        let components: Vec<serde_json::Value> = (0..10)
            .map(|idx| {
                serde_json::json!({
                    "name": format!("c{idx}"),
                    "purpose": "",
                    "modeling_notes": long_notes.as_str(),
                    "size": [1.0, 1.0, 1.0],
                    "anchors": [],
                    "contacts": contacts.clone(),
                    "attach_to": if idx == 0 { serde_json::Value::Null } else { serde_json::json!({"parent":"c0","parent_anchor":"origin","child_anchor":"origin"}) },
                })
            })
            .collect();
        let plan = serde_json::json!({
            "version": 8,
            "assembly_notes": long_assembly,
            "mobility": { "kind": "static" },
            "root_component": "c0",
            "components": components,
        });

        let max_bytes = 4096;
        let (trimmed, report) =
            super::fit_plan_template_to_budget(plan, super::PlanTemplateMode::Auto, max_bytes)
                .expect("should fit after trimming");
        assert!(report.bytes_full > report.bytes, "{report:?}");
        assert!(report.bytes <= max_bytes, "{report:?}");
        assert!(report.truncated, "{report:?}");
        assert!(
            report
                .omitted_fields
                .contains(&"components[].modeling_notes"),
            "{report:?}"
        );
        assert!(
            report.omitted_fields.contains(&"components[].contacts"),
            "{report:?}"
        );
        assert!(
            report.omitted_fields.contains(&"assembly_notes"),
            "{report:?}"
        );

        assert!(super::json_compact_bytes(&trimmed) <= max_bytes);
        assert_eq!(
            trimmed
                .get("assembly_notes")
                .and_then(|v| v.as_str())
                .unwrap_or("<missing>"),
            ""
        );
        let comp0_notes = trimmed
            .get("components")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|v| v.get("modeling_notes"))
            .and_then(|v| v.as_str())
            .unwrap_or("<missing>");
        assert_eq!(comp0_notes, "");
    }

    fn find_component<'a>(
        plan: &'a serde_json::Value,
        name: &str,
    ) -> Option<&'a serde_json::Value> {
        plan.get("components")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("name").and_then(|v| v.as_str()) == Some(name))
            })
    }

    #[test]
    fn plan_template_scope_trims_non_scoped_anchors() {
        let plan = serde_json::json!({
            "version": 8,
            "assembly_notes": "",
            "mobility": { "kind": "static" },
            "root_component": "body",
            "attack": {
                "kind": "ranged_projectile",
                "cooldown_secs": 0.5,
                "muzzle": { "component": "head", "anchor": "muzzle" },
                "projectile": { "shape": "sphere", "radius": 0.1, "color": [1.0, 1.0, 1.0, 1.0], "unlit": true, "speed": 10.0, "ttl_secs": 1.0, "damage": 1, "obstacle_rule": null, "spawn_energy_impact": false }
            },
            "components": [
                {
                    "name": "body",
                    "purpose": "",
                    "modeling_notes": "",
                    "size": [1.0, 1.0, 1.0],
                    "anchors": [
                        { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                        { "name": "extra", "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
                    ],
                    "contacts": [],
                    "attach_to": null
                },
                {
                    "name": "head",
                    "purpose": "",
                    "modeling_notes": "",
                    "size": [0.5, 0.5, 0.5],
                    "anchors": [
                        { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                        { "name": "muzzle", "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                        { "name": "decor", "pos": [0.0, 0.0, 0.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
                    ],
                    "contacts": [],
                    "attach_to": { "parent": "body", "parent_anchor": "mount", "child_anchor": "mount" }
                }
            ]
        });

        let mut scoped = plan.clone();
        let report =
            super::scope_plan_template_anchors_to_components(&mut scoped, &["head".into()])
                .expect("scope should succeed");
        assert!(report.scoped, "{report:?}");
        assert_eq!(report.scope_components_total, 1, "{report:?}");
        assert_eq!(report.anchors_total_full, 5, "{report:?}");
        assert_eq!(report.anchors_total, 4, "{report:?}");
        assert_eq!(report.anchors_dropped, 1, "{report:?}");
        assert_eq!(report.components_with_anchors_trimmed, 1, "{report:?}");

        let body = find_component(&scoped, "body").expect("body");
        let body_anchor_names: Vec<&str> = body
            .get("anchors")
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(body_anchor_names, vec!["mount"]);

        let head = find_component(&scoped, "head").expect("head");
        let head_anchor_names: Vec<&str> = head
            .get("anchors")
            .and_then(|v| v.as_array())
            .unwrap()
            .iter()
            .filter_map(|a| a.get("name").and_then(|v| v.as_str()))
            .collect();
        assert_eq!(head_anchor_names, vec!["mount", "muzzle", "decor"]);
    }

    #[test]
    fn plan_template_scope_requires_existing_component_names() {
        let mut plan = serde_json::json!({
            "version": 8,
            "mobility": { "kind": "static" },
            "root_component": "body",
            "components": [
                { "name": "body", "anchors": [], "contacts": [], "attach_to": null }
            ],
        });
        let err = super::scope_plan_template_anchors_to_components(&mut plan, &["nope".into()])
            .expect_err("unknown scope should error");
        assert!(err.contains("Unknown scope_components"), "{err}");
    }

    #[test]
    fn plan_template_full_mode_errors_over_budget() {
        let plan = serde_json::json!({
            "version": 8,
            "assembly_notes": "a".repeat(4096),
            "components": [
                { "name": "c0", "modeling_notes": "x".repeat(4096), "contacts": vec!["c"; 32] }
            ],
        });
        let err = super::fit_plan_template_to_budget(plan, super::PlanTemplateMode::Full, 1024)
            .expect_err("full mode should error");
        assert!(err.contains("Retry with mode"), "{err}");
    }

    #[derive(Resource, Default)]
    struct CapturedToolResult(pub(crate) Option<crate::gen3d::agent::Gen3dToolResultJsonV1>);

    fn capture_review_delta_budget_gate(
        config: Res<crate::config::AppConfig>,
        time: Res<Time>,
        mut commands: Commands,
        mut images: ResMut<Assets<Image>>,
        mut workshop: ResMut<crate::gen3d::state::Gen3dWorkshop>,
        mut feedback_history: ResMut<crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory>,
        mut job: ResMut<crate::gen3d::ai::Gen3dAiJob>,
        mut draft: ResMut<crate::gen3d::state::Gen3dDraft>,
        mut preview: ResMut<crate::gen3d::state::Gen3dPreview>,
        mut preview_model: Query<
            (
                &mut crate::types::AnimationChannelsActive,
                &mut crate::types::LocomotionClock,
                &mut crate::types::AttackClock,
                &mut crate::types::ActionClock,
            ),
            With<crate::gen3d::state::Gen3dPreviewModelRoot>,
        >,
        mut captured: ResMut<CapturedToolResult>,
    ) {
        if captured.0.is_some() {
            return;
        }
        let call = Gen3dToolCallJsonV1 {
            call_id: "call_1".into(),
            tool_id: crate::gen3d::agent::tools::TOOL_ID_LLM_REVIEW_DELTA.to_string(),
            args: serde_json::json!({}),
        };
        let outcome = super::execute_tool_call(
            &config,
            &time,
            &mut commands,
            &mut images,
            &mut workshop,
            &mut feedback_history,
            &mut job,
            &mut draft,
            &mut preview,
            &mut preview_model,
            call,
        );
        if let super::ToolCallOutcome::Immediate(result) = outcome {
            captured.0 = Some(result);
        }
    }

    #[test]
    fn llm_review_delta_budget_gate_returns_actionable_error() {
        let step_dir = make_temp_dir("gravimera_review_delta_budget_gate_test");

        let openai = crate::config::OpenAiConfig {
            base_url: "mock://gen3d".into(),
            model: "mock".into(),
            reasoning_effort: "none".into(),
            api_key: "mock".into(),
        };

        let mut config = crate::config::AppConfig {
            openai: Some(openai.clone()),
            ..Default::default()
        };
        config.gen3d_review_delta_rounds_max = 2;

        let mut job = crate::gen3d::ai::Gen3dAiJob::default();
        job.ai = Some(super::super::ai_service::Gen3dAiServiceConfig::OpenAi(
            openai,
        ));
        job.step_dir = Some(step_dir);
        job.review_delta_rounds_used = 2;

        let mut app = App::new();
        app.insert_resource(config);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(crate::gen3d::state::Gen3dWorkshop::default());
        app.insert_resource(crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory::default());
        app.insert_resource(job);
        app.insert_resource(crate::gen3d::state::Gen3dDraft::default());
        app.insert_resource(crate::gen3d::state::Gen3dPreview::default());
        app.insert_resource(CapturedToolResult::default());
        app.add_systems(Update, capture_review_delta_budget_gate);

        app.update();

        let captured = app
            .world()
            .resource::<CapturedToolResult>()
            .0
            .clone()
            .expect("expected tool result");
        assert!(!captured.ok, "expected ok=false, got {captured:?}");
        let err = captured.error.as_deref().unwrap_or("");
        assert!(err.contains("budget exhausted"), "unexpected error={err}");
        let result = captured.result.expect("expected structured error result");
        assert_eq!(
            result.get("kind").and_then(|v| v.as_str()),
            Some("review_delta_budget_exhausted"),
            "{result:?}"
        );
        assert_eq!(result.get("used").and_then(|v| v.as_u64()), Some(2));
        assert_eq!(result.get("max").and_then(|v| v.as_u64()), Some(2));
        assert!(
            result
                .get("guidance")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("Do not call llm_review_delta_v1 again")),
            "expected actionable guidance, got {result:?}"
        );
    }
}
