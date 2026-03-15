# Gen3D: Agent prompt actionable tool summaries (bounded)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D is a tool-driven agent loop. The agent decides what to do next based on a text prompt that includes a compact summary of recent tool results. When those summaries omit the specific fields needed to take the next deterministic step, the agent can get stuck in “inspection loops” such as repeating `qa_v1` over and over, even though a deterministic fix exists.

After this change, the Gen3D agent prompt will expose *actionable* (copy/pasteable) payloads for deterministic repair tools (notably `suggest_motion_repairs_v1`), and it will expose the minimum required navigation fields for Info Store tools (notably `info_events_list_v1`). This enables the agent (or a human operator) to apply one explicit mutation step (typically `apply_draft_ops_v1`) rather than looping QA, while keeping the prompt size bounded to avoid blowing up LLM context.

Info Store browsing should be treated as stateless paging:

- list/search tools return IDs + a paging cursor (`next_cursor`) under strict bounds
- get tools fetch the exact payload for one ID (`event_id`, `kv_rev`, `blob_id`) optionally using `json_pointer` + `max_bytes`

## Progress

- [x] (2026-03-16 01:03Z) Write ExecPlan, capture the root-cause evidence, and define bounded summarization rules.
- [ ] Implement actionable `suggest_motion_repairs_v1` summaries (bounded; no silent apply).
- [ ] Implement actionable Info Store summaries (start with `info_events_list_v1`).
- [ ] Audit remaining tools for “actionability vs size” and patch summaries as needed.
- [ ] Add unit tests for prompt summarization budgets and key fields.
- [ ] Run rendered smoke test and commit.

## Surprises & Discoveries

- Observation: The agent can call a tool that returns ready-to-apply `apply_draft_ops_v1` patches, but the prompt compaction currently hides those patches and/or hides the `event_id` needed to fetch them from the Info Store.
  Evidence: A `suggest_motion_repairs_v1` tool result can be summarized as only `suggestions=<N> truncated=true`, which does not include any `apply_draft_ops_args`, leaving the agent unable to proceed without additional inspection calls. In the same flow, `info_events_list_v1` can be summarized as only `keys=[...]`, which hides `event_id` so the agent cannot call `info_events_get_v1` to retrieve the payload.

## Decision Log

- Decision: Prefer “two-tier” information: (1) include small, directly actionable payloads inline in the prompt when they are below a strict size budget; (2) always include deterministic pointers (event_id / kv_rev) so the agent can fetch full details with Info Store tools when inline inclusion would exceed the budget.
  Rationale: Keeps the prompt small while preserving the ability to drill down deterministically. This avoids heuristics and avoids silent mutation.
  Date/Author: 2026-03-16 / Codex CLI agent

- Decision: Add explicit per-tool summarizers for Info Store tools instead of relying on the generic “keys-only” fallback.
  Rationale: For navigation tools, omitting `event_id`, `kv_rev`, and `record.summary` removes the only information that makes the tool useful, and leads to loops.
  Date/Author: 2026-03-16 / Codex CLI agent

- Decision: Do not dump full JSON blobs into the prompt by default; implement strict per-tool character budgets and omit payloads that exceed them.
  Rationale: Tool outputs can be large. The agent only needs a small subset to act; the rest should be available via deterministic fetch tools.
  Date/Author: 2026-03-16 / Codex CLI agent

- Decision: For paged list/search tools, include the exact `next_cursor` token (never truncate it); if the summary budget is tight, drop less-critical fields (messages/items) before omitting the cursor.
  Rationale: Paging is only deterministic if the agent can pass back the cursor. A `next_cursor=true/false` flag is not actionable.
  Date/Author: 2026-03-16 / Codex CLI agent

## Outcomes & Retrospective

- Outcome: (pending)

## Context and Orientation

The Gen3D agent prompt is built in `src/gen3d/ai/agent_prompt.rs`.

Key concepts used in this plan:

- “Recent tool results”: the prompt includes a section that summarizes the previous step’s tool results so the agent can decide the next step without re-running tools.
- “Tool result summarization”: in `src/gen3d/ai/agent_prompt.rs`, the helper `summarize_tool_result` converts a JSON tool result into a single-line text summary.
- “Info Store”: an internal store of structured KV records and append-only events. Gen3D exposes tools like `info_events_list_v1` and `info_kv_get_v1` to fetch large outputs in a bounded way. These tools are only helpful if their summaries expose navigation keys like `event_id` and `kv_rev`.

The motivating failure mode:

1) `qa_v1` reports a deterministic motion validation error (example: `hinge_limit_exceeded`).
2) The agent calls `suggest_motion_repairs_v1`, which returns concrete `apply_draft_ops_args` patches to fix the error.
3) The prompt summarizes the result without including those patches, so the agent cannot call `apply_draft_ops_v1` next.
4) The agent then calls Info Store tools to try to fetch the full payload, but those tool results are also summarized without `event_id`/`kv_rev`, preventing targeted retrieval.
5) The agent falls back to repeating `qa_v1` (or other inspection tools), producing a “repeating QA pass” loop.

## Plan of Work

### 1) Make `suggest_motion_repairs_v1` summaries directly actionable, but bounded

In `src/gen3d/ai/agent_prompt.rs` within `summarize_tool_result`, expand the `TOOL_ID_SUGGEST_MOTION_REPAIRS` branch from count-only (`suggestions=<n> truncated=<bool>`) to a bounded, actionable list.

The summary must:

- Always include `suggestions=<n>` and `truncated=<bool>`.
- Include up to a strict maximum number of suggestions (use the tool’s `max_suggestions` bound; cap at 8 for safety).
- For each included suggestion, include:
  - `id` (stable identifier)
  - `kind` (e.g. `relax_joint_limits`, `scale_animation_slot_rotation`)
  - `component_name`
  - A compact `impact` summary when present (for example, scale factor or degrees of relaxation; keep it short).
  - `apply_draft_ops_args` as **exact** minified JSON if and only if it fits a per-suggestion character budget (so it can be copied into an `apply_draft_ops_v1` call without truncation).
- If `apply_draft_ops_args` exceeds the per-suggestion budget, omit it (explicitly mark it omitted) and rely on the Info Store fetch path (see below).

Boundedness rules (deterministic; non-heuristic):

- Per-suggestion `apply_draft_ops_args` inline budget: 800 characters (minified JSON). If longer, do not inline.
- Total `suggest_motion_repairs_v1` summary budget: 3,000 characters. If exceeded, include fewer suggestions (starting from the front) until within budget and note how many were omitted.

Success condition for the motivating case:

- For the common `hinge_limit_exceeded` case (small, single-op patches), all suggested `apply_draft_ops_args` should fit and be inlined, enabling a one-step follow-up `apply_draft_ops_v1` without extra Info Store navigation.

### 2) Make `info_events_list_v1` summaries expose navigation fields (event_id), but bounded

Add a dedicated summarizer branch for `TOOL_ID_INFO_EVENTS_LIST` in `summarize_tool_result`.

The summary must:

- Include `items=<n>`.
- Include, for up to the first 3 items:
  - `event_id`
  - `kind`
  - `tool_id` if present
  - `call_id` if present
  - `pass`
  - a short `message` snippet (single-line; aggressively truncated)
- Include `next_cursor` as an **exact** string when present (so the agent can page deterministically by re-calling with `page.cursor=next_cursor`).
- If the summary budget is tight, drop less-critical fields (messages/items) before omitting the cursor; never truncate the cursor token.
- Never include `data_preview` in the prompt summary; use `info_events_get_v1` with `event_id` (+ `json_pointer` + `max_bytes`) to fetch details.

This makes the Info Store usable without inflating the prompt, and it unlocks targeted follow-ups like `info_events_get_v1` with a specific `event_id` and `json_pointer`.

### 3) Comprehensive check: audit other tools and add minimal “actionable fields” summaries where needed

Do a one-by-one audit of all tools in `src/gen3d/agent/tools.rs` against `summarize_tool_result` coverage. For each tool, decide whether the generic fallback (“keys-only”) is sufficient for making the next decision. If not sufficient, add a dedicated summarizer that prints only the minimal fields required for the next deterministic action.

Prioritize (because they are navigation primitives and frequent in loops):

- `info_kv_get_v1`: include `namespace/key`, `kv_rev`, `record.summary`, and a tiny value summary if it has `ok/errors/warnings`.
- `info_events_get_v1`: include `event_id`, `kind`, `tool_id`, `pass`, and whether content is truncated; avoid full payload.
- `info_events_search_v1`: mirror `info_events_list_v1` summarization rules.
- `info_kv_list_keys_v1`, `info_kv_list_history_v1`, `info_kv_get_many_v1`: include counts and `next_cursor` token when present, plus a small sample of keys/revs.
- `info_blobs_list_v1`, `info_blobs_get_v1`: include `blob_id`, `bytes`, and a short label/kind, plus `next_cursor` token when present.

Also verify mutation tools remain easy to confirm:

- `apply_draft_ops_v1`: summary must include whether it applied, which components/channels were touched, and whether it wrote an Info Store record (if applicable). Keep it bounded.

### 4) Add tests and guardrails to prevent prompt-size regressions

Add Rust unit tests that build representative `Gen3dToolResultJsonV1` payloads and assert:

- `suggest_motion_repairs_v1` summary contains at least one exact `apply_draft_ops_args` JSON when the patch is small.
- `info_events_list_v1` summary contains `event_id` for listed items.
- If `info_events_list_v1` includes `next_cursor`, the summary includes the exact token (not just a boolean).
- `info_events_list_v1` summary does not include `data_preview`.
- Each new summary respects the per-tool character budgets.

If practical, also add one test for `build_agent_user_text` that ensures the “Recent tool results” section remains bounded when given 16 results, and that it includes the required navigation fields for Info Store flows.

## Concrete Steps

All commands are run from the repository root (`/Users/flow/workspace/github/gravimera`).

1) Run targeted tests for Gen3D prompt summarization:

    cargo test -p gravimera gen3d::ai::agent_prompt -- --nocapture

2) Run full Gen3D-focused tests if the repo already has them:

    cargo test -p gravimera gen3d -- --nocapture

3) Run the required smoke test with rendered UI (no headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expected behavior: the game starts, renders for ~2 seconds, then exits without crash.

## Validation and Acceptance

Acceptance criteria (human-verifiable):

- When `suggest_motion_repairs_v1` returns small patch suggestions (single-op `apply_draft_ops_args`), the agent prompt’s “Recent tool results” line(s) include the exact JSON needed to call `apply_draft_ops_v1` without extra fetch steps.
- When `info_events_list_v1` is called, the agent prompt summary includes `event_id` values so a follow-up `info_events_get_v1` call is possible without guessing.
- When `info_events_list_v1` returns `next_cursor`, the agent prompt summary includes the exact cursor token so a follow-up page can be fetched without guessing.
- The prompt remains bounded:
  - `suggest_motion_repairs_v1` summary never exceeds 3,000 characters.
  - `info_events_list_v1` summary never exceeds 800 characters.
  - The “Recent tool results” section remains readable with 16 entries.
- No silent mutation is added: these are prompt/summarization changes only.

Acceptance criteria (test-verifiable):

- New unit tests for summarization pass locally and do not require network access.
- The required rendered smoke test runs without crash.

## Idempotence and Recovery

- Tool-result summarization changes are safe to run repeatedly; they do not mutate any state.
- If a summary becomes too large, reduce the deterministic caps (suggestion count, per-suggestion budget, total budget) and rely on Info Store pointers for full retrieval.

## Artifacts and Notes

Example of the intended “actionable but bounded” style for a suggestion tool:

    - suggest_motion_repairs_v1 (call_1): ok suggestions=8 truncated=true items=[{id:...,kind:...,component:...,apply_draft_ops_args:{...}}, ...]

Example of the intended “navigation-first” style for `info_events_list_v1`:

    - info_events_list_v1 (call_1): ok items=1 first=[{event_id:16,kind:tool_call_result,tool_id:suggest_motion_repairs_v1,call_id:call_1,pass:7,message:\"Tool call ok: ...\"}] next_cursor=\"<opaque_cursor>\"

## Interfaces and Dependencies

The implementation should be limited to agent-prompt summarization and its tests:

- `src/gen3d/ai/agent_prompt.rs`
  - Extend `summarize_tool_result` to handle:
    - `TOOL_ID_SUGGEST_MOTION_REPAIRS` (existing branch; make actionable + bounded)
    - `TOOL_ID_INFO_EVENTS_LIST` (new branch; include event_id and minimal item metadata)
    - Additional `info_*` tools as needed for the audit (bounded, navigation-first)
  - Add helper(s) if needed for deterministic size budgeting (counting characters and stopping before cap).

- `src/gen3d/agent/tools.rs`
  - No functional changes required; use existing tool ids for matching.

- Unit tests
  - Add tests adjacent to `src/gen3d/ai/agent_prompt.rs` (follow existing test layout in the crate).
