# Gen3D: Paged Info Store + no-progress gates for tool loops (generic, motion-aware)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D is a tool-driven agent loop. When the agent hits a blocking QA condition and lacks an actionable next mutation step, it can get stuck repeatedly calling inspection tools (`qa_v1`, `info_kv_get_v1`, `get_scene_graph_summary_v1`) without changing the assembled draft. This wastes time, blows prompt budget, and produces “repeating QA” runs that never converge.

Separately, some tool results can be large enough that returning them inline (or even summarizing them naïvely) either exceeds LLM context limits or forces truncation that removes the one field needed to proceed (cursor tokens, KV pointers, IDs). We already have an Info Store with paging for event/blob lists; we should extend the same “stateless paging” pattern to large JSON payloads so the agent can retrieve exactly what it needs without dumping entire blobs into the prompt.

After implementing this plan, Gen3D will:

1) Prevent “no-progress” inspection loops at the tool layer, by having tools detect repeated calls on unchanged state and return an actionable “no new information; mutate first” outcome (with deterministic pointers and fixits when possible).

2) Provide deterministic, bounded retrieval of large JSON outputs via Info Store paging, so the agent can page through arrays/records and fetch specific items without requesting huge payloads.

3) Do this generically for motion/behavior problems (not “attack-specific”): the mechanisms must work for any missing or inconsistent capability (root behavior fields, motion channels, motion validation errors, rig metadata), and must follow the tool-authoring “contract-first” rule: enforce gates in tools, and return actionable results/errors with explicit diffs and explicit apply payloads.

## Progress

- [x] (2026-03-16 14:04Z) Write ExecPlan (this document) with concrete contracts, file targets, and acceptance criteria.
- [ ] Add Info Store paging for KV JSON arrays (new tool or extension) + docs.
- [ ] Add tool-level no-progress gates for repeated inspection calls (start with `qa_v1`) + tests.
- [ ] Add generic “capability gaps” + “fixits” fields to QA/smoke results (bounded, deterministic) + prompt summaries.
- [ ] Run rendered smoke test (2 seconds) and commit implementation.

## Surprises & Discoveries

- Observation: The existing agent no-progress guard intentionally does not stop runs that are not “complete enough”, to avoid prematurely ending before required QA is green.
  Evidence: `src/gen3d/ai/agent_step.rs` resets the no-progress counters when `run_complete_enough_for_auto_finish(...)` is false and continues requesting steps. This is correct when the agent can still make progress, but it enables infinite loops when QA is blocked by missing tools or missing plan interfaces.

- Observation: `info_kv_get_v1` is bounded by `max_bytes`, but has no paging mechanism for large arrays; the only guidance is “use json_pointer”.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` returns an error when the selected value exceeds `max_bytes` and tells the agent to choose a smaller subset via `json_pointer`. JSON Pointer cannot express array slices, so this is not sufficient when the agent needs to browse a large list.

- Observation: The repository already has a deterministic paging cursor mechanism for list/search tools (events, blobs, KV history).
  Evidence: `src/gen3d/ai/info_store.rs` implements offset cursors via `encode_offset_cursor(...)` / `decode_offset_cursor(...)`, and `agent_tool_dispatch.rs` uses `store.page_from_args(...)` and `store.page_out(...)` for list tools.

- Observation: Tool-level `ok=false` (tool call error) is summarized as `ERROR:` in the agent prompt; using tool errors for “no-progress” gates is likely to be counterproductive for inspection tools.
  Evidence: `src/gen3d/ai/agent_prompt.rs` treats any `Gen3dToolResultJsonV1.ok=false` as an error and expands “expected args” boilerplate, which is useful for schema mistakes but not for “same result; no new info”.

- Observation: `smoke_check_v1` currently includes prompt-keyword heuristics (e.g. “attack required”) and should be treated as legacy logic; this plan must avoid expanding heuristic requirement inference.
  Evidence: `src/gen3d/ai/orchestration.rs` sets `attack_required_by_prompt` via `prompt.contains(...)` checks.

## Decision Log

- Decision: Add a paging tool for KV JSON arrays (rather than dumping large values in prompt summaries).
  Rationale: Large tool outputs should be retrieved “on demand” in small pieces. Paging is deterministic and avoids heuristics while keeping LLM context bounded.
  Date/Author: 2026-03-16 / Codex CLI agent

- Decision: Implement no-progress gates in tools (starting with `qa_v1`) instead of adding more agent-prompt “call X before Y” rules.
  Rationale: Tool authoring rules (`docs/agent_skills/tool_authoring_rules.md`) prefer gates enforced in tools, with actionable errors and explicit fixits. This prevents loops even when the LLM ignores prompt guidance.
  Date/Author: 2026-03-16 / Codex CLI agent

- Decision: Represent missing/inconsistent motion/behavior requirements as generic “capability gaps” with optional fixits.
  Rationale: This keeps the solution generic across motion channels and root behavior (mobility/attack/aim/rig/collider) and avoids one-off “attack” logic that would reappear for other capabilities later.
  Date/Author: 2026-03-16 / Codex CLI agent

## Outcomes & Retrospective

Not implemented yet. This section will be updated as milestones complete.

## Context and Orientation

This plan touches Gen3D’s agent loop, tool dispatch, Info Store, and QA surfaces.

Key concepts and where they live:

- “Tool dispatch”: the code that implements agent-facing tools, in `src/gen3d/ai/agent_tool_dispatch.rs`. Each tool ID is handled in a `match` branch.

- “Info Store”: a run-local store for KV records, events, and blobs, implemented in `src/gen3d/ai/info_store.rs`. It already supports list paging via cursor tokens.

- “Agent prompt”: the text sent to the LLM each step, built/summarized in `src/gen3d/ai/agent_prompt.rs`. Prompt size must remain bounded; do not inline huge JSON.

- “QA”: the combined validate + smoke check tool `qa_v1`. The smoke portion is built in `src/gen3d/ai/orchestration.rs` (see `build_gen3d_smoke_results(...)`). `qa_v1` results are also written to the Info Store as `ws.<workspace>.qa`.

Contract-first requirement:

- Tool contracts and “actionability” requirements must be updated in docs under `docs/gen3d/` and in the tool registry (`src/gen3d/agent/tools.rs`). Follow `docs/agent_skills/tool_authoring_rules.md`: return actionable diffs, actionable errors, and enforce gates in tools.

## Plan of Work

The work is split into four milestones that can land independently while providing incremental value. Each milestone must preserve determinism and keep the agent prompt bounded by relying on Info Store pointers rather than inline large payloads.

### Milestone 1: KV paging for large JSON arrays (Info Store)

Goal: enable the agent to browse large arrays stored in Info Store KV without requesting an oversized payload.

Design: add a new tool `info_kv_get_paged_v1` (preferred over extending `info_kv_get_v1` because the result shape is different and we want a crisp contract).

Proposed contract:

- Args:
  - `namespace: string`
  - `key: string`
  - `selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number }`
  - `json_pointer?: string` (must resolve to a JSON array; default is the full KV value)
  - `page?: { limit?: number, cursor?: string }` (stateless paging; uses the existing offset-cursor encoding)
    - `page.limit`: default 50, max 200 (match existing Info Store paging defaults)
  - `max_item_bytes?: number` (default 4096; clamp [256, 64k]) to bound per-item previews (and therefore total tool output)

- Result:
  - `ok: true`
  - `record`: exactly the same `record` object shape as `info_kv_get_v1` returns (includes `written_at_ms`, `workspace_id`, and `key:{namespace,key}`)
  - `array_len: number` (the total length of the selected array; used to reason about paging without fetching the whole array)
  - `items: [{ index: number, value_preview: any, truncated: bool, bytes: number }]`
    - `index`: absolute index into the selected array.
    - `bytes`: the serialized JSON byte size of the item, **capped** at `max_item_bytes + 1` (so implementations can avoid allocating huge buffers). If the true size is greater, `truncated=true`.
    - `value_preview`: either the full item (when `truncated=false`) or a deterministic “shape preview” (when `truncated=true`, see below).
  - `truncated: bool` (true if there are more items beyond this page)
  - `next_cursor?: string` (exact token; never truncate)
  - `json_pointer?: string` (echo, if provided)

Paging semantics (regression-critical):

- Paging must be against a single, frozen KV record. Even when `selector.kind="latest"`, the tool must select a concrete record first and bind paging to that record’s `kv_rev` (so pages don’t “shift” if a newer KV revision is written between calls).
- The paging cursor must reject mismatches deterministically by encoding all shape-affecting params into the cursor `params_sig`:
  - `namespace`, `key`, **selected `kv_rev`**, `json_pointer` (or `""`), `max_item_bytes`, and the tool id/kind.

Deterministic “shape preview” algorithm (generic; no heuristics):

- For `null`/`bool`/`number`: return the scalar directly.
- For `string`: never return an oversized string. If it does not fit, return `{ "kind":"string", "len_bytes": <n> }` as the preview.
- For arrays: return `{ "kind":"array", "len": <n> }`.
- For objects: return `{ "kind":"object", "keys_sample": [sorted first 16 keys], "keys_total": <n> }`.

This keeps each item bounded without trying to “summarize meaning”.

Implementation locations:

- Add tool descriptor in `src/gen3d/agent/tools.rs` with args schema + example.
- Implement in `src/gen3d/ai/agent_tool_dispatch.rs` by reusing:
  - `select_kv_record(...)` for selecting the record
  - `store.page_from_args(...)` and `store.page_out(...)` for cursor paging
  - `encode_offset_cursor(...)` semantics already used for other paging tools

Docs:

- Add `docs/gen3d/info_kv_get_paged_v1.md` describing contract, bounds, paging semantics, and examples.

Tests:

- Add unit tests in `src/gen3d/ai/info_store.rs` (or a new test module near tool dispatch) verifying:
  - Cursor roundtrip rejects mismatch (already exists; extend for new kind string).
  - Paging a known JSON array returns stable indices and correct `next_cursor`.
  - `max_item_bytes` behavior produces previews and sets `truncated=true` per item when needed.

### Milestone 2: Tool-level no-progress gates (start with `qa_v1`)

Goal: prevent repeated inspection tool calls when the underlying assembled state is unchanged, and return an actionable response that pushes the agent toward mutation or toward a “blocked” conclusion.

We already compute a “state hash” for the agent no-progress guard (`compute_agent_state_hash`), but the guard intentionally does not stop incomplete runs. We need an additional layer: tools themselves should detect “same input basis; same result; no new information”.

Design: add `basis` + `cached` fields to selected inspection tools, and for repeated calls with unchanged basis:

- Return the cached prior result with:
  - `cached=true`
  - `no_new_information=true`
  - `basis:{...}` echoed back for debugging
  - a short actionable message that pushes the agent toward mutation or toward a “blocked” conclusion.

Do **not** use tool-call `ok=false` for the no-progress path: tool errors are summarized as `ERROR:` in the prompt and are reserved for invalid args / runtime failures (see `Surprises & Discoveries` above).

Start with `qa_v1` because it is the most expensive/loop-prone tool. Consider also `get_scene_graph_summary_v1`, `validate_v1`, `smoke_check_v1`, and Info Store “latest” getters when used as inspection loops.

Proposed QA basis:

- Equality key (what determines “no new info”):
  - `workspace_id` (active workspace)
  - `state_hash` from `compute_agent_state_hash(job, draft)` (already ignores revision counters like `assembly_rev` and includes motion value digests)
- Debug fields (include in `basis` for visibility, but do not use for equality):
  - `plan_hash`
  - `assembly_rev`

Implementation locations:

- `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_QA` branch:
  - Add a `force?: bool` arg (default false) to bypass caching explicitly when needed (debugging or when external state changes).
  - Compute the QA `basis` up front.
  - If `force!=true` and basis matches the last QA basis stored on `job.agent`, return a cached result (and do **not** write new KV records). Ensure the cached JSON still includes the original `info_kv` pointer so the agent can inspect the last full QA output.
  - When returning cached due to repetition, include an actionable “mutate before retry” hint that references concrete tool IDs (`apply_draft_ops_v1`, `apply_plan_ops_v1`, `llm_generate_plan_v1`, `llm_generate_motion_authoring_v1`) but does not prescribe a specific fix.
  - Update `src/gen3d/ai/agent_prompt.rs` QA summarizer to surface `cached` / `no_new_information` (otherwise the gate is invisible to the agent).

Docs:

- Update `docs/gen3d/qa_v1.md` (or create it if missing) to document caching/no-progress semantics.

Tests:

- Add a deterministic unit test that constructs a minimal job/draft, runs `qa_v1` twice without mutation, and asserts the second result has `cached=true` and `no_new_information=true` (and preserves the prior `info_kv` pointer).

### Milestone 3: Generic “capability gaps” and “fixits” in QA/smoke results (motion-aware)

Goal: when QA is red, the agent should see (a) what capability is missing/inconsistent, and (b) what deterministic tool calls could fix it, without guessing or repeatedly inspecting.

This must be generic across motion/behavior, not “attack-only”. Examples of capabilities:

- Root behavior interfaces: `mobility`, `attack`, `aim`, `rig.move_cycle_m`, `collider`.
- Motion channels: `idle`, `move`, `attack_primary` (and future channels).
- Motion validation errors: hinge limits exceeded, fixed joint rotates, off-axis rotations, contact stance missing, etc.

Design:

- Extend `qa_v1` result with a bounded `capability_gaps[]` list. Each gap is a structured object:
  - `kind`: machine-readable string (e.g. `missing_root_field`, `missing_motion_channel`, `inconsistent_root_fields`, `motion_validation_error`)
  - `severity`: `error` or `warn`
  - `message`: short, human-readable (single sentence)
  - `evidence`: small, structured fields (component name/id, channel, json path)
  - `fixits`: optional list of explicit tool payloads (never applied silently)
  - `blocked`: optional boolean + `blocked_reason` when no fixit is possible with current tools/constraints

Boundedness rules:

- Cap `capability_gaps` at 16 entries in the tool result.
- Cap `fixits` per gap at 3.
- Any fixit payload must be <= 2,000 characters when minified JSON; otherwise store it in KV and return a pointer (`info_kv`) instead of inlining.

Fixit generation rules (generic, deterministic, tool-first):

- Only generate fixits for deterministic tools (`apply_draft_ops_v1`, `apply_plan_ops_v1`, `recenter_attachment_motion_v1`, `suggest_motion_repairs_v1` output references).
- Do not invent “LLM” fixits as if they were deterministic; instead suggest the tool ID to call (without payload) if the next step must be LLM-driven (`llm_generate_motion_authoring_v1`, `llm_generate_plan_v1`).
- When a fix requires a tool capability that does not exist (example: setting root behavior fields via PlanOps today), mark the gap as `blocked=true` with a concrete reason: “No deterministic tool can set plan.root.attack; add PlanOp `set_attack` or a dedicated root-patch tool.”

Note: This milestone intentionally does not decide how requirements are inferred from raw prompt text. The goal is to expose the gap *given whatever requirement logic exists*, and to make the follow-up deterministic. Requirement inference can be revisited later (and should avoid keyword heuristics where possible).

Implementation locations:

- `src/gen3d/ai/orchestration.rs` for smoke/summary shaping.
- `src/gen3d/ai/agent_tool_dispatch.rs` in `qa_v1` composition.
- `src/gen3d/ai/agent_prompt.rs` summarizer: ensure the prompt includes the top 3 gaps with their kinds + one fixit pointer (and always includes any `info_kv` pointers / cursor tokens).

Docs:

- Update `docs/gen3d/qa_v1.md` and `docs/gen3d/smoke_check_v1.md` to document the new fields.
- Ensure tool schemas/examples remain accurate in `src/gen3d/agent/tools.rs`.

Tests:

- Add unit tests covering:
  - A missing motion channel produces `missing_motion_channel` gap.
  - A motion validation error with available deterministic repair produces a fixit that can be applied via `apply_draft_ops_v1`.
  - A missing root interface produces a `blocked` gap when no deterministic fix exists.

### Milestone 4: Finish the loop (prompt summaries + regression case)

Goal: prove the whole system prevents repeating-QA loops and remains usable under bounded prompt budgets.

Work:

- Add or update a regression test that simulates the “repeating QA” failure mode:
  - Set up a draft where QA is red for a capability gap that is blocked (no deterministic fix).
  - Ensure the second `qa_v1` call returns cached/no-progress result and includes the `blocked` gap so the agent can terminate best-effort rather than looping.

- Ensure the agent prompt’s “Recent tool results” includes the exact navigation tokens (`kv_rev`, `event_id`, `next_cursor`) even when summaries are tight. The existing ExecPlan `docs/execplans/execplan_gen3d_agent_prompt_actionable_tool_summaries.md` already established this direction; reuse its conventions.

## Concrete Steps

All commands are run from the repository root (`/Users/flow/workspace/github/gravimera`).

1) Identify candidate large KV arrays to page (for examples and tests) by inspecting existing tool outputs:

    rg -n \"\\\"key\\\":\\{\\\"key\\\":\\\"ws\\.main\\..*\\\"\" /Users/flow/.gravimera/cache/gen3d -S | head

2) After implementation, run focused tests for Info Store and tool dispatch:

    cargo test -p gravimera gen3d::ai::info_store -- --nocapture
    cargo test -p gravimera gen3d::ai::agent_tool_dispatch -- --nocapture

3) Run the required rendered smoke test:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance is behavioral and must be observable without reading code.

1) KV paging tool works:

- Storing a KV record whose selected `json_pointer` is a long array allows browsing it in pages.
- Repeating the same `info_kv_get_paged_v1` call with `page.cursor` returns deterministic next pages and a stable `next_cursor`.
- No page response exceeds the configured per-item and per-page bounds.

2) No-progress gate works (QA):

- Calling `qa_v1` twice with no draft/plan changes returns a second result that is explicitly marked as cached/no-new-info, and includes an actionable hint (“mutate before re-running QA”).

3) Capability gaps are actionable and bounded:

- When QA is red, the response includes `capability_gaps[]` with at least one gap that explains what is missing in structured form.
- When a deterministic fix exists, at least one fixit payload is included (or a KV pointer to it), never applied silently.
- When a deterministic fix does not exist, QA marks the gap as blocked with a concrete reason, so the agent can stop best-effort rather than looping.

## Idempotence and Recovery

The work should be safe to iterate and roll back.

- New tools should be additive. If a tool is renamed or replaced, keep the previous tool for a short deprecation window unless we explicitly choose to break compatibility (allowed by project policy).
- Paging cursors must be stateless and reject mismatches (tool kind or params signature) deterministically. Reuse the existing cursor mechanism to avoid introducing a second cursor format.
- If the no-progress gate causes accidental premature stops, add a `force: true` escape hatch on the tool call (explicitly documented), and ensure it is never used by default in agent prompts.

## Interfaces and Dependencies

New or extended interfaces must be explicit and tested.

- `info_kv_get_paged_v1` tool contract in:
  - `src/gen3d/agent/tools.rs` (schema + example)
  - `src/gen3d/ai/agent_tool_dispatch.rs` (implementation)
  - `docs/gen3d/info_kv_get_paged_v1.md` (documentation)

- `qa_v1` additions:
  - `cached`, `basis`, `capability_gaps[]` fields in the tool result JSON.
  - Prompt summarization updates in `src/gen3d/ai/agent_prompt.rs` to keep the agent’s context bounded while preserving navigation tokens.

## Plan Change Log

- (2026-03-16) Initial draft created to address repeating-QA loops generically and to introduce Info Store paging for large tool outputs. Rationale: avoid “attack-specific” fixes and keep tool results actionable and bounded.
