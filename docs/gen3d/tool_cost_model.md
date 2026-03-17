# Gen3D tool cost model (latest-20 cache snapshot) + budget-aware scheduling design

Goal: reduce wasted Gen3D passes/time by making the agent (and tools) explicitly aware of which actions are expensive, and by enforcing deterministic “budget gates” inside tools (contract-first).

This doc is a design note only (no code changes yet).

Related:
- Tool authoring rules (gates belong in tools): `docs/agent_skills/tool_authoring_rules.md`
- Existing inspection/gating notes: `docs/gen3d/run_quality_improvements.md`

## Measurements: latest 20 cache runs

Data source: the 20 most recent run directories under `~/.gravimera/cache/gen3d/`, sorted by directory mtime, analyzed on 2026-03-18 (Asia/Shanghai).

Notes:
- Only runs with `agent_trace.jsonl` can be used for tool duration stats.
- In this snapshot, 15/20 runs had `agent_trace.jsonl` and 5/20 were missing traces (likely aborted very early).
- Durations are computed from `tool_call.ts_ms → tool_result.ts_ms` pairs.
- These numbers are *environment- and provider-dependent* (model/base_url/network). Treat them as “typical in this environment”, not absolute.

### Cost-level thresholds (for this snapshot)

These thresholds are chosen to cleanly separate remote LLM-backed tools from local/deterministic tools:

- **High**: median ≥ 60s **or** p90 ≥ 120s
- **Medium**: median ≥ 1s **or** p90 ≥ 5s
- **Low**: otherwise (sub-second typical)

### Measured tool costs (observed in the traced runs)

| tool_id | n | median_s | p90_s | max_s | cost |
|---|---:|---:|---:|---:|---|
| `llm_generate_plan_v1` | 10 | 245.87 | 438.40 | 461.64 | high |
| `llm_generate_components_v1` | 12 | 145.23 | 244.10 | 282.59 | high |
| `llm_generate_plan_ops_v1` | 3 | 122.38 | 186.38 | 202.38 | high |
| `llm_generate_motion_authoring_v1` | 20 | 118.81 | 213.78 | 245.54 | high |
| `render_preview_v1` | 4 | 28.19 | 46.57 | 54.31 | medium |
| `llm_review_delta_v1` | 1 | 23.65 | 23.65 | 23.65 | medium |
| `get_scene_graph_summary_v1` | 54 | 0.01 | 0.02 | 0.27 | low |
| `info_events_list_v1` | 1 | 0.01 | 0.01 | 0.01 | low |
| `qa_v1` | 61 | 0.01 | 0.02 | 0.06 | low |
| `apply_draft_ops_v1` | 1 | 0.01 | 0.01 | 0.01 | low |
| `get_plan_template_v1` | 17 | 0.01 | 0.02 | 0.06 | low |
| `copy_component_v1` | 2 | 0.00 | 0.00 | 0.01 | low |
| `info_kv_get_many_v1` | 10 | 0.00 | 0.01 | 0.03 | low |
| `get_state_summary_v1` | 1 | 0.00 | 0.00 | 0.00 | low |
| `info_kv_get_paged_v1` | 10 | 0.00 | 0.01 | 0.06 | low |
| `query_component_parts_v1` | 25 | 0.00 | 0.02 | 0.05 | low |
| `get_tool_detail_v1` | 25 | 0.00 | 0.00 | 0.01 | low |
| `get_user_inputs_v2` | 1 | 0.00 | 0.00 | 0.00 | low |
| `info_kv_get_v1` | 62 | 0.00 | 0.01 | 0.06 | low |
| `info_kv_list_keys_v1` | 7 | 0.00 | 0.00 | 0.00 | low |
| `mirror_component_subtree_v1` | 1 | 0.00 | 0.00 | 0.00 | low |
| `mirror_component_v1` | 5 | 0.00 | 0.01 | 0.02 | low |
| `snapshot_v1` | 5 | 0.00 | 0.02 | 0.03 | low |

### What this implies

1) The **only truly “budget-dominant” tools** are the `llm_generate_*` family (plan/components/motion) and (often) `llm_review_delta_v1`.

2) Many tools are “low cost”, but **they can still be expensive indirectly**:
   - Each additional *pass* generally requires at least one `agent_step` LLM call.
   - So repeated “inspection-only” passes can burn minutes even if the tool calls inside the pass are sub-second.

## Full Gen3D tool catalog: default cost labels

This is the current tool-id set (from `src/gen3d/agent/tools.rs`) labeled with a default cost level.
Use the measured table above when available; otherwise fall back to the defaults below.

Rules of thumb:
- `llm_*` tools are **High** by default (remote + variable). `llm_review_delta_v1` can be “Medium” in practice but should still be treated as “budget sensitive”.
- `render_preview_v1` is **Medium** (scales with `views[]`, `image_size`, and `include_motion_sheets`).
- Everything else is **Low** by default (local/deterministic).

| tool_id | default cost | measured? |
|---|---|---|
| `apply_draft_ops_v1` | low | yes |
| `apply_plan_ops_v1` | low | no |
| `basis_from_up_forward_v1` | low | no |
| `copy_component_subtree_v1` | low | no |
| `copy_component_v1` | low | yes |
| `copy_from_workspace_v1` | low | no |
| `create_workspace_v1` | low | no |
| `delete_workspace_v1` | low | no |
| `detach_component_v1` | low | no |
| `diff_snapshots_v1` | low | no |
| `diff_workspaces_v1` | low | no |
| `get_plan_template_v1` | low | yes |
| `get_scene_graph_summary_v1` | low | yes |
| `get_state_summary_v1` | low | yes |
| `get_tool_detail_v1` | low | yes |
| `get_user_inputs_v2` | low | yes |
| `info_blobs_get_v1` | low | no |
| `info_blobs_list_v1` | low | no |
| `info_events_get_v1` | low | no |
| `info_events_list_v1` | low | yes |
| `info_events_search_v1` | low | no |
| `info_kv_get_many_v1` | low | yes |
| `info_kv_get_paged_v1` | low | yes |
| `info_kv_get_v1` | low | yes |
| `info_kv_list_history_v1` | low | no |
| `info_kv_list_keys_v1` | low | yes |
| `inspect_plan_v1` | low | no |
| `list_snapshots_v1` | low | no |
| `llm_generate_component_v1` | high | no |
| `llm_generate_components_v1` | high | yes |
| `llm_generate_motion_authoring_v1` | high | yes |
| `llm_generate_plan_ops_v1` | high | yes |
| `llm_generate_plan_v1` | high | yes |
| `llm_review_delta_v1` | high (budget sensitive) | yes (small n) |
| `merge_workspace_v1` | low | no |
| `mirror_component_subtree_v1` | low | yes |
| `mirror_component_v1` | low | yes |
| `motion_metrics_v1` | low | no |
| `qa_v1` | low | yes |
| `query_component_parts_v1` | low | yes |
| `recenter_attachment_motion_v1` | low | no |
| `render_preview_v1` | medium | yes |
| `restore_snapshot_v1` | low | no |
| `set_active_workspace_v1` | low | no |
| `set_descriptor_meta_v1` | low | no |
| `smoke_check_v1` | low | no |
| `submit_tooling_feedback_v1` | low | no |
| `suggest_motion_repairs_v1` | low | no |
| `validate_v1` | low | no |

## Design: make the agent budget-aware (without heuristics)

The agent already sees `state_summary.budgets.time` (elapsed/remaining/max), but it lacks:
- a *cost model* it can reliably use, and
- *deterministic enforcement* that prevents repeated high-cost actions.

This design adds both, contract-first.

### 1) Add tool cost hints to the agent prompt (small + stable)

Add a short, low-token “Tool costs” section to `agent_step` user text, e.g.:

- High: `llm_generate_motion_authoring_v1 (~120s median)`, `llm_generate_components_v1 (~145s)`, `llm_generate_plan_v1 (~245s)`, …
- Medium: `render_preview_v1 (~28s median)`
- Low: “everything else typically < 0.1s”

Important: keep this list bounded (don’t paste a full table every pass).

### 2) Add deterministic gates inside high-cost tools (most important)

Per `docs/agent_skills/tool_authoring_rules.md`, gates belong inside tools so the agent can’t “forget”.

Examples (no heuristics; fully state-driven):
- `llm_generate_motion_authoring_v1`: allow at most N calls per run (or per-channel) unless the latest `qa_v1` has motion_validation `severity="error"`.
- `llm_generate_plan_v1` / `llm_generate_components_v1`: require `remaining_seconds >= (estimated_tool_seconds + safety_margin)`.
- All `llm_*`: optionally require a “budget justification” arg in the tool contract (e.g. `reason: "required_by_prompt"` vs `reason: "optional_polish"`).

Tool results should remain actionable:
- return `ok:false` with a *structured error* including current remaining budget + fixits (e.g. “use deterministic ops instead”, “finish with done”, etc).

### 3) Track measured latencies and feed them back deterministically

Introduce a minimal, deterministic rolling estimate:
- per-tool EMA/p50 for the current run (and optionally persisted across runs).
- expose it in `state_summary` as compact numbers for high-cost tools only.

This avoids hardcoding “~120s” when the provider/network is slower/faster.

### 4) Encourage bundling to reduce passes

Because pass count is expensive (each pass needs an `agent_step`), prefer a policy of:
- “If you’re going to do inspection, do it in the same pass as the mutation you’ll do next.”
- “Batch low-cost reads in one pass; don’t spread them across multiple passes.”

This doesn’t require heuristics; it’s a prompt + gate + (optional) engine-side no-progress guard refinement.

## Why this would prevent “re-authored motion 6 times”

With the above:
- The tool gate caps `llm_generate_motion_authoring_v1` to 1–2 attempts unless QA reports an **error**.
- The prompt cost hints steer the agent to “one-shot + QA + done” rather than “try again”.
- If time is tight, the tool gate rejects optional retries *before* spending minutes on another generation.

## Appendix: regenerating the table locally (manual)

This doc’s “Measured tool costs” table was generated from `agent_trace.jsonl` in the cache.
If needed, re-run a local analysis in your shell and update this doc with the new medians/p90s.

