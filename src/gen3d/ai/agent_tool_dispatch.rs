use bevy::prelude::*;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE,
    TOOL_ID_COPY_FROM_WORKSPACE, TOOL_ID_CREATE_WORKSPACE, TOOL_ID_DELETE_WORKSPACE,
    TOOL_ID_DETACH_COMPONENT, TOOL_ID_DIFF_SNAPSHOTS, TOOL_ID_DIFF_WORKSPACES,
    TOOL_ID_INFO_BLOBS_GET, TOOL_ID_INFO_BLOBS_LIST, TOOL_ID_INFO_EVENTS_GET,
    TOOL_ID_INFO_EVENTS_LIST, TOOL_ID_INFO_EVENTS_SEARCH, TOOL_ID_INFO_KV_GET,
    TOOL_ID_INFO_KV_GET_MANY, TOOL_ID_INFO_KV_LIST_HISTORY, TOOL_ID_INFO_KV_LIST_KEYS,
    TOOL_ID_GET_PLAN_TEMPLATE, TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_STATE_SUMMARY,
    TOOL_ID_GET_TOOL_DETAIL, TOOL_ID_GET_USER_INPUTS, TOOL_ID_INSPECT_PLAN,
    TOOL_ID_LIST_SNAPSHOTS, TOOL_ID_LLM_GENERATE_COMPONENT,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_MERGE_WORKSPACE,
    TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_MOTION_METRICS, TOOL_ID_QA,
    TOOL_ID_QUERY_COMPONENT_PARTS, TOOL_ID_RECENTER_ATTACHMENT_MOTION,
    TOOL_ID_RENDER_PREVIEW, TOOL_ID_RESTORE_SNAPSHOT,
    TOOL_ID_SET_ACTIVE_WORKSPACE, TOOL_ID_SET_DESCRIPTOR_META, TOOL_ID_SMOKE_CHECK,
    TOOL_ID_SNAPSHOT, TOOL_ID_SUBMIT_TOOLING_FEEDBACK, TOOL_ID_SUGGEST_MOTION_REPAIRS,
    TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{Gen3dToolCallJsonV1, Gen3dToolRegistryV1, Gen3dToolResultJsonV1};
use crate::threaded_result::{new_shared_result, SharedResult};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dWorkshop};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_parsing::{
    normalize_identifier_for_match, parse_delta_transform, resolve_component_index_by_name_hint,
};
use super::agent_prompt::draft_summary;
use super::agent_regen_budget::consume_regen_budget;
use super::agent_review_delta::start_agent_llm_review_delta_call;
use super::agent_review_images::{
    motion_sheets_needed_from_smoke_results, parse_review_preview_blob_ids_from_args,
    review_capture_dimensions_for_max_dim,
};
use super::agent_step::ToolCallOutcome;
use super::agent_utils::{build_component_subset_workspace_defs, sanitize_prefix};
use super::artifacts::{
    append_gen3d_run_log, write_gen3d_assembly_snapshot, write_gen3d_json_artifact,
};
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
    let pass = job.pass;
    let assembly_rev = job.assembly_rev;
    let store = job.ensure_info_store()?;
    store.kv_put(
        attempt,
        pass,
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
    if let Some(dir) = job.pass_dir.as_deref() {
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
        &job.user_prompt_raw,
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
    if let Some(dir) = job.pass_dir.as_deref() {
        write_gen3d_json_artifact(Some(dir), "smoke_results.json", &json);
    }
    job.agent.ever_smoke_checked = true;
    Ok(json)
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

    let registry = Gen3dToolRegistryV1::default();
    match call.tool_id.as_str() {
        TOOL_ID_GET_TOOL_DETAIL => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct GetToolDetailArgsV1 {
                tool_id: String,
            }

            let args: GetToolDetailArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_GET_TOOL_DETAIL}`: {err}"),
                    ));
                }
            };
            let tool_id = args.tool_id.trim();
            if tool_id.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("`{TOOL_ID_GET_TOOL_DETAIL}` requires a non-empty `tool_id` string."),
                ));
            }

            let all_tools = registry.list();
            let Some(tool) = all_tools.iter().find(|t| t.tool_id == tool_id) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unknown tool_id `{}` (see the Available tools list in the prompt).",
                        tool_id
                    ),
                ));
            };

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "tool": tool }),
            ))
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
        TOOL_ID_GET_STATE_SUMMARY => {
            let workspace_id = job.active_workspace_id().trim().to_string();
            let key = format!("ws.{workspace_id}.state_summary");
            let mut json = draft_summary(config, job);
            let record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                key.as_str(),
                json.clone(),
                "state summary".into(),
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

            if let Some(dir) = job.pass_dir.as_deref() {
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
                job.pass_dir.as_deref(),
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
                job.pass,
                &job.plan_hash,
                job.assembly_rev,
                &job.planned_components,
                draft,
            );
            if let Some(dir) = job.pass_dir.as_deref() {
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
            if let Some(dir) = job.pass_dir.as_deref() {
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
            if args.version != 0 && args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_GET_PLAN_TEMPLATE}` version {}.",
                        args.version
                    ),
                ));
            }

            let Some(pass_dir) = job.pass_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir.".into(),
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

            let plan_pretty =
                serde_json::to_string_pretty(&plan).unwrap_or_else(|_| plan.to_string());
            let bytes = plan_pretty.as_bytes().len();
            const MAX_TEMPLATE_BYTES: usize = 60 * 1024;
            if bytes > MAX_TEMPLATE_BYTES {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "`{TOOL_ID_GET_PLAN_TEMPLATE}` output is too large ({bytes} bytes > {MAX_TEMPLATE_BYTES}). Use `get_scene_graph_summary_v1` or simplify the plan before templating."
                    ),
                ));
            }

            let filename = format!("plan_template_{}.json", sanitize_prefix(&call.call_id));
            write_gen3d_json_artifact(Some(pass_dir), &filename, &plan);

            let attempt = job.attempt;
            let pass = job.pass;
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
                    pass,
                    assembly_rev,
                    workspace_id.as_str(),
                    namespace,
                    key.as_str(),
                    plan.clone(),
                    format!("plan template preserve_mode v1 (components={components_total})"),
                    Some(super::info_store::InfoProvenance {
                        tool_id: call.tool_id.clone(),
                        call_id: call.call_id.clone(),
                    }),
                ),
                Err(err) => return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Failed to open Info Store: {err}"),
                )),
            };
            let record = match record {
                Ok(v) => v,
                Err(err) => return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    err,
                )),
            };

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "version": 1,
                    "plan_template_kv": {
                        "namespace": namespace,
                        "key": key,
                        "selector": { "kind": "kv_rev", "kv_rev": record.kv_rev },
                    },
                    "bytes": bytes,
                    "components_total": components_total,
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
            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "motion_metrics.json", &json);
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SUGGEST_MOTION_REPAIRS => {
            #[derive(Debug, Deserialize)]
            #[serde(deny_unknown_fields)]
            struct SuggestMotionRepairsArgsV1 {
                #[serde(default)]
                version: u32,
                #[serde(default)]
                max_suggestions: Option<usize>,
                #[serde(default)]
                safety_margin_degrees: Option<f32>,
            }

            let args: SuggestMotionRepairsArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_SUGGEST_MOTION_REPAIRS}`: {err}"),
                    ));
                }
            };

            if args.version != 0 && args.version != 1 {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "Unsupported `{TOOL_ID_SUGGEST_MOTION_REPAIRS}` version {}.",
                        args.version
                    ),
                ));
            }

            let max_suggestions = args.max_suggestions.unwrap_or(8).clamp(1, 32);
            let safety_margin_degrees = args.safety_margin_degrees.unwrap_or(0.2).clamp(0.0, 5.0);
            let json = super::motion_repairs::suggest_motion_repairs_report_v1(
                job.rig_move_cycle_m,
                &job.planned_components,
                job.assembly_rev(),
                max_suggestions,
                safety_margin_degrees,
            );
            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "suggest_motion_repairs.json", &json);
            }
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_QA => {
            let validate = run_validate_v1(job, draft);
            let smoke = match run_smoke_check_v1(job, draft) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let validate_ok = validate
                .get("ok")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let smoke_ok = smoke.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut errors: Vec<serde_json::Value> = Vec::new();
            let mut warnings: Vec<serde_json::Value> = Vec::new();

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
            );
            collect("smoke", smoke.get("issues"), &mut errors, &mut warnings);
            collect(
                "motion_validation",
                smoke.get("motion_validation").and_then(|v| v.get("issues")),
                &mut errors,
                &mut warnings,
            );

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
            });

            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "qa.json", &json);
            }

            let workspace_id = job.active_workspace_id().trim().to_string();

            // Also persist validate/smoke individually so agents can fetch them via stable keys even
            // when they only run `qa_v1`.
            let validate_json = json.get("validate").cloned().unwrap_or(serde_json::Value::Null);
            let smoke_json = json.get("smoke").cloned().unwrap_or(serde_json::Value::Null);
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
                    call.tool_id.as_str(),
                    call.call_id.as_str(),
                    validate_key.as_str(),
                    validate_json,
                    format!("validate (ok={validate_ok} issues={validate_issues})"),
                ) {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            }
            if smoke_json != serde_json::Value::Null {
                let smoke_ok = smoke_json.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                let smoke_issues = smoke_json
                    .get("issues")
                    .and_then(|v| v.as_array())
                    .map(|v| v.len())
                    .unwrap_or(0);
                if let Err(err) = info_kv_put_for_tool(
                    job,
                    workspace_id.as_str(),
                    call.tool_id.as_str(),
                    call.call_id.as_str(),
                    smoke_key.as_str(),
                    smoke_json,
                    format!("smoke (ok={smoke_ok} issues={smoke_issues})"),
                ) {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            }

            let error_count = json.get("errors").and_then(|v| v.as_array()).map(|v| v.len()).unwrap_or(0);
            let warning_count = json.get("warnings").and_then(|v| v.as_array()).map(|v| v.len()).unwrap_or(0);
            let qa_record = match info_kv_put_for_tool(
                job,
                workspace_id.as_str(),
                call.tool_id.as_str(),
                call.call_id.as_str(),
                qa_key.as_str(),
                json.clone(),
                format!("qa (ok={ok} errors={error_count} warnings={warning_count})"),
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
                    info_kv_ref_json(INFO_KV_NAMESPACE_GEN3D, qa_key.as_str(), qa_record.kv_rev),
                );
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
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
            let page =
                store.page_out(&items, TOOL_ID_INFO_KV_LIST_KEYS, params_sig.as_str(), limit, offset);

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

            let args: InfoKvGetArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_KV_GET}`: {err}"),
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

            let max_bytes = args
                .max_bytes
                .unwrap_or(64 * 1024)
                .clamp(1024, 512 * 1024) as usize;

            let selector_kind = args
                .selector
                .as_ref()
                .map(|s| s.kind.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "latest".into());

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

            let record = match select_kv_record(store, namespace, key, selector_kind.as_str(), args.selector.as_ref()) {
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
                        "KV not found: namespace={namespace:?} key={key:?}. Use `{TOOL_ID_INFO_KV_LIST_KEYS}`."
                    ),
                ));
            };

            let json_pointer = args
                .json_pointer
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string);

            let selected = if let Some(ptr) = json_pointer.as_deref() {
                record.value.pointer(ptr).cloned().ok_or_else(|| {
                    format!("JSON pointer not found in KV value: {ptr}")
                })
            } else {
                Ok(record.value.clone())
            };
            let selected = match selected {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };

            let selected_bytes = match serde_json::to_vec(&selected) {
                Ok(v) => v.len(),
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Failed to serialize KV value: {err}"),
                    ));
                }
            };
            if selected_bytes > max_bytes {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!(
                        "KV value is too large ({selected_bytes} bytes > max_bytes={max_bytes}). Use `json_pointer` to select a smaller subset."
                    ),
                ));
            }

            let mut out = serde_json::Map::new();
            out.insert("ok".into(), serde_json::Value::Bool(true));
            let mut record_json = serde_json::Map::new();
            record_json.insert("kv_rev".into(), serde_json::json!(record.kv_rev));
            record_json.insert("written_at_ms".into(), serde_json::json!(record.written_at_ms));
            record_json.insert("attempt".into(), serde_json::json!(record.attempt));
            record_json.insert("pass".into(), serde_json::json!(record.pass));
            record_json.insert("assembly_rev".into(), serde_json::json!(record.assembly_rev));
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
            out.insert("value".into(), selected);
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
        TOOL_ID_INFO_KV_GET_MANY => {
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

            let args: InfoKvGetManyArgsV1 = match serde_json::from_value(call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        format!("Invalid args for `{TOOL_ID_INFO_KV_GET_MANY}`: {err}"),
                    ));
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

            let selector_ref = args.selector.as_ref();

            let mut out_items: Vec<serde_json::Value> = Vec::with_capacity(requested.len());
            for item in requested {
                let namespace = item.namespace.trim().to_string();
                let key = item.key.trim().to_string();
                if namespace.is_empty() || key.is_empty() {
                    out_items.push(serde_json::json!({
                        "namespace": namespace,
                        "key": key,
                        "ok": false,
                        "error": "Missing namespace/key.",
                    }));
                    continue;
                }

                let max_bytes = item
                    .max_bytes
                    .unwrap_or(64 * 1024)
                    .clamp(1024, 512 * 1024) as usize;

                let record = match select_kv_record(
                    store,
                    namespace.as_str(),
                    key.as_str(),
                    selector_kind.as_str(),
                    selector_ref,
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
                    out_items.push(serde_json::json!({
                        "namespace": namespace,
                        "key": key,
                        "ok": false,
                        "error": "KV not found for selector.",
                    }));
                    continue;
                };

                let json_pointer = item
                    .json_pointer
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let selected = if let Some(ptr) = json_pointer.as_deref() {
                    match record.value.pointer(ptr).cloned() {
                        Some(v) => Ok(v),
                        None => Err(format!("JSON pointer not found: {ptr}")),
                    }
                } else {
                    Ok(record.value.clone())
                };
                let selected = match selected {
                    Ok(v) => v,
                    Err(err) => {
                        out_items.push(serde_json::json!({
                            "namespace": namespace,
                            "key": key,
                            "ok": false,
                            "error": err,
                        }));
                        continue;
                    }
                };
                let selected_bytes = match serde_json::to_vec(&selected) {
                    Ok(v) => v.len(),
                    Err(err) => {
                        out_items.push(serde_json::json!({
                            "namespace": namespace,
                            "key": key,
                            "ok": false,
                            "error": format!("Failed to serialize KV value: {err}"),
                        }));
                        continue;
                    }
                };
                if selected_bytes > max_bytes {
                    out_items.push(serde_json::json!({
                        "namespace": namespace,
                        "key": key,
                        "ok": false,
                        "error": format!("KV value is too large ({selected_bytes} bytes > max_bytes={max_bytes}). Use json_pointer."),
                    }));
                    continue;
                }

                let mut out = serde_json::Map::new();
                out.insert("namespace".into(), serde_json::Value::String(namespace));
                out.insert("key".into(), serde_json::Value::String(key));
                out.insert("ok".into(), serde_json::Value::Bool(true));
                let mut record_json = serde_json::Map::new();
                record_json.insert("kv_rev".into(), serde_json::json!(record.kv_rev));
                record_json.insert("written_at_ms".into(), serde_json::json!(record.written_at_ms));
                record_json.insert("attempt".into(), serde_json::json!(record.attempt));
                record_json.insert("pass".into(), serde_json::json!(record.pass));
                record_json.insert("assembly_rev".into(), serde_json::json!(record.assembly_rev));
                record_json.insert(
                    "workspace_id".into(),
                    serde_json::Value::String(record.workspace_id.clone()),
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
                out.insert("value".into(), selected);
                out.insert("truncated".into(), serde_json::Value::Bool(false));
                if let Some(ptr) = json_pointer {
                    out.insert("json_pointer".into(), serde_json::Value::String(ptr));
                }
                out_items.push(serde_json::Value::Object(out));
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "ok": true,
                    "items": out_items,
                    "truncated": truncated,
                }),
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
                events.sort_by(|a, b| a.ts_ms.cmp(&b.ts_ms).then_with(|| a.event_id.cmp(&b.event_id)));
            } else {
                events.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms).then_with(|| b.event_id.cmp(&a.event_id)));
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
                    let json = serde_json::to_string(&ev.data).unwrap_or_else(|_| ev.data.to_string());
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
                    item.insert("tool_id".into(), serde_json::Value::String(tool_id.to_string()));
                }
                if let Some(call_id) = ev.call_id.as_deref() {
                    item.insert("call_id".into(), serde_json::Value::String(call_id.to_string()));
                }
                item.insert("message".into(), serde_json::Value::String(truncate_chars(&ev.message, 400)));
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

            let max_bytes = args
                .max_bytes
                .unwrap_or(64 * 1024)
                .clamp(1024, 512 * 1024) as usize;

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
                out_event.insert("tool_id".into(), serde_json::Value::String(tool_id.to_string()));
            }
            if let Some(call_id) = event.call_id.as_deref() {
                out_event.insert("call_id".into(), serde_json::Value::String(call_id.to_string()));
            }
            out_event.insert("message".into(), serde_json::Value::String(event.message.clone()));
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
                    let data = serde_json::to_string(&ev.data).unwrap_or_else(|_| ev.data.to_string());
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
        TOOL_ID_RECENTER_ATTACHMENT_MOTION => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::motion_recenter::recenter_attachment_motion_v1(
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
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call_id, tool_id, json))
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
                    "Missing args.source_component (name or index)".into(),
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
            write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
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
                    "Missing args.source_root (name or index)".into(),
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
            write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
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
                    "Missing component (name or index)".into(),
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
            write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
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
        TOOL_ID_LLM_GENERATE_PLAN => {
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating plan…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for plan…");
            job.agent.pending_llm_repair_attempt = 0;

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
                            "plan_template_kv is too large ({} bytes). Re-generate a smaller template or replan without a template.",
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
            let reasoning_effort = super::openai::cap_reasoning_effort(
                ai.model_reasoning_effort(),
                &config.gen3d_reasoning_effort_plan,
            );
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                pass_dir,
                sanitize_prefix(&format!("tool_plan_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::GeneratePlan);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
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
                    job.pass_dir.as_deref(),
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
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
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
            let reasoning_effort = super::openai::cap_reasoning_effort(
                ai.model_reasoning_effort(),
                &config.gen3d_reasoning_effort_component,
            );
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                pass_dir,
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
            let Some(_pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
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
                    job.pass_dir.as_deref(),
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
            for idx in &requested_indices {
                if *idx < job.component_attempts.len() {
                    job.component_attempts[*idx] = 0;
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
        TOOL_ID_LLM_GENERATE_MOTION_AUTHORING => {
            if job.planned_components.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No planned components yet. Generate a plan first.".into(),
                ));
            }
            let Some(ai) = job.ai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing AI config".into(),
                ));
            };
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };
            let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Authoring motion…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for motion authoring…");
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
            let user_text = super::prompts::build_gen3d_motion_authoring_user_text(
                &job.user_prompt_raw,
                image_object_summary,
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
            let reasoning_effort =
                super::openai::cap_reasoning_effort(ai.model_reasoning_effort(), "medium");
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::MotionAuthoringV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                Vec::new(),
                pass_dir,
                sanitize_prefix(&format!("tool_motion_authoring_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool =
                Some(super::Gen3dAgentLlmToolKind::GenerateMotionAuthoring);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            workshop.status = "Authoring motion…".into();
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
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
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
                &pass_dir,
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
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };

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
                    &job.user_prompt_raw,
                    !job.user_images.is_empty(),
                    job.rig_move_cycle_m,
                    &job.planned_components,
                    draft,
                );
                let (include_move_sheet, include_attack_sheet) =
                    motion_sheets_needed_from_smoke_results(&smoke_results);
                let include_motion_sheets = include_move_sheet || include_attack_sheet;

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
                    commands, images, &pass_dir, draft, false, &prefix, &views, width_px, height_px,
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
    use super::normalize_tool_call_args;
    use crate::gen3d::agent::Gen3dToolCallJsonV1;
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

        let latest = super::select_kv_record(&store, "gen3d", "ws.main.state_summary", "latest", None)
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
}
