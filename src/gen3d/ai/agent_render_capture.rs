use bevy::prelude::*;
use std::path::PathBuf;

use crate::config::AppConfig;
use crate::gen3d::agent::tools::TOOL_ID_LLM_REVIEW_DELTA;
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolCallJsonV1, Gen3dToolResultJsonV1,
};
use crate::types::{ActionClock, AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{Gen3dDraft, Gen3dPreviewModelRoot, Gen3dWorkshop};
use super::agent_review_delta::start_agent_llm_review_delta_call;
use super::agent_utils::note_observable_tool_result;
use super::artifacts::append_gen3d_jsonl_artifact;
use super::{fail_job, set_progress, Gen3dAiJob, Gen3dAiPhase};

pub(super) fn poll_agent_render_capture(
    config: &AppConfig,
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
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    #[derive(Clone, Debug)]
    struct RenderCaptureBlobs {
        blob_ids: Vec<String>,
        static_blob_ids: Vec<String>,
        move_sheet_blob_id: Option<String>,
        action_sheet_blob_id: Option<String>,
        attack_sheet_blob_id: Option<String>,
    }

    fn view_label_from_png_filename(file_name: &str) -> Option<&'static str> {
        let stem = file_name.strip_suffix(".png")?;
        let views = [
            "front_left",
            "front_right",
            "left_back",
            "right_back",
            "front",
            "back",
            "top",
            "bottom",
        ];
        for view in views {
            if stem == view || stem.ends_with(&format!("_{view}")) {
                return Some(view);
            }
        }
        None
    }

    fn finish_paths(
        workshop: &mut Gen3dWorkshop,
        job: &mut Gen3dAiJob,
        paths: Vec<PathBuf>,
    ) -> Option<(Gen3dToolCallJsonV1, RenderCaptureBlobs)> {
        for path in &paths {
            if std::fs::metadata(path).is_err() {
                fail_job(
                    workshop,
                    job,
                    format!("Render missing output file: {}", path.display()),
                );
                return None;
            }
        }

        let Some(run_dir) = job.run_dir.clone() else {
            fail_job(
                workshop,
                job,
                "Internal error: missing run_dir for render capture.",
            );
            return None;
        };
        let attempt = job.attempt;
        let pass = job.pass;
        let assembly_rev = job.assembly_rev;
        let workspace_id = job.active_workspace_id().trim().to_string();

        let mut blobs = RenderCaptureBlobs {
            blob_ids: Vec::new(),
            static_blob_ids: Vec::new(),
            move_sheet_blob_id: None,
            action_sheet_blob_id: None,
            attack_sheet_blob_id: None,
        };

        let mut error: Option<String> = None;
        {
            let store = match job.ensure_info_store() {
                Ok(s) => s,
                Err(err) => {
                    fail_job(
                        workshop,
                        job,
                        format!("Internal error: failed to open info store: {err}"),
                    );
                    return None;
                }
            };

            for path in &paths {
                let meta = match std::fs::metadata(path) {
                    Ok(v) => v,
                    Err(err) => {
                        error = Some(format!(
                            "Failed to stat render output {}: {err}",
                            path.display()
                        ));
                        break;
                    }
                };
                let bytes = meta.len();
                let Some(rel) = path
                    .strip_prefix(&run_dir)
                    .ok()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                else {
                    error = Some(format!(
                        "Internal error: render output is outside run_dir: {}",
                        path.display()
                    ));
                    break;
                };

                let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                let mut labels: Vec<String> = Vec::new();
                labels.push(format!("workspace:{workspace_id}"));

                if file_name == "move_sheet.png" {
                    labels.push("kind:motion_sheet".into());
                    labels.push("motion:move".into());
                } else if file_name == "action_sheet.png" {
                    labels.push("kind:motion_sheet".into());
                    labels.push("motion:action".into());
                } else if file_name == "attack_sheet.png" {
                    labels.push("kind:motion_sheet".into());
                    labels.push("motion:attack".into());
                } else if let Some(view) = view_label_from_png_filename(file_name) {
                    labels.push("kind:render_preview".into());
                    labels.push(format!("view:{view}"));
                } else {
                    labels.push("kind:render_preview".into());
                }

                let blob = match store.register_blob_file(
                    attempt,
                    pass,
                    assembly_rev,
                    "image/png",
                    bytes,
                    labels,
                    rel,
                ) {
                    Ok(v) => v,
                    Err(err) => {
                        error = Some(format!("Failed to register render output blob: {err}"));
                        break;
                    }
                };

                match file_name {
                    "move_sheet.png" => blobs.move_sheet_blob_id = Some(blob.blob_id.clone()),
                    "action_sheet.png" => blobs.action_sheet_blob_id = Some(blob.blob_id.clone()),
                    "attack_sheet.png" => blobs.attack_sheet_blob_id = Some(blob.blob_id.clone()),
                    _ => blobs.static_blob_ids.push(blob.blob_id.clone()),
                }
                blobs.blob_ids.push(blob.blob_id);
            }
        }
        if let Some(err) = error {
            fail_job(workshop, job, err);
            return None;
        }

        job.agent.rendered_since_last_review = true;
        job.agent.ever_rendered = true;
        job.agent.last_render_blob_ids = blobs.blob_ids.clone();
        job.agent.last_render_assembly_rev = Some(job.assembly_rev);

        let Some(call) = job.agent.pending_tool_call.take() else {
            fail_job(workshop, job, "Internal error: missing pending tool call");
            return None;
        };
        Some((call, blobs))
    }

    // If motion capture is active, keep polling it until it finishes, then finalize the tool result.
    if job.motion_capture.is_some() {
        super::poll_gen3d_motion_capture(
            time,
            commands,
            images,
            workshop,
            job,
            draft,
            preview_model,
        );
        if job.motion_capture.is_some() {
            return;
        }
    }

    // If motion capture finished, the combined static+motion paths live in `job.review_static_paths`.
    if job.agent.pending_render.is_none() && !job.review_static_paths.is_empty() {
        let paths = std::mem::take(&mut job.review_static_paths);
        let Some((call, blobs)) = finish_paths(workshop, job, paths) else {
            return;
        };

        if call.tool_id == TOOL_ID_LLM_REVIEW_DELTA {
            let call_id = call.call_id.clone();
            let tool_id = call.tool_id.clone();
            match start_agent_llm_review_delta_call(config, job, draft, call) {
                Ok(()) => return,
                Err(err) => {
                    let result = Gen3dToolResultJsonV1::err(
                        call_id,
                        tool_id,
                        format!("Review prerender completed, but review call failed: {err}"),
                    );
                    job.metrics.note_tool_result(&result);
                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::ToolResult {
                            call_id: result.call_id.clone(),
                            tool_id: result.tool_id.clone(),
                            ok: result.ok,
                            result: result.result.clone(),
                            error: result.error.clone(),
                        },
                    );
                    append_gen3d_jsonl_artifact(
                        job.pass_dir.as_deref(),
                        "tool_results.jsonl",
                        &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                    );
                    let message = if result.ok {
                        format!("Tool call ok: {}", result.tool_id)
                    } else {
                        let err = result.error.as_deref().unwrap_or("").trim();
                        let first_line = err.split('\n').next().unwrap_or("");
                        if first_line.is_empty() {
                            format!("Tool call error: {}", result.tool_id)
                        } else {
                            format!(
                                "Tool call error: {}: {}",
                                result.tool_id,
                                super::truncate_for_ui(first_line, 240)
                            )
                        }
                    };
                    job.append_info_event_best_effort(
                        super::info_store::InfoEventKindV1::ToolCallResult,
                        Some(result.tool_id.clone()),
                        Some(result.call_id.clone()),
                        message,
                        serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                    );
                    note_observable_tool_result(job, &result);
                    job.agent.step_tool_results.push(result);
                    job.phase = Gen3dAiPhase::AgentExecutingActions;
                    return;
                }
            }
        }

        let result = Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "blob_ids": blobs.blob_ids,
                "static_blob_ids": blobs.static_blob_ids,
                "motion_sheet_blob_ids": {
                    "move": blobs.move_sheet_blob_id,
                    "action": blobs.action_sheet_blob_id,
                    "attack": blobs.attack_sheet_blob_id
                },
            }),
        );
        job.metrics.note_tool_result(&result);
        append_agent_trace_event_v1(
            job.run_dir.as_deref(),
            &AgentTraceEventV1::ToolResult {
                call_id: result.call_id.clone(),
                tool_id: result.tool_id.clone(),
                ok: result.ok,
                result: result.result.clone(),
                error: result.error.clone(),
            },
        );
        append_gen3d_jsonl_artifact(
            job.pass_dir.as_deref(),
            "tool_results.jsonl",
            &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
        );
        let message = if result.ok {
            format!("Tool call ok: {}", result.tool_id)
        } else {
            let err = result.error.as_deref().unwrap_or("").trim();
            let first_line = err.split('\n').next().unwrap_or("");
            if first_line.is_empty() {
                format!("Tool call error: {}", result.tool_id)
            } else {
                format!(
                    "Tool call error: {}: {}",
                    result.tool_id,
                    super::truncate_for_ui(first_line, 240)
                )
            }
        };
        job.append_info_event_best_effort(
            super::info_store::InfoEventKindV1::ToolCallResult,
            Some(result.tool_id.clone()),
            Some(result.call_id.clone()),
            message,
            serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
        );
        note_observable_tool_result(job, &result);
        job.agent.step_tool_results.push(result);
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        return;
    }

    // Otherwise poll the static render capture.
    let Some(state) = job.agent.pending_render.as_ref() else {
        fail_job(workshop, job, "Internal error: missing pending render");
        return;
    };
    let (done, expected) = state
        .progress
        .lock()
        .map(|g| (g.completed, g.expected))
        .unwrap_or((0, 1));
    if done < expected {
        if let Some(progress) = job.shared_progress.as_ref() {
            set_progress(progress, format!("Rendering… ({done}/{expected})"));
        }
        return;
    }

    let Some(state) = job.agent.pending_render.take() else {
        return;
    };
    for cam in state.cameras.iter().copied() {
        commands.entity(cam).try_despawn();
    }
    let paths = state.image_paths.clone();

    if job.agent.pending_render_include_motion_sheets {
        // Capture motion sprite sheets (move + action + attack) and return them alongside the
        // static renders.
        job.review_static_paths = paths;
        job.motion_capture = Some(super::Gen3dMotionCaptureState::new());
        super::poll_gen3d_motion_capture(
            time,
            commands,
            images,
            workshop,
            job,
            draft,
            preview_model,
        );
        return;
    }

    let Some((call, blobs)) = finish_paths(workshop, job, paths) else {
        return;
    };
    if call.tool_id == TOOL_ID_LLM_REVIEW_DELTA {
        let call_id = call.call_id.clone();
        let tool_id = call.tool_id.clone();
        match start_agent_llm_review_delta_call(config, job, draft, call) {
            Ok(()) => return,
            Err(err) => {
                let result = Gen3dToolResultJsonV1::err(
                    call_id,
                    tool_id,
                    format!("Review prerender completed, but review call failed: {err}"),
                );
                job.metrics.note_tool_result(&result);
                append_agent_trace_event_v1(
                    job.run_dir.as_deref(),
                    &AgentTraceEventV1::ToolResult {
                        call_id: result.call_id.clone(),
                        tool_id: result.tool_id.clone(),
                        ok: result.ok,
                        result: result.result.clone(),
                        error: result.error.clone(),
                    },
                );
                append_gen3d_jsonl_artifact(
                    job.pass_dir.as_deref(),
                    "tool_results.jsonl",
                    &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                );
                let message = if result.ok {
                    format!("Tool call ok: {}", result.tool_id)
                } else {
                    let err = result.error.as_deref().unwrap_or("").trim();
                    let first_line = err.split('\n').next().unwrap_or("");
                    if first_line.is_empty() {
                        format!("Tool call error: {}", result.tool_id)
                    } else {
                        format!(
                            "Tool call error: {}: {}",
                            result.tool_id,
                            super::truncate_for_ui(first_line, 240)
                        )
                    }
                };
                job.append_info_event_best_effort(
                    super::info_store::InfoEventKindV1::ToolCallResult,
                    Some(result.tool_id.clone()),
                    Some(result.call_id.clone()),
                    message,
                    serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
                );
                note_observable_tool_result(job, &result);
                job.agent.step_tool_results.push(result);
                job.phase = Gen3dAiPhase::AgentExecutingActions;
                return;
            }
        }
    }

    let result = Gen3dToolResultJsonV1::ok(
        call.call_id,
        call.tool_id,
        serde_json::json!({
            "blob_ids": blobs.blob_ids,
            "static_blob_ids": blobs.static_blob_ids,
            "motion_sheet_blob_ids": {
                "move": blobs.move_sheet_blob_id,
                "action": blobs.action_sheet_blob_id,
                "attack": blobs.attack_sheet_blob_id
            },
        }),
    );
    job.metrics.note_tool_result(&result);
    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::ToolResult {
            call_id: result.call_id.clone(),
            tool_id: result.tool_id.clone(),
            ok: result.ok,
            result: result.result.clone(),
            error: result.error.clone(),
        },
    );
    append_gen3d_jsonl_artifact(
        job.pass_dir.as_deref(),
        "tool_results.jsonl",
        &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
    );
    let message = if result.ok {
        format!("Tool call ok: {}", result.tool_id)
    } else {
        let err = result.error.as_deref().unwrap_or("").trim();
        let first_line = err.split('\n').next().unwrap_or("");
        if first_line.is_empty() {
            format!("Tool call error: {}", result.tool_id)
        } else {
            format!(
                "Tool call error: {}: {}",
                result.tool_id,
                super::truncate_for_ui(first_line, 240)
            )
        }
    };
    job.append_info_event_best_effort(
        super::info_store::InfoEventKindV1::ToolCallResult,
        Some(result.tool_id.clone()),
        Some(result.call_id.clone()),
        message,
        serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
    );
    note_observable_tool_result(job, &result);
    job.agent.step_tool_results.push(result);
    job.phase = Gen3dAiPhase::AgentExecutingActions;
}
