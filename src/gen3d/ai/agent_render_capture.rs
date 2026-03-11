use bevy::prelude::*;
use std::path::PathBuf;

use crate::config::AppConfig;
use crate::gen3d::agent::tools::TOOL_ID_LLM_REVIEW_DELTA;
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolCallJsonV1, Gen3dToolResultJsonV1,
};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

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
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    fn finish_paths(
        workshop: &mut Gen3dWorkshop,
        job: &mut Gen3dAiJob,
        paths: Vec<PathBuf>,
    ) -> Option<(Gen3dToolCallJsonV1, Vec<PathBuf>)> {
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

        job.agent.rendered_since_last_review = true;
        job.agent.ever_rendered = true;
        job.agent.last_render_images = paths.clone();
        job.agent.last_render_assembly_rev = Some(job.assembly_rev);

        let Some(call) = job.agent.pending_tool_call.take() else {
            fail_job(workshop, job, "Internal error: missing pending tool call");
            return None;
        };
        Some((call, paths))
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
        let Some((call, paths)) = finish_paths(workshop, job, paths) else {
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
                    note_observable_tool_result(job, &result);
                    job.agent.step_tool_results.push(result);
                    job.phase = Gen3dAiPhase::AgentExecutingActions;
                    return;
                }
            }
        }

        let mut images: Vec<String> = Vec::with_capacity(paths.len());
        let mut static_images: Vec<String> = Vec::new();
        let mut move_sheet: Option<String> = None;
        let mut attack_sheet: Option<String> = None;
        for path in &paths {
            let s = path.display().to_string();
            images.push(s.clone());
            match path.file_name().and_then(|v| v.to_str()) {
                Some("move_sheet.png") => move_sheet = Some(s),
                Some("attack_sheet.png") => attack_sheet = Some(s),
                _ => static_images.push(s),
            }
        }

        let result = Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "images": images,
                "static_images": static_images,
                "motion_sheets": { "move": move_sheet, "attack": attack_sheet },
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
        // Capture motion sprite sheets (move + attack) and return them alongside the static renders.
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

    let Some((call, paths)) = finish_paths(workshop, job, paths) else {
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
                note_observable_tool_result(job, &result);
                job.agent.step_tool_results.push(result);
                job.phase = Gen3dAiPhase::AgentExecutingActions;
                return;
            }
        }
    }

    let mut images: Vec<String> = Vec::with_capacity(paths.len());
    let mut static_images: Vec<String> = Vec::new();
    let mut move_sheet: Option<String> = None;
    let mut attack_sheet: Option<String> = None;
    for path in &paths {
        let s = path.display().to_string();
        images.push(s.clone());
        match path.file_name().and_then(|v| v.to_str()) {
            Some("move_sheet.png") => move_sheet = Some(s),
            Some("attack_sheet.png") => attack_sheet = Some(s),
            _ => static_images.push(s),
        }
    }

    let result = Gen3dToolResultJsonV1::ok(
        call.call_id,
        call.tool_id,
        serde_json::json!({
            "images": images,
            "static_images": static_images,
            "motion_sheets": { "move": move_sheet, "attack": attack_sheet },
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
    note_observable_tool_result(job, &result);
    job.agent.step_tool_results.push(result);
    job.phase = Gen3dAiPhase::AgentExecutingActions;
}
