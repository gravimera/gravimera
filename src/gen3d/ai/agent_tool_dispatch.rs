use bevy::prelude::*;
use serde::Deserialize;
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE,
    TOOL_ID_COPY_FROM_WORKSPACE, TOOL_ID_CREATE_WORKSPACE, TOOL_ID_DELETE_WORKSPACE,
    TOOL_ID_DETACH_COMPONENT, TOOL_ID_DIFF_SNAPSHOTS, TOOL_ID_DIFF_WORKSPACES,
    TOOL_ID_GET_PLAN_TEMPLATE, TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_STATE_SUMMARY,
    TOOL_ID_GET_TOOL_DETAIL, TOOL_ID_GET_USER_INPUTS, TOOL_ID_INSPECT_PLAN,
    TOOL_ID_LIST_RUN_ARTIFACTS, TOOL_ID_LIST_SNAPSHOTS, TOOL_ID_LLM_GENERATE_COMPONENT,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_MERGE_WORKSPACE,
    TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_MOTION_METRICS, TOOL_ID_QA,
    TOOL_ID_QUERY_COMPONENT_PARTS, TOOL_ID_READ_ARTIFACT, TOOL_ID_RECENTER_ATTACHMENT_MOTION,
    TOOL_ID_RENDER_PREVIEW, TOOL_ID_RESTORE_SNAPSHOT, TOOL_ID_SEARCH_ARTIFACTS,
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
    motion_sheets_needed_from_smoke_results, parse_review_preview_images_from_args,
    review_capture_dimensions_for_max_dim,
};
use super::agent_step::ToolCallOutcome;
use super::agent_utils::{build_component_subset_workspace_defs, sanitize_prefix};
use super::artifacts::{
    append_gen3d_run_log, list_run_artifacts_v1, read_artifact_v1, search_artifacts_v1,
    write_gen3d_assembly_snapshot, write_gen3d_json_artifact,
};
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
        TOOL_ID_GET_STATE_SUMMARY => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            draft_summary(config, job),
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
            let json = super::build_gen3d_scene_graph_summary(
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

            let Some(run_dir) = job.run_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing run dir.".into(),
                ));
            };
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

            let artifact_path = pass_dir.join(&filename);
            let artifact_ref = artifact_path
                .strip_prefix(run_dir)
                .ok()
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|| filename.clone());

            let components_total = plan
                .get("components")
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({
                    "version": 1,
                    "artifact_ref": artifact_ref,
                    "bytes": bytes,
                    "components_total": components_total,
                }),
            ))
        }
        TOOL_ID_QUERY_COMPONENT_PARTS => {
            let json = match super::draft_ops::query_component_parts_v1(job, draft, call.args) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_VALIDATE => {
            let json = run_validate_v1(job, draft);
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SMOKE_CHECK => {
            let json = match run_smoke_check_v1(job, draft) {
                Ok(v) => v,
                Err(err) => {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        err,
                    ));
                }
            };
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
            let json = serde_json::json!({
                "ok": ok,
                "validate": validate,
                "smoke": smoke,
                "errors": errors,
                "warnings": warnings,
            });

            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "qa.json", &json);
            }

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_LIST_RUN_ARTIFACTS => {
            let Some(run_dir) = job.run_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No active Gen3D run (missing run_dir).".into(),
                ));
            };

            let path_prefix = call
                .args
                .get("path_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let max_items = call
                .args
                .get("max_items")
                .and_then(|v| v.as_u64())
                .unwrap_or(500)
                .clamp(1, 500) as usize;

            let (items, truncated) =
                match list_run_artifacts_v1(run_dir, path_prefix.as_deref(), max_items) {
                    Ok(v) => v,
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            err,
                        ));
                    }
                };

            let json = serde_json::json!({
                "ok": true,
                "run_dir": run_dir.display().to_string(),
                "path_prefix": path_prefix,
                "items": items,
                "truncated": truncated,
            });
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_READ_ARTIFACT => {
            let Some(run_dir) = job.run_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No active Gen3D run (missing run_dir).".into(),
                ));
            };

            let Some(artifact_ref) = call.args.get("artifact_ref").and_then(|v| v.as_str()) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.artifact_ref".into(),
                ));
            };

            let max_bytes = call
                .args
                .get("max_bytes")
                .and_then(|v| v.as_u64())
                .unwrap_or(64 * 1024) as usize;
            let tail_lines = call
                .args
                .get("tail_lines")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let json_pointer = call
                .args
                .get("json_pointer")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());

            let json = match read_artifact_v1(
                run_dir,
                artifact_ref,
                max_bytes,
                tail_lines,
                json_pointer.as_deref(),
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
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SEARCH_ARTIFACTS => {
            let Some(run_dir) = job.run_dir.as_deref() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "No active Gen3D run (missing run_dir).".into(),
                ));
            };

            let Some(query) = call.args.get("query").and_then(|v| v.as_str()) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.query".into(),
                ));
            };

            let path_prefix = call
                .args
                .get("path_prefix")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let max_matches = call
                .args
                .get("max_matches")
                .and_then(|v| v.as_u64())
                .unwrap_or(200) as usize;
            let max_bytes_per_file = call
                .args
                .get("max_bytes_per_file")
                .and_then(|v| v.as_u64())
                .unwrap_or(64 * 1024) as usize;

            let (matches_out, truncated) = match search_artifacts_v1(
                run_dir,
                query,
                path_prefix.as_deref(),
                max_matches,
                max_bytes_per_file,
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

            let json = serde_json::json!({
                "ok": true,
                "query": query.trim(),
                "path_prefix": path_prefix,
                "matches": matches_out,
                "truncated": truncated,
            });
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_APPLY_DRAFT_OPS => {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            let json = match super::draft_ops::apply_draft_ops_v1(
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
            let plan_template_artifact_ref = call
                .args
                .get("plan_template_artifact_ref")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
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

            let prompt_text = prompt_override
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or(job.user_prompt_raw.as_str());

            let plan_template_json: Option<serde_json::Value> = if let Some(template_ref) =
                plan_template_artifact_ref.as_deref()
            {
                if !preserve_existing_components || job.planned_components.is_empty() {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!(
                                "`plan_template_artifact_ref` requires preserve mode (constraints.preserve_existing_components=true) and an existing plan."
                            ),
                        ));
                }
                let Some(run_dir) = job.run_dir.as_deref() else {
                    return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                        call.call_id,
                        call.tool_id,
                        "Missing run dir (needed to read plan_template_artifact_ref).".into(),
                    ));
                };
                match read_artifact_v1(run_dir, template_ref, 64 * 1024, None, None) {
                    Ok(v) => {
                        let truncated = v
                            .get("truncated")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                        if truncated {
                            return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                                    call.call_id,
                                    call.tool_id,
                                    format!(
                                        "plan_template_artifact_ref `{}` is truncated when read (too large). Re-generate a smaller template or replan without a template.",
                                        template_ref
                                    ),
                                ));
                        }
                        v.get("json").cloned()
                    }
                    Err(err) => {
                        return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                            call.call_id,
                            call.tool_id,
                            format!(
                                "Failed to read plan_template_artifact_ref `{}`: {}",
                                template_ref, err
                            ),
                        ));
                    }
                }
            } else {
                None
            };

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

            let preview_images_were_explicit =
                !parse_review_preview_images_from_args(&call.args).is_empty();
            let last_render_fresh = !job.agent.last_render_images.is_empty()
                && job.agent.last_render_assembly_rev == Some(job.assembly_rev);
            let can_render = draft.total_non_projectile_primitive_parts() > 0;

            if job.review_appearance
                && !preview_images_were_explicit
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
}
