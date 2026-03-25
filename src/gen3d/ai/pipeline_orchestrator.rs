use bevy::log::debug;
use bevy::prelude::*;

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_APPLY_REUSE_GROUPS, TOOL_ID_GET_PLAN_TEMPLATE,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_DRAFT_OPS, TOOL_ID_LLM_GENERATE_MOTIONS,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_GENERATE_PLAN_OPS, TOOL_ID_LLM_REVIEW_DELTA,
    TOOL_ID_LLM_SELECT_EDIT_STRATEGY, TOOL_ID_QA, TOOL_ID_QUERY_COMPONENT_PARTS,
    TOOL_ID_RENDER_PREVIEW,
};
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dToolCallJsonV1, Gen3dToolResultJsonV1,
};
use crate::threaded_result::take_shared_result;
use crate::types::{ActionClock, AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::agent_render_capture::poll_agent_render_capture;
use super::agent_step::{
    poll_agent_descriptor_meta, poll_agent_pass_snapshot_capture, start_finish_run_sequence,
};
use super::agent_tool_dispatch::execute_tool_call;
use super::agent_tool_poll::poll_agent_tool;
use super::agent_utils::{compute_agent_state_hash, truncate_json_for_log};
use super::artifacts::{append_gen3d_jsonl_artifact, append_gen3d_run_log};
use super::status_steps;
use super::{
    fail_job, finish_job_best_effort, Gen3dAiJob, Gen3dAiPhase, Gen3dPendingFinishRun,
    Gen3dPipelineStage,
};

fn truncate_text_to_max_words_preserving_whitespace(
    text: &str,
    max_words: usize,
) -> (String, bool, usize) {
    let mut out = String::new();
    let mut in_word = false;
    let mut words = 0usize;

    for ch in text.chars() {
        let is_ws = ch.is_whitespace();
        if !is_ws && !in_word {
            if words >= max_words {
                let out = out.trim().to_string();
                let words_out = crate::gen3d::gen3d_count_whitespace_separated_words(&out);
                return (out, true, words_out);
            }
            words += 1;
            in_word = true;
        } else if is_ws {
            in_word = false;
        }
        out.push(ch);
    }

    let out = out.trim().to_string();
    let words_out = crate::gen3d::gen3d_count_whitespace_separated_words(&out);
    (out, false, words_out)
}

pub(super) fn poll_gen3d_pipeline(
    config: &AppConfig,
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
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
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    match job.phase {
        Gen3dAiPhase::AgentWaitingUserImageSummary => {
            poll_pipeline_user_image_summary(config, workshop, job);
        }
        Gen3dAiPhase::AgentWaitingPromptIntent => {
            poll_pipeline_prompt_intent(config, workshop, job);
        }
        Gen3dAiPhase::AgentWaitingTool => {
            poll_agent_tool(
                config,
                commands,
                images,
                workshop,
                feedback_history,
                job,
                draft,
                preview,
            );
        }
        Gen3dAiPhase::AgentCapturingRender => {
            poll_agent_render_capture(
                config,
                time,
                commands,
                images,
                workshop,
                job,
                draft,
                preview_model,
            );
        }
        Gen3dAiPhase::AgentCapturingPassSnapshot => {
            poll_agent_pass_snapshot_capture(
                config,
                commands,
                images,
                workshop,
                feedback_history,
                job,
            );
        }
        Gen3dAiPhase::AgentWaitingDescriptorMeta => {
            poll_agent_descriptor_meta(config, commands, images, workshop, job, draft);
        }
        Gen3dAiPhase::AgentExecutingActions => {
            poll_pipeline_tick(
                config,
                time,
                commands,
                images,
                review_cameras,
                workshop,
                feedback_history,
                job,
                draft,
                preview,
                preview_model,
            );
        }
        Gen3dAiPhase::Idle => {}
    }
}

fn stop_pipeline_best_effort(
    commands: &mut Commands,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    reason: String,
) {
    finish_job_best_effort(commands, review_cameras, workshop, job, reason);
}

fn poll_pipeline_user_image_summary(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) {
    // One-time user-image summarization request (pipeline bootstrap).
    if job.user_images.is_empty() || job.user_image_object_summary.is_some() {
        job.shared_result = None;
        job.shared_progress = None;
        job.phase = if job.prompt_intent.is_some() {
            Gen3dAiPhase::AgentExecutingActions
        } else {
            Gen3dAiPhase::AgentWaitingPromptIntent
        };
        return;
    }

    if job.shared_result.is_none() {
        let Some(step_dir) = job.step_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D step dir.");
            return;
        };
        if let Err(err) = super::bootstrap_requests::spawn_gen3d_user_image_summary_request(
            config, workshop, job, step_dir,
        ) {
            fail_job(workshop, job, err);
        }
        return;
    }

    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing Gen3D shared_result.",
        );
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };
    job.shared_result = None;
    job.shared_progress = None;

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }

            // Keep parsing logic identical to agent mode.
            let normalized = resp.text.replace("\r\n", "\n").replace('\r', "\n");
            let (text, truncated, word_count) = truncate_text_to_max_words_preserving_whitespace(
                normalized.trim(),
                crate::gen3d::GEN3D_IMAGE_OBJECT_SUMMARY_MAX_WORDS,
            );
            if text.trim().is_empty() {
                fail_job(
                    workshop,
                    job,
                    "Reference image summary was empty. Add a text prompt or try again.",
                );
                return;
            }

            job.user_image_object_summary = Some(super::job::Gen3dUserImageObjectSummary {
                text: text.clone(),
                truncated,
                word_count,
            });

            if let Some(run_dir) = job.run_dir.clone() {
                let attempt_dir = run_dir.join(format!("attempt_{}", job.attempt));
                super::artifacts::write_gen3d_text_artifact(
                    Some(&attempt_dir),
                    "inputs/image_object_summary.txt",
                    &text,
                );
                super::artifacts::write_gen3d_json_artifact(
                    Some(&attempt_dir),
                    "inputs/image_object_summary.json",
                    &serde_json::json!({
                        "version": 1,
                        "images_count": job.user_images.len(),
                        "word_count": word_count,
                        "truncated": truncated,
                    }),
                );
            }

            workshop.status = "Reference images summarized.\nAnalyzing prompt…".to_string();
            job.phase = Gen3dAiPhase::AgentWaitingPromptIntent;
        }
        Err(err) => {
            fail_job(
                workshop,
                job,
                format!(
                    "Reference image pre-processing failed: {err}\nTip: try again or use a text prompt without images."
                ),
            );
        }
    }
}

fn poll_pipeline_prompt_intent(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) {
    if job.prompt_intent.is_some() {
        job.shared_result = None;
        job.shared_progress = None;
        job.phase = Gen3dAiPhase::AgentExecutingActions;
        return;
    }

    if job.shared_result.is_none() {
        let Some(step_dir) = job.step_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D step dir.");
            return;
        };
        if let Err(err) = super::bootstrap_requests::spawn_gen3d_prompt_intent_request(
            config, workshop, job, step_dir,
        ) {
            fail_job(workshop, job, err);
        }
        return;
    }

    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing Gen3D shared_result.",
        );
        return;
    };
    let Some(result) = take_shared_result(shared) else {
        return;
    };
    job.shared_result = None;
    job.shared_progress = None;

    match result {
        Ok(resp) => {
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                job.add_tokens(tokens);
            }

            let parsed = match super::parse::parse_ai_prompt_intent_from_text(&resp.text) {
                Ok(v) => v,
                Err(err) => {
                    fail_job(
                        workshop,
                        job,
                        format!("Prompt intent classification failed: {err}"),
                    );
                    return;
                }
            };
            let requires_attack = parsed.requires_attack;
            job.prompt_intent = Some(parsed.clone());

            if let Some(run_dir) = job.run_dir.clone() {
                let attempt_dir = run_dir.join(format!("attempt_{}", job.attempt));
                super::artifacts::write_gen3d_json_artifact(
                    Some(&attempt_dir),
                    "inputs/prompt_intent.json",
                    &serde_json::to_value(&parsed).unwrap_or_else(
                        |_| serde_json::json!({"version": 1, "requires_attack": requires_attack}),
                    ),
                );
            }

            workshop.status = "Prompt analyzed.\nPipeline: planning…".to_string();
            job.phase = Gen3dAiPhase::AgentExecutingActions;
        }
        Err(err) => {
            fail_job(
                workshop,
                job,
                format!("Prompt intent classification failed: {err}"),
            );
        }
    }
}

fn is_edit_session(job: &Gen3dAiJob) -> bool {
    job.edit_base_prefab_id.is_some() && !job.user_prompt_raw.trim().is_empty()
}

fn appearance_review_enabled(job: &Gen3dAiJob) -> bool {
    let llm_available = job
        .ai
        .as_ref()
        .map(|ai| !ai.base_url().starts_with("mock://gen3d"))
        .unwrap_or(true);
    llm_available && job.review_appearance
}

fn pipeline_missing_move_slot_coverage(job: &Gen3dAiJob, draft: &Gen3dDraft) -> bool {
    let movable = draft
        .root_def()
        .and_then(|def| def.mobility.as_ref())
        .is_some();
    if !movable {
        return false;
    }

    let mut has_move = false;
    let mut has_action = false;
    for comp in job.planned_components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        for slot in att.animations.iter() {
            match slot.channel.as_ref() {
                "move" => has_move = true,
                "action" => has_action = true,
                _ => {}
            }
        }
        if has_move && has_action {
            break;
        }
    }

    !has_move || !has_action
}

fn pipeline_last_motion_tool_failure_lines(job: &Gen3dAiJob) -> Vec<String> {
    for result in job.agent.step_tool_results.iter().rev() {
        if result.tool_id != TOOL_ID_LLM_GENERATE_MOTIONS {
            continue;
        }

        if let Some(err) = result
            .error
            .as_deref()
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        {
            return vec![err.replace('\n', " ")];
        }

        let Some(json) = result.result.as_ref() else {
            return Vec::new();
        };
        let Some(items) = json.get("failed").and_then(|v| v.as_array()) else {
            return Vec::new();
        };
        let mut out: Vec<String> = Vec::new();
        for item in items.iter().take(8) {
            let channel = item.get("channel").and_then(|v| v.as_str()).unwrap_or("");
            let error = item.get("error").and_then(|v| v.as_str()).unwrap_or("");
            let channel = channel.trim();
            let error = error.trim();
            if error.is_empty() {
                continue;
            }
            let mut line = String::new();
            if !channel.is_empty() {
                line.push_str(channel);
                line.push_str(": ");
            }
            line.push_str(error);
            out.push(line.replace('\n', " "));
        }
        return out;
    }
    Vec::new()
}

fn append_motion_authoring_retry_feedback(job: &Gen3dAiJob, feedback: &mut String) {
    if !feedback.trim().is_empty() && !feedback.ends_with('\n') {
        feedback.push('\n');
    }
    feedback.push_str(
        "Schema reminder: top-level `version` MUST be the number 1 (not a string).\n",
    );

    let failures = pipeline_last_motion_tool_failure_lines(job);
    if failures.is_empty() {
        return;
    }
    feedback.push_str("Previous motion authoring failures:\n");
    for line in failures {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        feedback.push_str("- ");
        feedback.push_str(line);
        feedback.push('\n');
    }
}

fn run_complete_enough_for_pipeline_finish(job: &Gen3dAiJob, draft: &Gen3dDraft) -> bool {
    if draft.total_non_projectile_primitive_parts() == 0 {
        return false;
    }
    if job
        .planned_components
        .iter()
        .any(|c| c.actual_size.is_none())
    {
        return false;
    }
    if !job.agent.pending_regen_component_indices.is_empty() {
        return false;
    }
    if job.agent.last_validate_ok != Some(true) {
        return false;
    }
    if job.agent.last_smoke_ok != Some(true) {
        return false;
    }
    if job.agent.last_motion_ok == Some(false) {
        return false;
    }
    if pipeline_missing_move_slot_coverage(job, draft) {
        return false;
    }

    if appearance_review_enabled(job) {
        if !job.agent.ever_rendered
            || !job.agent.ever_reviewed
            || job.agent.rendered_since_last_review
        {
            return false;
        }
    }

    true
}

fn pipeline_make_call_id(job: &mut Gen3dAiJob, tool_id: &str) -> String {
    let seq = job.pipeline.tool_seq;
    job.pipeline.tool_seq = job.pipeline.tool_seq.saturating_add(1);
    let tool_seg = tool_id
        .trim()
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect::<String>();
    format!("pipe_{}_s{}_a{}", tool_seg, job.step, seq)
}

fn pipeline_record_tool_call_start(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    call: &Gen3dToolCallJsonV1,
) {
    job.metrics
        .note_tool_call_started(call.call_id.as_str(), call.tool_id.as_str());
    status_steps::log_tool_call_started(workshop, call);
    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::ToolCall {
            call_id: call.call_id.clone(),
            tool_id: call.tool_id.clone(),
            args: call.args.clone(),
        },
    );
    append_gen3d_jsonl_artifact(
        job.step_dir.as_deref(),
        "tool_calls.jsonl",
        &serde_json::json!({
            "call_id": call.call_id.clone(),
            "tool_id": call.tool_id.clone(),
            "args": call.args.clone(),
        }),
    );
    append_gen3d_run_log(
        job.step_dir.as_deref(),
        format!(
            "pipeline_tool_call_start call_id={} tool_id={} args={}",
            call.call_id,
            call.tool_id,
            truncate_json_for_log(&call.args, 600)
        ),
    );
    job.append_info_event_best_effort(
        super::info_store::InfoEventKindV1::ToolCallStart,
        Some(call.tool_id.clone()),
        Some(call.call_id.clone()),
        format!("Tool call start: {}", call.tool_id),
        serde_json::json!({ "args": call.args.clone() }),
    );
}

fn pipeline_record_tool_result(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    result: &Gen3dToolResultJsonV1,
) {
    job.metrics.note_tool_result(result);
    status_steps::log_tool_call_finished(workshop, job, draft, result);
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
        job.step_dir.as_deref(),
        "tool_results.jsonl",
        &serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
    );
    append_gen3d_run_log(
        job.step_dir.as_deref(),
        format!(
            "pipeline_tool_call_result call_id={} tool_id={} ok={} {}",
            result.call_id,
            result.tool_id,
            result.ok,
            if result.ok {
                result
                    .result
                    .as_ref()
                    .map(|v| format!("result={}", truncate_json_for_log(v, 900)))
                    .unwrap_or_else(|| "result=<none>".into())
            } else {
                format!("error={}", result.error.as_deref().unwrap_or("<none>"))
            }
        ),
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
                super::orchestration::truncate_for_ui(first_line, 240)
            )
        }
    };
    job.append_info_event_best_effort(
        super::info_store::InfoEventKindV1::ToolCallResult,
        Some(result.tool_id.clone()),
        Some(result.call_id.clone()),
        message,
        serde_json::to_value(result).unwrap_or(serde_json::Value::Null),
    );

    job.agent.step_tool_results.push(result.clone());
    if job.agent.step_tool_results.len() > 32 {
        let drain = job.agent.step_tool_results.len() - 32;
        job.agent.step_tool_results.drain(0..drain);
    }
}

fn start_pipeline_tool_call(
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
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
    tool_id: &str,
    args: serde_json::Value,
) -> Option<Gen3dToolResultJsonV1> {
    if let Err(err) = super::orchestration::gen3d_advance_step(job) {
        let call_id = pipeline_make_call_id(job, tool_id);
        fail_job(workshop, job, err.clone());
        return Some(Gen3dToolResultJsonV1::err(
            call_id,
            tool_id.to_string(),
            format!("Failed to create Gen3D step dir: {err}"),
        ));
    }

    let call = Gen3dToolCallJsonV1 {
        call_id: pipeline_make_call_id(job, tool_id),
        tool_id: tool_id.to_string(),
        args,
    };
    pipeline_record_tool_call_start(workshop, job, &call);

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
        super::agent_step::ToolCallOutcome::Immediate(result) => Some(result),
        super::agent_step::ToolCallOutcome::StartedAsync => None,
    }
}

fn poll_pipeline_tick(
    config: &AppConfig,
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
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
            &mut ActionClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    // Stage bootstrap.
    if matches!(job.pipeline.stage, Gen3dPipelineStage::Start) {
        if is_edit_session(job) && job.preserve_existing_components_mode {
            job.pipeline.stage = Gen3dPipelineStage::EditSelectStrategy;
        } else {
            if job.edit_base_prefab_id.is_some() {
                // Seeded session with preserve mode disabled: treat as a full rebuild by default.
                job.pipeline.force_replan = true;
            }
            job.pipeline.stage = Gen3dPipelineStage::CreatePlan;
        }
    }

    // Handle the latest unprocessed tool result (stage transitions + caches).
    if let Some(last) = job.agent.step_tool_results.last() {
        if job.pipeline.last_processed_tool_call_id.as_deref() != Some(last.call_id.as_str()) {
            job.pipeline.last_processed_tool_call_id = Some(last.call_id.clone());

            let state_hash = compute_agent_state_hash(job, draft);
            let changed = job
                .pipeline
                .no_progress_state_hash
                .as_deref()
                .map(|h| h != state_hash.as_str())
                .unwrap_or(true);
            if changed {
                job.pipeline.no_progress_state_hash = Some(state_hash.clone());
                job.pipeline.no_progress_tries = 0;
            } else {
                // Count no-progress tries only for mutating tools. Pure inspection tools should not
                // trigger fallback: QA/render/template/parts snapshots can be repeated safely.
                let is_inspection_tool = matches!(
                    last.tool_id.as_str(),
                    TOOL_ID_QA
                        | TOOL_ID_GET_PLAN_TEMPLATE
                        | TOOL_ID_QUERY_COMPONENT_PARTS
                        | TOOL_ID_RENDER_PREVIEW
                        | TOOL_ID_LLM_SELECT_EDIT_STRATEGY
                        | TOOL_ID_LLM_GENERATE_DRAFT_OPS
                );
                if !is_inspection_tool {
                    job.pipeline.no_progress_tries =
                        job.pipeline.no_progress_tries.saturating_add(1);
                }
            }

            if !last.ok {
                let stage = format!("{:?}", job.pipeline.stage);
                let err = last.error.as_deref().unwrap_or("").trim();
                let first = err.lines().next().unwrap_or("").trim();
                let hint = if first.is_empty() {
                    "Retry Build. If this repeats, open a bug with the run cache directory.".into()
                } else {
                    format!(
                        "Error: {}. Retry Build. If this repeats, open a bug with the run cache directory.",
                        super::orchestration::truncate_for_ui(first, 420)
                    )
                };
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    format!(
                        "Pipeline stopped at stage={stage} due to tool failure: tool_id={} call_id={}. {hint}",
                        last.tool_id, last.call_id
                    ),
                );
                return;
            }

            if job.pipeline.no_progress_tries > 0
                && config.gen3d_no_progress_tries_max > 0
                && job.pipeline.no_progress_tries >= config.gen3d_no_progress_tries_max
            {
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    format!(
                        "Pipeline stopped: no-progress guard triggered (tool_id={} tries={}/{}). Retry Build; if this repeats, simplify the edit request.",
                        last.tool_id, job.pipeline.no_progress_tries, config.gen3d_no_progress_tries_max
                    ),
                );
                return;
            }

            if last.tool_id == TOOL_ID_GET_PLAN_TEMPLATE {
                if let Some(kv) = last
                    .result
                    .as_ref()
                    .and_then(|v| v.get("plan_template_kv"))
                    .cloned()
                {
                    job.pipeline.plan_template_kv = Some(kv);
                }
            }
            if last.tool_id == TOOL_ID_LLM_SELECT_EDIT_STRATEGY {
                if let Some(value) = last.result.clone() {
                    match serde_json::from_value::<super::schema::AiEditStrategyJsonV1>(value) {
                        Ok(strategy) => {
                            job.pipeline.edit_scope_components =
                                strategy.snapshot_components.clone();
                            job.pipeline.edit_strategy = Some(strategy);
                        }
                        Err(err) => {
                            debug!("Gen3D pipeline: invalid edit strategy tool result: {err}");
                        }
                    }
                }
            }
            if last.tool_id == TOOL_ID_LLM_GENERATE_PLAN
                || last.tool_id == TOOL_ID_LLM_GENERATE_PLAN_OPS
            {
                // Plan changes invalidate DraftOps caches and completion markers.
                job.pipeline.edit_draft_ops_done = false;
                job.pipeline.draft_ops_suggested = None;
                job.pipeline.draft_ops_last_rejected = None;
                job.pipeline.query_parts_next_idx = 0;
                // Any plan change invalidates motion authoring attempt budgeting for this run.
                job.pipeline.motion_authoring_attempts = 0;
            }
            if last.tool_id == TOOL_ID_LLM_GENERATE_DRAFT_OPS {
                if let Some(result) = last.result.clone() {
                    job.pipeline.draft_ops_suggested = Some(result);
                }
            }
            if last.tool_id == TOOL_ID_RENDER_PREVIEW {
                if let Some(blob_ids) = last
                    .result
                    .as_ref()
                    .and_then(|v| v.get("blob_ids"))
                    .and_then(|v| v.as_array())
                {
                    job.pipeline.pending_preview_blob_ids = blob_ids
                        .iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            if last.tool_id == TOOL_ID_LLM_REVIEW_DELTA {
                if let Some(replan_reason) = last
                    .result
                    .as_ref()
                    .and_then(|v| v.get("replan_reason"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    debug!("Gen3D pipeline: review-delta requested replan: {replan_reason}");
                    job.pipeline.plan_template_kv = None;
                    job.pipeline.edit_draft_ops_done = false;
                    job.pipeline.draft_ops_suggested = None;
                    job.pipeline.draft_ops_last_rejected = None;
                    job.pipeline.query_parts_next_idx = 0;
                    if job.preserve_existing_components_mode {
                        job.pipeline.stage = Gen3dPipelineStage::PreserveReplanTemplate;
                    } else {
                        job.pipeline.force_replan = true;
                        job.pipeline.stage = Gen3dPipelineStage::CreatePlan;
                    }
                }
            }
        }
    }

    // Terminal condition: finish when complete enough and pipeline reached finish stage, or when
    // the stage machine reaches Finish with a good draft.
    if matches!(job.pipeline.stage, Gen3dPipelineStage::Finish)
        || (run_complete_enough_for_pipeline_finish(job, draft)
            && matches!(
                job.pipeline.stage,
                Gen3dPipelineStage::Qa | Gen3dPipelineStage::ReviewDelta
            ))
    {
        let mut status =
            "Build finished. Orbit/zoom the preview. Click Build/Edit to start a new run."
                .to_string();
        if appearance_review_enabled(job) && job.agent.last_qa_warnings_count.unwrap_or(0) > 0 {
            status.push_str("\n(See Status for QA warnings.)");
        }

        start_finish_run_sequence(
            config,
            commands,
            images,
            workshop,
            job,
            draft,
            Gen3dPendingFinishRun {
                workshop_status: status.clone(),
                run_log: "pipeline_finish".into(),
                info_log: "Gen3D pipeline: finish".into(),
            },
        );
        return;
    }

    // Select and start exactly one tool call per tick.
    let edit_session = is_edit_session(job);
    match job.pipeline.stage {
        Gen3dPipelineStage::CreatePlan => {
            if job.pipeline.force_replan
                || job.plan_hash.trim().is_empty()
                || job.planned_components.is_empty()
            {
                job.pipeline.force_replan = false;
                workshop.status = "Pipeline: planning…".into();
                job.pipeline.plan_authoring_attempts =
                    job.pipeline.plan_authoring_attempts.saturating_add(1);
                let mut args = serde_json::json!({ "prompt": job.user_prompt_raw });
                if let Some(feedback) = job.pipeline.pending_plan_qa_feedback.take() {
                    args.as_object_mut().unwrap().insert(
                        "qa_feedback".into(),
                        serde_json::Value::String(feedback),
                    );
                }
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_LLM_GENERATE_PLAN,
                    args,
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                return;
            }
            job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
        }
        Gen3dPipelineStage::EditSelectStrategy => {
            // Ensure we have an accepted plan to edit.
            if job.planned_components.is_empty() || job.plan_hash.trim().is_empty() {
                let _ = config;
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    "Pipeline stopped: missing accepted plan for seeded edit strategy selection (expected existing plan_hash + planned_components). Retry Build.".into(),
                );
                return;
            }

            if job.pipeline.edit_strategy.is_none() {
                workshop.status = "Pipeline: selecting edit strategy…".into();
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_LLM_SELECT_EDIT_STRATEGY,
                    serde_json::json!({ "prompt": job.user_prompt_raw }),
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                return;
            }

            let strategy = job
                .pipeline
                .edit_strategy
                .as_ref()
                .map(|s| s.strategy)
                .unwrap_or(super::schema::AiEditStrategyKindJsonV1::Unknown);
            // Keep scope synced even if tool-result caching is disabled in some call sites.
            job.pipeline.edit_scope_components = job
                .pipeline
                .edit_strategy
                .as_ref()
                .map(|s| s.snapshot_components.clone())
                .unwrap_or_default();

            match strategy {
                super::schema::AiEditStrategyKindJsonV1::DraftOpsOnly => {
                    job.pipeline.edit_draft_ops_done = false;
                    job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
                }
                super::schema::AiEditStrategyKindJsonV1::PlanOpsThenDraftOps => {
                    job.pipeline.edit_draft_ops_done = false;
                    job.pipeline.stage = Gen3dPipelineStage::EditPlanTemplate;
                }
                super::schema::AiEditStrategyKindJsonV1::PlanOpsOnly => {
                    job.pipeline.edit_draft_ops_done = true;
                    job.pipeline.edit_scope_components.clear();
                    job.pipeline.stage = Gen3dPipelineStage::EditPlanTemplate;
                }
                super::schema::AiEditStrategyKindJsonV1::Rebuild => {
                    job.preserve_existing_components_mode = false;
                    job.pipeline.edit_draft_ops_done = true;
                    job.pipeline.edit_scope_components.clear();
                    job.pipeline.force_replan = true;
                    job.pipeline.stage = Gen3dPipelineStage::CreatePlan;
                }
                super::schema::AiEditStrategyKindJsonV1::Unknown => {
                    // Safe default: preserve-mode plan ops + DraftOps.
                    job.pipeline.edit_draft_ops_done = false;
                    job.pipeline.stage = Gen3dPipelineStage::EditPlanTemplate;
                }
            }
            return;
        }
        Gen3dPipelineStage::PreserveReplanTemplate | Gen3dPipelineStage::EditPlanTemplate => {
            // Ensure we have an accepted plan to template.
            if job.planned_components.is_empty() || job.plan_hash.trim().is_empty() {
                let _ = config;
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    "Pipeline stopped: missing accepted plan for preserve-mode template (expected existing plan_hash + planned_components). Retry Build.".into(),
                );
                return;
            }
            if job.pipeline.plan_template_kv.is_none() {
                workshop.status = "Pipeline: preparing plan template…".into();
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_GET_PLAN_TEMPLATE,
                    serde_json::json!({ "version": 2, "mode": "auto" }),
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                return;
            }
            job.pipeline.stage = if matches!(
                job.pipeline.stage,
                Gen3dPipelineStage::PreserveReplanTemplate
            ) {
                Gen3dPipelineStage::PreserveReplanPlan
            } else {
                Gen3dPipelineStage::EditPlanOps
            };
        }
        Gen3dPipelineStage::PreserveReplanPlan => {
            let Some(kv) = job.pipeline.plan_template_kv.clone() else {
                job.pipeline.stage = Gen3dPipelineStage::PreserveReplanTemplate;
                return;
            };
            workshop.status = "Pipeline: replanning…".into();
            job.pipeline.plan_authoring_attempts = job.pipeline.plan_authoring_attempts.saturating_add(1);
            let mut args = serde_json::json!({
                "prompt": job.user_prompt_raw,
                "plan_template_kv": kv,
                "constraints": { "preserve_existing_components": true, "preserve_edit_policy": "allow_offsets" }
            });
            if let Some(feedback) = job.pipeline.pending_plan_qa_feedback.take() {
                args.as_object_mut().unwrap().insert(
                    "qa_feedback".into(),
                    serde_json::Value::String(feedback),
                );
            }
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_LLM_GENERATE_PLAN,
                args,
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
            }
            job.pipeline.plan_template_kv = None;
            job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
            return;
        }
        Gen3dPipelineStage::EditPlanOps => {
            let Some(kv) = job.pipeline.plan_template_kv.clone() else {
                job.pipeline.stage = Gen3dPipelineStage::EditPlanTemplate;
                return;
            };
            workshop.status = "Pipeline: plan ops…".into();
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_LLM_GENERATE_PLAN_OPS,
                serde_json::json!({
                    "prompt": job.user_prompt_raw,
                    "plan_template_kv": kv,
                    "constraints": {
                        "preserve_existing_components": true,
                        "preserve_edit_policy": "allow_offsets"
                    },
                    "max_ops": 32
                }),
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
            }
            job.pipeline.edit_plan_ops_done = true;
            job.pipeline.plan_template_kv = None;
            job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
            return;
        }
        Gen3dPipelineStage::EnsureComponents => {
            let regen_indices = job.agent.pending_regen_component_indices.clone();
            let missing_any = job
                .planned_components
                .iter()
                .any(|c| c.actual_size.is_none());

            if !regen_indices.is_empty() {
                workshop.status = "Pipeline: regenerating components…".into();
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_LLM_GENERATE_COMPONENTS,
                    serde_json::json!({ "component_indices": regen_indices, "force": true }),
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                return;
            }

            if missing_any {
                let mut has_generated_reuse_source = false;
                let mut has_missing_reuse_target = false;
                for group in job.reuse_groups.iter() {
                    if !has_generated_reuse_source {
                        has_generated_reuse_source = job
                            .planned_components
                            .get(group.source_root_idx)
                            .is_some_and(|c| c.actual_size.is_some());
                    }
                    if !has_missing_reuse_target {
                        for &target_idx in group.target_root_indices.iter() {
                            if job
                                .planned_components
                                .get(target_idx)
                                .is_some_and(|c| c.actual_size.is_none())
                            {
                                has_missing_reuse_target = true;
                                break;
                            }
                        }
                    }
                    if has_generated_reuse_source && has_missing_reuse_target {
                        break;
                    }
                }

                if has_generated_reuse_source && has_missing_reuse_target {
                    workshop.status = "Pipeline: applying reuse groups…".into();
                    if let Some(result) = start_pipeline_tool_call(
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
                        TOOL_ID_APPLY_REUSE_GROUPS,
                        serde_json::json!({ "version": 1 }),
                    ) {
                        pipeline_record_tool_result(workshop, job, &*draft, &result);
                    }
                    return;
                }

                job.pipeline.components_attempts =
                    job.pipeline.components_attempts.saturating_add(1);
                if job.pipeline.components_attempts > 6 {
                    let stage = format!("{:?}", job.pipeline.stage);
                    stop_pipeline_best_effort(
                        commands,
                        review_cameras,
                        workshop,
                        job,
                        format!(
                            "Pipeline stopped at stage={stage}: component generation stalled (attempts={} > 6). Retry Build; if this repeats, check the model/service health.",
                            job.pipeline.components_attempts
                        ),
                    );
                    return;
                }
                workshop.status = "Pipeline: generating components…".into();
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_LLM_GENERATE_COMPONENTS,
                    serde_json::json!({ "missing_only": true }),
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                return;
            }

            job.pipeline.components_attempts = 0;

            job.pipeline.stage = if edit_session && !job.pipeline.edit_draft_ops_done {
                job.pipeline.query_parts_next_idx = 0;
                job.pipeline.draft_ops_attempts = 0;
                job.pipeline.draft_ops_suggested = None;
                job.pipeline.draft_ops_last_rejected = None;
                Gen3dPipelineStage::EditQueryComponentParts
            } else {
                Gen3dPipelineStage::Qa
            };
        }
        Gen3dPipelineStage::EditQueryComponentParts => {
            if job.planned_components.is_empty() {
                let _ = config;
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    "Pipeline stopped: no components available for query_component_parts_v1. Retry Build.".into(),
                );
                return;
            }
            let mut scope_components: Vec<String> = if job.pipeline.edit_scope_components.is_empty()
            {
                job.planned_components
                    .iter()
                    .map(|c| c.name.clone())
                    .collect()
            } else {
                job.pipeline.edit_scope_components.clone()
            };
            // Filter unknown names (plan could have changed after replans).
            let existing: std::collections::HashSet<&str> = job
                .planned_components
                .iter()
                .map(|c| c.name.as_str())
                .collect();
            scope_components.retain(|name| existing.contains(name.as_str()));
            // De-dup while preserving order.
            let mut seen = std::collections::HashSet::<String>::new();
            scope_components.retain(|name| seen.insert(name.clone()));

            if scope_components.is_empty() {
                scope_components = job
                    .planned_components
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
            }

            if job.pipeline.query_parts_next_idx >= scope_components.len() {
                job.pipeline.stage = Gen3dPipelineStage::EditSuggestDraftOps;
                return;
            }
            let idx = job.pipeline.query_parts_next_idx;
            let name = scope_components[idx].clone();
            workshop.status = format!(
                "Pipeline: capturing part snapshots… ({}/{})",
                idx + 1,
                scope_components.len()
            );
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_QUERY_COMPONENT_PARTS,
                serde_json::json!({ "component": name, "max_parts": 128 }),
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
                if result.ok {
                    job.pipeline.query_parts_next_idx =
                        job.pipeline.query_parts_next_idx.saturating_add(1);
                }
            }
            return;
        }
        Gen3dPipelineStage::EditSuggestDraftOps => {
            let attempts = job.pipeline.draft_ops_attempts;
            if attempts >= 2 {
                let rejected = job
                    .pipeline
                    .draft_ops_last_rejected
                    .as_ref()
                    .map(|v| super::agent_utils::truncate_json_for_log(v, 600))
                    .unwrap_or_else(|| "<none>".into());
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    format!(
                        "Pipeline stopped: DraftOps suggestion attempts exhausted (attempts={attempts}, max=2). rejected_ops={rejected}. Retry Build; if this repeats, simplify the edit request."
                    ),
                );
                return;
            }

            let mut prompt = job.user_prompt_raw.clone();
            if let Some(rejected) = job.pipeline.draft_ops_last_rejected.as_ref() {
                let mut tail = serde_json::to_string(rejected).unwrap_or_default();
                if tail.len() > 2000 {
                    tail.truncate(2000);
                }
                prompt.push_str("\n\nPrevious DraftOps attempt rejected_ops: ");
                prompt.push_str(&tail);
            }

            workshop.status = "Pipeline: suggesting DraftOps…".into();
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_LLM_GENERATE_DRAFT_OPS,
                serde_json::json!({
                    "prompt": prompt,
                    "scope_components": job.pipeline.edit_scope_components.clone(),
                    "max_ops": 24,
                    "strategy": "conservative"
                }),
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
            }
            job.pipeline.draft_ops_attempts = job.pipeline.draft_ops_attempts.saturating_add(1);
            job.pipeline.stage = Gen3dPipelineStage::EditApplyDraftOps;
            return;
        }
        Gen3dPipelineStage::EditApplyDraftOps => {
            let Some(suggested) = job.pipeline.draft_ops_suggested.clone() else {
                job.pipeline.stage = Gen3dPipelineStage::EditSuggestDraftOps;
                return;
            };
            let ops = suggested
                .get("ops")
                .cloned()
                .unwrap_or(serde_json::Value::Array(Vec::new()));
            workshop.status = "Pipeline: applying DraftOps…".into();
            let args = serde_json::json!({
                "version": 1,
                "atomic": true,
                "if_assembly_rev": job.assembly_rev,
                "ops": ops,
            });
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_APPLY_DRAFT_OPS,
                args,
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
                if let Some(rejected) = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("rejected_ops"))
                    .cloned()
                {
                    let rejected_len = rejected.as_array().map(|a| a.len()).unwrap_or(0);
                    if rejected_len > 0 {
                        job.pipeline.draft_ops_last_rejected = Some(rejected);
                        job.pipeline.draft_ops_suggested = None;
                        job.pipeline.stage = Gen3dPipelineStage::EditSuggestDraftOps;
                        return;
                    }
                }
                if result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("ok"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
                {
                    job.pipeline.edit_draft_ops_done = true;
                }
            }
            job.pipeline.draft_ops_last_rejected = None;
            job.pipeline.draft_ops_suggested = None;
            job.pipeline.stage = Gen3dPipelineStage::Qa;
            return;
        }
        Gen3dPipelineStage::Qa => {
            job.pipeline.qa_attempts = job.pipeline.qa_attempts.saturating_add(1);
            if job.pipeline.qa_attempts > 12 {
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    format!(
                        "Pipeline stopped: QA loop exhausted (attempts={} > 12). Retry Build; if this repeats, capture the QA output and file a bug.",
                        job.pipeline.qa_attempts
                    ),
                );
                return;
            }

            workshop.status = "Pipeline: QA…".into();
            let qa_result = start_pipeline_tool_call(
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
                TOOL_ID_QA,
                serde_json::json!({}),
            );
            if let Some(result) = qa_result {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
                let ok = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("ok"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if ok {
                    // "QA complaints" are non-fatal quality hints. When present, the pipeline should
                    // spend the second chance on LLM improvement (plan first, then motion).
                    let complaints: Vec<&serde_json::Value> = result
                        .result
                        .as_ref()
                        .and_then(|v| v.get("complaints"))
                        .and_then(|v| v.as_array())
                        .map(|arr| arr.iter().collect())
                        .unwrap_or_default();
                    if !complaints.is_empty() {
                        let mut motion_msgs: Vec<String> = Vec::new();
                        for c in complaints.iter() {
                            let fix_step = c.get("fix_step").and_then(|v| v.as_str()).unwrap_or("");
                            let kind = c.get("kind").and_then(|v| v.as_str()).unwrap_or("").trim();
                            let msg = c
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim();
                            if fix_step != "motion" {
                                continue;
                            }
                            let mut line = String::new();
                            if !kind.is_empty() {
                                line.push_str(kind);
                                line.push_str(": ");
                            }
                            line.push_str(msg);
                            let line = line.trim().to_string();
                            if line.is_empty() {
                                continue;
                            }
                            motion_msgs.push(line);
                        }

                        if !motion_msgs.is_empty() && job.pipeline.motion_authoring_attempts < 2 {
                            let next_attempt = job
                                .pipeline
                                .motion_authoring_attempts
                                .saturating_add(1);
                            job.pipeline.motion_authoring_attempts = next_attempt;
                            // Force a QA re-run after motion authoring (do not reuse stale ok flags).
                            job.agent.last_validate_ok = None;
                            job.agent.last_smoke_ok = None;
                            job.agent.last_motion_ok = None;

                            workshop.status = "Pipeline: improving motion…".into();
                            let mut channels: Vec<&'static str> = vec!["move", "action"];
                            let attack_present =
                                draft.root_def().and_then(|r| r.attack.as_ref()).is_some();
                            if attack_present {
                                channels.push("attack_primary");
                            }

                            let mut feedback = String::new();
                            feedback.push_str(&format!("QA complaints (attempt {next_attempt}/2):\n"));
                            for item in motion_msgs.iter().take(12) {
                                feedback.push_str("- ");
                            feedback.push_str(item);
                            feedback.push('\n');
                        }
                            append_motion_authoring_retry_feedback(job, &mut feedback);
                            feedback.push_str(
                                "Fix these if applicable; if you believe they don't apply, keep the current motion.\n",
                            );

                            if let Some(result) = start_pipeline_tool_call(
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
                                TOOL_ID_LLM_GENERATE_MOTIONS,
                                serde_json::json!({ "channels": channels, "qa_feedback": feedback }),
                            ) {
                                pipeline_record_tool_result(workshop, job, &*draft, &result);
                            }
                            return;
                        }
                    }

                    if pipeline_missing_move_slot_coverage(job, draft)
                        && job.pipeline.motion_authoring_attempts < 2
                    {
                        let next_attempt = job.pipeline.motion_authoring_attempts.saturating_add(1);
                        job.pipeline.motion_authoring_attempts = next_attempt;
                        // Force a QA re-run after motion authoring (do not reuse stale ok flags).
                        job.agent.last_validate_ok = None;
                        job.agent.last_smoke_ok = None;
                        job.agent.last_motion_ok = None;

                        workshop.status = "Pipeline: authoring motion…".into();
                        let mut missing_channels: Vec<&'static str> = Vec::new();
                        let mut has_move = false;
                        let mut has_action = false;
                        for comp in job.planned_components.iter() {
                            let Some(att) = comp.attach_to.as_ref() else {
                                continue;
                            };
                            for slot in att.animations.iter() {
                                match slot.channel.as_ref() {
                                    "move" => has_move = true,
                                    "action" => has_action = true,
                                    _ => {}
                                }
                            }
                            if has_move && has_action {
                                break;
                            }
                        }
                        if !has_move {
                            missing_channels.push("move");
                        }
                        if !has_action {
                            missing_channels.push("action");
                        }
                        let mut feedback = String::new();
                        feedback.push_str(&format!(
                            "Missing required motion channels (attempt {next_attempt}/2): {}\n",
                            missing_channels.join(", ")
                        ));
                        append_motion_authoring_retry_feedback(job, &mut feedback);
                        if let Some(result) = start_pipeline_tool_call(
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
                            TOOL_ID_LLM_GENERATE_MOTIONS,
                            serde_json::json!({ "channels": missing_channels, "qa_feedback": feedback }),
                        ) {
                            pipeline_record_tool_result(workshop, job, &*draft, &result);
                        }
                        return;
                    }

                    if appearance_review_enabled(job) {
                        job.pipeline.stage = Gen3dPipelineStage::RenderPreview;
                        return;
                    }
                    if run_complete_enough_for_pipeline_finish(job, draft) {
                        job.pipeline.stage = Gen3dPipelineStage::Finish;
                        return;
                    }

                    stop_pipeline_best_effort(
                        commands,
                        review_cameras,
                        workshop,
                        job,
                        format!(
                            "Pipeline stopped: QA ok but run not complete (missing_move_slots={} motion_authoring_attempts={} appearance_review_enabled={}). Retry Build; if this repeats, file a bug with the run cache directory.",
                            pipeline_missing_move_slot_coverage(job, draft),
                            job.pipeline.motion_authoring_attempts,
                            appearance_review_enabled(job)
                        ),
                    );
                    return;
                }

                // Deterministic QA remediation: apply DraftOps fixits when present.
                let mut fixits: Vec<serde_json::Value> = Vec::new();
                if let Some(gaps) = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("capability_gaps"))
                    .and_then(|v| v.as_array())
                {
                    for gap in gaps {
                        let Some(items) = gap.get("fixits").and_then(|v| v.as_array()) else {
                            continue;
                        };
                        for fixit in items {
                            if fixit.get("tool_id").and_then(|v| v.as_str())
                                != Some(TOOL_ID_APPLY_DRAFT_OPS)
                            {
                                continue;
                            }
                            if let Some(args) = fixit.get("args").cloned() {
                                fixits.push(args);
                            }
                        }
                    }
                }
                if !fixits.is_empty() && job.pipeline.qa_fixits_applied < 6 {
                    let args = fixits[0].clone();
                    let mut args_obj = args.as_object().cloned().unwrap_or_default();
                    args_obj.insert("version".into(), serde_json::json!(1));
                    args_obj.insert("atomic".into(), serde_json::json!(true));
                    args_obj.insert(
                        "if_assembly_rev".into(),
                        serde_json::json!(job.assembly_rev),
                    );
                    workshop.status = "Pipeline: applying QA fixit…".into();
                    if let Some(result) = start_pipeline_tool_call(
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
                        TOOL_ID_APPLY_DRAFT_OPS,
                        serde_json::Value::Object(args_obj),
                    ) {
                        pipeline_record_tool_result(workshop, job, &*draft, &result);
                    }
                    job.pipeline.qa_fixits_applied =
                        job.pipeline.qa_fixits_applied.saturating_add(1);
                    return;
                }

                let motion_failed = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("smoke"))
                    .and_then(|v| v.get("motion_validation"))
                    .and_then(|v| v.get("ok"))
                    .and_then(|v| v.as_bool())
                    .map(|v| !v)
                    .unwrap_or(false);

                if motion_failed && job.pipeline.motion_authoring_attempts < 2 {
                    let next_attempt = job
                        .pipeline
                        .motion_authoring_attempts
                        .saturating_add(1);
                    job.pipeline.motion_authoring_attempts = next_attempt;
                    // Force a QA re-run after motion authoring (do not reuse stale ok flags).
                    job.agent.last_validate_ok = None;
                    job.agent.last_smoke_ok = None;
                    job.agent.last_motion_ok = None;
                    workshop.status = "Pipeline: authoring motion…".into();
                    let mut channels: Vec<&'static str> = vec!["move", "action"];
                    let attack_present = draft.root_def().and_then(|r| r.attack.as_ref()).is_some();
                    if attack_present {
                        channels.push("attack_primary");
                    }

                    let mut feedback = String::new();
                    feedback.push_str(&format!(
                        "QA motion validation issues (attempt {next_attempt}/2):\n"
                    ));
                    if let Some(issues) = result
                        .result
                        .as_ref()
                        .and_then(|v| v.get("smoke"))
                        .and_then(|v| v.get("motion_validation"))
                        .and_then(|v| v.get("issues"))
                        .and_then(|v| v.as_array())
                    {
                        for issue in issues.iter().take(12) {
                            let severity =
                                issue.get("severity").and_then(|v| v.as_str()).unwrap_or("");
                            let kind = issue.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                            let msg = issue.get("message").and_then(|v| v.as_str()).unwrap_or("");
                            let component = issue
                                .get("component_name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            let channel = issue.get("channel").and_then(|v| v.as_str()).unwrap_or("");

                            let mut line = String::new();
                            if !severity.trim().is_empty() {
                                line.push_str(severity.trim());
                                line.push_str(": ");
                            }
                            if !kind.trim().is_empty() {
                                line.push_str(kind.trim());
                                line.push_str(": ");
                            }
                            if !component.trim().is_empty() {
                                line.push_str("component=");
                                line.push_str(component.trim());
                                line.push(' ');
                            }
                            if !channel.trim().is_empty() {
                                line.push_str("channel=");
                                line.push_str(channel.trim());
                                line.push(' ');
                            }
                            if !msg.trim().is_empty() {
                                line.push_str(msg.trim());
                            }
                            let line = line.trim().to_string();
                            if !line.is_empty() {
                                feedback.push_str("- ");
                                feedback.push_str(&line);
                                feedback.push('\n');
                            }
                        }
                    }
                    append_motion_authoring_retry_feedback(job, &mut feedback);
                    feedback.push_str(
                        "Fix these if applicable; if you believe they don't apply, keep the current motion.\n",
                    );
                    if let Some(result) = start_pipeline_tool_call(
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
                        TOOL_ID_LLM_GENERATE_MOTIONS,
                        serde_json::json!({ "channels": channels, "qa_feedback": feedback }),
                    ) {
                        pipeline_record_tool_result(workshop, job, &*draft, &result);
                    }
                    return;
                }

                let rounds_max = config.gen3d_review_delta_rounds_max;
                let rounds_used = job.review_delta_rounds_used;
                let rounds_remaining = rounds_max.saturating_sub(rounds_used);
                if rounds_max == 0 || rounds_remaining == 0 {
                    stop_pipeline_best_effort(
                        commands,
                        review_cameras,
                        workshop,
                        job,
                        format!(
                            "Pipeline stopped: QA failed and review-delta budget exhausted (used={rounds_used} max={rounds_max}). Retry Build; if this repeats, simplify the request or adjust review-delta budget.",
                        ),
                    );
                    return;
                }
                job.pipeline.review_delta_attempts =
                    job.pipeline.review_delta_attempts.saturating_add(1);
                workshop.status = "Pipeline: review-delta remediation…".into();
                if let Some(result) = start_pipeline_tool_call(
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
                    TOOL_ID_LLM_REVIEW_DELTA,
                    serde_json::json!({}),
                ) {
                    pipeline_record_tool_result(workshop, job, &*draft, &result);
                }
                job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
                return;
            }
        }
        Gen3dPipelineStage::RenderPreview => {
            if !appearance_review_enabled(job) {
                job.pipeline.stage = Gen3dPipelineStage::Qa;
                return;
            }
            workshop.status = "Pipeline: rendering preview…".into();
            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_RENDER_PREVIEW,
                serde_json::json!({
                    "views": ["front","left_back","right_back","top","bottom"],
                    "image_size": 768,
                    "prefix": "pipeline_review",
                    "include_motion_sheets": false
                }),
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
            }
            job.pipeline.stage = Gen3dPipelineStage::ReviewDelta;
            return;
        }
        Gen3dPipelineStage::ReviewDelta => {
            let rounds_max = config.gen3d_review_delta_rounds_max;
            let rounds_used = job.review_delta_rounds_used;
            let rounds_remaining = rounds_max.saturating_sub(rounds_used);
            if rounds_max == 0 || rounds_remaining == 0 {
                stop_pipeline_best_effort(
                    commands,
                    review_cameras,
                    workshop,
                    job,
                    format!(
                        "Pipeline stopped: review-delta budget exhausted (used={rounds_used} max={rounds_max}). Retry Build; if this repeats, simplify the request or adjust review-delta budget."
                    ),
                );
                return;
            }

            workshop.status = "Pipeline: review delta…".into();
            let args = if job.pipeline.pending_preview_blob_ids.is_empty() {
                serde_json::json!({})
            } else {
                serde_json::json!({ "preview_blob_ids": job.pipeline.pending_preview_blob_ids })
            };
            job.pipeline.pending_preview_blob_ids = Vec::new();

            if let Some(result) = start_pipeline_tool_call(
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
                TOOL_ID_LLM_REVIEW_DELTA,
                args,
            ) {
                pipeline_record_tool_result(workshop, job, &*draft, &result);
                if let Some(replan_reason) = result
                    .result
                    .as_ref()
                    .and_then(|v| v.get("replan_reason"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                {
                    debug!("Gen3D pipeline: review-delta requested replan: {replan_reason}");
                    if job.preserve_existing_components_mode {
                        job.pipeline.stage = Gen3dPipelineStage::PreserveReplanTemplate;
                    } else {
                        job.pipeline.force_replan = true;
                        job.pipeline.stage = Gen3dPipelineStage::CreatePlan;
                    }
                    return;
                }
            }
            job.pipeline.stage = Gen3dPipelineStage::EnsureComponents;
            return;
        }
        Gen3dPipelineStage::Finish | Gen3dPipelineStage::Start => {}
    }
}
