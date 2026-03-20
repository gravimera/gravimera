use crate::config::AppConfig;
use crate::gen3d::agent::tools::{
    Gen3dToolDescriptorV1, TOOL_ID_APPLY_DRAFT_OPS, TOOL_ID_APPLY_PLAN_OPS, TOOL_ID_COPY_COMPONENT,
    TOOL_ID_COPY_COMPONENT_SUBTREE, TOOL_ID_DIFF_SNAPSHOTS, TOOL_ID_GET_PLAN_TEMPLATE,
    TOOL_ID_GET_SCENE_GRAPH_SUMMARY, TOOL_ID_GET_TOOL_DETAIL, TOOL_ID_INFO_BLOBS_GET,
    TOOL_ID_INFO_BLOBS_LIST, TOOL_ID_INFO_EVENTS_GET, TOOL_ID_INFO_EVENTS_LIST,
    TOOL_ID_INFO_EVENTS_SEARCH, TOOL_ID_INFO_KV_GET, TOOL_ID_INFO_KV_GET_MANY,
    TOOL_ID_INFO_KV_GET_PAGED, TOOL_ID_INFO_KV_LIST_HISTORY, TOOL_ID_INFO_KV_LIST_KEYS,
    TOOL_ID_INSPECT_PLAN, TOOL_ID_LIST_SNAPSHOTS, TOOL_ID_LLM_GENERATE_COMPONENT,
    TOOL_ID_LLM_GENERATE_COMPONENTS, TOOL_ID_LLM_GENERATE_MOTION_AUTHORING,
    TOOL_ID_LLM_GENERATE_PLAN, TOOL_ID_LLM_GENERATE_PLAN_OPS, TOOL_ID_LLM_REVIEW_DELTA,
    TOOL_ID_MIRROR_COMPONENT, TOOL_ID_MIRROR_COMPONENT_SUBTREE, TOOL_ID_MOTION_METRICS, TOOL_ID_QA,
    TOOL_ID_QUERY_COMPONENT_PARTS, TOOL_ID_RECENTER_ATTACHMENT_MOTION, TOOL_ID_RENDER_PREVIEW,
    TOOL_ID_RESTORE_SNAPSHOT, TOOL_ID_SMOKE_CHECK, TOOL_ID_SNAPSHOT,
    TOOL_ID_SUGGEST_MOTION_REPAIRS, TOOL_ID_VALIDATE,
};
use crate::gen3d::agent::{Gen3dToolRegistryV1, Gen3dToolResultJsonV1};
use uuid::Uuid;

use super::super::state::Gen3dWorkshop;
use super::Gen3dAiJob;

pub(super) fn build_agent_system_instructions() -> String {
    // Keep this short; the user text provides tool lists and current state.
    // The agent must output strict JSON only.
    "You are the Gravimera Gen3D agent.\n\
Return ONLY a single JSON object for gen3d_agent_step_v1 (no markdown, no prose).\n\n\
IMPORTANT:\n\
- Output EXACTLY ONE JSON object. Do NOT output multiple JSON objects.\n\
- Do NOT \"simulate\" multiple steps. If you need another step, the engine will ask again.\n\
- If you need tool outputs, return ONE tool-call step. Do NOT output a second JSON object.\n\
- A \"done\" action ENDS the Build run immediately. Never use \"done\" to mean \"waiting\".\n\n\
- Self-check: your entire response must parse as a single JSON object (no extra prose).\n\n\
Schema:\n\
{\n\
  \"version\": 1,\n\
  \"status_summary\": \"short, user-facing summary\",\n\
  \"actions\": [\n\
    {\"kind\":\"tool_call\",\"call_id\":\"call_1\",\"tool_id\":\"qa_v1\",\"args\":{}}\n\
  ]\n\
}\n\n\
Example done:\n\
{\"version\":1,\"status_summary\":\"Build finished.\",\"actions\":[{\"kind\":\"done\",\"reason\":\"All required QA is ok.\"}]}\n\n\
Rules:\n\
- Use tools to read/modify state. Do not assume the engine will auto-fix anything.\n\
- Tool args are strict JSON objects. Do NOT invent arg keys.\n\
  - The user prompt includes a brief args signature and example for each tool.\n\
    - Only call a tool with empty `{}` args if its args signature is exactly `{}`.\n\
  - Tool results are only visible in the NEXT step (you cannot \"read\" tool outputs mid-step).\n\
  - If you need full args_schema/args_example details beyond the brief tool list, call `get_tool_detail_v1` with `tool_id` and END THE STEP (no other actions).\n\
    - If you need details for multiple tools, call `get_tool_detail_v1` multiple times in the SAME step.\n\
  - Some tools reject unknown keys (hard error). Example: `snapshot_v1` uses `label` (NOT `name`).\n\
  - `query_component_parts_v1` requires `component` or `component_index` (never call it with empty `{}` args).\n\
- Prefer small, explainable steps that improve basic structure and correctness.\n\
- Prioritize BASIC STRUCTURE over tiny details. This is a voxel/pixel-art game; do not chase micro-adjustments forever.\n\
- STOP when the model is good enough:\n\
  - If the latest review delta accepts the model / has no actionable fixes AND required QA is ok (`state_summary.qa.last_validate_ok=true` AND `state_summary.qa.last_smoke_ok=true` AND `state_summary.qa.last_motion_ok` is not false), output a \"done\" action.\n\
  - If required QA is not ok, do NOT output \"done\". Run `qa_v1`, apply the provided fixits, then rerun `qa_v1`.\n\
  - If review_appearance=true and you did one more render+review after applying fixes and it still suggests no further actions (and required QA is ok), output a \"done\" action.\n\
  - If budgets prevent further improvement (regen budgets, review-delta rounds, time, tokens), output a \"done\" action with a best-effort reason.\n\
  - `done.reason` is treated as an unverified agent note; keep it brief and factual. Do NOT claim tool actions that did not occur.\n\
  - `qa_v1` may report warnings (non-fatal). Treat warnings as informational: do NOT spend steps trying to eliminate warnings.\n\
    - If warnings>0, mention them explicitly in \"done.reason\" (do not claim \"no warnings\").\n\
- Motion authoring (required for movable units):\n\
  - If the draft is a movable unit (mobility is ground/air) and `state_summary.motion_coverage.has_move` is false, call `llm_generate_motion_authoring_v1` before finishing.\n\
  - This tool authors explicit per-edge animation clips (idle/move/attack) baked into the prefab; the engine does not provide runtime motion algorithms.\n\
  - If the prompt implies stylized/custom motion (slither/coil/tentacle/undulate/tremble/majestic/etc), you MAY call `llm_generate_motion_authoring_v1` even if move slots already exist.\n\
  - If `qa_v1` reports `joint_rest_bias_large`, prefer calling `recenter_attachment_motion_v1` on the offending child components/channels first; it is deterministic and preserves motion exactly by re-parameterizing offset vs delta.\n\
    - If it returns applied=false or the issue persists, then call `llm_generate_motion_authoring_v1` to re-author the offending clips/channels.\n\
  - If `qa_v1` reports `hinge_limit_exceeded`, call `suggest_motion_repairs_v1` to get deterministic patch options (relax joint limits vs scale rotation), then explicitly apply ONE chosen patch via `apply_draft_ops_v1`.\n\
    - Only fall back to `llm_generate_motion_authoring_v1` if the suggestions are unsuitable (ex: would relax limits too much, or would scale motion too aggressively).\n\
  - If `qa_v1` reports motion_validation issues with severity=\"error\" that are primarily animation-delta problems (examples: `hinge_off_axis`, `time_offset_no_effect`), prefer calling `llm_generate_motion_authoring_v1` to re-author the offending clips/channels (do NOT loop `llm_review_delta_v1` repeatedly for these).\n\
  - Do NOT chase warn-only motion_validation issues (example: `attack_self_intersection`). Treat them as informational and finish once required QA is ok.\n\
  - If `qa_v1` reports `contact_stance_missing`, prefer `llm_review_delta_v1` to add/fix `contacts[].stance` (motion authoring cannot create stance metadata).\n\
  - If `qa_v1` reports `hinge_axis_missing` or `hinge_axis_invalid`, fix the joint axis (replan OR `apply_draft_ops_v1` set_attachment_joint) before motion authoring.\n\
  - If `qa_v1` reports `fixed_joint_rotates` on a joint you INTEND to rotate, update that edge's joint metadata (usually to `hinge` with a valid `axis_join`) so QA reflects the intended degrees-of-freedom.\n\
  - If the user complains about stride/step size (\"stride too small\", \"bigger steps\", \"feet barely move\"), call `motion_metrics_v1` to measure stride + planted-contact slip/lift BEFORE re-authoring motion; then use those numeric metrics explicitly (goal + measurement) in your fix.\n\
  - `render_preview_v1` is local-only rendering; it does NOT send images to the LLM. Use `render_preview_v1` with `include_motion_sheets=true` to generate motion sprite sheets for quick inspection.\n\
- Visual QA / appearance review:\n\
  - The state summary includes `review_appearance` (bool).\n\
  - If review_appearance=false (default): STRUCTURE-ONLY. Prefer qa_v1 + llm_review_delta_v1 (no preview images). Do NOT chase cosmetic regen/transform tweaks.\n\
    - If `state_summary.seed.kind` is `edit_overwrite` or `fork`: this is a seeded edit session. Even with `review_appearance=false`, you SHOULD apply machine-appliable alignment/attachment tweaks to satisfy the user notes (do not wait for QA errors).\n\
  - Descriptor meta (prefab descriptor short name `name` (<=3 words) + `short` + `tags`): in seeded edit sessions, preserve existing values unless the user explicitly requests changes. If requested, call `set_descriptor_meta_v1` before finishing.\n\
  - qa_v1 runs validate_v1 + smoke_check_v1 and returns a combined summary.\n\
- If review_appearance=true: do visual QA in WAVES to reduce LLM wall time.\n\
  - Preferred loop: plan -> generate components (batch) -> render_preview_v1 -> llm_review_delta_v1.\n\
  - IMPORTANT: planning must be its OWN step.\n\
    - If you call llm_generate_plan_v1, DO NOT include llm_generate_components_v1/llm_generate_component_v1 in the same step.\n\
    - End the step after planning so you can observe `reuse_groups`/state before deciding what to generate.\n\
    - The engine will end the step after a successful llm_generate_plan_v1 even if you requested more actions.\n\
  - Avoid calling llm_review_delta_v1 after every single component if you can generate a batch first.\n\
  - If review_appearance=true: after any render_preview_v1, immediately call llm_review_delta_v1 using the rendered images.\n\
- Do NOT use placeholder references like `$CALL_1.blob_ids[0]` in tool args; the engine does not substitute tool outputs into later tool calls.\n\
  To review the latest render, call llm_review_delta_v1 with args `{\"preview_blob_ids\":[]}` (empty list means: use the latest render cache).\n\
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
  - Default: use llm_generate_components_v1 with explicit component_indices/names for the unique set.\n\
  - If `state_summary.preserve_existing_components_mode` is true: prefer generating ONLY missing components (omit component_indices/names and omit force) so you don't accidentally regenerate already-generated components.\n\
  - Preserve-mode replanning (`llm_generate_plan_v1` with `constraints.preserve_existing_components=true`) is plan-diff validated:\n\
    - Default `constraints.preserve_edit_policy` is `additive` (no rewires; offsets frozen).\n\
    - For moving existing parts without rewiring, prefer `apply_draft_ops_v1` or set `preserve_edit_policy` to `allow_offsets`.\n\
    - For rewires, set `preserve_edit_policy` to `allow_rewire` and provide `constraints.rewire_components` as an explicit allow-list.\n\
	  - Preserve-mode planning helpers (no silent mutation):\n\
	    - If `llm_generate_plan_v1` fails with a semantic error (unknown parent/root, missing required names, policy diff rejection), call `inspect_plan_v1` next (NOT `get_scene_graph_summary_v1`).\n\
	    - If your preserve-mode plan change is local (small bounded edit), you MAY prefer `llm_generate_plan_ops_v1` (diff-first replanning) instead of re-emitting the full plan via `llm_generate_plan_v1`.\n\
	    - If the fix is local/deterministic (rename a parent, add a missing component definition, add missing anchors), you MAY call `apply_plan_ops_v1` to patch and revalidate (base_plan=\"pending\" patches the pending rejected attempt; base_plan=\"current\" patches the current accepted plan) instead of rerunning `llm_generate_plan_v1`.\n\
	    - Preserve-mode replanning with an existing plan requires a template:\n\
	      - Call `get_plan_template_v1` first (mode=\"auto\"), then call `llm_generate_plan_v1` with `plan_template_kv`.\n\
	      - The tool will refuse preserve-mode replans without `plan_template_kv`.\n\
	      - If the template is too large, retry `get_plan_template_v1` with `mode=\"lean\"` and/or `scope_components=[...]`.\n\
	  - To explicitly regenerate already-generated components in preserve mode, pass force=true (regen budgets still apply).\n\
	  - IMPORTANT: `force=true` regeneration is ONLY allowed when the latest QA indicates errors.\n\
    - The engine refuses force-regeneration unless `state_summary.qa.last_validate_ok=false` OR `state_summary.qa.last_smoke_ok=false`.\n\
    - If `state_summary.qa.last_validate_ok`/`last_smoke_ok` is null or true, do NOT use force-regeneration.\n\
      Run `qa_v1`, then fix assembly/placement with `llm_review_delta_v1` / `apply_draft_ops_v1` instead of regenerating geometry.\n\
  - If the plan declares `reuse_groups`, the engine will skip reuse targets in missing_only batches and auto-copy them after sources are generated.\n\
- IMPORTANT: If the state summary contains `pending_regen_component_indices` (non-empty), APPLY THEM NEXT:\n\
  - Call llm_generate_components_v1 with component_indices set to that list.\n\
  - Only set force=true if QA indicates errors (`state_summary.qa.last_validate_ok=false` OR `state_summary.qa.last_smoke_ok=false`).\n\
  - Then run QA and confirm:\n\
    - Always: qa_v1\n\
    - If review_appearance=true: render_preview_v1\n\
    - Then: llm_review_delta_v1\n\
  - Do NOT call llm_review_delta_v1 repeatedly without applying the pending work or rerunning qa_v1 (and render_preview_v1 if review_appearance=true).\n\
  - Budget: llm_review_delta_v1 is capped per run (see state_summary.budgets.review_delta). Use it intentionally:\n\
    - Round 1: broad (fix all objective errors + satisfy the request).\n\
    - Round 2: focused (fix objective errors + main issue only), then accept.\n\
- If `state_summary.pending_regen_component_indices_blocked_due_to_qa_gate` is non-empty:\n\
  - Do NOT retry force-regeneration while QA is clean/unknown.\n\
  - Exit this state deterministically (do NOT keep inspecting):\n\
    - Prefer deterministic edits via `apply_draft_ops_v1` (ex: recolor primitives, adjust attachment offsets).\n\
    - For LLM-driven primitive edits, use DraftOps: `query_component_parts_v1` → `llm_generate_draft_ops_v1` → `apply_last_draft_ops_v1` (or `apply_draft_ops_from_event_v1`).\n\
    - If the request truly requires regeneration/style rebuild, disable preserve mode via `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`, then regenerate without `force`.\n\
    - If QA is ok and you have no deterministic fixes to apply, output `done` and mention the blocked indices.\n\
- Regen budgets: regenerating an already-generated component counts against a regen budget. If a regen tool returns skipped_due_to_regen_budget, stop trying to regenerate and fix via transform/anchor tweaks instead.\n\
- IMPORTANT: A \"done\" action ENDS the Build run immediately. Only use \"done\" when you want to stop NOW.\n\
  If you want the run to continue, DO NOT include a \"done\" action; the engine will request another step automatically.\n\
- To inspect run outputs, use Info Store tools:\n\
  - KV (structured latest state): info_kv_get_v1 / info_kv_list_keys_v1\n\
  - Events (tool logs/errors): info_events_list_v1 / info_events_search_v1 / info_events_get_v1\n\
  - Blobs (render previews): info_blobs_list_v1 / info_blobs_get_v1\n"
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

    fn char_count(text: &str) -> usize {
        text.chars().count()
    }

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

    fn first_line(text: &str) -> &str {
        text.split('\n').next().unwrap_or("")
    }

    fn one_line_snip(text: &str, max_chars: usize) -> String {
        let sanitized = text.replace('\r', " ").replace('\n', " ");
        truncate_for_prompt(sanitized.trim(), max_chars)
    }

    fn required_keys_from_args_sig(args_sig: &str) -> Vec<String> {
        let sig = args_sig.trim();
        if sig == "{}" {
            return Vec::new();
        }

        let Some(inner) = sig.strip_prefix('{').and_then(|s| s.strip_suffix('}')) else {
            return Vec::new();
        };

        fn push_required_key_from_item(keys: &mut Vec<String>, item: &str) {
            let item = item.trim();
            if item.is_empty() {
                return;
            }
            let Some((key, _)) = item.split_once(':') else {
                return;
            };
            let key = key.trim();
            if key.is_empty() || key.ends_with('?') {
                return;
            }
            keys.push(key.to_string());
        }

        let mut keys = Vec::new();
        let mut item_buf = String::new();
        let mut depth: i32 = 0;
        let mut in_string = false;
        let mut escaped = false;

        for ch in inner.chars() {
            if in_string {
                item_buf.push(ch);
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '"' {
                    in_string = false;
                }
                continue;
            }

            match ch {
                '"' => {
                    in_string = true;
                    item_buf.push(ch);
                }
                '{' | '[' | '(' => {
                    depth += 1;
                    item_buf.push(ch);
                }
                '}' | ']' | ')' => {
                    depth -= 1;
                    item_buf.push(ch);
                }
                ',' if depth == 0 => {
                    push_required_key_from_item(&mut keys, &item_buf);
                    item_buf.clear();
                }
                _ => item_buf.push(ch),
            }
        }
        push_required_key_from_item(&mut keys, &item_buf);

        keys.sort();
        keys.dedup();
        keys
    }

    fn find_tool<'a>(
        tools: &'a [Gen3dToolDescriptorV1],
        tool_id: &str,
    ) -> Option<&'a Gen3dToolDescriptorV1> {
        tools.iter().find(|t| t.tool_id == tool_id)
    }

    fn summarize_tool_result(
        result: &Gen3dToolResultJsonV1,
        tools: &[Gen3dToolDescriptorV1],
    ) -> String {
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
                320,
            ));

            if let Some(tool) = find_tool(tools, result.tool_id.as_str()) {
                let args_sig = first_line(tool.args_schema).trim();
                let args_sig = if args_sig.is_empty() { "{}" } else { args_sig };
                let required_keys = required_keys_from_args_sig(args_sig);

                out.push_str(" | expected_args=");
                out.push_str(&truncate_for_prompt(args_sig, 240));
                if !required_keys.is_empty() {
                    out.push_str(" required_keys=");
                    out.push_str(&truncate_for_prompt(&format!("{required_keys:?}"), 200));
                }
                out.push_str(" example=");
                out.push_str(&truncate_for_prompt(&tool.args_example.to_string(), 200));
            }

            // Prefer surfacing "fixits" for actionable inspection errors.
            if result.tool_id.as_str() == TOOL_ID_INFO_KV_GET {
                if let Some(diag) = result.result.as_ref().and_then(|v| v.as_object()) {
                    if let Some(kv_rev) = diag
                        .get("record")
                        .and_then(|v| v.get("kv_rev"))
                        .and_then(|v| v.as_u64())
                    {
                        out.push_str(&format!(" kv_rev={kv_rev}"));
                    }
                    if let Some(fixits) = diag.get("fixits").and_then(|v| v.as_array()) {
                        let mut parts: Vec<String> = Vec::new();
                        for fixit in fixits.iter().take(3) {
                            let tool_id = fixit
                                .get("tool_id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim();
                            if tool_id.is_empty() {
                                continue;
                            }
                            let ptr = fixit
                                .get("args")
                                .and_then(|v| v.get("json_pointer"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .trim();
                            if ptr.is_empty() {
                                parts.push(tool_id.to_string());
                            } else {
                                parts.push(format!("{tool_id} json_pointer={ptr}"));
                            }
                        }
                        if !parts.is_empty() {
                            out.push_str(" fixits=");
                            out.push_str(&truncate_for_prompt(&format!("{parts:?}"), 240));
                        }
                    }
                }
            }
            return out;
        }

        let Some(value) = result.result.as_ref() else {
            out.push_str("ok");
            return out;
        };

        match result.tool_id.as_str() {
            TOOL_ID_GET_SCENE_GRAPH_SUMMARY => {
                fn edge_to_string(edge: &serde_json::Value) -> Option<String> {
                    let child = edge
                        .get("child")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let parent = edge
                        .get("parent")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    if child.is_empty() || parent.is_empty() {
                        return None;
                    }
                    let parent_anchor = edge
                        .get("parent_anchor")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let child_anchor = edge
                        .get("child_anchor")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let joint_kind = edge
                        .get("joint_kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("null")
                        .trim();
                    let offset_pos = edge
                        .get("offset_pos")
                        .map(|v| truncate_for_prompt(&v.to_string(), 64))
                        .unwrap_or_else(|| "null".into());

                    let mut out = String::new();
                    out.push_str(child);
                    out.push_str("->");
                    out.push_str(parent);
                    if !parent_anchor.is_empty() {
                        out.push('.');
                        out.push_str(parent_anchor);
                    }
                    if !child_anchor.is_empty() {
                        out.push_str(" child=");
                        out.push_str(child_anchor);
                    }
                    out.push_str(" off=");
                    out.push_str(offset_pos.trim());
                    out.push_str(" joint=");
                    out.push_str(joint_kind);
                    Some(truncate_for_prompt(out.trim(), 120))
                }

                out.push_str("ok");

                let components_total = value
                    .get("components_total")
                    .and_then(|v| v.as_u64())
                    .or_else(|| {
                        value
                            .get("components")
                            .and_then(|v| v.as_array())
                            .map(|a| a.len() as u64)
                    })
                    .unwrap_or(0);
                out.push_str(&format!(" components={components_total}"));

                if let Some(info_kv) = value.get("info_kv").and_then(|v| v.as_object()) {
                    let namespace = info_kv
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let key = info_kv
                        .get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let kv_rev = info_kv
                        .get("selector")
                        .and_then(|v| v.get("kv_rev"))
                        .and_then(|v| v.as_u64());
                    if !namespace.is_empty() || !key.is_empty() || kv_rev.is_some() {
                        out.push_str(" info_kv={");
                        if !namespace.is_empty() {
                            out.push_str("namespace=");
                            out.push_str(&truncate_for_prompt(namespace, 32));
                            out.push(' ');
                        }
                        if !key.is_empty() {
                            out.push_str("key=");
                            out.push_str(&truncate_for_prompt(key, 96));
                            out.push(' ');
                        }
                        if let Some(kv_rev) = kv_rev {
                            out.push_str(&format!("kv_rev={kv_rev}"));
                        }
                        out.push('}');
                    }
                }

                let Some(attachment_edges) =
                    value.get("attachment_edges").and_then(|v| v.as_array())
                else {
                    return out;
                };
                let total = attachment_edges.len();
                out.push_str(&format!(" attachment_edges_total={total}"));
                if total > 0 {
                    const SAMPLE_MAX_CHARS: usize = 480;
                    let mut head = total.min(6);
                    let mut tail = total.saturating_sub(head).min(6);

                    let mut sample_text = String::new();
                    while head > 0 || tail > 0 {
                        let mut parts: Vec<String> = Vec::new();
                        for edge in attachment_edges.iter().take(head) {
                            if let Some(s) = edge_to_string(edge) {
                                parts.push(s);
                            }
                        }
                        if tail > 0 {
                            let start = total.saturating_sub(tail);
                            for edge in attachment_edges.iter().skip(start) {
                                if let Some(s) = edge_to_string(edge) {
                                    parts.push(s);
                                }
                            }
                        }
                        sample_text = parts.join(" | ");
                        if char_count(&sample_text) <= SAMPLE_MAX_CHARS {
                            break;
                        }
                        if head > 0 {
                            head = head.saturating_sub(1);
                            continue;
                        }
                        if tail > 0 {
                            tail = tail.saturating_sub(1);
                            continue;
                        }
                    }
                    if !sample_text.trim().is_empty() {
                        out.push_str(" attachment_edges_sample=[");
                        out.push_str(&sample_text);
                        out.push(']');
                    }
                }
            }
            TOOL_ID_QUERY_COMPONENT_PARTS => {
                let component = value
                    .get("component")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let component_index = value.get("component_index").and_then(|v| v.as_u64());
                let parts = value.get("parts").and_then(|v| v.as_array());
                let parts_len = parts.map(|a| a.len()).unwrap_or(0);

                let mut part_examples: Vec<String> = Vec::new();
                if let Some(parts) = parts {
                    for part in parts.iter().take(8) {
                        let part_id = part
                            .get("part_id_uuid")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let mesh = part
                            .get("primitive")
                            .and_then(|v| v.get("mesh"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let color = part
                            .get("primitive")
                            .and_then(|v| v.get("color_rgba"))
                            .map(|v| truncate_for_prompt(&v.to_string(), 80))
                            .unwrap_or_else(|| "null".into());
                        let pos = part
                            .get("transform")
                            .and_then(|v| v.get("pos"))
                            .map(|v| truncate_for_prompt(&v.to_string(), 80))
                            .unwrap_or_else(|| "null".into());
                        let scale = part
                            .get("transform")
                            .and_then(|v| v.get("scale"))
                            .map(|v| truncate_for_prompt(&v.to_string(), 80))
                            .unwrap_or_else(|| "null".into());

                        let mut ex = String::new();
                        if !part_id.trim().is_empty() {
                            ex.push_str(part_id.trim());
                        } else {
                            ex.push_str("<no_part_id>");
                        }
                        if !mesh.trim().is_empty() {
                            ex.push_str(" mesh=");
                            ex.push_str(mesh.trim());
                        }
                        ex.push_str(" color=");
                        ex.push_str(color.trim());
                        ex.push_str(" pos=");
                        ex.push_str(pos.trim());
                        ex.push_str(" scale=");
                        ex.push_str(scale.trim());
                        part_examples.push(ex);
                    }
                }

                out.push_str("ok");
                if !component.trim().is_empty() {
                    out.push_str(&format!(
                        " component={}",
                        truncate_for_prompt(component, 64)
                    ));
                }
                if let Some(component_index) = component_index {
                    out.push_str(&format!(" idx={component_index}"));
                }
                out.push_str(&format!(" parts={parts_len}"));
                if !part_examples.is_empty() {
                    let total = part_examples.len();
                    let shown: Vec<&str> =
                        part_examples.iter().take(6).map(|s| s.as_str()).collect();
                    out.push_str(&format!(
                        " part_examples={}",
                        truncate_for_prompt(&format!("{shown:?}"), 420)
                    ));
                    if total > shown.len() {
                        out.push_str(&format!(" part_examples_total={total}"));
                    }
                }
            }
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
            TOOL_ID_LLM_GENERATE_PLAN_OPS => {
                let accepted = value.get("accepted").and_then(|v| v.as_bool());
                let ops_total = value.get("ops_total").and_then(|v| v.as_u64());
                let touched = value
                    .get("diff_summary")
                    .and_then(|v| v.get("touched_components"))
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());

                out.push_str("ok");
                if let Some(accepted) = accepted {
                    out.push_str(&format!(" accepted={accepted}"));
                }
                if let Some(ops_total) = ops_total {
                    out.push_str(&format!(" ops_total={ops_total}"));
                }
                if let Some(touched) = touched {
                    out.push_str(&format!(" touched={touched}"));
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
                let preserve_skipped = value
                    .get("skipped_due_to_preserve_existing_components")
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
                if let Some(total) = preserve_skipped {
                    if total > 0 {
                        out.push_str(&format!(" preserve_skipped={total}"));
                    }
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
                let regen_blocked = value
                    .get("regen_component_indices_blocked_due_to_qa_gate")
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
                if let Some(regen_blocked) = regen_blocked {
                    let total = regen_blocked.len();
                    let blocked: Vec<u64> = regen_blocked
                        .iter()
                        .filter_map(|v| v.as_u64())
                        .take(12)
                        .collect();
                    if !blocked.is_empty() {
                        out.push_str(&format!(" regen_blocked_by_qa={blocked:?}"));
                    }
                    if total > blocked.len() {
                        out.push_str(&format!(" regen_blocked_by_qa_total={total}"));
                    }
                }
            }
            TOOL_ID_RECENTER_ATTACHMENT_MOTION => {
                let applied_any = value.get("applied_any").and_then(|v| v.as_bool());
                let children = value.get("children").and_then(|v| v.as_array());
                let applied_children = children.map(|arr| {
                    arr.iter()
                        .filter(|child| {
                            child.get("applied").and_then(|v| v.as_bool()) == Some(true)
                        })
                        .count()
                });
                out.push_str("ok");
                if let Some(applied_any) = applied_any {
                    out.push_str(&format!(" applied_any={applied_any}"));
                }
                if let Some(applied_children) = applied_children {
                    out.push_str(&format!(" applied_children={applied_children}"));
                }
                if let Some(children) = children {
                    out.push_str(&format!(" children={}", children.len()));
                }
            }
            TOOL_ID_RENDER_PREVIEW => {
                fn join_exact_ids(ids: &[&str], max_chars: usize) -> String {
                    let mut out = String::new();
                    for id in ids {
                        if id.trim().is_empty() {
                            continue;
                        }
                        let sep = if out.is_empty() { "" } else { "," };
                        let candidate_len = char_count(&out) + char_count(sep) + char_count(id);
                        if candidate_len > max_chars {
                            break;
                        }
                        out.push_str(sep);
                        out.push_str(id);
                    }
                    out
                }

                let blob_ids_arr = value.get("blob_ids").and_then(|v| v.as_array());
                let static_blob_ids_arr = value.get("static_blob_ids").and_then(|v| v.as_array());

                let blob_ids_total = blob_ids_arr.map(|a| a.len()).unwrap_or(0);
                let static_blob_ids_total = static_blob_ids_arr.map(|a| a.len()).unwrap_or(0);

                let blob_ids_sample = blob_ids_arr
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .take(3)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let static_blob_ids_sample = static_blob_ids_arr
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str())
                            .take(3)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let motion_sheet_blob_ids = value.get("motion_sheet_blob_ids");
                let move_sheet = motion_sheet_blob_ids
                    .and_then(|v| v.get("move"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                let attack_sheet = motion_sheet_blob_ids
                    .and_then(|v| v.get("attack"))
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty());
                out.push_str("ok");
                out.push_str(&format!(" blob_ids={blob_ids_total}"));
                if !blob_ids_sample.is_empty() {
                    let sample = join_exact_ids(&blob_ids_sample, 240);
                    if !sample.is_empty() {
                        out.push_str(" blob_ids_sample=[");
                        out.push_str(&sample);
                        out.push(']');
                    }
                }
                out.push_str(&format!(" static_blob_ids={static_blob_ids_total}"));
                if !static_blob_ids_sample.is_empty() {
                    let sample = join_exact_ids(&static_blob_ids_sample, 240);
                    if !sample.is_empty() {
                        out.push_str(" static_blob_ids_sample=[");
                        out.push_str(&sample);
                        out.push(']');
                    }
                }
                if move_sheet.is_some() || attack_sheet.is_some() {
                    out.push_str(" motion_sheets={");
                    if let Some(move_sheet) = move_sheet {
                        out.push_str("move=");
                        out.push_str(move_sheet);
                        out.push(' ');
                    }
                    if let Some(attack_sheet) = attack_sheet {
                        out.push_str("attack=");
                        out.push_str(attack_sheet);
                    }
                    out.push('}');
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
            TOOL_ID_QA => {
                let ok = value.get("ok").and_then(|v| v.as_bool());
                let errors = value
                    .get("errors")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let warnings = value
                    .get("warnings")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let cached = value
                    .get("cached")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let no_new_information = value
                    .get("no_new_information")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let capability_gaps = value
                    .get("capability_gaps")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                out.push_str("ok");
                if let Some(ok) = ok {
                    out.push_str(&format!(" ok={ok}"));
                }
                if let Some(errors) = errors {
                    out.push_str(&format!(" errors={errors}"));
                }
                let warnings_count = warnings.unwrap_or(0);
                out.push_str(&format!(" warnings={warnings_count}"));
                if cached {
                    out.push_str(" cached=true");
                }
                if no_new_information {
                    out.push_str(" no_new_information=true");
                }
                if capability_gaps > 0 {
                    out.push_str(&format!(" capability_gaps={capability_gaps}"));
                    if let Some(kind) = value
                        .get("capability_gaps")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                        .and_then(|g| g.get("kind"))
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        out.push_str(&format!(" gap_example={}", truncate_for_prompt(kind, 64)));
                    }
                }
                if warnings_count > 0 {
                    if let Some(first) = value
                        .get("warnings")
                        .and_then(|v| v.as_array())
                        .and_then(|a| a.first())
                    {
                        let component_name = first
                            .get("component_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let kind = first.get("kind").and_then(|v| v.as_str()).unwrap_or("");
                        let message = first.get("message").and_then(|v| v.as_str()).unwrap_or("");
                        let mut example = String::new();
                        if !component_name.trim().is_empty() {
                            example.push_str(component_name.trim());
                            example.push(' ');
                        }
                        if !kind.trim().is_empty() {
                            example.push_str(kind.trim());
                            example.push_str(": ");
                        }
                        example.push_str(message.trim());
                        let example = truncate_for_prompt(example.trim(), 160);
                        if !example.is_empty() {
                            out.push_str(&format!(" warn_example={example}"));
                        }
                    }
                }
            }
            TOOL_ID_SMOKE_CHECK => {
                let ok = value.get("ok").and_then(|v| v.as_bool());
                let issues = value
                    .get("issues")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let capability_gaps = value
                    .get("capability_gaps")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(ok) = ok {
                    out.push_str(&format!(" ok={ok}"));
                }
                if let Some(issues) = issues {
                    out.push_str(&format!(" issues={issues}"));
                }
                if let Some(gaps) = capability_gaps {
                    out.push_str(&format!(" capability_gaps={gaps}"));
                }
            }
            TOOL_ID_MOTION_METRICS => {
                let cycle_m = value
                    .get("rig_summary")
                    .and_then(|v| v.get("cycle_m"))
                    .and_then(|v| v.as_f64());
                let contacts_ground = value
                    .get("rig_summary")
                    .and_then(|v| v.get("contacts_ground_total"))
                    .and_then(|v| v.as_u64());
                let stance_slip_max = value
                    .get("summary")
                    .and_then(|v| v.get("stance_slip_max_m_xz"))
                    .and_then(|v| v.get("max"))
                    .and_then(|v| v.as_f64());
                let forward_range_mean = value
                    .get("summary")
                    .and_then(|v| v.get("root_frame_forward_range_m"))
                    .and_then(|v| v.get("mean"))
                    .and_then(|v| v.as_f64());

                out.push_str("ok");
                if let Some(cycle_m) = cycle_m {
                    out.push_str(&format!(" cycle_m={:.3}", cycle_m.max(0.0)));
                }
                if let Some(contacts_ground) = contacts_ground {
                    out.push_str(&format!(" ground_contacts={contacts_ground}"));
                }
                if let Some(forward_range_mean) = forward_range_mean {
                    out.push_str(&format!(
                        " forward_range_mean_m={:.3}",
                        forward_range_mean.max(0.0)
                    ));
                }
                if let Some(stance_slip_max) = stance_slip_max {
                    out.push_str(&format!(
                        " stance_slip_max_m_xz={:.3}",
                        stance_slip_max.max(0.0)
                    ));
                }
            }
            TOOL_ID_SUGGEST_MOTION_REPAIRS => {
                const MAX_LINE_CHARS: usize = 3000;
                const MAX_APPLY_ARGS_CHARS: usize = 800;
                const MAX_SUGGESTIONS: usize = 8;

                fn impact_snip(impact: &serde_json::Value) -> Option<String> {
                    let obj = impact.as_object()?;
                    if let Some(scale) = obj.get("scale_factor").and_then(|v| v.as_f64()) {
                        if scale.is_finite() {
                            return Some(format!("scale_factor={scale:.4}"));
                        }
                    }
                    if let Some(relax) = obj.get("relax_degrees").and_then(|v| v.as_f64()) {
                        if relax.is_finite() {
                            return Some(format!("relax_degrees={relax:.3}"));
                        }
                    }
                    if let Some(limits) = obj.get("new_limits_degrees").and_then(|v| v.as_array()) {
                        if limits.len() == 2 {
                            let a = limits.first().and_then(|v| v.as_f64());
                            let b = limits.get(1).and_then(|v| v.as_f64());
                            if let (Some(a), Some(b)) = (a, b) {
                                if a.is_finite() && b.is_finite() {
                                    return Some(format!("new_limits_degrees=[{a:.3},{b:.3}]"));
                                }
                            }
                        }
                    }
                    let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                    keys.sort();
                    (!keys.is_empty())
                        .then(|| truncate_for_prompt(&format!("impact_keys={keys:?}"), 120))
                }

                let suggestions_arr = value.get("suggestions").and_then(|v| v.as_array());
                let suggestions_total = suggestions_arr.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let mut summary =
                    format!("ok suggestions={suggestions_total} truncated={truncated}");

                let Some(suggestions_arr) = suggestions_arr else {
                    out.push_str(&truncate_for_prompt(&summary, budget));
                    return out;
                };

                let mut items_text = String::new();
                let mut included = 0usize;
                for suggestion in suggestions_arr.iter().take(MAX_SUGGESTIONS) {
                    let id = suggestion.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let kind = suggestion
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let component_name = suggestion
                        .get("component_name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let channel = suggestion
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let impact = suggestion
                        .get("impact")
                        .and_then(|v| (!v.is_null()).then_some(v))
                        .and_then(impact_snip);

                    let apply_args_field = match suggestion.get("apply_draft_ops_args") {
                        Some(apply_args) if !apply_args.is_null() => {
                            let json = apply_args.to_string();
                            if char_count(&json) <= MAX_APPLY_ARGS_CHARS {
                                format!("apply_draft_ops_args={json}")
                            } else {
                                format!(
                                    "apply_draft_ops_args=<omitted chars={}>",
                                    char_count(&json)
                                )
                            }
                        }
                        _ => "apply_draft_ops_args=<missing>".to_string(),
                    };

                    let mut item = String::new();
                    item.push('{');
                    if !id.trim().is_empty() {
                        item.push_str("id=");
                        item.push_str(&truncate_for_prompt(id.trim(), 96));
                        item.push_str(", ");
                    }
                    if !kind.trim().is_empty() {
                        item.push_str("kind=");
                        item.push_str(&truncate_for_prompt(kind.trim(), 64));
                        item.push_str(", ");
                    }
                    if !component_name.trim().is_empty() {
                        item.push_str("component=");
                        item.push_str(&truncate_for_prompt(component_name.trim(), 64));
                        item.push_str(", ");
                    }
                    if !channel.trim().is_empty() {
                        item.push_str("channel=");
                        item.push_str(&truncate_for_prompt(channel.trim(), 64));
                        item.push_str(", ");
                    }
                    if let Some(impact) = impact.as_deref() {
                        if !impact.trim().is_empty() {
                            item.push_str("impact=");
                            item.push_str(&truncate_for_prompt(impact.trim(), 120));
                            item.push_str(", ");
                        }
                    }
                    item.push_str(&apply_args_field);
                    item.push('}');

                    let sep = if included == 0 { "" } else { ", " };
                    let omitted_after = suggestions_total.saturating_sub(included + 1);
                    let omitted_seg = if omitted_after > 0 {
                        format!(" omitted_suggestions={omitted_after}")
                    } else {
                        String::new()
                    };
                    let candidate_len = char_count(&summary)
                        + char_count(" items=[")
                        + char_count(&items_text)
                        + char_count(sep)
                        + char_count(&item)
                        + char_count("]")
                        + char_count(&omitted_seg);
                    if candidate_len > budget {
                        break;
                    }

                    items_text.push_str(sep);
                    items_text.push_str(&item);
                    included += 1;
                }

                if included > 0 {
                    summary.push_str(" items=[");
                    summary.push_str(&items_text);
                    summary.push(']');
                }
                let omitted = suggestions_total.saturating_sub(included);
                if omitted > 0 {
                    summary.push_str(&format!(" omitted_suggestions={omitted}"));
                }

                out.push_str(&truncate_for_prompt(&summary, budget));
            }
            TOOL_ID_GET_PLAN_TEMPLATE => {
                let plan_template_kv = value.get("plan_template_kv");
                let kv_key = plan_template_kv
                    .and_then(|v| v.get("key"))
                    .and_then(|v| v.as_str());
                let kv_rev = plan_template_kv
                    .and_then(|v| v.get("selector"))
                    .and_then(|v| v.get("kv_rev"))
                    .and_then(|v| v.as_u64());
                let bytes = value.get("bytes").and_then(|v| v.as_u64());
                let components_total = value.get("components_total").and_then(|v| v.as_u64());
                out.push_str("ok");
                if let Some(kv_key) = kv_key {
                    out.push_str(&format!(
                        " plan_template_kv={}",
                        truncate_for_prompt(kv_key, 96)
                    ));
                }
                if let Some(kv_rev) = kv_rev {
                    out.push_str(&format!(" kv_rev={kv_rev}"));
                }
                if let Some(bytes) = bytes {
                    out.push_str(&format!(" bytes={bytes}"));
                }
                if let Some(components_total) = components_total {
                    out.push_str(&format!(" components_total={components_total}"));
                }
            }
            TOOL_ID_APPLY_PLAN_OPS => {
                let accepted = value.get("accepted").and_then(|v| v.as_bool());
                let still_pending = value.get("still_pending").and_then(|v| v.as_bool());
                let committed = value.get("committed").and_then(|v| v.as_bool());
                let applied_ops = value
                    .get("applied_ops")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let rejected_ops = value
                    .get("rejected_ops")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                let new_errors = value
                    .get("new_errors")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len());
                out.push_str("ok");
                if let Some(accepted) = accepted {
                    out.push_str(&format!(" accepted={accepted}"));
                }
                if let Some(still_pending) = still_pending {
                    out.push_str(&format!(" still_pending={still_pending}"));
                }
                if let Some(committed) = committed {
                    out.push_str(&format!(" committed={committed}"));
                }
                if let Some(applied_ops) = applied_ops {
                    out.push_str(&format!(" applied_ops={applied_ops}"));
                }
                if let Some(rejected_ops) = rejected_ops {
                    out.push_str(&format!(" rejected_ops={rejected_ops}"));
                }
                if let Some(new_errors) = new_errors {
                    out.push_str(&format!(" new_errors={new_errors}"));
                }
            }
            TOOL_ID_INSPECT_PLAN => {
                let has_pending = value.get("has_pending_plan").and_then(|v| v.as_bool());
                let analysis_ok = value
                    .get("analysis")
                    .and_then(|v| v.get("ok"))
                    .and_then(|v| v.as_bool());
                let errors = value
                    .get("analysis")
                    .and_then(|v| v.get("errors"))
                    .and_then(|v| v.as_array());
                let first_error_kind = errors
                    .and_then(|e| e.first())
                    .and_then(|v| v.get("kind"))
                    .and_then(|v| v.as_str());
                out.push_str("ok");
                if let Some(has_pending) = has_pending {
                    out.push_str(&format!(" has_pending_plan={has_pending}"));
                }
                if let Some(analysis_ok) = analysis_ok {
                    out.push_str(&format!(" analysis_ok={analysis_ok}"));
                }
                if let Some(errors) = errors {
                    out.push_str(&format!(" errors={}", errors.len()));
                }
                if let Some(kind) = first_error_kind {
                    out.push_str(&format!(
                        " first_error_kind={}",
                        truncate_for_prompt(kind, 64)
                    ));
                }
            }
            TOOL_ID_INFO_EVENTS_LIST => {
                const MAX_LINE_CHARS: usize = 800;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());

                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                fn item_snip(item: &serde_json::Value, include_message: bool) -> String {
                    let event_id = item.get("event_id").and_then(|v| v.as_u64());
                    let kind = item
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let tool_id = item
                        .get("tool_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let call_id = item
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let pass = item.get("pass").and_then(|v| v.as_u64());
                    let message = item.get("message").and_then(|v| v.as_str()).unwrap_or("");
                    let message = one_line_snip(message, 120);

                    let mut out = String::new();
                    out.push('{');
                    if let Some(event_id) = event_id {
                        out.push_str(&format!("event_id={event_id} "));
                    }
                    if !kind.is_empty() {
                        out.push_str("kind=");
                        out.push_str(kind);
                        out.push(' ');
                    }
                    if !tool_id.is_empty() {
                        out.push_str("tool_id=");
                        out.push_str(tool_id);
                        out.push(' ');
                    }
                    if !call_id.is_empty() {
                        out.push_str("call_id=");
                        out.push_str(call_id);
                        out.push(' ');
                    }
                    if let Some(pass) = pass {
                        out.push_str(&format!("pass={pass} "));
                    }
                    if include_message && !message.trim().is_empty() {
                        out.push_str("message=");
                        out.push_str(message.trim());
                    }
                    out.push('}');
                    out
                }

                let mut base = format!("ok items={items_total} truncated={truncated}");

                let mut chosen_sample: Option<String> = None;
                for include_message in [true, false] {
                    let mut candidates: Vec<String> = Vec::new();
                    if let Some(items) = items {
                        candidates = items
                            .iter()
                            .take(3)
                            .map(|item| item_snip(item, include_message))
                            .collect();
                    }
                    for n in (1..=candidates.len()).rev() {
                        let joined = candidates
                            .iter()
                            .take(n)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        let candidate = format!("{base} first=[{joined}]{cursor_seg}");
                        if char_count(&candidate) <= budget {
                            chosen_sample = Some(format!(" first=[{joined}]"));
                            break;
                        }
                    }
                    if chosen_sample.is_some() {
                        break;
                    }
                }

                let mut summary = base.clone();
                if let Some(sample) = chosen_sample.as_deref() {
                    if char_count(&(summary.clone() + sample + &cursor_seg)) <= budget {
                        summary.push_str(sample);
                    }
                }
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    // Drop truncated/sample before losing the cursor.
                    base = format!("ok items={items_total}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!("ok items={items_total} next_cursor={cursor}");
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out = format!("ok items={items_total}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }
                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_EVENTS_SEARCH => {
                const MAX_LINE_CHARS: usize = 800;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let matches = value.get("matches").and_then(|v| v.as_array());
                let matches_total = matches.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());
                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                fn match_snip(item: &serde_json::Value, include_message: bool) -> String {
                    let event_id = item.get("event_id").and_then(|v| v.as_u64());
                    let kind = item
                        .get("kind")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let message = item
                        .get("message_excerpt")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let message = one_line_snip(message, 120);

                    let mut out = String::new();
                    out.push('{');
                    if let Some(event_id) = event_id {
                        out.push_str(&format!("event_id={event_id} "));
                    }
                    if !kind.is_empty() {
                        out.push_str("kind=");
                        out.push_str(kind);
                        out.push(' ');
                    }
                    if include_message && !message.trim().is_empty() {
                        out.push_str("message_excerpt=");
                        out.push_str(message.trim());
                    }
                    out.push('}');
                    out
                }

                let base = format!("ok matches={matches_total} truncated={truncated}");

                let mut chosen_sample: Option<String> = None;
                for include_message in [true, false] {
                    let mut candidates: Vec<String> = Vec::new();
                    if let Some(matches) = matches {
                        candidates = matches
                            .iter()
                            .take(3)
                            .map(|item| match_snip(item, include_message))
                            .collect();
                    }
                    for n in (1..=candidates.len()).rev() {
                        let joined = candidates
                            .iter()
                            .take(n)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                        let candidate = format!("{base} first=[{joined}]{cursor_seg}");
                        if char_count(&candidate) <= budget {
                            chosen_sample = Some(format!(" first=[{joined}]"));
                            break;
                        }
                    }
                    if chosen_sample.is_some() {
                        break;
                    }
                }

                let mut summary = base.clone();
                if let Some(sample) = chosen_sample.as_deref() {
                    if char_count(&(summary.clone() + sample + &cursor_seg)) <= budget {
                        summary.push_str(sample);
                    }
                }
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    // Drop truncated/sample before losing the cursor.
                    let base = format!("ok matches={matches_total}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!("ok matches={matches_total} next_cursor={cursor}");
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out = format!("ok matches={matches_total}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }
                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_EVENTS_GET => {
                const MAX_LINE_CHARS: usize = 1000;

                let Some(event) = value.get("event").and_then(|v| v.as_object()) else {
                    out.push_str("ok");
                    return out;
                };

                let event_id = event.get("event_id").and_then(|v| v.as_u64());
                let kind = event
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let tool_id = event
                    .get("tool_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let call_id = event
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let pass = event.get("pass").and_then(|v| v.as_u64());
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let json_pointer = value
                    .get("json_pointer")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();

                out.push_str("ok");
                if let Some(event_id) = event_id {
                    out.push_str(&format!(" event_id={event_id}"));
                }
                if !kind.is_empty() {
                    out.push_str(&format!(" kind={}", truncate_for_prompt(kind, 64)));
                }
                if !tool_id.is_empty() {
                    out.push_str(&format!(" tool_id={}", truncate_for_prompt(tool_id, 64)));
                }
                if !call_id.is_empty() {
                    out.push_str(&format!(" call_id={}", truncate_for_prompt(call_id, 32)));
                }
                if let Some(pass) = pass {
                    out.push_str(&format!(" pass={pass}"));
                }
                out.push_str(&format!(" truncated={truncated}"));
                if !json_pointer.is_empty() {
                    out.push_str(&format!(
                        " json_pointer={}",
                        truncate_for_prompt(json_pointer, 96)
                    ));
                }

                if let Some(data) = event.get("data") {
                    if !data.is_null() {
                        let json = data.to_string();
                        let remaining = MAX_LINE_CHARS.saturating_sub(char_count(&out));
                        let needed = char_count(" data=") + char_count(&json);
                        if char_count(&json) <= 320 && needed <= remaining {
                            out.push_str(" data=");
                            out.push_str(&json);
                        } else if let Some(obj) = data.as_object() {
                            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                            keys.sort();
                            if !keys.is_empty() {
                                out.push_str(" data_keys=");
                                out.push_str(&truncate_for_prompt(&format!("{keys:?}"), 160));
                            }
                        } else if let Some(arr) = data.as_array() {
                            out.push_str(&format!(" data_len={}", arr.len()));
                        } else if let Some(s) = data.as_str() {
                            out.push_str(" data=");
                            out.push_str(&one_line_snip(s, 200));
                        } else {
                            out.push_str(" data_type=");
                            out.push_str(match data {
                                serde_json::Value::Bool(_) => "bool",
                                serde_json::Value::Number(_) => "number",
                                serde_json::Value::String(_) => "string",
                                serde_json::Value::Array(_) => "array",
                                serde_json::Value::Object(_) => "object",
                                serde_json::Value::Null => "null",
                            });
                        }
                    }
                }

                if char_count(&out) > MAX_LINE_CHARS {
                    out = truncate_for_prompt(&out, MAX_LINE_CHARS);
                }
            }
            TOOL_ID_INFO_KV_GET => {
                const MAX_LINE_CHARS: usize = 900;

                let record = value.get("record").and_then(|v| v.as_object());
                let key = record
                    .and_then(|r| r.get("key"))
                    .and_then(|v| v.as_object());
                let namespace = key
                    .and_then(|k| k.get("namespace"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let kv_key = key
                    .and_then(|k| k.get("key"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let kv_rev = record
                    .and_then(|r| r.get("kv_rev"))
                    .and_then(|v| v.as_u64());
                let summary = record
                    .and_then(|r| r.get("summary"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let json_pointer = value
                    .get("json_pointer")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let cached = value
                    .get("cached")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let no_new_information = value
                    .get("no_new_information")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                out.push_str("ok");
                if !namespace.is_empty() {
                    out.push_str(&format!(
                        " namespace={}",
                        truncate_for_prompt(namespace, 32)
                    ));
                }
                if !kv_key.is_empty() {
                    out.push_str(&format!(" key={}", truncate_for_prompt(kv_key, 96)));
                }
                if let Some(kv_rev) = kv_rev {
                    out.push_str(&format!(" kv_rev={kv_rev}"));
                }
                if !summary.is_empty() {
                    out.push_str(&format!(" summary={}", one_line_snip(summary, 160)));
                }
                if !json_pointer.is_empty() {
                    out.push_str(&format!(
                        " json_pointer={}",
                        truncate_for_prompt(json_pointer, 96)
                    ));
                }
                if cached {
                    out.push_str(" cached=true");
                }
                if no_new_information {
                    out.push_str(" no_new_information=true");
                }

                if let Some(selected_value) = value.get("value") {
                    if let Some(obj) = selected_value.as_object() {
                        if let Some(ok) = obj.get("ok").and_then(|v| v.as_bool()) {
                            out.push_str(&format!(" value_ok={ok}"));
                        }
                        if let Some(errors) = obj.get("errors").and_then(|v| v.as_array()) {
                            out.push_str(&format!(" errors={}", errors.len()));
                        }
                        if let Some(warnings) = obj.get("warnings").and_then(|v| v.as_array()) {
                            out.push_str(&format!(" warnings={}", warnings.len()));
                        }
                        if let Some(issues) = obj.get("issues").and_then(|v| v.as_array()) {
                            out.push_str(&format!(" issues={}", issues.len()));
                        }
                        if obj.get("ok").is_none()
                            && obj.get("errors").is_none()
                            && obj.get("warnings").is_none()
                            && obj.get("issues").is_none()
                        {
                            let mut keys: Vec<&str> = obj.keys().map(|k| k.as_str()).collect();
                            keys.sort();
                            if !keys.is_empty() {
                                out.push_str(" value_keys=");
                                out.push_str(&truncate_for_prompt(&format!("{keys:?}"), 160));
                            }
                        }
                    } else if let Some(arr) = selected_value.as_array() {
                        out.push_str(&format!(" value_len={}", arr.len()));
                    } else if let Some(s) = selected_value.as_str() {
                        out.push_str(" value=");
                        out.push_str(&one_line_snip(s, 200));
                    }
                }

                if char_count(&out) > MAX_LINE_CHARS {
                    out = truncate_for_prompt(&out, MAX_LINE_CHARS);
                }
            }
            TOOL_ID_INFO_KV_GET_PAGED => {
                const MAX_LINE_CHARS: usize = 900;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let record = value.get("record").and_then(|v| v.as_object());
                let key = record
                    .and_then(|r| r.get("key"))
                    .and_then(|v| v.as_object());
                let namespace = key
                    .and_then(|k| k.get("namespace"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let kv_key = key
                    .and_then(|k| k.get("key"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let kv_rev = record
                    .and_then(|r| r.get("kv_rev"))
                    .and_then(|v| v.as_u64());
                let json_pointer = value
                    .get("json_pointer")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let array_len = value.get("array_len").and_then(|v| v.as_u64()).unwrap_or(0);
                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());
                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                let (idx_first, idx_last) = items
                    .and_then(|items| {
                        let first = items
                            .first()
                            .and_then(|v| v.get("index"))
                            .and_then(|v| v.as_u64());
                        let last = items
                            .last()
                            .and_then(|v| v.get("index"))
                            .and_then(|v| v.as_u64());
                        Some((first, last))
                    })
                    .unwrap_or((None, None));

                let mut base = String::new();
                base.push_str("ok");
                if !namespace.is_empty() {
                    base.push_str(&format!(
                        " namespace={}",
                        truncate_for_prompt(namespace, 32)
                    ));
                }
                if !kv_key.is_empty() {
                    base.push_str(&format!(" key={}", truncate_for_prompt(kv_key, 96)));
                }
                if let Some(kv_rev) = kv_rev {
                    base.push_str(&format!(" kv_rev={kv_rev}"));
                }
                if !json_pointer.is_empty() {
                    base.push_str(&format!(
                        " json_pointer={}",
                        truncate_for_prompt(json_pointer, 96)
                    ));
                }
                base.push_str(&format!(" array_len={array_len}"));
                base.push_str(&format!(" items={items_total} truncated={truncated}"));
                if let (Some(a), Some(b)) = (idx_first, idx_last) {
                    base.push_str(&format!(" index_range={a}..={b}"));
                }

                let mut summary = base.clone();
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    let base = format!("ok items={items_total} array_len={array_len}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!(
                            "ok items={items_total} array_len={array_len} next_cursor={cursor}"
                        );
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out =
                                    format!("ok items={items_total} array_len={array_len}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }

                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_KV_LIST_KEYS => {
                const MAX_LINE_CHARS: usize = 800;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());
                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                let mut base = format!("ok items={items_total} truncated={truncated}");
                let mut sample = String::new();
                if let Some(items) = items {
                    let mut parts: Vec<String> = Vec::new();
                    for item in items.iter().take(3) {
                        let ns = item
                            .get("namespace")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let key = item
                            .get("key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let latest = item.get("latest").and_then(|v| v.as_object());
                        let kv_rev = latest
                            .and_then(|o| o.get("kv_rev"))
                            .and_then(|v| v.as_u64());
                        let summary = latest
                            .and_then(|o| o.get("summary"))
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if ns.is_empty() && key.is_empty() {
                            continue;
                        }
                        let mut s = String::new();
                        s.push('{');
                        if !ns.is_empty() {
                            s.push_str("ns=");
                            s.push_str(ns);
                            s.push(' ');
                        }
                        if !key.is_empty() {
                            s.push_str("key=");
                            s.push_str(&truncate_for_prompt(key, 64));
                            s.push(' ');
                        }
                        if let Some(kv_rev) = kv_rev {
                            s.push_str(&format!("kv_rev={kv_rev} "));
                        }
                        let summary = one_line_snip(summary, 80);
                        if !summary.trim().is_empty() {
                            s.push_str("summary=");
                            s.push_str(summary.trim());
                        }
                        s.push('}');
                        parts.push(s);
                    }
                    if !parts.is_empty() {
                        sample = format!(" sample=[{}]", parts.join(", "));
                    }
                }

                let mut summary = base.clone();
                if !sample.is_empty()
                    && char_count(&(summary.clone() + &sample + &cursor_seg)) <= budget
                {
                    summary.push_str(&sample);
                }
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    base = format!("ok items={items_total}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!("ok items={items_total} next_cursor={cursor}");
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out = format!("ok items={items_total}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }
                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_KV_LIST_HISTORY => {
                const MAX_LINE_CHARS: usize = 800;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());
                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                let base = format!("ok items={items_total} truncated={truncated}");
                let mut sample = String::new();
                if let Some(items) = items {
                    let mut parts: Vec<String> = Vec::new();
                    for item in items.iter().take(3) {
                        let kv_rev = item.get("kv_rev").and_then(|v| v.as_u64());
                        let summary = item.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                        let mut s = String::new();
                        s.push('{');
                        if let Some(kv_rev) = kv_rev {
                            s.push_str(&format!("kv_rev={kv_rev} "));
                        }
                        let summary = one_line_snip(summary, 100);
                        if !summary.trim().is_empty() {
                            s.push_str("summary=");
                            s.push_str(summary.trim());
                        }
                        s.push('}');
                        parts.push(s);
                    }
                    if !parts.is_empty() {
                        sample = format!(" sample=[{}]", parts.join(", "));
                    }
                }

                let mut summary = base.clone();
                if !sample.is_empty()
                    && char_count(&(summary.clone() + &sample + &cursor_seg)) <= budget
                {
                    summary.push_str(&sample);
                }
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    let base = format!("ok items={items_total}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!("ok items={items_total} next_cursor={cursor}");
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out = format!("ok items={items_total}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }
                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_KV_GET_MANY => {
                const MAX_LINE_CHARS: usize = 900;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let cached = value
                    .get("cached")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let no_new_information = value
                    .get("no_new_information")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let mut ok_total = 0usize;
                let mut err_total = 0usize;
                let mut samples: Vec<String> = Vec::new();
                if let Some(items) = items {
                    for item in items.iter().take(3) {
                        let ns = item
                            .get("namespace")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let key = item
                            .get("key")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let ok = item.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                        if ok {
                            ok_total += 1;
                        } else {
                            err_total += 1;
                        }
                        let kv_rev = item
                            .get("record")
                            .and_then(|v| v.get("kv_rev"))
                            .and_then(|v| v.as_u64());
                        let mut s = String::new();
                        s.push('{');
                        if !ns.is_empty() {
                            s.push_str("ns=");
                            s.push_str(ns);
                            s.push(' ');
                        }
                        if !key.is_empty() {
                            s.push_str("key=");
                            s.push_str(&truncate_for_prompt(key, 64));
                            s.push(' ');
                        }
                        s.push_str(&format!("ok={ok} "));
                        if let Some(kv_rev) = kv_rev {
                            s.push_str(&format!("kv_rev={kv_rev} "));
                        }
                        if !ok {
                            let err = item.get("error").and_then(|v| v.as_str()).unwrap_or("");
                            let err = one_line_snip(err, 120);
                            if !err.trim().is_empty() {
                                s.push_str("error=");
                                s.push_str(err.trim());
                            }
                        }
                        s.push('}');
                        samples.push(s);
                    }
                    for item in items.iter().skip(3) {
                        let ok = item.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                        if ok {
                            ok_total += 1;
                        } else {
                            err_total += 1;
                        }
                    }
                }

                let mut summary =
                    format!("ok items={items_total} ok_items={ok_total} err_items={err_total} truncated={truncated}");
                if cached {
                    summary.push_str(" cached=true");
                }
                if no_new_information {
                    summary.push_str(" no_new_information=true");
                }
                if !samples.is_empty() {
                    let sample = format!(" sample=[{}]", samples.join(", "));
                    if char_count(&(summary.clone() + &sample)) <= budget {
                        summary.push_str(&sample);
                    }
                }
                out.push_str(&truncate_for_prompt(&summary, budget));
            }
            TOOL_ID_INFO_BLOBS_LIST => {
                const MAX_LINE_CHARS: usize = 800;
                let budget = MAX_LINE_CHARS.saturating_sub(char_count(&out));

                let items = value.get("items").and_then(|v| v.as_array());
                let items_total = items.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let next_cursor = value.get("next_cursor").and_then(|v| v.as_str());
                let cursor_seg = next_cursor
                    .map(|c| format!(" next_cursor={c}"))
                    .unwrap_or_default();

                let base = format!("ok items={items_total} truncated={truncated}");
                let mut sample = String::new();
                if let Some(items) = items {
                    let mut parts: Vec<String> = Vec::new();
                    for item in items.iter().take(3) {
                        let blob_id = item
                            .get("blob_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let bytes = item.get("bytes").and_then(|v| v.as_u64());
                        let content_type = item
                            .get("content_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        if blob_id.is_empty() {
                            continue;
                        }
                        let mut s = String::new();
                        s.push('{');
                        s.push_str("blob_id=");
                        s.push_str(&truncate_for_prompt(blob_id, 64));
                        if let Some(bytes) = bytes {
                            s.push_str(&format!(" bytes={bytes}"));
                        }
                        if !content_type.is_empty() {
                            s.push_str(" type=");
                            s.push_str(&truncate_for_prompt(content_type, 48));
                        }
                        s.push('}');
                        parts.push(s);
                    }
                    if !parts.is_empty() {
                        sample = format!(" sample=[{}]", parts.join(", "));
                    }
                }

                let mut summary = base.clone();
                if !sample.is_empty()
                    && char_count(&(summary.clone() + &sample + &cursor_seg)) <= budget
                {
                    summary.push_str(&sample);
                }
                if !cursor_seg.is_empty() && char_count(&(summary.clone() + &cursor_seg)) <= budget
                {
                    summary.push_str(&cursor_seg);
                } else if !cursor_seg.is_empty() {
                    let base = format!("ok items={items_total}");
                    summary = base.clone();
                    if char_count(&(summary.clone() + &cursor_seg)) <= budget {
                        summary.push_str(&cursor_seg);
                    }
                }

                let mut summary_out = summary;
                if char_count(&summary_out) > budget {
                    if let Some(cursor) = next_cursor.map(str::trim).filter(|s| !s.is_empty()) {
                        let minimal = format!("ok items={items_total} next_cursor={cursor}");
                        if char_count(&minimal) <= budget {
                            summary_out = minimal;
                        } else {
                            let cursor_only = format!("next_cursor={cursor}");
                            if char_count(&cursor_only) <= budget {
                                summary_out = cursor_only;
                            } else {
                                summary_out = format!("ok items={items_total}");
                            }
                        }
                    } else {
                        summary_out = truncate_for_prompt(&summary_out, budget);
                    }
                }
                out.push_str(&summary_out);
            }
            TOOL_ID_INFO_BLOBS_GET => {
                const MAX_LINE_CHARS: usize = 500;

                let blob = value.get("blob").and_then(|v| v.as_object());
                let blob_id = blob
                    .and_then(|b| b.get("blob_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let bytes = blob.and_then(|b| b.get("bytes")).and_then(|v| v.as_u64());
                let content_type = blob
                    .and_then(|b| b.get("content_type"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();

                out.push_str("ok");
                if !blob_id.is_empty() {
                    out.push_str(&format!(" blob_id={}", truncate_for_prompt(blob_id, 96)));
                }
                if let Some(bytes) = bytes {
                    out.push_str(&format!(" bytes={bytes}"));
                }
                if !content_type.is_empty() {
                    out.push_str(&format!(
                        " content_type={}",
                        truncate_for_prompt(content_type, 64)
                    ));
                }

                if char_count(&out) > MAX_LINE_CHARS {
                    out = truncate_for_prompt(&out, MAX_LINE_CHARS);
                }
            }
            TOOL_ID_APPLY_DRAFT_OPS => {
                const MAX_LINE_CHARS: usize = 900;

                let ok = value.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
                let committed = value
                    .get("committed")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let atomic = value
                    .get("atomic")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let new_assembly_rev = value.get("new_assembly_rev").and_then(|v| v.as_u64());
                let applied_ops = value
                    .get("applied_ops")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);
                let rejected_ops = value
                    .get("rejected_ops")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0);

                out.push_str("ok");
                out.push_str(&format!(" ok={ok} committed={committed} atomic={atomic}"));
                if let Some(new_assembly_rev) = new_assembly_rev {
                    out.push_str(&format!(" new_assembly_rev={new_assembly_rev}"));
                }
                out.push_str(&format!(
                    " applied_ops={applied_ops} rejected_ops={rejected_ops}"
                ));

                if let Some(diff) = value.get("diff_summary").and_then(|v| v.as_object()) {
                    let anchors = diff
                        .get("anchors_updated")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let attachments = diff
                        .get("attachments_updated")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let prim = diff.get("primitive_parts").and_then(|v| v.as_object());
                    let prim_added = prim
                        .and_then(|o| o.get("added"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let prim_removed = prim
                        .and_then(|o| o.get("removed"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let prim_updated = prim
                        .and_then(|o| o.get("updated"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let anim = diff.get("animation_slots").and_then(|v| v.as_object());
                    let anim_upserted = anim
                        .and_then(|o| o.get("upserted"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let anim_scaled = anim
                        .and_then(|o| o.get("scaled"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let anim_removed = anim
                        .and_then(|o| o.get("removed"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    out.push_str(&format!(
                        " diff={{anchors:{anchors},attachments:{attachments},prim:+{prim_added}/-{prim_removed}/~{prim_updated},anim:upsert:{anim_upserted},scale:{anim_scaled},rm:{anim_removed}}}"
                    ));
                }

                if let Some(info_kv) = value.get("info_kv").and_then(|v| v.as_object()) {
                    let namespace = info_kv
                        .get("namespace")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let key = info_kv
                        .get("key")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim();
                    let kv_rev = info_kv
                        .get("selector")
                        .and_then(|v| v.get("kv_rev"))
                        .and_then(|v| v.as_u64());
                    if !namespace.is_empty() || !key.is_empty() || kv_rev.is_some() {
                        out.push_str(" info_kv={");
                        if !namespace.is_empty() {
                            out.push_str("namespace=");
                            out.push_str(&truncate_for_prompt(namespace, 32));
                            out.push(' ');
                        }
                        if !key.is_empty() {
                            out.push_str("key=");
                            out.push_str(&truncate_for_prompt(key, 96));
                            out.push(' ');
                        }
                        if let Some(kv_rev) = kv_rev {
                            out.push_str(&format!("kv_rev={kv_rev}"));
                        }
                        out.push('}');
                    }
                }

                if char_count(&out) > MAX_LINE_CHARS {
                    out = truncate_for_prompt(&out, MAX_LINE_CHARS);
                }
            }
            TOOL_ID_SNAPSHOT => {
                let snapshot_id = value
                    .get("snapshot_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let label = value
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let assembly_rev = value.get("assembly_rev").and_then(|v| v.as_u64());
                out.push_str("ok");
                if !snapshot_id.is_empty() {
                    out.push_str(&format!(
                        " snapshot_id={}",
                        truncate_for_prompt(snapshot_id, 64)
                    ));
                }
                if !label.is_empty() {
                    out.push_str(&format!(" label={}", one_line_snip(label, 64)));
                }
                if let Some(assembly_rev) = assembly_rev {
                    out.push_str(&format!(" assembly_rev={assembly_rev}"));
                }
            }
            TOOL_ID_LIST_SNAPSHOTS => {
                let snaps = value.get("snapshots").and_then(|v| v.as_array());
                let total = snaps.map(|a| a.len()).unwrap_or(0);
                let truncated = value
                    .get("truncated")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                out.push_str("ok");
                out.push_str(&format!(" snapshots={total} truncated={truncated}"));
                if let Some(snaps) = snaps {
                    let mut ids: Vec<String> = Vec::new();
                    for s in snaps.iter().take(3) {
                        let id = s
                            .get("snapshot_id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        let label = s.get("label").and_then(|v| v.as_str()).unwrap_or("").trim();
                        if id.is_empty() {
                            continue;
                        }
                        if label.is_empty() {
                            ids.push(id.to_string());
                        } else {
                            ids.push(format!("{}({})", id, one_line_snip(label, 32)));
                        }
                    }
                    if !ids.is_empty() {
                        out.push_str(&format!(
                            " sample={}",
                            truncate_for_prompt(&format!("{ids:?}"), 200)
                        ));
                    }
                }
            }
            TOOL_ID_DIFF_SNAPSHOTS => {
                let a = value.get("a").and_then(|v| v.as_object());
                let b = value.get("b").and_then(|v| v.as_object());
                let a_id = a
                    .and_then(|o| o.get("snapshot_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let b_id = b
                    .and_then(|o| o.get("snapshot_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let summary = value.get("diff_summary").and_then(|v| v.as_object());
                let changed = summary
                    .and_then(|o| o.get("components_changed"))
                    .and_then(|v| v.as_u64());
                out.push_str("ok");
                if !a_id.is_empty() {
                    out.push_str(&format!(" a={}", truncate_for_prompt(a_id, 64)));
                }
                if !b_id.is_empty() {
                    out.push_str(&format!(" b={}", truncate_for_prompt(b_id, 64)));
                }
                if let Some(changed) = changed {
                    out.push_str(&format!(" components_changed={changed}"));
                }
                if let Some(summary) = summary {
                    let geo = summary
                        .get("geometry_changed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let anc = summary
                        .get("anchors_changed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let att = summary
                        .get("attachments_changed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let anim = summary
                        .get("animations_changed")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    out.push_str(&format!(
                        " diff={{geo:{geo},anchors:{anc},attachments:{att},anim:{anim}}}"
                    ));
                }
            }
            TOOL_ID_RESTORE_SNAPSHOT => {
                let snapshot_id = value
                    .get("snapshot_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .trim();
                let before = value.get("assembly_rev_before").and_then(|v| v.as_u64());
                let after = value.get("assembly_rev_after").and_then(|v| v.as_u64());
                out.push_str("ok");
                if !snapshot_id.is_empty() {
                    out.push_str(&format!(
                        " snapshot_id={}",
                        truncate_for_prompt(snapshot_id, 64)
                    ));
                }
                if let (Some(before), Some(after)) = (before, after) {
                    out.push_str(&format!(" assembly_rev={before}->{after}"));
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
            TOOL_ID_GET_TOOL_DETAIL => {
                let tool = value.get("tool").and_then(|v| v.as_object());
                out.push_str("ok");
                if let Some(tool) = tool {
                    let tool_id = tool.get("tool_id").and_then(|v| v.as_str()).unwrap_or("");
                    let args_schema = tool
                        .get("args_schema")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    let args_example = tool.get("args_example");

                    if !tool_id.trim().is_empty() {
                        out.push_str(&format!(" tool_id={tool_id}"));
                    }

                    out.push_str(&format!(
                        " args_schema={}",
                        truncate_for_prompt(args_schema, 240)
                    ));
                    if let Some(args_example) = args_example {
                        out.push_str(&format!(
                            " args_example={}",
                            truncate_for_prompt(&args_example.to_string(), 240)
                        ));
                    }
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
    out.push_str(&format!("Input images: {}\n", job.user_images.len()));
    if let Some(summary) = job
        .user_image_object_summary
        .as_ref()
        .map(|s| s.text.trim())
        .filter(|s| !s.is_empty())
    {
        out.push_str("\nReference image main-object summary:\n");
        out.push_str(summary);
        out.push('\n');
    }
    out.push('\n');

    let tools = registry.list();

    out.push_str("Available tools (args signature + example shown; call get_tool_detail_v1 for full schema/examples):\n");
    for tool in &tools {
        let args_sig = first_line(tool.args_schema).trim();
        let args_sig = if args_sig.is_empty() { "{}" } else { args_sig };
        let args_example = truncate_for_prompt(&tool.args_example.to_string(), 200);
        out.push_str(&format!(
            "- {}: {} args={} example={}\n",
            tool.tool_id,
            tool.one_line_summary,
            truncate_for_prompt(args_sig, 240),
            args_example
        ));
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
            out.push_str(&summarize_tool_result(r, &tools));
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
    let no_progress_tries_remaining = if config.gen3d_no_progress_tries_max == 0 {
        None
    } else {
        Some(
            config
                .gen3d_no_progress_tries_max
                .saturating_sub(job.agent.no_progress_tries),
        )
    };
    let inspection_steps_remaining = if config.gen3d_inspection_steps_max == 0 {
        None
    } else {
        Some(
            config
                .gen3d_inspection_steps_max
                .saturating_sub(job.agent.no_progress_inspection_steps),
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
    let review_delta_rounds_remaining = config
        .gen3d_review_delta_rounds_max
        .saturating_sub(job.review_delta_rounds_used);

    let motion_authoring_status = {
        match job.motion_authoring.as_ref() {
            Some(authored) => {
                let applies_to_current = job.motion_authoring_for_current_draft().is_some();
                let decision = match authored.decision {
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

    let descriptor_meta_seeded = job
        .seed_descriptor_meta
        .as_ref()
        .map(|meta| serde_json::json!({ "short": meta.short.as_str(), "tags": &meta.tags }))
        .unwrap_or(serde_json::Value::Null);
    let descriptor_meta_override = job
        .descriptor_meta_override
        .as_ref()
        .map(|meta| serde_json::json!({ "short": meta.short.as_str(), "tags": &meta.tags }))
        .unwrap_or(serde_json::Value::Null);
    let descriptor_meta_effective = job
        .descriptor_meta_for_save()
        .map(|(policy, meta)| {
            serde_json::json!({
                "policy": match policy {
                    super::Gen3dDescriptorMetaPolicy::Suggest => "suggest",
                    super::Gen3dDescriptorMetaPolicy::Preserve => "preserve",
                },
                "short": meta.short.as_str(),
                "tags": &meta.tags,
            })
        })
        .unwrap_or(serde_json::Value::Null);

    serde_json::json!({
        "run_id": job.run_id.map(|id| id.to_string()),
        "seed": match (job.edit_base_prefab_id, job.save_overwrite_prefab_id) {
            (Some(prefab_id), Some(_)) => serde_json::json!({
                "kind": "edit_overwrite",
                "prefab_id": Uuid::from_u128(prefab_id).to_string(),
            }),
            (Some(prefab_id), None) => serde_json::json!({
                "kind": "fork",
                "prefab_id": Uuid::from_u128(prefab_id).to_string(),
            }),
            (None, Some(prefab_id)) => serde_json::json!({
                "kind": "edit_overwrite",
                "prefab_id": Uuid::from_u128(prefab_id).to_string(),
            }),
            (None, None) => serde_json::Value::Null,
        },
        "descriptor_meta": {
            "seeded": descriptor_meta_seeded,
            "override": descriptor_meta_override,
            "effective": descriptor_meta_effective,
        },
        "attempt": job.attempt,
        "pass": job.pass,
        "plan_hash": job.plan_hash,
        "preserve_existing_components_mode": job.preserve_existing_components_mode,
        "assembly_rev": job.assembly_rev,
        "motion_authoring": motion_authoring_status,
        "motion_coverage": motion_coverage,
        "review_appearance": job.review_appearance,
        "needs_review": job.agent.rendered_since_last_review,
        "no_progress": {
            "tries": job.agent.no_progress_tries,
            "inspection_steps": job.agent.no_progress_inspection_steps,
        },
        "pending_regen_component_indices": &job.agent.pending_regen_component_indices,
        "pending_regen_component_indices_skipped_due_to_budget": &job
            .agent
            .pending_regen_component_indices_skipped_due_to_budget,
        "pending_regen_component_indices_blocked_due_to_qa_gate": &job
            .agent
            .pending_regen_component_indices_blocked_due_to_qa_gate,
        "reuse_groups": reuse_groups_json,
        "reuse_group_warnings": &job.reuse_group_warnings,
        "missing_only_generation_indices": missing_only_generation_indices,
        "reuse_suggestions": build_reuse_suggestions(&job.planned_components),
        "qa": {
            "ever_rendered": job.agent.ever_rendered,
            "ever_reviewed": job.agent.ever_reviewed,
            "ever_validated": job.agent.ever_validated,
            "ever_smoke_checked": job.agent.ever_smoke_checked,
            "last_validate_ok": job.agent.last_validate_ok,
            "last_smoke_ok": job.agent.last_smoke_ok,
            "last_motion_ok": job.agent.last_motion_ok,
            "done_ignored_due_to_qa_errors": job.agent.done_ignored_due_to_qa_errors,
        },
        "budgets": {
            "regen": {
                "max_total": config.gen3d_max_regen_total,
                "used_total": job.regen_total,
                "remaining_total": regen_remaining_total,
                "max_per_component": config.gen3d_max_regen_per_component,
            },
            "review_delta": {
                "rounds_max": config.gen3d_review_delta_rounds_max,
                "rounds_used": job.review_delta_rounds_used,
                "rounds_remaining": review_delta_rounds_remaining,
            },
            "no_progress": {
                "tries_max": config.gen3d_no_progress_tries_max,
                "tries_used": job.agent.no_progress_tries,
                "tries_remaining": no_progress_tries_remaining,
                "inspection_steps_max": config.gen3d_inspection_steps_max,
                "inspection_steps_used": job.agent.no_progress_inspection_steps,
                "inspection_steps_remaining": inspection_steps_remaining,
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
        "last_render_blob_ids": job
            .agent
            .last_render_blob_ids
            .iter()
            .rev()
            .take(12)
            .cloned()
            .collect::<Vec<String>>()
            .into_iter()
            .rev()
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::gen3d::state::Gen3dWorkshop;

    #[test]
    fn agent_system_instructions_ignore_warnings_policy_is_present() {
        let text = build_agent_system_instructions();
        assert!(text.contains("do NOT spend steps trying to eliminate warnings"));
        assert!(text.contains("Do NOT chase warn-only motion_validation issues"));
        assert!(text.contains("attack_self_intersection"));
    }

    #[test]
    fn agent_system_instructions_forbid_empty_args_unless_no_arg_tool() {
        let text = build_agent_system_instructions();
        assert!(text.contains(
            "Only call a tool with empty `{}` args if its args signature is exactly `{}`"
        ));
    }

    #[test]
    fn agent_prompt_does_not_mention_removed_artifact_tools_or_paths() {
        let system = build_agent_system_instructions();
        for forbidden in [
            "list_run_artifacts_v1",
            "read_artifact_v1",
            "search_artifacts_v1",
            "artifact_ref",
            "plan_template_artifact_ref",
            "preview_images",
            "/.gravimera/cache/gen3d/",
        ] {
            assert!(
                !system.contains(forbidden),
                "system instructions should not contain {forbidden:?}"
            );
        }

        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();
        let user_text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &[],
            &registry,
        );
        for forbidden in [
            "list_run_artifacts_v1",
            "read_artifact_v1",
            "search_artifacts_v1",
            "artifact_ref",
            "plan_template_artifact_ref",
            "preview_images",
            "/.gravimera/cache/gen3d/",
        ] {
            assert!(
                !user_text.contains(forbidden),
                "user text should not contain {forbidden:?}"
            );
        }

        let tool_ids = registry
            .list()
            .into_iter()
            .map(|d| d.tool_id.to_string())
            .collect::<Vec<_>>();
        for forbidden in [
            "list_run_artifacts_v1",
            "read_artifact_v1",
            "search_artifacts_v1",
        ] {
            assert!(
                !tool_ids.iter().any(|t| t == forbidden),
                "tool registry should not include {forbidden:?}"
            );
        }
    }

    #[test]
    fn agent_user_text_includes_tool_args_signatures() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &[],
            &registry,
        );

        assert!(text.contains("Available tools (args signature + example shown"));
        assert!(text
            .lines()
            .any(|line| line.contains("- qa_v1:") && line.contains("args={ force?: bool")));
        assert!(text.lines().any(|line| {
            line.contains("- info_events_search_v1:") && line.contains("args={ query: string")
        }));
    }

    #[test]
    fn error_tool_results_include_contract_hints() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let recent_tool_results = vec![Gen3dToolResultJsonV1::err(
            "call_1".to_string(),
            "info_events_search_v1".to_string(),
            "Missing args.query".to_string(),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        assert!(text.contains("Recent tool results (previous step, compact):"));
        assert!(text.contains("info_events_search_v1 (call_1): ERROR: Missing args.query"));
        assert!(text.contains("expected_args={ query: string"));
        assert!(text.contains("required_keys=[\"query\"]"));
        assert!(text.contains("example="));
        assert!(text.contains("\"query\""));
    }

    #[test]
    fn summarize_suggest_motion_repairs_includes_inline_apply_args_when_small() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let patch = serde_json::json!({
            "version": 1,
            "atomic": true,
            "if_assembly_rev": 7,
            "ops": [
                {
                    "kind": "set_attachment_joint",
                    "child_component": "arm",
                    "set_joint": { "kind": "hinge", "axis_join": [1.0, 0.0, 0.0], "limits_degrees": [-30.0, 90.0] },
                }
            ]
        });
        let expected_json = patch.to_string();

        let recent_tool_results = vec![Gen3dToolResultJsonV1::ok(
            "call_1".to_string(),
            TOOL_ID_SUGGEST_MOTION_REPAIRS.to_string(),
            serde_json::json!({
                "ok": true,
                "version": 1,
                "suggestions": [
                    {
                        "id": "hinge_limit_exceeded/arm/move/relax_joint_limits",
                        "kind": "relax_joint_limits",
                        "component_name": "arm",
                        "channel": "move",
                        "impact": { "relax_degrees": 3.2 },
                        "apply_draft_ops_args": patch,
                    }
                ],
                "truncated": false,
            }),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        let line = text
            .lines()
            .find(|line| line.contains("suggest_motion_repairs_v1 (call_1):"))
            .unwrap_or("");
        assert!(
            line.contains(&format!("apply_draft_ops_args={expected_json}")),
            "expected inline apply args JSON in summary line: {line}"
        );
        assert!(
            line.chars().count() <= 3000,
            "suggest_motion_repairs_v1 summary too long: {} chars",
            line.chars().count()
        );
    }

    #[test]
    fn summarize_info_events_list_includes_event_id_and_exact_cursor_and_omits_data_preview() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let cursor = "CURSOR_TOKEN_0123456789abcdefghijklmnopqrstuvwxyz_-";
        let recent_tool_results = vec![Gen3dToolResultJsonV1::ok(
            "call_1".to_string(),
            TOOL_ID_INFO_EVENTS_LIST.to_string(),
            serde_json::json!({
                "ok": true,
                "items": [
                    {
                        "event_id": 16,
                        "ts_ms": 0,
                        "attempt": 1,
                        "pass": 7,
                        "assembly_rev": 9,
                        "kind": "tool_call_result",
                        "tool_id": "suggest_motion_repairs_v1",
                        "call_id": "call_1",
                        "message": "Tool call ok: suggest_motion_repairs_v1",
                        "data_preview": "SHOULD_NOT_APPEAR",
                    }
                ],
                "truncated": false,
                "next_cursor": cursor,
            }),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        let line = text
            .lines()
            .find(|line| line.contains("info_events_list_v1 (call_1):"))
            .unwrap_or("");
        assert!(
            line.contains("event_id=16"),
            "expected event_id in summary line: {line}"
        );
        assert!(
            line.contains(&format!("next_cursor={cursor}")),
            "expected exact next_cursor token in summary line: {line}"
        );
        assert!(
            !line.contains("data_preview") && !line.contains("SHOULD_NOT_APPEAR"),
            "summary line must not include data_preview: {line}"
        );
        assert!(
            line.chars().count() <= 800,
            "info_events_list_v1 summary too long: {} chars",
            line.chars().count()
        );
    }

    #[test]
    fn summarize_info_kv_get_paged_includes_exact_cursor() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let cursor = "CURSOR_TOKEN_0123456789abcdefghijklmnopqrstuvwxyz_-";
        let recent_tool_results = vec![Gen3dToolResultJsonV1::ok(
            "call_1".to_string(),
            TOOL_ID_INFO_KV_GET_PAGED.to_string(),
            serde_json::json!({
                "ok": true,
                "record": {
                    "kv_rev": 123,
                    "written_at_ms": 0,
                    "attempt": 0,
                    "pass": 0,
                    "assembly_rev": 0,
                    "workspace_id": "main",
                    "key": { "namespace": "gen3d", "key": "ws.main.qa" },
                    "summary": "qa",
                    "bytes": 10,
                },
                "json_pointer": "/errors",
                "array_len": 5,
                "items": [
                    { "index": 0, "bytes": 2, "truncated": false, "value_preview": {"kind":"example"} },
                    { "index": 1, "bytes": 2, "truncated": false, "value_preview": {"kind":"example"} }
                ],
                "truncated": true,
                "next_cursor": cursor,
            }),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        let line = text
            .lines()
            .find(|line| line.contains("info_kv_get_paged_v1 (call_1):"))
            .unwrap_or("");
        assert!(
            line.contains(&format!("next_cursor={cursor}")),
            "expected exact next_cursor token in summary line: {line}"
        );
        assert!(
            line.contains("array_len=5") && line.contains("items=2"),
            "expected array_len/items in summary line: {line}"
        );
        assert!(
            line.chars().count() <= 900,
            "info_kv_get_paged_v1 summary too long: {} chars",
            line.chars().count()
        );
    }

    #[test]
    fn summarize_qa_cached_includes_cached_flags() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let recent_tool_results = vec![Gen3dToolResultJsonV1::ok(
            "call_1".to_string(),
            TOOL_ID_QA.to_string(),
            serde_json::json!({
                "ok": false,
                "errors": [{ "severity": "error", "message": "x" }],
                "warnings": [],
                "cached": true,
                "no_new_information": true,
                "capability_gaps": [{ "kind": "missing_motion_channel" }],
            }),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        let line = text
            .lines()
            .find(|line| line.contains("qa_v1 (call_1):"))
            .unwrap_or("");
        assert!(
            line.contains("cached=true") && line.contains("no_new_information=true"),
            "expected cached/no_new_information flags in summary line: {line}"
        );
        assert!(
            line.contains("capability_gaps=1"),
            "expected capability_gaps count in summary line: {line}"
        );
    }

    #[test]
    fn summarize_scene_graph_summary_is_tail_safe_and_includes_info_kv_ref() {
        let config = AppConfig::default();
        let job = Gen3dAiJob::default();
        let workshop = Gen3dWorkshop::default();
        let registry = Gen3dToolRegistryV1::default();

        let mut edges: Vec<serde_json::Value> = Vec::new();
        for i in 0..19u32 {
            edges.push(serde_json::json!({
                "child": format!("child_{i}"),
                "parent": "body",
                "parent_anchor": "origin",
                "child_anchor": "origin",
                "offset_pos": [0.0, 0.0, 0.0],
                "joint_kind": "fixed",
            }));
        }
        edges.push(serde_json::json!({
            "child": "grass_bundle",
            "parent": "body",
            "parent_anchor": "grass_bundle_attach",
            "child_anchor": "origin",
            "offset_pos": [0.0, 1.0, 0.0],
            "joint_kind": "fixed",
        }));

        let recent_tool_results = vec![Gen3dToolResultJsonV1::ok(
            "call_1".to_string(),
            TOOL_ID_GET_SCENE_GRAPH_SUMMARY.to_string(),
            serde_json::json!({
                "ok": true,
                "version": 1,
                "components_total": 3,
                "attachment_edges": edges,
                "info_kv": {
                    "namespace": "gen3d",
                    "key": "ws.main.scene_graph_summary",
                    "selector": { "kind": "kv_rev", "kv_rev": 42 }
                }
            }),
        )];

        let text = build_agent_user_text(
            &config,
            &job,
            &workshop,
            serde_json::json!({}),
            &recent_tool_results,
            &registry,
        );

        let line = text
            .lines()
            .find(|line| line.contains("get_scene_graph_summary_v1 (call_1):"))
            .unwrap_or("");
        assert!(
            line.contains("attachment_edges_total=20"),
            "expected total edge count: {line}"
        );
        assert!(
            line.contains("grass_bundle->body.grass_bundle_attach"),
            "expected tail edge sample to include last edge: {line}"
        );
        assert!(
            line.contains("info_kv={") && line.contains("kv_rev=42"),
            "expected info_kv ref: {line}"
        );
    }
}
