use bevy::log::{debug, warn};
use bevy::prelude::*;

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_LLM_GENERATE_MOTION_AUTHORING, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_RENDER_PREVIEW,
    TOOL_ID_SMOKE_CHECK, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dAgentActionJsonV1, Gen3dToolCallJsonV1,
    Gen3dToolResultJsonV1,
};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_loop::spawn_agent_step_request;
use super::agent_parsing::{is_transient_ai_error_message, parse_agent_step};
use super::agent_tool_dispatch::execute_tool_call;
use super::agent_utils::{
    compute_agent_state_hash, note_observable_tool_result, truncate_json_for_log,
};
use super::artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_text_artifact,
};
use super::GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES;
use super::{fail_job, gen3d_advance_pass, set_progress, Gen3dAiJob, Gen3dAiPhase};

pub(super) fn poll_agent_step(
    config: &AppConfig,
    commands: &mut Commands,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) {
    let Some(shared) = job.shared_result.as_ref() else {
        return;
    };
    let result = shared.lock().ok().and_then(|mut g| g.take());
    let Some(result) = result else {
        return;
    };
    job.shared_result = None;
    job.metrics.note_agent_step_response_received();

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }
            job.agent.step_request_retry_attempt = 0;

            let text = resp.text;
            if let Some(pass_dir) = job.pass_dir.as_deref() {
                write_gen3d_text_artifact(Some(pass_dir), "agent_step_raw.txt", text.trim());
            }

            match parse_agent_step(&text) {
                Ok(step) => {
                    workshop.error = None;
                    if !step.status_summary.trim().is_empty() {
                        workshop.status = step.status_summary.trim().to_string();
                    }

                    let mut actions_summary = Vec::new();
                    for action in step.actions.iter() {
                        match action {
                            Gen3dAgentActionJsonV1::ToolCall { tool_id, .. } => {
                                actions_summary.push(format!("tool_call:{tool_id}"));
                            }
                            Gen3dAgentActionJsonV1::Done { .. } => {
                                actions_summary.push("done".to_string());
                            }
                        }
                    }
                    let actions_summary = actions_summary.join(", ");

                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::Info {
                            message: format!(
                                "Gen3D agent step parsed: status_summary={:?} actions=[{}]",
                                step.status_summary.trim(),
                                actions_summary
                            ),
                        },
                    );
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        format!(
                            "agent_step_parsed status_summary={:?} actions=[{}]",
                            step.status_summary.trim(),
                            actions_summary
                        ),
                    );
                    debug!(
                        "Gen3D agent step parsed: status_summary={:?} actions=[{}]",
                        step.status_summary.trim(),
                        actions_summary
                    );

                    job.agent.step_actions = step.actions;
                    job.agent.step_action_idx = 0;
                    job.agent.step_tool_results.clear();
                    job.agent.step_had_observable_output = false;
                    job.phase = Gen3dAiPhase::AgentExecutingActions;
                }
                Err(err) => {
                    job.agent.step_repair_attempt = job.agent.step_repair_attempt.saturating_add(1);
                    let attempt = job.agent.step_repair_attempt;
                    if attempt <= 2 {
                        workshop.status =
                            format!("Agent output invalid (attempt {attempt}/2). Retrying…");
                        workshop.error = Some(err.clone());
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "agent_step_parse_failed attempt={attempt} err={}",
                                err.trim()
                            ),
                        );
                        warn!(
                            "Gen3D agent step parse error (attempt {attempt}/2): {}",
                            err.trim()
                        );
                        if let Some(pass_dir) = job.pass_dir.clone() {
                            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
                            job.phase = Gen3dAiPhase::AgentWaitingStep;
                            return;
                        }
                    }
                    fail_job(
                        workshop,
                        job,
                        format!("Gen3D agent step parse error: {err}"),
                    );
                }
            }
        }
        Err(err) => {
            if is_transient_ai_error_message(&err) {
                job.agent.step_request_retry_attempt =
                    job.agent.step_request_retry_attempt.saturating_add(1);
                let attempt = job.agent.step_request_retry_attempt;
                if attempt <= GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES {
                    workshop.status = format!(
                        "AI request failed (attempt {attempt}/{GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES}); retrying…"
                    );
                    workshop.error = Some(err.clone());
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        format!(
                            "agent_step_request_failed transient attempt={attempt} err={}",
                            super::truncate_for_ui(&err, 600)
                        ),
                    );
                    warn!(
                        "Gen3D agent step request transient failure; retrying (attempt {attempt}/{GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES}) err={}",
                        super::truncate_for_ui(&err, 240)
                    );
                    if let Some(pass_dir) = job.pass_dir.clone() {
                        let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
                        job.phase = Gen3dAiPhase::AgentWaitingStep;
                        return;
                    }
                }

                if draft.total_non_projectile_primitive_parts() > 0 {
                    super::finish_job_best_effort(
                        commands,
                        review_cameras,
                        workshop,
                        job,
                        format!(
                            "AI transient failure after {attempt} retry attempt(s). Last error: {}",
                            super::truncate_for_ui(&err, 600)
                        ),
                    );
                    return;
                }
            }

            fail_job(workshop, job, err);
        }
    }

    let _ = feedback_history;
}

pub(super) fn execute_agent_actions(
    config: &AppConfig,
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    preview: &mut Gen3dPreview,
    preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    let max_actions_per_tick = 4usize;
    let mut executed = 0usize;

    while executed < max_actions_per_tick {
        if job.agent.step_action_idx >= job.agent.step_actions.len() {
            // No-progress guard: if the agent keeps asking for steps but nothing changes and we
            // don't change the draft/assembly, stop best-effort.
            let state_hash = compute_agent_state_hash(job, draft);
            let changed = job
                .agent
                .last_state_hash
                .as_deref()
                .map(|h| h != state_hash.as_str())
                .unwrap_or(true);
            if changed {
                job.agent.no_progress_steps = 0;
                job.agent.last_state_hash = Some(state_hash.clone());
            } else {
                job.agent.no_progress_steps = job.agent.no_progress_steps.saturating_add(1);
            }
            job.agent.step_had_observable_output = false;

            let max_steps = config.gen3d_no_progress_max_steps;
            if max_steps > 0 && job.agent.no_progress_steps >= max_steps {
                let visual_qa_required = job
                    .ai
                    .as_ref()
                    .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
                    .unwrap_or(true);
                let qa_ok = job.agent.ever_validated
                    && job.agent.ever_smoke_checked
                    && (!visual_qa_required
                        || (job.agent.ever_rendered && job.agent.ever_reviewed));
                if !qa_ok {
                    // Prefer continuing so the agent can run the required QA sequence.
                    // If it refuses, budgets will stop the run anyway.
                    job.agent.no_progress_steps = 0;
                    job.agent.last_state_hash = Some(state_hash);
                } else {
                    workshop.error = None;
                    let status = format!(
                        "Build finished (best effort).\nReason: No-progress guard triggered ({} step(s) without progress).",
                        job.agent.no_progress_steps
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
                                "no_progress_guard_stop steps={}",
                                job.agent.no_progress_steps
                            ),
                            info_log: format!(
                                "Gen3D agent: best-effort stop (no-progress guard; steps={}).",
                                job.agent.no_progress_steps
                            ),
                        },
                    ) {
                        workshop.status = status;
                        return;
                    }

                    workshop.status = status;
                    job.finish_run_metrics();
                    job.running = false;
                    job.build_complete = true;
                    job.phase = Gen3dAiPhase::Idle;
                    job.shared_progress = None;
                    job.shared_result = None;
                    return;
                }
            }

            // Step complete: request next step.
            if maybe_start_pass_snapshot_capture(
                config,
                commands,
                images,
                workshop,
                job,
                draft,
                super::Gen3dAgentAfterPassSnapshot::AdvancePassAndRequestStep,
            ) {
                return;
            }
            if let Err(err) = gen3d_advance_pass(job) {
                fail_job(workshop, job, err);
                return;
            }
            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing pass dir");
                return;
            };
            append_gen3d_run_log(Some(&pass_dir), "agent_step_complete; requesting next step");
            debug!("Gen3D agent: step complete; requesting next step");
            job.phase = Gen3dAiPhase::AgentWaitingStep;
            job.agent.step_repair_attempt = 0;
            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
            return;
        }

        let action = job.agent.step_actions[job.agent.step_action_idx].clone();
        match action {
            Gen3dAgentActionJsonV1::Done { reason } => {
                // Guardrail: some models treat "done" as "end of step" rather than "end of run".
                // Only stop the run if we have a usable draft (at least one non-projectile primitive part).
                if draft.total_non_projectile_primitive_parts() == 0 {
                    workshop.error = Some(
                        "Agent requested done before generating any primitives; continuing."
                            .to_string(),
                    );
                    workshop.status = "Continuing Gen3D build… (agent ended early)".into();
                    job.agent.step_action_idx = job.agent.step_actions.len();
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        "agent_done_ignored (no primitives yet); continuing",
                    );
                    warn!("Gen3D agent requested done before primitives existed; continuing");
                    continue;
                }
                let llm_available = job
                    .ai
                    .as_ref()
                    .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
                    .unwrap_or(true);
                let appearance_review_enabled = llm_available && job.review_appearance;

                if appearance_review_enabled && job.agent.rendered_since_last_review {
                    let images: Vec<String> = job
                        .agent
                        .last_render_images
                        .iter()
                        .map(|p| p.display().to_string())
                        .collect();
                    workshop.error = Some(format!(
                        "Agent requested done, but preview renders have not been reviewed yet. Call `llm_review_delta_v1` with `preview_images` set to the latest render outputs: {images:?}"
                    ));
                    workshop.status = "Continuing Gen3D build… (review required)".into();
                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::Info {
                            message: "agent_done_ignored (review required)".to_string(),
                        },
                    );
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        "agent_done_ignored (review required); continuing",
                    );
                    warn!(
                        "Gen3D agent requested done without reviewing latest renders; continuing"
                    );
                    job.agent.step_action_idx = job.agent.step_actions.len();
                    continue;
                }

                let mut missing: Vec<&str> = Vec::new();
                if appearance_review_enabled && !job.agent.ever_rendered {
                    missing.push(TOOL_ID_RENDER_PREVIEW);
                }
                if llm_available && !job.agent.ever_reviewed {
                    missing.push(TOOL_ID_LLM_REVIEW_DELTA);
                }
                if !job.agent.ever_validated {
                    missing.push(TOOL_ID_VALIDATE);
                }
                if !job.agent.ever_smoke_checked {
                    missing.push(TOOL_ID_SMOKE_CHECK);
                }
                if !missing.is_empty() {
                    let missing_list = missing.join(", ");
                    let qa_sequence = if appearance_review_enabled {
                        format!(
                            "{TOOL_ID_RENDER_PREVIEW} -> {TOOL_ID_LLM_REVIEW_DELTA} -> {TOOL_ID_VALIDATE} -> {TOOL_ID_SMOKE_CHECK}"
                        )
                    } else if llm_available {
                        format!(
                            "{TOOL_ID_LLM_REVIEW_DELTA} -> {TOOL_ID_VALIDATE} -> {TOOL_ID_SMOKE_CHECK}"
                        )
                    } else {
                        format!("{TOOL_ID_VALIDATE} -> {TOOL_ID_SMOKE_CHECK}")
                    };
                    workshop.error = Some(format!(
                        "Agent requested done, but required QA tools have not been run yet: {missing_list}. Continue and run the minimal QA sequence: {qa_sequence}."
                    ));
                    workshop.status = "Continuing Gen3D build… (QA required)".into();
                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::Info {
                            message: "agent_done_ignored (QA required)".to_string(),
                        },
                    );
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        format!(
                            "agent_done_ignored (QA required missing={missing_list}); continuing"
                        ),
                    );
                    warn!(
                        "Gen3D agent requested done without required QA tools; continuing (missing: {missing_list})"
                    );
                    job.agent.step_action_idx = job.agent.step_actions.len();
                    continue;
                }
                if job.agent.last_motion_ok == Some(false) {
                    workshop.error = Some(
                        "Agent requested done, but motion_validation failed in the latest smoke_check_v1. Continue and repair motion (llm_review_delta_v1), or re-run smoke_check_v1 until validation passes."
                            .to_string(),
                    );
                    workshop.status = "Continuing Gen3D build…(motion repair required)".into();
                    append_agent_trace_event_v1(
                        job.run_dir.as_deref(),
                        &AgentTraceEventV1::Info {
                            message: "agent_done_ignored (motion_validation failed)".to_string(),
                        },
                    );
                    append_gen3d_run_log(
                        job.pass_dir.as_deref(),
                        "agent_done_ignored (motion_validation failed); continuing",
                    );
                    warn!("Gen3D agent requested done while motion_validation failed; continuing");
                    job.agent.step_action_idx = job.agent.step_actions.len();
                    continue;
                }

                let movable = draft
                    .root_def()
                    .and_then(|def| def.mobility.as_ref())
                    .is_some();
                if movable {
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

                    let has_move = job.planned_components.iter().any(|c| {
                        c.attach_to.as_ref().is_some_and(|att| {
                            att.animations
                                .iter()
                                .any(|slot| slot.channel.as_ref() == "move")
                        })
                    });

                    if runtime_candidate.is_none() && !has_move {
                        workshop.error = Some(format!(
                            "Agent requested done, but this is a movable unit with no runtime motion rig candidate and no authored `move` animation slots.\n\
Continue and call `{TOOL_ID_LLM_GENERATE_MOTION_AUTHORING}` to bake motion clips onto attachment edges, then run `{TOOL_ID_SMOKE_CHECK}`."
                        ));
                        workshop.status =
                            "Continuing Gen3D build…(motion authoring required)".into();
                        append_agent_trace_event_v1(
                            job.run_dir.as_deref(),
                            &AgentTraceEventV1::Info {
                                message: "agent_done_ignored (missing move animation)".to_string(),
                            },
                        );
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            "agent_done_ignored (missing move animation); continuing",
                        );
                        warn!("Gen3D agent requested done without any motion path; continuing");
                        job.agent.step_action_idx = job.agent.step_actions.len();
                        continue;
                    }
                }
                let status = if reason.trim().is_empty() {
                    "Build finished.".to_string()
                } else {
                    format!("Build finished.\nReason: {}", reason.trim())
                };
                if maybe_start_pass_snapshot_capture(
                    config,
                    commands,
                    images,
                    workshop,
                    job,
                    draft,
                    super::Gen3dAgentAfterPassSnapshot::FinishRun {
                        workshop_status: status.clone(),
                        run_log: format!("agent_done reason={:?}", reason.trim()),
                        info_log: format!("Gen3D agent: done. reason={:?}", reason.trim()),
                    },
                ) {
                    workshop.status = status;
                    return;
                }

                workshop.status = status;
                append_gen3d_run_log(
                    job.pass_dir.as_deref(),
                    format!("agent_done reason={:?}", reason.trim()),
                );
                info!("Gen3D agent: done. reason={:?}", reason.trim());

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                return;
            }
            Gen3dAgentActionJsonV1::ToolCall {
                call_id,
                tool_id,
                args,
            } => {
                let call = Gen3dToolCallJsonV1 {
                    call_id,
                    tool_id,
                    args,
                };
                job.metrics
                    .note_tool_call_started(call.call_id.as_str(), call.tool_id.as_str());
                append_agent_trace_event_v1(
                    job.run_dir.as_deref(),
                    &AgentTraceEventV1::ToolCall {
                        call_id: call.call_id.clone(),
                        tool_id: call.tool_id.clone(),
                        args: call.args.clone(),
                    },
                );
                append_gen3d_jsonl_artifact(
                    job.pass_dir.as_deref(),
                    "tool_calls.jsonl",
                    &serde_json::json!({
                        "call_id": call.call_id.clone(),
                        "tool_id": call.tool_id.clone(),
                        "args": call.args.clone(),
                    }),
                );
                let call_id_for_log = call.call_id.clone();
                let tool_id_for_log = call.tool_id.clone();
                append_gen3d_run_log(
                    job.pass_dir.as_deref(),
                    format!(
                        "tool_call_start call_id={} tool_id={} args={}",
                        call.call_id,
                        call.tool_id,
                        truncate_json_for_log(&call.args, 600)
                    ),
                );
                debug!(
                    "Gen3D tool call start: call_id={} tool_id={} args={}",
                    call.call_id,
                    call.tool_id,
                    truncate_json_for_log(&call.args, 600)
                );

                match execute_tool_call(
                    config,
                    time,
                    commands,
                    images,
                    workshop,
                    feedback_history,
                    job,
                    draft,
                    preview,
                    preview_model,
                    call,
                ) {
                    ToolCallOutcome::Immediate(result) => {
                        job.metrics.note_tool_result(&result);
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "tool_call_result call_id={} tool_id={} ok={} {}",
                                result.call_id,
                                result.tool_id,
                                result.ok,
                                if result.ok {
                                    format!(
                                        "result={}",
                                        result
                                            .result
                                            .as_ref()
                                            .map(|v| truncate_json_for_log(v, 900))
                                            .unwrap_or_else(|| "<none>".into())
                                    )
                                } else {
                                    format!("error={}", result.error.as_deref().unwrap_or("<none>"))
                                }
                            ),
                        );
                        if result.ok {
                            debug!(
                                "Gen3D tool call ok: call_id={} tool_id={} result={}",
                                result.call_id,
                                result.tool_id,
                                result
                                    .result
                                    .as_ref()
                                    .map(|v| truncate_json_for_log(v, 900))
                                    .unwrap_or_else(|| "<none>".into())
                            );
                        } else {
                            warn!(
                                "Gen3D tool call failed: call_id={} tool_id={} error={}",
                                result.call_id,
                                result.tool_id,
                                result.error.as_deref().unwrap_or("<none>")
                            );
                        }
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
                        if job
                            .agent
                            .step_tool_results
                            .last()
                            .map(|r| !r.ok)
                            .unwrap_or(false)
                        {
                            // End the step early on tool failures so the agent can adapt.
                            // Continuing to execute the remaining tool calls tends to cascade
                            // errors because later actions usually depend on earlier outputs.
                            job.agent.step_action_idx = job.agent.step_actions.len();
                            return;
                        }
                        job.agent.step_action_idx += 1;
                        executed += 1;
                        continue;
                    }
                    ToolCallOutcome::StartedAsync => {
                        // Tool execution will resume once async work completes.
                        append_gen3d_run_log(
                            job.pass_dir.as_deref(),
                            format!(
                                "tool_call_async_started call_id={} tool_id={}",
                                call_id_for_log, tool_id_for_log
                            ),
                        );
                        debug!(
                            "Gen3D tool call started async: call_id={} tool_id={}",
                            call_id_for_log, tool_id_for_log
                        );
                        job.agent.step_action_idx += 1;
                        return;
                    }
                }
            }
        }
    }
}

pub(super) fn maybe_start_pass_snapshot_capture(
    config: &AppConfig,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    after: super::Gen3dAgentAfterPassSnapshot,
) -> bool {
    if !config.gen3d_save_pass_screenshots {
        return false;
    }
    if draft.total_non_projectile_primitive_parts() == 0 {
        return false;
    }
    if job.agent.pending_pass_snapshot.is_some() {
        return false;
    }
    let Some(pass_dir) = job.pass_dir.clone() else {
        return false;
    };

    let views = [
        super::Gen3dReviewView::Front,
        super::Gen3dReviewView::LeftBack,
        super::Gen3dReviewView::RightBack,
        super::Gen3dReviewView::Top,
        super::Gen3dReviewView::Bottom,
    ];
    match super::start_gen3d_review_capture(
        commands,
        images,
        &pass_dir,
        draft,
        false,
        "pass",
        &views,
        super::super::GEN3D_PREVIEW_WIDTH_PX,
        super::super::GEN3D_PREVIEW_HEIGHT_PX,
    ) {
        Ok(state) => {
            job.agent.pending_pass_snapshot = Some(state);
            job.agent.pending_after_pass_snapshot = Some(after);
            job.phase = Gen3dAiPhase::AgentCapturingPassSnapshot;
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Saving pass screenshots… (0/5)");
            }
            true
        }
        Err(err) => {
            warn!(
                "Gen3D: failed to start pass snapshot capture in {}: {err}",
                pass_dir.display()
            );
            workshop.error = Some(format!("Gen3D: pass screenshot capture failed: {err}"));
            false
        }
    }
}

pub(super) fn poll_agent_pass_snapshot_capture(
    config: &AppConfig,
    commands: &mut Commands,
    _images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    _feedback_history: &mut Gen3dToolFeedbackHistory,
    job: &mut Gen3dAiJob,
) {
    let Some(state) = job.agent.pending_pass_snapshot.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending pass snapshot",
        );
        return;
    };
    let (done, expected) = state
        .progress
        .lock()
        .map(|g| (g.completed, g.expected))
        .unwrap_or((0, 1));
    if done < expected {
        if let Some(progress) = job.shared_progress.as_ref() {
            set_progress(
                progress,
                format!("Saving pass screenshots… ({done}/{expected})"),
            );
        }
        return;
    }

    let Some(state) = job.agent.pending_pass_snapshot.take() else {
        return;
    };
    for cam in state.cameras.iter().copied() {
        commands.entity(cam).try_despawn();
    }
    let paths = state.image_paths.clone();
    for path in &paths {
        if std::fs::metadata(path).is_err() {
            warn!(
                "Gen3D: pass snapshot missing output file: {}",
                path.display()
            );
        }
    }

    let Some(after) = job.agent.pending_after_pass_snapshot.take() else {
        warn!("Gen3D: missing after-pass-snapshot continuation; resuming build.");
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        return;
    };

    match after {
        super::Gen3dAgentAfterPassSnapshot::AdvancePassAndRequestStep => {
            if let Err(err) = gen3d_advance_pass(job) {
                fail_job(workshop, job, err);
                return;
            }
            let Some(pass_dir) = job.pass_dir.clone() else {
                fail_job(workshop, job, "Internal error: missing pass dir");
                return;
            };
            append_gen3d_run_log(Some(&pass_dir), "agent_step_complete; requesting next step");
            debug!("Gen3D agent: step complete; requesting next step");
            job.phase = Gen3dAiPhase::AgentWaitingStep;
            job.agent.step_repair_attempt = 0;
            let _ = spawn_agent_step_request(config, workshop, job, pass_dir);
        }
        super::Gen3dAgentAfterPassSnapshot::FinishRun {
            workshop_status,
            run_log,
            info_log,
        } => {
            workshop.status = workshop_status;
            append_gen3d_run_log(job.pass_dir.as_deref(), run_log);
            info!("{info_log}");
            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            job.shared_progress = None;
            job.shared_result = None;
        }
    }
}

pub(super) enum ToolCallOutcome {
    Immediate(Gen3dToolResultJsonV1),
    StartedAsync,
}
