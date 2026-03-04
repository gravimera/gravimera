use std::path::PathBuf;

use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    TOOL_ID_COPY_COMPONENT, TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_DESCRIBE,
    TOOL_ID_LLM_GENERATE_COMPONENT, TOOL_ID_LLM_GENERATE_COMPONENTS,
    TOOL_ID_LLM_GENERATE_MOTION_AUTHORING, TOOL_ID_LLM_GENERATE_MOTION_ROLES,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_REVIEW_DELTA,
    TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_RENDER_PREVIEW,
    TOOL_ID_SMOKE_CHECK, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{Gen3dToolRegistryV1, Gen3dToolResultJsonV1};

use super::super::state::Gen3dWorkshop;
use super::Gen3dAiJob;

pub(super) fn build_agent_system_instructions() -> String {
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
  - If review_appearance=true and you did one more render+review after applying fixes and it still suggests no further actions, output a \"done\" action.\n\
  - If budgets prevent further improvement (regen budgets, time, tokens), output a \"done\" action with a best-effort reason.\n\
- Runtime motion roles (recommended for movable units):\n\
  - If the draft is a movable unit (mobility is ground/air) and `state_summary.motion_roles.applies_to_current` is false, call `llm_generate_motion_roles_v1` before finishing.\n\
  - This produces an explicit, non-heuristic mapping of locomotion effectors (legs/wheels) so the engine can inject generic `move` algorithms at runtime.\n\
- Motion authoring fallback (required when runtime motion cannot be used):\n\
  - If the draft is a movable unit AND `state_summary.motion_runtime_candidate` is null AND `state_summary.motion_coverage.has_move` is false, call `llm_generate_motion_authoring_v1`.\n\
  - This tool can author explicit per-edge animation clips (idle/move/attack) so the unit will not end up with zero animation.\n\
  - If the prompt implies stylized/custom motion (slither/coil/tentacle/undulate/tremble/majestic/etc), you MAY call `llm_generate_motion_authoring_v1` even if runtime motion is available.\n\
- Visual QA / appearance review:\n\
  - The state summary includes `review_appearance` (bool).\n\
  - If review_appearance=false (default): STRUCTURE-ONLY. Prefer validate_v1 + smoke_check_v1 + llm_review_delta_v1 (no preview images). Do NOT chase cosmetic regen/transform tweaks.\n\
  - If review_appearance=true: do visual QA in WAVES to reduce LLM wall time.\n\
    - Preferred loop: plan -> generate components (batch) -> render_preview_v1 -> llm_review_delta_v1.\n\
  - IMPORTANT: planning must be its OWN step.\n\
    - If you call llm_generate_plan_v1, DO NOT include llm_generate_components_v1/llm_generate_component_v1 in the same step.\n\
    - End the step after planning so you can observe `reuse_groups`/state before deciding what to generate.\n\
    - The engine will end the step after a successful llm_generate_plan_v1 even if you requested more actions.\n\
  - Avoid calling llm_review_delta_v1 after every single component if you can generate a batch first.\n\
  - If review_appearance=true: after any render_preview_v1, immediately call llm_review_delta_v1 using the rendered images.\n\
  - Do NOT use placeholder paths like `$CALL_1.images[0]` in tool args; the engine does not substitute tool outputs into later tool calls.\n\
    To review the latest render, call llm_review_delta_v1 with no `preview_images` (it will use the latest render cache), or pass `{ \"rendered_images_from_cache\": true }`.\n\
  - If review_appearance=true: do not finish a run without reviewing the latest renders.\n\
  - For vehicles/wheeled objects, always include TOP and BOTTOM views (they reveal wheel/axle/undercarriage issues). A good default is: views=[\"front\",\"left_back\",\"right_back\",\"top\",\"bottom\"].\n\
  - For speed, prefer smaller preview renders during iteration (example: render_preview_v1 image_size=768). Only increase resolution if you truly need extra detail.\n\
  - Do NOT render/review before any geometry exists. If components_generated==0 or the draft has 0 primitive parts, generate components first; renders will be blank.\n\
- Avoid duplicated LLM work: reuse geometry for symmetric/repeated parts (major speed win):\n\
  - If multiple planned components should be IDENTICAL (wheels, repeated legs, numbered sets like leg_0..leg_7), generate ONE of them, then fill the others using copy_component_v1 instead of calling llm_generate_component_v1 repeatedly.\n\
  - If multiple planned components should be LEFT/RIGHT MIRRORS of each other (mirrors, headlights, handles, wheels with one-sided details), generate ONE side, then use mirror_component_v1 for the other side(s).\n\
  - If the repeated part is a CHAIN (a component with attached descendants, like a leg/arm), use copy_component_subtree_v1 for identical chains, or mirror_component_subtree_v1 for L/R mirrored chains.\n\
  - Anchors: prefer anchors=preserve_interfaces so TARGET mount interfaces stay stable while internal anchors stay consistent with copied/mirrored geometry. Use anchors=preserve_target only when you must keep ALL target anchors unchanged. Use anchors=copy_source only when you need to overwrite the TARGET's anchors to match the SOURCE exactly.\n\
  - Prefer mode=linked when copying many LEAF components; call detach_component_v1 if any copy must diverge later.\n\
  - The state summary may include `reuse_suggestions` with ready-to-use tool args; use them when appropriate.\n\
- When you DO need LLM generation, prefer batching UNIQUE components in parallel:\n\
  - Use llm_generate_components_v1 with explicit component_indices/names for the unique set.\n\
  - Use missing_only=true ONLY when you truly want ALL missing components.\n\
    - If the plan declares `reuse_groups`, the engine will skip reuse targets in missing_only batches and auto-copy them after sources are generated.\n\
- IMPORTANT: If the state summary contains `pending_regen_component_indices` (non-empty), APPLY THEM NEXT:\n\
  - Call llm_generate_components_v1 with component_indices set to that list and force=true (regen is expected).\n\
  - Then run QA and confirm:\n\
    - Always: smoke_check_v1\n\
    - If review_appearance=true: render_preview_v1\n\
    - Then: llm_review_delta_v1\n\
  - Do NOT call llm_review_delta_v1 repeatedly without applying the pending regen or rerunning smoke_check/validate (and render_preview_v1 if review_appearance=true).\n\
- Regen budgets: regenerating an already-generated component counts against a regen budget. If a regen tool returns skipped_due_to_regen_budget, stop trying to regenerate and fix via transform/anchor tweaks instead.\n\
- IMPORTANT: A \"done\" action ENDS the Build run immediately. Only use \"done\" when you want to stop NOW.\n\
  If you want the run to continue, DO NOT include a \"done\" action; the engine will request another step automatically.\n\
- If you need tool schemas, call list_tools_v1 / describe_tool_v1.\n"
        .to_string()
}

pub(super) fn build_agent_user_text(
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
            TOOL_ID_LLM_GENERATE_MOTION_ROLES => {
                let move_effectors = value.get("move_effectors").and_then(|v| v.as_u64());
                out.push_str("ok");
                if let Some(move_effectors) = move_effectors {
                    out.push_str(&format!(" move_effectors={move_effectors}"));
                }
            }
            TOOL_ID_LLM_GENERATE_MOTION_AUTHORING => {
                let decision = value.get("decision").and_then(|v| v.as_str());
                let edges = value.get("edges").and_then(|v| v.as_u64());
                out.push_str("ok");
                if let Some(decision) = decision {
                    out.push_str(&format!(" decision={decision}"));
                }
                if let Some(edges) = edges {
                    out.push_str(&format!(" edges={edges}"));
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
            TOOL_ID_COPY_COMPONENT
            | TOOL_ID_MIRROR_COMPONENT
            | TOOL_ID_COPY_COMPONENT_SUBTREE
            | TOOL_ID_MIRROR_COMPONENT_SUBTREE => {
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

pub(super) fn draft_summary(config: &AppConfig, job: &Gen3dAiJob) -> serde_json::Value {
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
                let kind = if is_side_group {
                    "mirror_component_subtree"
                } else {
                    "copy_component_subtree"
                };
                let recommended_tool = if is_side_group {
                    "mirror_component_subtree_v1"
                } else {
                    "copy_component_subtree_v1"
                };
                out.push(serde_json::json!({
                    "kind": kind,
                    "group_key": key,
                    "source": source_name.clone(),
                    "source_generated": source_generated_now,
                    "targets": targets.clone(),
                    "targets_omitted": targets_omitted,
                    "recommended_tool": recommended_tool,
                    "note": "If source_generated=false, generate the source subtree first, then run the reuse tool.",
                    "recommended_args": {
                        "source_root": source_name,
                        "targets": targets,
                        "mode": "detached",
                        "anchors": "preserve_interfaces",
                    }
                }));
            } else {
                let kind = if is_side_group {
                    "mirror_component"
                } else {
                    "copy_component"
                };
                let recommended_tool = if is_side_group {
                    "mirror_component_v1"
                } else {
                    "copy_component_v1"
                };
                out.push(serde_json::json!({
                    "kind": kind,
                    "group_key": key,
                    "source": source_name.clone(),
                    "source_generated": source_generated_now,
                    "targets": targets.clone(),
                    "targets_omitted": targets_omitted,
                    "recommended_tool": recommended_tool,
                    "note": "If source_generated=false, generate the source component first, then run the reuse tool.",
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
            let alignment = match g.alignment {
                super::copy_component::Gen3dCopyAlignmentMode::Rotation => "rotation",
                super::copy_component::Gen3dCopyAlignmentMode::MirrorMountX => "mirror_mount_x",
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
                "alignment": alignment,
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

    let motion_roles_status = {
        let run_id = job.run_id.map(|id| id.to_string()).unwrap_or_default();
        match job.motion_roles.as_ref() {
            Some(roles) => serde_json::json!({
                "present": true,
                "applies_to_current": roles.applies_to.run_id.trim() == run_id.trim()
                    && roles.applies_to.attempt == job.attempt
                    && roles.applies_to.plan_hash.trim() == job.plan_hash.trim()
                    && roles.applies_to.assembly_rev == job.assembly_rev,
                "move_effectors": roles.move_effectors.len(),
            }),
            None => serde_json::json!({
                "present": false,
                "applies_to_current": false,
                "move_effectors": 0,
            }),
        }
    };

    let motion_authoring_status = {
        match job.motion_authoring.as_ref() {
            Some(authored) => {
                let applies_to_current = job.motion_authoring_for_current_draft().is_some();
                let decision = match authored.decision {
                    super::schema::AiMotionAuthoringDecisionJsonV1::RuntimeOk => "runtime_ok",
                    super::schema::AiMotionAuthoringDecisionJsonV1::AuthorClips => "author_clips",
                    super::schema::AiMotionAuthoringDecisionJsonV1::RegenGeometryRequired => {
                        "regen_geometry_required"
                    }
                    super::schema::AiMotionAuthoringDecisionJsonV1::Unknown => "unknown",
                };
                serde_json::json!({
                    "present": true,
                    "applies_to_current": applies_to_current,
                    "decision": decision,
                    "edges": authored.edges.len(),
                })
            }
            None => serde_json::json!({
                "present": false,
                "applies_to_current": false,
                "decision": null,
                "edges": 0,
            }),
        }
    };

    let motion_runtime_candidate = {
        let roles = job.motion_roles_for_current_draft();
        super::agent_utils::motion_runtime_candidate_kind(
            roles,
            &job.planned_components,
            None,
        )
    };

    let motion_coverage = {
        let mut edges_total = 0usize;
        let mut edges_with_any_slots = 0usize;
        let mut slots_total = 0usize;
        let mut move_edges = 0usize;
        let mut has_move = false;
        let mut has_idle = false;
        let mut has_attack = false;
        let mut has_ambient = false;
        let mut slots_by_channel: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();

        for comp in job.planned_components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            edges_total += 1;
            if !att.animations.is_empty() {
                edges_with_any_slots += 1;
            }
            let mut edge_has_move = false;
            for slot in att.animations.iter() {
                slots_total += 1;
                let channel = slot.channel.as_ref();
                *slots_by_channel.entry(channel.to_string()).or_insert(0) += 1;
                match channel {
                    "move" => {
                        has_move = true;
                        edge_has_move = true;
                    }
                    "idle" => has_idle = true,
                    "attack_primary" => has_attack = true,
                    "ambient" => has_ambient = true,
                    _ => {}
                }
            }
            if edge_has_move {
                move_edges += 1;
            }
        }

        serde_json::json!({
            "edges_total": edges_total,
            "edges_with_any_slots": edges_with_any_slots,
            "slots_total": slots_total,
            "move_edges": move_edges,
            "has_move": has_move,
            "has_idle": has_idle,
            "has_attack_primary": has_attack,
            "has_ambient": has_ambient,
            "slots_by_channel": slots_by_channel,
        })
    };

    serde_json::json!({
        "run_id": job.run_id.map(|id| id.to_string()),
        "attempt": job.attempt,
        "pass": job.pass,
        "plan_hash": job.plan_hash,
        "assembly_rev": job.assembly_rev,
        "motion_roles": motion_roles_status,
        "motion_authoring": motion_authoring_status,
        "motion_runtime_candidate": motion_runtime_candidate,
        "motion_coverage": motion_coverage,
        "review_appearance": job.review_appearance,
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
