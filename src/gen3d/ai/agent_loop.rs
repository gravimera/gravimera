use bevy::log::{debug, info, warn};
use bevy::prelude::*;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::gen3d::agent::{
    append_agent_trace_event_v1, AgentTraceEventV1, Gen3dAgentActionJsonV1, Gen3dAgentStepJsonV1,
    Gen3dToolCallJsonV1, Gen3dToolRegistryV1, Gen3dToolResultJsonV1,
};

use super::artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_assembly_snapshot,
    write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::parse;
use super::parse::extract_json_object;
use super::{
    fail_job, gen3d_advance_pass, set_progress, spawn_gen3d_ai_text_thread, Gen3dAiJob,
    Gen3dAiPhase, Gen3dAiProgress, Gen3dAiTextResponse,
};
use crate::gen3d::agent::tools::{
    TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_CREATE_WORKSPACE,
    TOOL_ID_DELETE_WORKSPACE, TOOL_ID_DESCRIBE, TOOL_ID_DETACH_COMPONENT,
    TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_STATE_SUMMARY, TOOL_ID_GET_USER_INPUTS,
    TOOL_ID_LIST, TOOL_ID_LLM_GENERATE_COMPONENT, TOOL_ID_LLM_GENERATE_COMPONENTS,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA, TOOL_ID_RENDER_PREVIEW,
    TOOL_ID_SET_ACTIVE_WORKSPACE, TOOL_ID_SMOKE_CHECK, TOOL_ID_SUBMIT_TOOLING_FEEDBACK,
    TOOL_ID_VALIDATE,
};
use crate::types::{AnimationChannelsActive, AttackClock, LocomotionClock};

use super::super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dWorkshop,
};
use super::super::tool_feedback::Gen3dToolFeedbackHistory;
use super::{GEN3D_MAX_REQUEST_IMAGES, GEN3D_PREVIEW_DEFAULT_PITCH, GEN3D_PREVIEW_DEFAULT_YAW};

const GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS: u8 = 2;
const GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES: u8 = 6;

pub(super) fn poll_gen3d_agent(
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
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    match job.phase {
        Gen3dAiPhase::AgentWaitingStep => poll_agent_step(
            config,
            commands,
            review_cameras,
            workshop,
            feedback_history,
            job,
            draft,
        ),
        Gen3dAiPhase::AgentExecutingActions => execute_agent_actions(
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
        ),
        Gen3dAiPhase::AgentWaitingTool => poll_agent_tool(
            config,
            commands,
            images,
            workshop,
            feedback_history,
            job,
            draft,
            preview,
        ),
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
        Gen3dAiPhase::AgentCapturingPassSnapshot => poll_agent_pass_snapshot_capture(
            config,
            commands,
            images,
            workshop,
            feedback_history,
            job,
        ),
        _ => fail_job(
            workshop,
            job,
            "Internal error: agent entered an unexpected phase.",
        ),
    }
}

pub(super) fn spawn_agent_step_request(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    pass_dir: PathBuf,
) -> Result<(), String> {
    let Some(openai) = job.openai.clone() else {
        return Err("Internal error: missing OpenAI config.".into());
    };

    let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
        Arc::new(Mutex::new(None));
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Starting agent…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    job.metrics.note_agent_step_request_started();

    set_progress(&progress, "Thinking…");

    let registry = Gen3dToolRegistryV1::default();
    let system = build_agent_system_instructions();
    let user_text = build_agent_user_text(
        config,
        job,
        workshop,
        draft_summary(config, job),
        &job.agent.step_tool_results,
        &registry,
    );

    append_agent_trace_event_v1(
        job.run_dir.as_deref(),
        &AgentTraceEventV1::Info {
            message: "Gen3D agent: requesting next step".into(),
        },
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "agent_step_request attempt={} pass={} plan_hash={} assembly_rev={} active_ws={} components_generated={}/{}",
            job.attempt,
            job.pass,
            job.plan_hash(),
            job.assembly_rev(),
            job.active_workspace_id(),
            job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
            job.planned_components.len(),
        ),
    );
    debug!(
        "Gen3D agent: requesting step (attempt={}, pass={}, plan_hash={}, assembly_rev={}, active_ws={}, components_generated={}/{})",
        job.attempt,
        job.pass,
        job.plan_hash(),
        job.assembly_rev(),
        job.active_workspace_id(),
        job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
        job.planned_components.len(),
    );

    let reasoning_effort = super::openai::cap_reasoning_effort(
        &openai.model_reasoning_effort,
        &config.gen3d_reasoning_effort_agent_step,
    );
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.session.clone(),
        None,
        openai,
        reasoning_effort,
        system,
        user_text,
        Vec::new(),
        pass_dir,
        "agent_step".into(),
    );

    Ok(())
}

fn build_agent_system_instructions() -> String {
    // Keep this short; the user text provides tool lists and current state.
    // The agent must output strict JSON only.
    "You are the Gravimera Gen3D agent.\n\
Return ONLY a single JSON object for gen3d_agent_step_v1 (no markdown, no prose).\n\n\
Schema:\n\
{\n\
  \"version\": 1,\n\
  \"status_summary\": \"short, user-facing summary\",\n\
  \"actions\": [\n\
    {\"kind\":\"tool_call\",\"call_id\":\"call_1\",\"tool_id\":\"list_tools_v1\",\"args\":{}},\n\
    {\"kind\":\"done\",\"reason\":\"why you are stopping\"}\n\
  ]\n\
}\n\n\
Rules:\n\
- Use tools to read/modify state. Do not assume the engine will auto-fix anything.\n\
- Prefer small, explainable steps that improve basic structure and correctness.\n\
- Prioritize BASIC STRUCTURE over tiny details. This is a voxel/pixel-art game; do not chase micro-adjustments forever.\n\
- STOP when the model is good enough:\n\
  - If the latest review delta accepts the model / has no actionable fixes (and QA has been run), output a \"done\" action.\n\
  - If you did one more render+review after applying fixes and it still suggests no further actions, output a \"done\" action.\n\
  - If budgets prevent further improvement (regen budgets, time, tokens), output a \"done\" action with a best-effort reason.\n\
- Be AGGRESSIVE about visual QA: render previews and review them early, but prefer doing work in WAVES to reduce LLM wall time.\n\
  - Preferred loop: plan -> generate components (batch) -> render_preview_v1 -> llm_review_delta_v1.\n\
  - IMPORTANT: planning must be its OWN step.\n\
    - If you call llm_generate_plan_v1, DO NOT include llm_generate_components_v1/llm_generate_component_v1 in the same step.\n\
    - End the step after planning so you can observe `reuse_groups`/state before deciding what to generate.\n\
    - The engine will end the step after a successful llm_generate_plan_v1 even if you requested more actions.\n\
  - Avoid calling llm_review_delta_v1 after every single component if you can generate a batch first.\n\
  - After any render_preview_v1, immediately call llm_review_delta_v1 using the rendered images.\n\
  - Do NOT use placeholder paths like `$CALL_1.images[0]` in tool args; the engine does not substitute tool outputs into later tool calls.\n\
    To review the latest render, call llm_review_delta_v1 with no `preview_images` (it will use the latest render cache), or pass `{ \"rendered_images_from_cache\": true }`.\n\
  - Do not finish a run without reviewing the latest renders.\n\
  - For vehicles/wheeled objects, always include TOP and BOTTOM views (they reveal wheel/axle/undercarriage issues). A good default is: views=[\"front\",\"left_back\",\"right_back\",\"top\",\"bottom\"].\n\
  - For speed, prefer smaller preview renders during iteration (example: render_preview_v1 image_size=768). Only increase resolution if you truly need extra detail.\n\
  - Do NOT render/review before any geometry exists. If components_generated==0 or the draft has 0 primitive parts, generate components first; renders will be blank.\n\
- Avoid duplicated LLM work: reuse geometry for symmetric/repeated parts (major speed win):\n\
  - If multiple planned components should be identical (wheels, legs, mirrored handles, numbered sets like leg_0..leg_7), generate ONE of them, then fill the others using copy_component_v1 instead of calling llm_generate_component_v1 repeatedly.\n\
  - If the repeated part is a CHAIN (a component with attached descendants, like a leg/arm), use copy_component_subtree_v1 to copy the whole subtree in one call.\n\
  - Anchors: prefer anchors=preserve_interfaces so TARGET mount interfaces stay stable while internal anchors stay consistent with copied geometry (recommended for mirrored parts and radial limbs). Use anchors=preserve_target only when you must keep ALL target anchors unchanged. Use anchors=copy_source only when you need to overwrite the TARGET's anchors to match the SOURCE exactly.\n\
  - Prefer mode=linked when copying many LEAF components; call detach_component_v1 if any copy must diverge later.\n\
  - The state summary may include `reuse_suggestions` with ready-to-use tool args; use them when appropriate.\n\
- When you DO need LLM generation, prefer batching UNIQUE components in parallel:\n\
  - Use llm_generate_components_v1 with explicit component_indices/names for the unique set.\n\
  - Use missing_only=true ONLY when you truly want ALL missing components.\n\
    - If the plan declares `reuse_groups`, the engine will skip reuse targets in missing_only batches and auto-copy them after sources are generated.\n\
- IMPORTANT: If the state summary contains `pending_regen_component_indices` (non-empty), APPLY THEM NEXT:\n\
  - Call llm_generate_components_v1 with component_indices set to that list and force=true (regen is expected).\n\
  - Then call render_preview_v1 and llm_review_delta_v1 to confirm the fixes.\n\
  - Do NOT call llm_review_delta_v1 repeatedly without applying the pending regen or making a new render.\n\
- Regen budgets: regenerating an already-generated component counts against a regen budget. If a regen tool returns skipped_due_to_regen_budget, stop trying to regenerate and fix via transform/anchor tweaks instead.\n\
- IMPORTANT: A \"done\" action ENDS the Build run immediately. Only use \"done\" when you want to stop NOW.\n\
  If you want the run to continue, DO NOT include a \"done\" action; the engine will request another step automatically.\n\
- If you need tool schemas, call list_tools_v1 / describe_tool_v1.\n"
        .to_string()
}

fn build_agent_user_text(
    config: &AppConfig,
    job: &Gen3dAiJob,
    workshop: &Gen3dWorkshop,
    state_summary: serde_json::Value,
    recent_tool_results: &[Gen3dToolResultJsonV1],
    registry: &Gen3dToolRegistryV1,
) -> String {
    let _ = config;

    fn truncate_for_prompt(text: &str, max_chars: usize) -> String {
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

    fn summarize_tool_result(result: &Gen3dToolResultJsonV1) -> String {
        let mut out = String::new();
        out.push_str("- ");
        out.push_str(result.tool_id.as_str());
        out.push_str(" (");
        out.push_str(result.call_id.as_str());
        out.push_str("): ");

        if !result.ok {
            out.push_str("ERROR: ");
            out.push_str(&truncate_for_prompt(
                result.error.as_deref().unwrap_or("<no error>"),
                240,
            ));
            return out;
        }

        let Some(value) = result.result.as_ref() else {
            out.push_str("ok");
            return out;
        };

        match result.tool_id.as_str() {
            TOOL_ID_LLM_GENERATE_PLAN => {
                let comps = value.get("components_total").and_then(|v| v.as_u64());
                let plan_hash = value.get("plan_hash").and_then(|v| v.as_str());
                out.push_str("ok");
                if let Some(comps) = comps {
                    out.push_str(&format!(" components_total={comps}"));
                }
                if let Some(plan_hash) = plan_hash {
                    out.push_str(&format!(" plan_hash={plan_hash}"));
                }
            }
            TOOL_ID_LLM_GENERATE_COMPONENT => {
                let idx = value.get("component_index").and_then(|v| v.as_u64());
                let name = value.get("component_name").and_then(|v| v.as_str());
                let skipped_due_to_regen_budget = value
                    .get("skipped_due_to_regen_budget")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let regen_count = value.get("regen_count").and_then(|v| v.as_u64());
                out.push_str("ok");
                if let Some(idx) = idx {
                    out.push_str(&format!(" idx={idx}"));
                }
                if let Some(name) = name {
                    out.push_str(&format!(" name={name}"));
                }
                if skipped_due_to_regen_budget {
                    out.push_str(" skipped_due_to_regen_budget=true");
                    if let Some(regen_count) = regen_count {
                        out.push_str(&format!(" regen_count={regen_count}"));
                    }
                }
            }
            TOOL_ID_LLM_GENERATE_COMPONENTS => {
                let requested = value.get("requested").and_then(|v| v.as_u64());
                let succeeded = value.get("succeeded").and_then(|v| v.as_u64());
                let failed = value
                    .get("failed")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let regen_skipped = value
                    .get("skipped_due_to_regen_budget")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let reuse_skipped = value
                    .get("skipped_due_to_reuse_groups")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(requested) = requested {
                    out.push_str(&format!(" requested={requested}"));
                }
                if let Some(succeeded) = succeeded {
                    out.push_str(&format!(" succeeded={succeeded}"));
                }
                if let Some(failed) = failed {
                    out.push_str(&format!(" failed={failed}"));
                }
                if let Some(total) = regen_skipped {
                    if total > 0 {
                        out.push_str(&format!(" regen_budget_skipped={total}"));
                    }
                }
                if let Some(total) = reuse_skipped {
                    if total > 0 {
                        out.push_str(&format!(" reuse_skipped={total}"));
                    }
                }
            }
            TOOL_ID_LLM_REVIEW_DELTA => {
                let accepted = value.get("accepted").and_then(|v| v.as_bool());
                let had_actions = value.get("had_actions").and_then(|v| v.as_bool());
                let regen_indices = value
                    .get("regen_component_indices")
                    .and_then(|v| v.as_array());
                let regen_skipped = value
                    .get("regen_component_indices_skipped_due_to_budget")
                    .and_then(|v| v.as_array());
                out.push_str("ok");
                if let Some(accepted) = accepted {
                    out.push_str(&format!(" accepted={accepted}"));
                }
                if let Some(had_actions) = had_actions {
                    out.push_str(&format!(" had_actions={had_actions}"));
                }
                if let Some(regen_indices) = regen_indices {
                    let total = regen_indices.len();
                    let indices: Vec<u64> = regen_indices
                        .iter()
                        .filter_map(|v| v.as_u64())
                        .take(12)
                        .collect();
                    if !indices.is_empty() {
                        out.push_str(&format!(" regen_indices={indices:?}"));
                    }
                    if total > indices.len() {
                        out.push_str(&format!(" regen_indices_total={total}"));
                    }
                }
                if let Some(regen_skipped) = regen_skipped {
                    let total = regen_skipped.len();
                    let skipped: Vec<u64> = regen_skipped
                        .iter()
                        .filter_map(|v| v.as_u64())
                        .take(12)
                        .collect();
                    if !skipped.is_empty() {
                        out.push_str(&format!(" regen_skipped={skipped:?}"));
                    }
                    if total > skipped.len() {
                        out.push_str(&format!(" regen_skipped_total={total}"));
                    }
                }
            }
            TOOL_ID_RENDER_PREVIEW => {
                let images = value
                    .get("images")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(images) = images {
                    out.push_str(&format!(" images={images}"));
                }
            }
            TOOL_ID_VALIDATE => {
                let ok = value.get("ok").and_then(|v| v.as_bool());
                let issues = value
                    .get("issues")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(ok) = ok {
                    out.push_str(&format!(" ok={ok}"));
                }
                if let Some(issues) = issues {
                    out.push_str(&format!(" issues={issues}"));
                }
            }
            TOOL_ID_SMOKE_CHECK => {
                let ok = value.get("ok").and_then(|v| v.as_bool());
                let issues = value
                    .get("issues")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(ok) = ok {
                    out.push_str(&format!(" ok={ok}"));
                }
                if let Some(issues) = issues {
                    out.push_str(&format!(" issues={issues}"));
                }
            }
            TOOL_ID_DESCRIBE => {
                let tool_id = value.get("tool_id").and_then(|v| v.as_str());
                let one_line = value.get("one_line_summary").and_then(|v| v.as_str());
                let args_example = value.get("args_example");
                let description = value.get("description").and_then(|v| v.as_str());
                out.push_str("ok");
                if let Some(tool_id) = tool_id {
                    out.push_str(&format!(" tool={tool_id}"));
                }
                if let Some(one_line) = one_line {
                    out.push_str(&format!(" summary={}", truncate_for_prompt(one_line, 120)));
                }
                if let Some(args_example) = args_example {
                    out.push_str(&format!(
                        " args_example={}",
                        truncate_for_prompt(&args_example.to_string(), 200)
                    ));
                }
                if let Some(description) = description {
                    out.push_str(&format!(" desc={}", truncate_for_prompt(description, 260)));
                }
            }
            TOOL_ID_COPY_COMPONENT | TOOL_ID_COPY_COMPONENT_SUBTREE => {
                let copies = value
                    .get("copies")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(copies) = copies {
                    out.push_str(&format!(" copies={copies}"));
                }
            }
            _ => {
                // Generic fallback: list top-level keys so the agent knows what's available
                // without embedding the full JSON blob.
                let keys = value
                    .as_object()
                    .map(|o| {
                        let mut keys: Vec<&str> = o.keys().map(|k| k.as_str()).collect();
                        keys.sort();
                        keys
                    })
                    .unwrap_or_default();
                out.push_str("ok");
                if !keys.is_empty() {
                    out.push_str(" keys=");
                    out.push_str(&truncate_for_prompt(&format!("{keys:?}"), 160));
                }
            }
        }
        out
    }

    let mut out = String::new();
    out.push_str("User prompt:\n");
    out.push_str(job.user_prompt_raw.trim());
    out.push('\n');
    out.push_str(&format!("Input images: {}\n\n", job.user_images.len()));

    out.push_str("Available tools (call list_tools_v1 to get the full JSON list):\n");
    for tool in registry.list() {
        out.push_str(&format!("- {}: {}\n", tool.tool_id, tool.one_line_summary));
    }

    out.push_str("\nCurrent state summary:\n");
    out.push_str(
        &serde_json::to_string_pretty(&state_summary).unwrap_or_else(|_| state_summary.to_string()),
    );
    out.push('\n');

    if !recent_tool_results.is_empty() {
        let mut recent: Vec<&Gen3dToolResultJsonV1> = recent_tool_results.iter().collect();
        if recent.len() > 16 {
            recent = recent.split_off(recent.len() - 16);
        }
        out.push_str("\nRecent tool results (previous step, compact):\n");
        for r in recent {
            out.push_str(&summarize_tool_result(r));
            out.push('\n');
        }
        out.push('\n');
    }

    if !workshop
        .error
        .as_ref()
        .map(|e| e.trim())
        .unwrap_or("")
        .is_empty()
    {
        out.push_str("\nLast error:\n");
        out.push_str(workshop.error.as_ref().unwrap().trim());
        out.push('\n');
    }

    out
}

fn draft_summary(config: &AppConfig, job: &Gen3dAiJob) -> serde_json::Value {
    fn normalize_copy_group_key(name: &str) -> Option<String> {
        let raw = name.trim();
        if raw.is_empty() {
            return None;
        }

        let mut changed = false;
        let mut out_parts: Vec<String> = Vec::new();
        for part in raw.split('_').filter(|p| !p.is_empty()) {
            let mut normalized = String::new();
            let mut chars = part.chars().peekable();
            while let Some(ch) = chars.next() {
                if ch.is_ascii_digit() {
                    changed = true;
                    while chars.peek().is_some_and(|c| c.is_ascii_digit()) {
                        chars.next();
                    }
                    normalized.push_str("{i}");
                    continue;
                }
                normalized.push(ch.to_ascii_lowercase());
            }

            match normalized.as_str() {
                "left" | "right" => {
                    changed = true;
                    out_parts.push("{side}".into());
                }
                // Common positional suffixes / tokens (helps group radial legs like `leg01_fr`, `leg02_r`, ...).
                "l" | "r" => {
                    changed = true;
                    out_parts.push("{side}".into());
                }
                "front" | "back" | "f" | "b" | "fl" | "fr" | "bl" | "br" => {
                    changed = true;
                    out_parts.push("{pos}".into());
                }
                _ => out_parts.push(normalized),
            }
        }

        if !changed {
            return None;
        }

        let key = out_parts.join("_");
        if key.trim().is_empty() {
            return None;
        }

        Some(key)
    }

    fn compute_child_counts(
        components: &[super::Gen3dPlannedComponent],
    ) -> std::collections::HashMap<String, usize> {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for comp in components {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            *counts.entry(att.parent.clone()).or_insert(0) += 1;
        }
        counts
    }

    fn build_reuse_suggestions(
        components: &[super::Gen3dPlannedComponent],
    ) -> Vec<serde_json::Value> {
        let child_counts = compute_child_counts(components);
        let mut groups: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();

        for (idx, comp) in components.iter().enumerate() {
            let Some(key) = normalize_copy_group_key(&comp.name) else {
                continue;
            };
            groups.entry(key).or_default().push(idx);
        }

        let mut out: Vec<serde_json::Value> = Vec::new();
        let mut keys: Vec<String> = groups.keys().cloned().collect();
        keys.sort();

        for key in keys {
            let Some(mut indices) = groups.remove(&key) else {
                continue;
            };
            if indices.len() < 2 {
                continue;
            }

            // Suppress low-signal suggestions like foo_v{i} with only 2 entries.
            let is_side_group = key.contains("{side}");
            if !is_side_group && indices.len() < 3 {
                continue;
            }

            indices.sort_by(|&a, &b| components[a].name.cmp(&components[b].name));

            let generated_source = indices
                .iter()
                .copied()
                .find(|&idx| components[idx].actual_size.is_some());

            let source_idx = generated_source.unwrap_or(indices[0]);
            let source_name = components[source_idx].name.clone();
            let source_generated_now = components[source_idx].actual_size.is_some();

            let mut targets: Vec<String> = Vec::new();
            for idx in indices.iter().copied() {
                if idx == source_idx {
                    continue;
                }
                if generated_source.is_some() && components[idx].actual_size.is_some() {
                    continue;
                }
                targets.push(components[idx].name.clone());
            }

            if targets.is_empty() {
                continue;
            }

            let mut targets_omitted: Option<usize> = None;
            const MAX_TARGETS_LISTED: usize = 16;
            if targets.len() > MAX_TARGETS_LISTED {
                targets_omitted = Some(targets.len() - MAX_TARGETS_LISTED);
                targets.truncate(MAX_TARGETS_LISTED);
            }

            let source_has_children = child_counts.get(&source_name).copied().unwrap_or(0) > 0;
            let group_has_children = indices.iter().any(|&idx| {
                let name = components[idx].name.as_str();
                child_counts.get(name).copied().unwrap_or(0) > 0
            });

            if group_has_children || source_has_children {
                out.push(serde_json::json!({
                    "kind": "copy_component_subtree",
                    "group_key": key,
                    "source": source_name.clone(),
                    "source_generated": source_generated_now,
                    "targets": targets.clone(),
                    "targets_omitted": targets_omitted,
                    "recommended_tool": "copy_component_subtree_v1",
                    "note": "If source_generated=false, generate the source subtree first, then run the copy tool.",
                    "recommended_args": {
                        "source_root": source_name,
                        "targets": targets,
                        "mode": "detached",
                        "anchors": "preserve_interfaces",
                    }
                }));
            } else {
                out.push(serde_json::json!({
                    "kind": "copy_component",
                    "group_key": key,
                    "source": source_name.clone(),
                    "source_generated": source_generated_now,
                    "targets": targets.clone(),
                    "targets_omitted": targets_omitted,
                    "recommended_tool": "copy_component_v1",
                    "note": "If source_generated=false, generate the source component first, then run the copy tool.",
                    "recommended_args": {
                        "source_component": source_name,
                        "targets": targets,
                        "mode": "detached",
                        "anchors": "preserve_interfaces",
                    }
                }));
            }
        }

        out
    }

    let mut workspaces: Vec<(&String, &super::Gen3dAgentWorkspace)> =
        job.agent.workspaces.iter().collect();
    workspaces.sort_by(|(a, _), (b, _)| a.cmp(b));
    let workspaces_json: Vec<serde_json::Value> = workspaces
        .into_iter()
        .map(|(id, ws)| {
            serde_json::json!({
                "id": id.as_str(),
                "name": ws.name.as_str(),
            })
        })
        .collect();

    let components_json: Vec<serde_json::Value> = job
        .planned_components
        .iter()
        .enumerate()
        .map(|(idx, c)| {
            let regen_count = job.regen_per_component.get(idx).copied().unwrap_or(0);
            let max_total = config.gen3d_max_regen_total;
            let max_per_component = config.gen3d_max_regen_per_component;
            let regen_total_blocked = max_total > 0 && job.regen_total >= max_total;
            let regen_component_blocked = max_per_component > 0 && regen_count >= max_per_component;
            let regen_budget_blocked = regen_total_blocked || regen_component_blocked;
            let regen_remaining = if max_per_component == 0 {
                None
            } else {
                Some(max_per_component.saturating_sub(regen_count))
            };
            serde_json::json!({
                "index": idx,
                "name": c.name.as_str(),
                "generated": c.actual_size.is_some(),
                "regen_count": regen_count,
                "regen_remaining": regen_remaining,
                "regen_budget_blocked": regen_budget_blocked,
            })
        })
        .collect();

    let reuse_groups_json: Vec<serde_json::Value> = job
        .reuse_groups
        .iter()
        .map(|g| {
            let kind = match g.kind {
                super::reuse_groups::Gen3dReuseGroupKind::Component => "component",
                super::reuse_groups::Gen3dReuseGroupKind::Subtree => "subtree",
            };
            let mode = match g.mode {
                super::copy_component::Gen3dCopyMode::Detached => "detached",
                super::copy_component::Gen3dCopyMode::Linked => "linked",
            };
            let anchors = match g.anchors_mode {
                super::copy_component::Gen3dCopyAnchorsMode::CopySourceAnchors => "copy_source",
                super::copy_component::Gen3dCopyAnchorsMode::PreserveTargetAnchors => {
                    "preserve_target"
                }
                super::copy_component::Gen3dCopyAnchorsMode::PreserveInterfaceAnchors => {
                    "preserve_interfaces"
                }
            };
            let source = job
                .planned_components
                .get(g.source_root_idx)
                .map(|c| c.name.as_str())
                .unwrap_or("<missing>");
            let targets: Vec<&str> = g
                .target_root_indices
                .iter()
                .copied()
                .filter_map(|idx| job.planned_components.get(idx).map(|c| c.name.as_str()))
                .collect();
            serde_json::json!({
                "kind": kind,
                "source": source,
                "targets": targets,
                "mode": mode,
                "anchors": anchors,
            })
        })
        .collect();

    let missing_only_generation_indices = super::reuse_groups::missing_only_generation_indices(
        &job.planned_components,
        &job.reuse_groups,
    );

    let regen_remaining_total = if config.gen3d_max_regen_total == 0 {
        None
    } else {
        Some(config.gen3d_max_regen_total.saturating_sub(job.regen_total))
    };
    let no_progress_steps_remaining = if config.gen3d_no_progress_max_steps == 0 {
        None
    } else {
        Some(
            config
                .gen3d_no_progress_max_steps
                .saturating_sub(job.agent.no_progress_steps),
        )
    };
    let run_elapsed_seconds = job.run_elapsed().map(|d| d.as_secs_f64());
    let time_budget_remaining_seconds = if config.gen3d_max_seconds == 0 {
        None
    } else {
        run_elapsed_seconds.map(|elapsed| (config.gen3d_max_seconds as f64 - elapsed).max(0.0))
    };
    let token_budget_remaining = if config.gen3d_max_tokens == 0 {
        None
    } else {
        Some(
            config
                .gen3d_max_tokens
                .saturating_sub(job.current_run_tokens()),
        )
    };

    serde_json::json!({
        "run_id": job.run_id.map(|id| id.to_string()),
        "attempt": job.attempt,
        "pass": job.pass,
        "plan_hash": job.plan_hash,
        "assembly_rev": job.assembly_rev,
        "needs_review": job.agent.rendered_since_last_review,
        "no_progress_steps": job.agent.no_progress_steps,
        "pending_regen_component_indices": &job.agent.pending_regen_component_indices,
        "pending_regen_component_indices_skipped_due_to_budget": &job
            .agent
            .pending_regen_component_indices_skipped_due_to_budget,
        "reuse_groups": reuse_groups_json,
        "reuse_group_warnings": &job.reuse_group_warnings,
        "missing_only_generation_indices": missing_only_generation_indices,
        "reuse_suggestions": build_reuse_suggestions(&job.planned_components),
        "qa": {
            "ever_rendered": job.agent.ever_rendered,
            "ever_reviewed": job.agent.ever_reviewed,
            "ever_validated": job.agent.ever_validated,
            "ever_smoke_checked": job.agent.ever_smoke_checked,
        },
        "budgets": {
            "regen": {
                "max_total": config.gen3d_max_regen_total,
                "used_total": job.regen_total,
                "remaining_total": regen_remaining_total,
                "max_per_component": config.gen3d_max_regen_per_component,
            },
            "no_progress": {
                "max_steps": config.gen3d_no_progress_max_steps,
                "used_steps": job.agent.no_progress_steps,
                "remaining_steps": no_progress_steps_remaining,
            },
            "time": {
                "max_seconds": config.gen3d_max_seconds,
                "elapsed_seconds": run_elapsed_seconds.map(|v| (v * 10.0).round() / 10.0),
                "remaining_seconds": time_budget_remaining_seconds.map(|v| (v * 10.0).round() / 10.0),
            },
            "tokens": {
                "max_tokens": config.gen3d_max_tokens,
                "used_run_tokens": job.current_run_tokens(),
                "remaining_run_tokens": token_budget_remaining,
            },
        },
        "last_render_images": job
            .agent
            .last_render_images
            .iter()
            .rev()
            .take(12)
            .cloned()
            .collect::<Vec<PathBuf>>()
            .into_iter()
            .rev()
            .map(|p| p.display().to_string())
            .collect::<Vec<String>>(),
        "active_workspace": job.agent.active_workspace_id.as_str(),
        "workspaces": workspaces_json,
        "components_total": job.planned_components.len(),
        "components_generated": job.planned_components.iter().filter(|c| c.actual_size.is_some()).count(),
        "components": components_json,
        "regen_total": job.regen_total,
        "regen_budget": {
            "max_total": config.gen3d_max_regen_total,
            "max_per_component": config.gen3d_max_regen_per_component,
        },
        "tokens_run": job.current_run_tokens(),
        "tokens_total": job.total_tokens(),
    })
}

fn parse_paths_array_from_args(args: &serde_json::Value, keys: &[&str]) -> Vec<PathBuf> {
    for key in keys {
        let Some(arr) = args.get(*key).and_then(|v| v.as_array()) else {
            continue;
        };
        let mut out = Vec::new();
        for value in arr {
            let Some(s) = value.as_str().map(|s| s.trim()).filter(|s| !s.is_empty()) else {
                continue;
            };
            // Some models attempt to reference previous tool results using placeholders like
            // `$CALL_1.images[0]`. Gravimera does not support templating tool outputs into args.
            // Ignore these placeholders and fall back to the latest rendered images in cache.
            if s.starts_with('$') {
                continue;
            }
            out.push(PathBuf::from(s));
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

fn parse_review_preview_images_from_args(args: &serde_json::Value) -> Vec<PathBuf> {
    parse_paths_array_from_args(
        args,
        &[
            "preview_images",
            "images",
            "image_paths",
            "paths",
            "preview_image_paths",
        ],
    )
}

fn file_name_lower(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
}

fn is_motion_preview_image(path: &Path) -> bool {
    let Some(name) = file_name_lower(path) else {
        return false;
    };
    name.contains("move_sheet")
        || name.contains("attack_sheet")
        || name.contains("move_frame")
        || name.contains("attack_frame")
}

fn select_review_preview_images(
    preview_images: &[PathBuf],
    include_motion_sheets: bool,
) -> Vec<PathBuf> {
    // Default policy for "routine" visual reviews:
    // - Prefer 5 static render views (front/left_back/right_back/top/bottom).
    // - Only include motion/attack sheets when smoke_check reports motion/attack issues.
    let preferred_static = [
        "render_front.png",
        "render_left_back.png",
        "render_right_back.png",
        "render_top.png",
        "render_bottom.png",
    ];

    let mut out: Vec<PathBuf> = Vec::new();
    for desired in preferred_static {
        if let Some(p) = preview_images
            .iter()
            .find(|p| file_name_lower(p).as_deref() == Some(desired))
        {
            out.push(p.clone());
        }
    }

    if out.is_empty() {
        for p in preview_images {
            if out.len() >= 5 {
                break;
            }
            if is_motion_preview_image(p) {
                continue;
            }
            out.push(p.clone());
        }
    }

    if out.is_empty() {
        out.extend(preview_images.iter().take(5).cloned());
    }

    if include_motion_sheets {
        for desired in ["move_sheet.png", "attack_sheet.png"] {
            if out
                .iter()
                .any(|p| file_name_lower(p).as_deref() == Some(desired))
            {
                continue;
            }
            if let Some(p) = preview_images
                .iter()
                .find(|p| file_name_lower(p).as_deref() == Some(desired))
            {
                out.push(p.clone());
            }
        }
    }

    out
}

fn ensure_agent_regen_budget_len(job: &mut Gen3dAiJob) {
    let planned_len = job.planned_components.len();
    if job.regen_per_component.len() != planned_len {
        job.regen_per_component.resize(planned_len, 0);
    }
}

fn regen_budget_allows(config: &AppConfig, job: &Gen3dAiJob, component_idx: usize) -> bool {
    let max_total = config.gen3d_max_regen_total;
    if max_total > 0 && job.regen_total >= max_total {
        return false;
    }
    let max_per_component = config.gen3d_max_regen_per_component;
    if max_per_component > 0
        && job
            .regen_per_component
            .get(component_idx)
            .copied()
            .unwrap_or(0)
            >= max_per_component
    {
        return false;
    }
    true
}

fn consume_regen_budget(config: &AppConfig, job: &mut Gen3dAiJob, component_idx: usize) -> bool {
    ensure_agent_regen_budget_len(job);
    if !regen_budget_allows(config, job, component_idx) {
        return false;
    }
    job.regen_total = job.regen_total.saturating_add(1);
    if component_idx < job.regen_per_component.len() {
        job.regen_per_component[component_idx] =
            job.regen_per_component[component_idx].saturating_add(1);
    }
    true
}

fn normalize_identifier_for_match(value: &str) -> String {
    let mut out = String::new();
    let mut prev_sep = true;
    for ch in value.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_sep = false;
        } else if !prev_sep {
            out.push('_');
            prev_sep = true;
        }
        if out.len() >= 64 {
            break;
        }
    }

    while out.starts_with('_') {
        out.remove(0);
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

fn resolve_component_index_by_name_hint(
    components: &[super::Gen3dPlannedComponent],
    hint: &str,
) -> Option<usize> {
    let hint_norm = normalize_identifier_for_match(hint);
    if hint_norm.is_empty() {
        return None;
    }

    for (idx, c) in components.iter().enumerate() {
        if normalize_identifier_for_match(c.name.as_str()) == hint_norm {
            return Some(idx);
        }
    }

    let hint_tokens: Vec<&str> = hint_norm.split('_').filter(|s| !s.is_empty()).collect();
    if hint_tokens.is_empty() {
        return None;
    }

    let mut best: Option<(usize, f32, usize)> = None;
    for (idx, c) in components.iter().enumerate() {
        let cand_norm = normalize_identifier_for_match(c.name.as_str());
        if cand_norm.is_empty() {
            continue;
        }
        let cand_tokens: Vec<&str> = cand_norm.split('_').filter(|s| !s.is_empty()).collect();
        if cand_tokens.is_empty() {
            continue;
        }

        let mut intersection = 0usize;
        for t in &hint_tokens {
            if cand_tokens.contains(t) {
                intersection += 1;
            }
        }
        let union = hint_tokens.len() + cand_tokens.len() - intersection;
        let mut score = if union == 0 {
            0.0
        } else {
            intersection as f32 / union as f32
        };

        if hint_norm.contains(&cand_norm) || cand_norm.contains(&hint_norm) {
            score += 0.25;
        }
        if hint_tokens.first() == cand_tokens.first() {
            score += 0.12;
        }

        let len_bonus = cand_norm.len().min(hint_norm.len());
        if best
            .as_ref()
            .map(|(_, s, l)| score > *s || (score == *s && len_bonus > *l))
            .unwrap_or(true)
        {
            best = Some((idx, score, len_bonus));
        }
    }

    let (idx, score, _) = best?;
    (score >= 0.34).then_some(idx)
}

fn parse_vec3(value: &serde_json::Value) -> Option<Vec3> {
    let arr = value.as_array()?;
    if arr.len() != 3 {
        return None;
    }
    let x = arr.get(0)?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    let z = arr.get(2)?.as_f64()? as f32;
    let v = Vec3::new(x, y, z);
    v.is_finite().then_some(v)
}

fn parse_quat_xyzw(value: &serde_json::Value) -> Option<Quat> {
    let arr = value.as_array()?;
    if arr.len() != 4 {
        return None;
    }
    let x = arr.get(0)?.as_f64()? as f32;
    let y = arr.get(1)?.as_f64()? as f32;
    let z = arr.get(2)?.as_f64()? as f32;
    let w = arr.get(3)?.as_f64()? as f32;
    let q = Quat::from_xyzw(x, y, z, w).normalize();
    q.is_finite().then_some(q)
}

fn parse_delta_transform(value: Option<&serde_json::Value>) -> Transform {
    let mut out = Transform::IDENTITY;
    let Some(value) = value else {
        return out;
    };
    if let Some(pos) = value
        .get("pos")
        .and_then(parse_vec3)
        .or_else(|| value.get("position").and_then(parse_vec3))
        .or_else(|| value.get("translation").and_then(parse_vec3))
    {
        out.translation = pos;
    }
    if let Some(scale) = value
        .get("scale")
        .and_then(parse_vec3)
        .or_else(|| value.get("size").and_then(parse_vec3))
    {
        out.scale = scale.abs().max(Vec3::splat(0.01));
    }

    // Rotation: accept rot_quat_xyzw / quat_xyzw, or basis forward+up.
    let mut rotation: Option<Quat> = value
        .get("rot_quat_xyzw")
        .and_then(parse_quat_xyzw)
        .or_else(|| value.get("quat_xyzw").and_then(parse_quat_xyzw));
    if rotation.is_none() {
        if let Some(rot) = value.get("rot").and_then(|v| v.as_object()) {
            rotation = rot
                .get("quat_xyzw")
                .and_then(parse_quat_xyzw)
                .or_else(|| rot.get("rot_quat_xyzw").and_then(parse_quat_xyzw));
            if rotation.is_none() {
                if let Some(fwd) = rot.get("forward").and_then(parse_vec3) {
                    let up = rot.get("up").and_then(parse_vec3);
                    rotation = Some(super::convert::plan_rotation_from_forward_up(fwd, up));
                }
            }
        }
    }
    if rotation.is_none() {
        if let Some(fwd) = value.get("forward").and_then(parse_vec3) {
            let up = value.get("up").and_then(parse_vec3);
            rotation = Some(super::convert::plan_rotation_from_forward_up(fwd, up));
        }
    }
    if let Some(q) = rotation {
        out.rotation = q;
    }
    out
}

fn parse_agent_step(text: &str) -> Result<Gen3dAgentStepJsonV1, String> {
    let json_text = extract_json_object(text).unwrap_or_else(|| text.to_string());
    let json_text = json_text.trim();
    let mut step: Gen3dAgentStepJsonV1 =
        serde_json::from_str(json_text).map_err(|err| format!("Failed to parse JSON: {err}"))?;
    if step.version == 0 {
        step.version = 1;
    }
    if step.version != 1 {
        return Err(format!(
            "Unsupported gen3d_agent_step version {} (expected 1)",
            step.version
        ));
    }
    if step.actions.len() > 32 {
        step.actions.truncate(32);
    }
    Ok(step)
}

fn is_transient_openai_error_message(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if !lower.contains("openai") {
        return false;
    }
    lower.contains("http 429")
        || lower.contains("http 408")
        || lower.contains("http 409")
        || lower.contains("http 425")
        || lower.contains("http 502")
        || lower.contains("http 503")
        || lower.contains("http 504")
        || lower.contains("http 5")
        || lower.contains("status=5")
        || lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("failed to connect")
        || lower.contains("econnreset")
        || lower.contains("econnrefused")
}

fn poll_agent_step(
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
            if is_transient_openai_error_message(&err) {
                job.agent.step_request_retry_attempt =
                    job.agent.step_request_retry_attempt.saturating_add(1);
                let attempt = job.agent.step_request_retry_attempt;
                if attempt <= GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES {
                    workshop.status = format!(
                        "OpenAI request failed (attempt {attempt}/{GEN3D_AGENT_STEP_REQUEST_MAX_RETRIES}); retrying…"
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
                            "OpenAI transient failure after {attempt} retry attempt(s). Last error: {}",
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

fn execute_agent_actions(
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
                    .openai
                    .as_ref()
                    .map(|openai| !openai.base_url.starts_with("mock://gen3d"))
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
                if job.agent.rendered_since_last_review {
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
                let visual_qa_required = job
                    .openai
                    .as_ref()
                    .map(|openai| !openai.base_url.starts_with("mock://gen3d"))
                    .unwrap_or(true);
                if visual_qa_required {
                    if !job.agent.ever_rendered {
                        missing.push(TOOL_ID_RENDER_PREVIEW);
                    }
                    if !job.agent.ever_reviewed {
                        missing.push(TOOL_ID_LLM_REVIEW_DELTA);
                    }
                }
                if !job.agent.ever_validated {
                    missing.push(TOOL_ID_VALIDATE);
                }
                if !job.agent.ever_smoke_checked {
                    missing.push(TOOL_ID_SMOKE_CHECK);
                }
                if !missing.is_empty() {
                    let missing_list = missing.join(", ");
                    let qa_sequence = if visual_qa_required {
                        format!(
                            "{TOOL_ID_RENDER_PREVIEW} -> {TOOL_ID_LLM_REVIEW_DELTA} -> {TOOL_ID_VALIDATE} -> {TOOL_ID_SMOKE_CHECK}"
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

fn maybe_start_pass_snapshot_capture(
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

fn poll_agent_pass_snapshot_capture(
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

enum ToolCallOutcome {
    Immediate(Gen3dToolResultJsonV1),
    StartedAsync,
}

fn execute_tool_call(
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
    let registry = Gen3dToolRegistryV1::default();
    match call.tool_id.as_str() {
        TOOL_ID_LIST => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({ "tools": registry.list() }),
        )),
        TOOL_ID_DESCRIBE => {
            let tool_id = call
                .args
                .get("tool_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if tool_id.trim().is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing args.tool_id".into(),
                ));
            }
            let Some(desc) = registry.describe(&tool_id) else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown tool_id: {tool_id}"),
                ));
            };
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::to_value(desc).unwrap(),
            ))
        }
        TOOL_ID_GET_USER_INPUTS => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "prompt": job.user_prompt_raw,
                "images": job.user_images.iter().map(|p| p.display().to_string()).collect::<Vec<String>>(),
            }),
        )),
        TOOL_ID_GET_STATE_SUMMARY => ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            draft_summary(config, job),
        )),
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
        TOOL_ID_VALIDATE => {
            let json = super::build_gen3d_validate_results(&job.planned_components, draft);
            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "validate.json", &json);
            }
            job.agent.ever_validated = true;
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_SMOKE_CHECK => {
            let mut json = super::build_gen3d_smoke_results(
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

            // If motion validation is healthy, clear error counters so we don't disable channels
            // due to stale issues from earlier passes.
            if job.agent.last_motion_ok == Some(true) {
                job.agent.motion_error_counts.clear();
            }

            // Fallback policy: after repeated motion validation errors, disable only the failing
            // animation channels (identity loop) so the model stays usable and non-broken.
            const MOTION_FALLBACK_THRESHOLD: u8 = 2;
            let mut newly_applied: Vec<serde_json::Value> = Vec::new();
            if job.agent.ever_reviewed {
                let issues = json
                    .get("motion_validation")
                    .and_then(|v| v.get("issues"))
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();

                for issue in issues {
                    let Some(obj) = issue.as_object() else {
                        continue;
                    };
                    if obj.get("severity").and_then(|v| v.as_str()) != Some("error") {
                        continue;
                    }
                    let component_id = obj
                        .get("component_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let channel = obj
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let kind = obj
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if component_id.is_empty() || channel.is_empty() || kind.is_empty() {
                        continue;
                    }

                    let key = format!("{component_id}|{channel}|{kind}");
                    let count = job.agent.motion_error_counts.entry(key).or_insert(0);
                    *count = count.saturating_add(1);
                    if *count < MOTION_FALLBACK_THRESHOLD {
                        continue;
                    }

                    let disable_key = format!("{component_id}|{channel}");
                    if !job
                        .agent
                        .motion_fallbacks_applied
                        .insert(disable_key.clone())
                    {
                        continue;
                    }

                    match super::convert::disable_attachment_animation_channel_identity_loop(
                        &mut job.planned_components,
                        draft,
                        component_id,
                        channel,
                    ) {
                        Ok(true) => {
                            let action = serde_json::json!({
                                "action": "disable_channel_identity_loop",
                                "component_id": component_id,
                                "channel": channel,
                                "issue_kind": kind,
                                "threshold": MOTION_FALLBACK_THRESHOLD,
                            });
                            job.agent.motion_fallback_actions.push(action.clone());
                            newly_applied.push(action);
                        }
                        Ok(false) => {}
                        Err(err) => {
                            append_gen3d_run_log(
                                job.pass_dir.as_deref(),
                                format!(
                                    "motion_fallback_failed component_id={} channel={} err={}",
                                    component_id,
                                    channel,
                                    super::truncate_for_ui(&err, 240)
                                ),
                            );
                        }
                    }
                }
            }

            if !newly_applied.is_empty() {
                append_gen3d_run_log(
                    job.pass_dir.as_deref(),
                    format!("motion_fallback_applied actions={}", newly_applied.len()),
                );
                if let Some(dir) = job.pass_dir.as_deref() {
                    write_gen3d_json_artifact(
                        Some(dir),
                        "motion_fallback_actions.json",
                        &serde_json::Value::Array(job.agent.motion_fallback_actions.clone()),
                    );
                }

                // Re-run smoke checks so tool output reflects the post-fallback state.
                json = super::build_gen3d_smoke_results(
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
            }
            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(Some(dir), "smoke_results.json", &json);
            }
            job.agent.ever_smoke_checked = true;
            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(call.call_id, call.tool_id, json))
        }
        TOOL_ID_COPY_COMPONENT => {
            fn parse_vec3(value: &serde_json::Value) -> Option<Vec3> {
                let arr = value.as_array()?;
                if arr.len() != 3 {
                    return None;
                }
                let x = arr.get(0)?.as_f64()? as f32;
                let y = arr.get(1)?.as_f64()? as f32;
                let z = arr.get(2)?.as_f64()? as f32;
                let v = Vec3::new(x, y, z);
                v.is_finite().then_some(v)
            }

            fn parse_quat_xyzw(value: &serde_json::Value) -> Option<Quat> {
                let arr = value.as_array()?;
                if arr.len() != 4 {
                    return None;
                }
                let x = arr.get(0)?.as_f64()? as f32;
                let y = arr.get(1)?.as_f64()? as f32;
                let z = arr.get(2)?.as_f64()? as f32;
                let w = arr.get(3)?.as_f64()? as f32;
                let q = Quat::from_xyzw(x, y, z, w).normalize();
                q.is_finite().then_some(q)
            }

            fn parse_delta_transform(value: Option<&serde_json::Value>) -> Transform {
                let mut out = Transform::IDENTITY;
                let Some(value) = value else {
                    return out;
                };
                if let Some(pos) = value
                    .get("pos")
                    .and_then(parse_vec3)
                    .or_else(|| value.get("position").and_then(parse_vec3))
                    .or_else(|| value.get("translation").and_then(parse_vec3))
                {
                    out.translation = pos;
                }
                if let Some(scale) = value
                    .get("scale")
                    .and_then(parse_vec3)
                    .or_else(|| value.get("size").and_then(parse_vec3))
                {
                    out.scale = scale.abs().max(Vec3::splat(0.01));
                }

                // Rotation: accept rot_quat_xyzw / quat_xyzw, or basis forward+up.
                let mut rotation: Option<Quat> = value
                    .get("rot_quat_xyzw")
                    .and_then(parse_quat_xyzw)
                    .or_else(|| value.get("quat_xyzw").and_then(parse_quat_xyzw));
                if rotation.is_none() {
                    if let Some(rot) = value.get("rot").and_then(|v| v.as_object()) {
                        rotation = rot
                            .get("quat_xyzw")
                            .and_then(parse_quat_xyzw)
                            .or_else(|| rot.get("rot_quat_xyzw").and_then(parse_quat_xyzw));
                        if rotation.is_none() {
                            if let Some(fwd) = rot.get("forward").and_then(parse_vec3) {
                                let up = rot.get("up").and_then(parse_vec3);
                                rotation =
                                    Some(super::convert::plan_rotation_from_forward_up(fwd, up));
                            }
                        }
                    }
                }
                if rotation.is_none() {
                    if let Some(fwd) = value.get("forward").and_then(parse_vec3) {
                        let up = value.get("up").and_then(parse_vec3);
                        rotation = Some(super::convert::plan_rotation_from_forward_up(fwd, up));
                    }
                }
                if let Some(q) = rotation {
                    out.rotation = q;
                }
                out
            }

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
        TOOL_ID_COPY_COMPONENT_SUBTREE => {
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
                    delta,
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
            let Some(openai) = job.openai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing OpenAI config".into(),
                ));
            };
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };

            let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                Arc::new(Mutex::new(None));
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

            let user_text = super::prompts::build_gen3d_plan_user_text_with_hints(
                prompt_text,
                !job.user_images.is_empty(),
                workshop.speed_mode,
                style_hint,
                &required_component_names,
            );
            let reasoning_effort = super::openai::cap_reasoning_effort(
                &openai.model_reasoning_effort,
                &config.gen3d_reasoning_effort_plan,
            );
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::PlanV1),
                openai,
                reasoning_effort,
                system,
                user_text,
                job.user_images.clone(),
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

            let Some(openai) = job.openai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing OpenAI config".into(),
                ));
            };
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };

            let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                Arc::new(Mutex::new(None));
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Generating component…".into(),
            }));
            job.shared_progress = Some(progress.clone());

            let system = super::prompts::build_gen3d_component_system_instructions();
            let user_text = super::prompts::build_gen3d_component_user_text(
                &job.user_prompt_raw,
                !job.user_images.is_empty(),
                workshop.speed_mode,
                &job.assembly_notes,
                &job.planned_components,
                idx,
            );
            job.agent.pending_llm_repair_attempt = 0;
            let reasoning_effort = super::openai::cap_reasoning_effort(
                &openai.model_reasoning_effort,
                &config.gen3d_reasoning_effort_component,
            );
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
                openai,
                reasoning_effort,
                system,
                user_text,
                job.user_images.clone(),
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
            let Some(_openai) = job.openai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing OpenAI config".into(),
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
            let missing_only = missing_only_arg.unwrap_or(requested_indices.is_empty());
            let mut optimized_by_reuse_groups = false;
            let mut skipped_due_to_reuse_groups: Vec<usize> = Vec::new();

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
                requested_indices.retain(|&idx| {
                    job.planned_components
                        .get(idx)
                        .map(|c| c.actual_size.is_none())
                        .unwrap_or(false)
                });
            }

            if requested_indices.is_empty() {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "requested": 0,
                        "succeeded": 0,
                        "failed": [],
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
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                    call.call_id,
                    call.tool_id,
                    serde_json::json!({
                        "requested": 0,
                        "succeeded": 0,
                        "failed": [],
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
            let Some(openai) = job.openai.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing OpenAI config".into(),
                ));
            };
            let Some(pass_dir) = job.pass_dir.clone() else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    "Missing pass dir".into(),
                ));
            };

            let mut preview_images = parse_review_preview_images_from_args(&call.args);
            let preview_images_were_explicit = !preview_images.is_empty();
            if preview_images.is_empty() {
                preview_images = job.agent.last_render_images.clone();
            }
            let include_original_images = call
                .args
                .get("include_original_images")
                .and_then(|v| v.as_bool())
                .unwrap_or(true);

            let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
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

            let motion_ok = smoke_results
                .get("motion_validation")
                .and_then(|v| v.get("ok"))
                .and_then(|v| v.as_bool())
                .unwrap_or(true);
            let include_motion_sheets = !motion_ok;

            if !preview_images_were_explicit {
                preview_images =
                    select_review_preview_images(&preview_images, include_motion_sheets);
            }

            let mut images_to_send: Vec<PathBuf> = Vec::new();
            if include_original_images {
                images_to_send.extend(job.user_images.clone());
            }
            images_to_send.extend(preview_images);
            if images_to_send.len() > GEN3D_MAX_REQUEST_IMAGES {
                images_to_send.truncate(GEN3D_MAX_REQUEST_IMAGES);
            }

            if let Some(dir) = job.pass_dir.as_deref() {
                write_gen3d_json_artifact(
                    Some(dir),
                    "scene_graph_summary.json",
                    &scene_graph_summary,
                );
                write_gen3d_json_artifact(Some(dir), "smoke_results.json", &smoke_results);
            }

            let system = super::prompts::build_gen3d_review_delta_system_instructions();
            let user_text = super::prompts::build_gen3d_review_delta_user_text(
                &run_id,
                job.attempt,
                &job.plan_hash,
                job.assembly_rev,
                &job.user_prompt_raw,
                !job.user_images.is_empty(),
                &scene_graph_summary,
                &smoke_results,
            );

            let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
                Arc::new(Mutex::new(None));
            job.shared_result = Some(shared.clone());
            let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
                message: "Reviewing…".into(),
            }));
            job.shared_progress = Some(progress.clone());
            set_progress(&progress, "Calling model for review delta…");
            job.agent.pending_llm_repair_attempt = 0;

            let reasoning_effort = super::openai::cap_reasoning_effort(
                &openai.model_reasoning_effort,
                &config.gen3d_reasoning_effort_review,
            );
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.session.clone(),
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ReviewDeltaV1),
                openai,
                reasoning_effort,
                system,
                user_text,
                images_to_send,
                pass_dir,
                sanitize_prefix(&format!("tool_review_{}", &call.call_id)),
            );
            job.agent.pending_tool_call = Some(call);
            job.agent.pending_llm_tool = Some(super::Gen3dAgentLlmToolKind::ReviewDelta);
            job.phase = Gen3dAiPhase::AgentWaitingTool;
            ToolCallOutcome::StartedAsync
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

            let source_defs = if from == job.agent.active_workspace_id {
                draft.defs.clone()
            } else if let Some(ws) = job.agent.workspaces.get(&from) {
                ws.defs.clone()
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
            if prev != "main" || !draft.defs.is_empty() {
                job.agent.workspaces.insert(
                    prev.clone(),
                    super::Gen3dAgentWorkspace {
                        name: prev.clone(),
                        defs: draft.defs.clone(),
                    },
                );
            }

            let next_defs = if workspace_id == "main" {
                job.agent
                    .workspaces
                    .get("main")
                    .map(|ws| ws.defs.clone())
                    .unwrap_or_default()
            } else if let Some(ws) = job.agent.workspaces.get(&workspace_id) {
                ws.defs.clone()
            } else {
                return ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::err(
                    call.call_id,
                    call.tool_id,
                    format!("Unknown workspace `{workspace_id}`"),
                ));
            };

            draft.defs = next_defs;
            job.agent.active_workspace_id = workspace_id;

            ToolCallOutcome::Immediate(Gen3dToolResultJsonV1::ok(
                call.call_id,
                call.tool_id,
                serde_json::json!({ "ok": true }),
            ))
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

fn poll_agent_tool(
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

        let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
            Arc::new(Mutex::new(None));
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
            super::Gen3dAgentLlmToolKind::ReviewDelta => {
                Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ReviewDeltaV1)
            }
        };
        spawn_gen3d_ai_text_thread(
            shared,
            progress,
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
                                        .map(|v| v.abs())
                                        .filter(|v| *v > 1e-3);
                                    job.plan_collider = plan.collider;
                                    draft.defs = defs;
                                    job.agent.workspaces.clear();
                                    job.agent.active_workspace_id = "main".to_string();
                                    job.agent.next_workspace_seq = 1;
                                    job.agent.rendered_since_last_review = false;
                                    job.agent.last_render_images.clear();
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
                                            let system = super::prompts::build_gen3d_review_delta_system_instructions();
                                            let user_text =
                                                super::prompts::build_gen3d_review_delta_user_text(
                                                    &run_id,
                                                    job.attempt,
                                                    &job.plan_hash,
                                                    job.assembly_rev,
                                                    &job.user_prompt_raw,
                                                    !job.user_images.is_empty(),
                                                    &scene_graph_summary,
                                                &smoke_results,
                                            );

                                            let mut preview_images =
                                                parse_review_preview_images_from_args(&call.args);
                                            if preview_images.is_empty() {
                                                preview_images =
                                                    job.agent.last_render_images.clone();
                                            }
                                            let include_original_images = call
                                                .args
                                                .get("include_original_images")
                                                .and_then(|v| v.as_bool())
                                                .unwrap_or(true);
                                            let mut images_to_send: Vec<PathBuf> = Vec::new();
                                            if include_original_images {
                                                images_to_send.extend(job.user_images.clone());
                                            }
                                            images_to_send.extend(preview_images);
                                            if images_to_send.len() > GEN3D_MAX_REQUEST_IMAGES {
                                                images_to_send.truncate(GEN3D_MAX_REQUEST_IMAGES);
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
                                let system =
                                    super::prompts::build_gen3d_review_delta_system_instructions();
                                let user_text = super::prompts::build_gen3d_review_delta_user_text(
                                    &run_id,
                                    job.attempt,
                                    &job.plan_hash,
                                    job.assembly_rev,
                                    &job.user_prompt_raw,
                                    !job.user_images.is_empty(),
                                    &scene_graph_summary,
                                    &smoke_results,
                                );

                                let mut preview_images =
                                    parse_review_preview_images_from_args(&call.args);
                                if preview_images.is_empty() {
                                    preview_images = job.agent.last_render_images.clone();
                                }
                                let include_original_images = call
                                    .args
                                    .get("include_original_images")
                                    .and_then(|v| v.as_bool())
                                    .unwrap_or(true);
                                let mut images_to_send: Vec<PathBuf> = Vec::new();
                                if include_original_images {
                                    images_to_send.extend(job.user_images.clone());
                                }
                                images_to_send.extend(preview_images);
                                if images_to_send.len() > GEN3D_MAX_REQUEST_IMAGES {
                                    images_to_send.truncate(GEN3D_MAX_REQUEST_IMAGES);
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

fn poll_agent_component_batch(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: super::super::state::Gen3dSpeedMode,
) -> Option<Gen3dToolResultJsonV1> {
    const MAX_COMPONENT_RETRIES: u8 = 1;

    let Some(mut batch) = job.agent.pending_component_batch.take() else {
        fail_job(
            workshop,
            job,
            "Internal error: missing pending component batch state.",
        );
        return None;
    };
    let Some(call_id_for_prefix) = job
        .agent
        .pending_tool_call
        .as_ref()
        .map(|c| c.call_id.clone())
    else {
        fail_job(workshop, job, "Internal error: missing pending tool call.");
        return None;
    };

    let total = batch.requested_indices.len();
    if total == 0 {
        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();
        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": 0,
                "succeeded": 0,
                "failed": [],
                "skipped_due_to_regen_budget": batch.skipped_due_to_regen_budget,
            }),
        ));
    }

    // 1) Apply any completed component results.
    let mut i = 0usize;
    while i < job.component_in_flight.len() {
        let maybe_result = {
            let task = &job.component_in_flight[i];
            let Ok(mut guard) = task.shared_result.lock() else {
                i += 1;
                continue;
            };
            guard.take()
        };
        let Some(result) = maybe_result else {
            i += 1;
            continue;
        };

        let task = job.component_in_flight.swap_remove(i);
        let idx = task.idx;
        let component_name = job
            .planned_components
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("component_{idx}"));

        match result {
            Ok(resp) => {
                debug!(
                    "Gen3D batch: component finished (idx={}, name={}, api={:?}, sent_images={})",
                    idx, component_name, resp.api, task.sent_images
                );
                job.note_api_used(resp.api);
                if let Some(tokens) = resp.total_tokens {
                    job.add_tokens(tokens);
                }
                if let Some(flag) = resp.session.responses_supported {
                    job.session.responses_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_continuation_supported {
                    job.session.responses_continuation_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_background_supported {
                    job.session.responses_background_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_structured_outputs_supported {
                    job.session.responses_structured_outputs_supported = Some(flag);
                }
                if let Some(flag) = resp.session.chat_structured_outputs_supported {
                    job.session.chat_structured_outputs_supported = Some(flag);
                }

                let ai = match parse::parse_ai_draft_from_text(&resp.text) {
                    Ok(ai) => ai,
                    Err(err) => {
                        if task.attempt < MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: component parse failed; retrying (idx={}, name={}, attempt {}/{}) err={}",
                                idx,
                                component_name,
                                next + 1,
                                MAX_COMPONENT_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        batch.completed_indices.insert(idx);
                        batch.failed.push(super::Gen3dComponentBatchFailure {
                            index: idx,
                            name: component_name,
                            error: err,
                        });
                        continue;
                    }
                };

                let component_def = match job
                    .planned_components
                    .get(idx)
                    .ok_or_else(|| {
                        format!("Internal error: missing planned component for idx={idx}")
                    })
                    .and_then(|planned| super::convert::ai_to_component_def(planned, ai))
                {
                    Ok(def) => def,
                    Err(err) => {
                        if task.attempt < MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D batch: component convert failed; retrying (idx={}, name={}, attempt {}/{}) err={}",
                                idx,
                                component_name,
                                next + 1,
                                MAX_COMPONENT_RETRIES + 1,
                                super::truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        batch.completed_indices.insert(idx);
                        batch.failed.push(super::Gen3dComponentBatchFailure {
                            index: idx,
                            name: component_name,
                            error: err,
                        });
                        continue;
                    }
                };

                if let Some(comp) = job.planned_components.get_mut(idx) {
                    comp.actual_size = Some(component_def.size);
                    comp.anchors = component_def.anchors.clone();
                }

                // Replace component def in-place, preserving existing object refs.
                let target_id = component_def.object_id;
                if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
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
                    let mut merged = component_def;
                    merged.parts.extend(preserved_refs);
                    *existing = merged;
                } else {
                    draft.defs.push(component_def);
                }

                if let Some(root_idx) = job
                    .planned_components
                    .iter()
                    .position(|c| c.attach_to.is_none())
                {
                    if let Err(err) = super::convert::resolve_planned_component_transforms(
                        &mut job.planned_components,
                        root_idx,
                    ) {
                        warn!("Gen3D batch: failed to resolve transforms: {err}");
                    }
                }
                super::convert::update_root_def_from_planned_components(
                    &job.planned_components,
                    &job.plan_collider,
                    draft,
                );
                write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
                job.assembly_rev = job.assembly_rev.saturating_add(1);

                batch.completed_indices.insert(idx);
            }
            Err(err) => {
                if task.attempt < MAX_COMPONENT_RETRIES {
                    let next = task.attempt + 1;
                    warn!(
                        "Gen3D batch: component request failed; retrying (idx={}, name={}, attempt {}/{}, sent_images={}) err={}",
                        idx,
                        component_name,
                        next + 1,
                        MAX_COMPONENT_RETRIES + 1,
                        task.sent_images,
                        super::truncate_for_ui(&err, 600),
                    );
                    if idx >= job.component_attempts.len() {
                        job.component_attempts
                            .resize(job.planned_components.len(), 0);
                    }
                    job.component_attempts[idx] = next;
                    job.component_queue.insert(0, idx);
                    continue;
                }
                batch.completed_indices.insert(idx);
                batch.failed.push(super::Gen3dComponentBatchFailure {
                    index: idx,
                    name: component_name,
                    error: err,
                });
            }
        }
    }

    // 2) Start new component requests up to the parallel limit.
    let mut parallel = job.max_parallel_components.max(1).min(total);
    if job.session.responses_previous_id.is_some()
        && job.session.responses_continuation_supported.is_none()
    {
        parallel = parallel.min(1);
    }
    while job.component_in_flight.len() < parallel && !job.component_queue.is_empty() {
        let idx = job.component_queue.remove(0);
        if idx >= job.planned_components.len() {
            continue;
        }
        if batch.completed_indices.contains(&idx) {
            continue;
        }

        let Some(openai) = job.openai.clone() else {
            fail_job(workshop, job, "Internal error: missing OpenAI config.");
            return None;
        };
        let Some(pass_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return None;
        };

        let attempt = *job.component_attempts.get(idx).unwrap_or(&0);
        let sent_images = !job.user_images.is_empty();
        let image_paths = if sent_images {
            job.user_images.clone()
        } else {
            Vec::new()
        };

        let shared: Arc<Mutex<Option<Result<Gen3dAiTextResponse, String>>>> =
            Arc::new(Mutex::new(None));
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let system = super::prompts::build_gen3d_component_system_instructions();
        let user_text = super::prompts::build_gen3d_component_user_text(
            &job.user_prompt_raw,
            !job.user_images.is_empty(),
            speed,
            &job.assembly_notes,
            &job.planned_components,
            idx,
        );

        let component_name = job.planned_components[idx].name.as_str();
        let prefix = if attempt == 0 {
            format!(
                "tool_component_batch_{call_id}_c{:02}_{}",
                idx + 1,
                component_name,
                call_id = call_id_for_prefix.as_str()
            )
        } else {
            format!(
                "tool_component_batch_{call_id}_c{:02}_{}_retry{}",
                idx + 1,
                component_name,
                attempt,
                call_id = call_id_for_prefix.as_str()
            )
        };

        let reasoning_effort = super::openai::cap_reasoning_effort(
            &openai.model_reasoning_effort,
            &config.gen3d_reasoning_effort_component,
        );
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.session.clone(),
            Some(super::structured_outputs::Gen3dAiJsonSchemaKind::ComponentDraftV1),
            openai,
            reasoning_effort,
            system,
            user_text,
            image_paths,
            pass_dir,
            sanitize_prefix(&prefix),
        );

        job.component_in_flight.push(super::Gen3dInFlightComponent {
            idx,
            attempt,
            sent_images,
            shared_result: shared,
            _progress: progress,
        });
    }

    let done = batch.completed_indices.len();
    let in_flight = job.component_in_flight.len();
    let pending = job.component_queue.len();
    workshop.status = format!(
        "Generating components (batch)… ({done}/{total})\nIn flight: {in_flight} | pending: {pending}\nParallel: {parallel}"
    );
    if let Some(progress) = job.shared_progress.as_ref() {
        set_progress(
            progress,
            format!("Generating components (batch)… ({done}/{total})"),
        );
    }

    if done == total && in_flight == 0 && pending == 0 {
        let succeeded = total.saturating_sub(batch.failed.len());
        let failed_json: Vec<serde_json::Value> = batch
            .failed
            .iter()
            .map(|f| {
                serde_json::json!({
                    "index": f.index,
                    "name": f.name,
                    "error": super::truncate_for_ui(&f.error, 600),
                })
            })
            .collect();

        let mut auto_copy = super::reuse_groups::apply_auto_copy(
            &mut job.planned_components,
            draft,
            &job.reuse_groups,
        );
        if auto_copy.component_copies_applied > 0 {
            if let Some(root_idx) = job
                .planned_components
                .iter()
                .position(|c| c.attach_to.is_none())
            {
                if let Err(err) = super::convert::resolve_planned_component_transforms(
                    &mut job.planned_components,
                    root_idx,
                ) {
                    auto_copy.errors.push(format!(
                        "auto_copy: failed to resolve transforms after copy: {err}"
                    ));
                }
            }
            super::convert::update_root_def_from_planned_components(
                &job.planned_components,
                &job.plan_collider,
                draft,
            );
            write_gen3d_assembly_snapshot(job.pass_dir.as_deref(), &job.planned_components);
            job.assembly_rev = job.assembly_rev.saturating_add(1);
        }

        let mut fallback_component_indices: Vec<usize> = auto_copy
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
        fallback_component_indices.sort_unstable();
        fallback_component_indices.dedup();

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

            append_gen3d_run_log(
                job.pass_dir.as_deref(),
                format!(
                    "auto_copy_preflight_fallback components={:?}",
                    fallback_component_indices
                ),
            );
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

        if let Some(pass_dir) = job.pass_dir.as_deref() {
            if auto_copy.component_copies_applied > 0
                || !auto_copy.errors.is_empty()
                || !auto_copy.preflight_mismatches.is_empty()
                || !fallback_components_json.is_empty()
            {
                let mut outcomes_json: Vec<serde_json::Value> = Vec::new();
                const MAX_OUTCOMES: usize = 16;
                for outcome in auto_copy.outcomes.iter().take(MAX_OUTCOMES) {
                    let mode = match outcome.mode_used {
                        super::copy_component::Gen3dCopyMode::Detached => "detached",
                        super::copy_component::Gen3dCopyMode::Linked => "linked",
                    };
                    outcomes_json.push(serde_json::json!({
                        "source": outcome.source_component_name.as_str(),
                        "target": outcome.target_component_name.as_str(),
                        "mode": mode,
                    }));
                }
                let outcomes_omitted = auto_copy.outcomes.len().saturating_sub(MAX_OUTCOMES);
                write_gen3d_json_artifact(
                    Some(pass_dir),
                    "auto_copy.json",
                    &serde_json::json!({
                        "version": 1,
                        "enabled": auto_copy.enabled,
                        "component_copies_applied": auto_copy.component_copies_applied,
                        "subtree_copies_applied": auto_copy.subtree_copies_applied,
                        "targets_skipped_already_generated": auto_copy.targets_skipped_already_generated,
                        "subtrees_skipped_partially_generated": auto_copy.subtrees_skipped_partially_generated,
                        "preflight_mismatches": &auto_copy.preflight_mismatches,
                        "fallback_component_indices": &fallback_component_indices,
                        "fallback_components": &fallback_components_json,
                        "errors": &auto_copy.errors,
                        "outcomes": outcomes_json,
                        "outcomes_omitted": outcomes_omitted,
                    }),
                );
            }
        }

        let mut outcomes_json: Vec<serde_json::Value> = Vec::new();
        const MAX_OUTCOMES: usize = 16;
        for outcome in auto_copy.outcomes.iter().take(MAX_OUTCOMES) {
            let mode = match outcome.mode_used {
                super::copy_component::Gen3dCopyMode::Detached => "detached",
                super::copy_component::Gen3dCopyMode::Linked => "linked",
            };
            outcomes_json.push(serde_json::json!({
                "source": outcome.source_component_name.as_str(),
                "target": outcome.target_component_name.as_str(),
                "mode": mode,
            }));
        }
        let outcomes_omitted = auto_copy.outcomes.len().saturating_sub(MAX_OUTCOMES);

        let skipped_due_to_reuse_groups_json: Vec<serde_json::Value> = batch
            .skipped_due_to_reuse_groups
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

        let call = job.agent.pending_tool_call.take().unwrap();
        job.agent.pending_llm_tool = None;
        job.agent.pending_component_batch = None;
        job.component_queue.clear();
        job.component_in_flight.clear();

        return Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "requested": total,
                "succeeded": succeeded,
                "failed": failed_json,
                "optimized_by_reuse_groups": batch.optimized_by_reuse_groups,
                "skipped_due_to_reuse_groups": skipped_due_to_reuse_groups_json,
                "skipped_due_to_regen_budget": batch.skipped_due_to_regen_budget,
                "auto_copy": {
                    "enabled": auto_copy.enabled,
                    "component_copies_applied": auto_copy.component_copies_applied,
                    "subtree_copies_applied": auto_copy.subtree_copies_applied,
                    "targets_skipped_already_generated": auto_copy.targets_skipped_already_generated,
                    "subtrees_skipped_partially_generated": auto_copy.subtrees_skipped_partially_generated,
                    "preflight_mismatches": auto_copy.preflight_mismatches,
                    "fallback_component_indices": fallback_component_indices,
                    "fallback_components": fallback_components_json,
                    "errors": auto_copy.errors,
                    "outcomes": outcomes_json,
                    "outcomes_omitted": outcomes_omitted,
                },
            }),
        ));
    }

    job.agent.pending_component_batch = Some(batch);
    None
}

fn poll_agent_render_capture(
    _config: &AppConfig,
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
    fn finish(
        workshop: &mut Gen3dWorkshop,
        job: &mut Gen3dAiJob,
        paths: Vec<PathBuf>,
    ) -> Option<Gen3dToolResultJsonV1> {
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

        let Some(call) = job.agent.pending_tool_call.take() else {
            fail_job(workshop, job, "Internal error: missing pending tool call");
            return None;
        };
        Some(Gen3dToolResultJsonV1::ok(
            call.call_id,
            call.tool_id,
            serde_json::json!({
                "images": paths.iter().map(|p| p.display().to_string()).collect::<Vec<String>>(),
            }),
        ))
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
        let Some(result) = finish(workshop, job, paths) else {
            return;
        };
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

    let Some(result) = finish(workshop, job, paths) else {
        return;
    };
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

fn note_observable_tool_result(job: &mut Gen3dAiJob, result: &Gen3dToolResultJsonV1) {
    if !result.ok {
        return;
    }

    if matches!(
        result.tool_id.as_str(),
        TOOL_ID_RENDER_PREVIEW | TOOL_ID_VALIDATE | TOOL_ID_SMOKE_CHECK
    ) {
        job.agent.step_had_observable_output = true;
    }
}

fn compute_agent_state_hash(job: &Gen3dAiJob, draft: &Gen3dDraft) -> String {
    let summary = super::build_gen3d_scene_graph_summary(
        "",
        0,
        0,
        &job.plan_hash,
        // No-progress guard should reflect *actual* assembly state, not revision counters.
        // Some tool results can bump `assembly_rev` even when the assembled draft doesn't change.
        0,
        &job.planned_components,
        draft,
    );
    let text = serde_json::to_string(&summary).unwrap_or_else(|_| summary.to_string());
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("sha256:{hex}")
}

fn build_component_subset_workspace_defs(
    source_defs: &[crate::object::registry::ObjectDef],
    include_components: &[String],
) -> Result<Vec<crate::object::registry::ObjectDef>, String> {
    use crate::object::registry::{AttachmentDef, ObjectDef, ObjectPartDef, ObjectPartKind};

    let root_id = super::super::gen3d_draft_object_id();
    let mut by_id: std::collections::HashMap<u128, ObjectDef> = std::collections::HashMap::new();
    for def in source_defs.iter().cloned() {
        by_id.insert(def.object_id, def);
    }

    let mut roots: Vec<u128> = Vec::new();
    for name in include_components {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        let object_id = crate::object::registry::builtin_object_id(&format!(
            "gravimera/gen3d/component/{name}"
        ));
        if !by_id.contains_key(&object_id) {
            return Err(format!(
                "Unknown component `{name}` (no matching object def in source draft)."
            ));
        }
        roots.push(object_id);
    }
    if roots.is_empty() {
        return Ok(source_defs.to_vec());
    }

    // Collect reachable defs from the requested roots.
    let mut reachable: std::collections::HashSet<u128> = std::collections::HashSet::new();
    let mut stack: Vec<u128> = roots.clone();
    while let Some(id) = stack.pop() {
        if !reachable.insert(id) {
            continue;
        }
        let Some(def) = by_id.get(&id) else {
            continue;
        };
        for part in def.parts.iter() {
            if let ObjectPartKind::ObjectRef { object_id } = part.kind {
                stack.push(object_id);
            }
        }
    }

    // Lay out the requested roots side-by-side so the agent can compare multiple variants.
    let margin = 0.6f32;
    let mut centers: Vec<f32> = Vec::with_capacity(roots.len());
    let mut cursor_x = 0.0f32;
    for root in &roots {
        let size = by_id
            .get(root)
            .map(|d| d.size)
            .unwrap_or(Vec3::ONE)
            .abs()
            .max(Vec3::splat(0.01));
        let half_x = size.x * 0.5;
        cursor_x += half_x;
        centers.push(cursor_x);
        cursor_x += half_x + margin;
    }

    // Recenter layout around origin.
    let mut min_x = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    for (idx, root) in roots.iter().enumerate() {
        let size = by_id
            .get(root)
            .map(|d| d.size)
            .unwrap_or(Vec3::ONE)
            .abs()
            .max(Vec3::splat(0.01));
        let half_x = size.x * 0.5;
        let center_x = centers.get(idx).copied().unwrap_or(0.0);
        min_x = min_x.min(center_x - half_x);
        max_x = max_x.max(center_x + half_x);
    }
    let shift_x = (min_x + max_x) * 0.5;
    for x in centers.iter_mut() {
        *x -= shift_x;
    }

    let mut root_parts: Vec<ObjectPartDef> = Vec::with_capacity(roots.len());
    for (idx, object_id) in roots.iter().copied().enumerate() {
        let x = centers.get(idx).copied().unwrap_or(0.0);
        let attachment = AttachmentDef {
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
        };
        let part = ObjectPartDef::object_ref(
            object_id,
            Transform::from_translation(Vec3::new(x, 0.0, 0.0)),
        )
        .with_attachment(attachment);
        root_parts.push(part);
    }

    // Root def: keep it simple for preview; disable mobility/attack/collider.
    let mut root_def = by_id.remove(&root_id).unwrap_or_else(|| ObjectDef {
        object_id: root_id,
        label: "gen3d_draft".into(),
        size: Vec3::ONE,
        collider: crate::object::registry::ColliderProfile::None,
        interaction: crate::object::registry::ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    });
    root_def.parts = root_parts;
    root_def.mobility = None;
    root_def.attack = None;
    root_def.collider = crate::object::registry::ColliderProfile::None;

    // Size: approximate from included children.
    let mut max_y = 0.0f32;
    let mut max_z = 0.0f32;
    for id in reachable.iter() {
        if let Some(def) = by_id.get(id) {
            let size = def.size.abs().max(Vec3::splat(0.01));
            max_y = max_y.max(size.y);
            max_z = max_z.max(size.z);
        }
    }
    let width = (max_x - min_x).abs().max(0.1);
    root_def.size = Vec3::new(width, max_y.max(0.1), max_z.max(0.1));

    // Final defs list: reachable components + root.
    let mut out: Vec<ObjectDef> = Vec::new();
    out.reserve(reachable.len() + 1);
    for (id, def) in by_id.into_iter() {
        if id == root_id {
            continue;
        }
        if reachable.contains(&id) {
            out.push(def);
        }
    }
    out.push(root_def);

    Ok(out)
}

fn sanitize_prefix(prefix: &str) -> String {
    let mut out = String::new();
    for ch in prefix.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-') {
            out.push(ch);
        } else {
            out.push('_');
        }
        if out.len() >= 48 {
            break;
        }
    }
    if out.is_empty() {
        "artifact".into()
    } else {
        out
    }
}

fn truncate_json_for_log(value: &serde_json::Value, max_chars: usize) -> String {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".into());
    let mut out = String::new();
    for ch in text.chars() {
        if ch == '\n' || ch == '\r' || ch == '\t' {
            out.push(' ');
        } else {
            out.push(ch);
        }
        if out.chars().count() >= max_chars {
            out.push_str("…");
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AppConfig, OpenAiConfig};
    use crate::gen3d::state::{Gen3dDraft, Gen3dPreview, Gen3dSpeedMode, Gen3dWorkshop};
    use crate::gen3d::tool_feedback::Gen3dToolFeedbackHistory;
    use uuid::Uuid;

    #[test]
    fn select_review_preview_images_prefers_five_static_views() {
        let images = vec![
            PathBuf::from("render_front.png"),
            PathBuf::from("render_left_back.png"),
            PathBuf::from("render_right_back.png"),
            PathBuf::from("render_top.png"),
            PathBuf::from("render_bottom.png"),
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, false);
        assert_eq!(
            selected,
            vec![
                PathBuf::from("render_front.png"),
                PathBuf::from("render_left_back.png"),
                PathBuf::from("render_right_back.png"),
                PathBuf::from("render_top.png"),
                PathBuf::from("render_bottom.png"),
            ]
        );
    }

    #[test]
    fn select_review_preview_images_includes_motion_sheets_when_requested() {
        let images = vec![
            PathBuf::from("render_front.png"),
            PathBuf::from("render_left_back.png"),
            PathBuf::from("render_right_back.png"),
            PathBuf::from("render_top.png"),
            PathBuf::from("render_bottom.png"),
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, true);
        assert_eq!(
            selected,
            vec![
                PathBuf::from("render_front.png"),
                PathBuf::from("render_left_back.png"),
                PathBuf::from("render_right_back.png"),
                PathBuf::from("render_top.png"),
                PathBuf::from("render_bottom.png"),
                PathBuf::from("move_sheet.png"),
                PathBuf::from("attack_sheet.png"),
            ]
        );
    }

    #[test]
    fn select_review_preview_images_falls_back_when_only_motion_present() {
        let images = vec![
            PathBuf::from("move_sheet.png"),
            PathBuf::from("attack_sheet.png"),
        ];
        let selected = select_review_preview_images(&images, false);
        assert_eq!(selected, images);
    }

    #[test]
    fn gen3d_no_progress_state_hash_ignores_assembly_rev_churn() {
        let mut job = Gen3dAiJob::default();
        job.plan_hash = "sha256:deadbeef".to_string();

        let draft = Gen3dDraft::default();

        job.assembly_rev = 1;
        let h1 = compute_agent_state_hash(&job, &draft);
        job.assembly_rev = 999;
        let h2 = compute_agent_state_hash(&job, &draft);

        assert_eq!(h1, h2, "state hash should ignore assembly_rev");
    }

    #[test]
    fn gen3d_agent_state_summary_exposes_budget_remaining() {
        let mut config = AppConfig::default();
        config.gen3d_max_seconds = 200;
        config.gen3d_max_tokens = 2000;
        config.gen3d_no_progress_max_steps = 12;
        config.gen3d_max_regen_total = 16;
        config.gen3d_max_regen_per_component = 2;

        let mut job = Gen3dAiJob::default();
        job.running = false;
        job.last_run_elapsed = Some(std::time::Duration::from_secs_f64(100.25));
        job.current_run_tokens = 1234;
        job.regen_total = 15;
        job.regen_per_component = vec![2, 1];
        job.agent.no_progress_steps = 5;
        job.planned_components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "A".into(),
                name: "a".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "B".into(),
                name: "b".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        let summary = draft_summary(&config, &job);
        let budgets = summary
            .get("budgets")
            .and_then(|v| v.as_object())
            .expect("expected budgets object in state summary");

        let regen_remaining = budgets
            .get("regen")
            .and_then(|v| v.get("remaining_total"))
            .and_then(|v| v.as_u64())
            .expect("expected regen remaining_total");
        assert_eq!(regen_remaining, 1);

        let no_progress_remaining = budgets
            .get("no_progress")
            .and_then(|v| v.get("remaining_steps"))
            .and_then(|v| v.as_u64())
            .expect("expected no_progress remaining_steps");
        assert_eq!(no_progress_remaining, 7);

        let token_remaining = budgets
            .get("tokens")
            .and_then(|v| v.get("remaining_run_tokens"))
            .and_then(|v| v.as_u64())
            .expect("expected tokens remaining_run_tokens");
        assert_eq!(token_remaining, 766);

        let elapsed = budgets
            .get("time")
            .and_then(|v| v.get("elapsed_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time elapsed_seconds");
        assert!(
            (elapsed - 100.3).abs() < 1e-6,
            "unexpected elapsed={elapsed}"
        );

        let remaining = budgets
            .get("time")
            .and_then(|v| v.get("remaining_seconds"))
            .and_then(|v| v.as_f64())
            .expect("expected time remaining_seconds");
        assert!(
            (remaining - 99.8).abs() < 1e-6,
            "unexpected remaining={remaining}"
        );

        let components = summary
            .get("components")
            .and_then(|v| v.as_array())
            .expect("expected components array");
        assert_eq!(
            components[0]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(0)
        );
        assert_eq!(
            components[0]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(true)
        );
        assert_eq!(
            components[1]
                .get("regen_remaining")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            components[1]
                .get("regen_budget_blocked")
                .and_then(|v| v.as_bool()),
            Some(false)
        );
    }

    #[test]
    fn gen3d_agent_step_prompt_compacts_large_tool_payloads() {
        let config = AppConfig::default();
        let mut job = Gen3dAiJob::default();
        job.user_prompt_raw = "test prompt".into();

        let workshop = Gen3dWorkshop::default();
        let state_summary = serde_json::json!({"run_id":"test"});
        let registry = crate::gen3d::agent::tools::Gen3dToolRegistryV1::default();

        let huge = "x".repeat(50_000);
        let tool_result = Gen3dToolResultJsonV1::ok(
            "call_0".into(),
            TOOL_ID_DESCRIBE.into(),
            serde_json::json!({
                "tool_id": "render_preview_v1",
                "one_line_summary": huge,
                "args_example": {"foo": huge},
                "description": huge,
            }),
        );

        let prompt = build_agent_user_text(
            &config,
            &job,
            &workshop,
            state_summary,
            &[tool_result],
            &registry,
        );

        // This prompt is written to disk each pass; keep it comfortably small so later passes don't balloon.
        assert!(
            prompt.len() < 24_000,
            "agent_step prompt unexpectedly large: {} bytes",
            prompt.len()
        );
        assert!(
            prompt.contains("…(truncated)"),
            "expected truncation marker in compact prompt summary"
        );
    }

    #[test]
    fn gen3d_component_name_hint_resolves_common_aliases() {
        let components = vec![
            super::super::Gen3dPlannedComponent {
                display_name: "Turret Base".into(),
                name: "turret_base".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
            super::super::Gen3dPlannedComponent {
                display_name: "Cannon Barrel".into(),
                name: "cannon_barrel".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: None,
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            },
        ];

        assert_eq!(
            resolve_component_index_by_name_hint(&components, "roof_turret_base"),
            Some(0)
        );
        assert_eq!(
            resolve_component_index_by_name_hint(&components, "cannon_weapon"),
            Some(1)
        );
    }

    #[test]
    fn gen3d_review_preview_image_args_ignore_tool_placeholders() {
        let args = serde_json::json!({
            "preview_images": [
                "$CALL_1.images[0]",
                "$CALL_2.render_paths[0]",
            ]
        });
        let paths = parse_review_preview_images_from_args(&args);
        assert!(
            paths.is_empty(),
            "expected placeholder-only paths to be ignored"
        );
    }

    #[test]
    fn gen3d_mock_agent_builds_warcar_prompt_end_to_end() {
        let prompt = "A warcar with a cannon as weapon";

        let run_id = Uuid::new_v4();
        let base_dir = std::env::temp_dir().join(format!("gravimera_gen3d_test_{run_id}"));
        let run_dir = base_dir.join(run_id.to_string());
        let pass_dir = run_dir.join("attempt_0").join("pass_0");
        std::fs::create_dir_all(&pass_dir).expect("create temp gen3d pass dir");

        let openai = OpenAiConfig {
            base_url: "mock://gen3d".into(),
            model: "mock".into(),
            model_reasoning_effort: "none".into(),
            api_key: "mock".into(),
        };

        let mut config = AppConfig {
            openai: Some(openai),
            ..Default::default()
        };
        // Keep budgets unlimited for the test, but bound execution time in the loop below.
        config.gen3d_max_seconds = 0;
        config.gen3d_max_tokens = 0;

        let mut workshop = Gen3dWorkshop::default();
        workshop.prompt = prompt.to_string();
        workshop.speed_mode = Gen3dSpeedMode::Level3;

        let mut job = Gen3dAiJob::default();
        job.running = true;
        job.build_complete = false;
        job.mode = super::super::Gen3dAiMode::Agent;
        job.phase = super::super::Gen3dAiPhase::AgentWaitingStep;
        job.openai = config.openai.clone();
        job.run_id = Some(run_id);
        job.attempt = 0;
        job.pass = 0;
        job.plan_hash.clear();
        job.assembly_rev = 0;
        job.max_parallel_components = 1;
        job.user_prompt_raw = prompt.to_string();
        job.user_images.clear();
        job.run_dir = Some(run_dir);
        job.pass_dir = Some(pass_dir.clone());
        job.agent = super::super::Gen3dAgentState::default();

        let draft = Gen3dDraft::default();
        let preview = Gen3dPreview::default();
        let feedback_history = Gen3dToolFeedbackHistory::default();

        spawn_agent_step_request(&config, &mut workshop, &mut job, pass_dir)
            .expect("spawn mock agent step");

        let mut app = App::new();
        app.insert_resource(config);
        app.insert_resource(Time::<()>::default());
        app.insert_resource(Assets::<Image>::default());
        app.insert_resource(workshop);
        app.insert_resource(feedback_history);
        app.insert_resource(job);
        app.insert_resource(draft);
        app.insert_resource(preview);

        app.add_systems(
            Update,
            |config: Res<AppConfig>,
             time: Res<Time>,
             mut commands: Commands,
             mut images: ResMut<Assets<Image>>,
             mut workshop: ResMut<Gen3dWorkshop>,
             mut feedback_history: ResMut<Gen3dToolFeedbackHistory>,
             mut job: ResMut<Gen3dAiJob>,
             mut draft: ResMut<Gen3dDraft>,
             mut preview: ResMut<Gen3dPreview>,
             mut preview_model: Query<
                (
                    &mut AnimationChannelsActive,
                    &mut LocomotionClock,
                    &mut AttackClock,
                ),
                With<Gen3dPreviewModelRoot>,
            >,
             review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>| {
                poll_gen3d_agent(
                    &config,
                    &time,
                    &mut commands,
                    &mut images,
                    &review_cameras,
                    &mut workshop,
                    &mut feedback_history,
                    &mut job,
                    &mut draft,
                    &mut preview,
                    &mut preview_model,
                );
            },
        );

        let started = std::time::Instant::now();
        loop {
            app.update();
            let running = app.world().resource::<Gen3dAiJob>().is_running();
            if !running {
                break;
            }
            if started.elapsed() > std::time::Duration::from_secs(5) {
                panic!("Gen3D mock agent test timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let draft = app.world().resource::<Gen3dDraft>();
        assert!(
            draft.total_non_projectile_primitive_parts() > 0,
            "expected generated primitive parts"
        );
        let root = draft.root_def().expect("expected Gen3D root def");
        assert!(root.mobility.is_some(), "expected mobility on root def");
        assert!(root.attack.is_some(), "expected attack on root def");
    }
}
