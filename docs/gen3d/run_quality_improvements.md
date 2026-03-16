# Gen3D run quality improvements (inspection/KV, gating, schema, done-evidence)

This is a design note capturing improvement ideas from a real seeded-edit run
(`run_id=374aa3b9-6d79-43fb-a40f-dce8329a1463`, 2026-03-17) where the agent
performed many repeated inspection steps (`info_kv_get*`) and produced at least
one misleading final "done" narrative.

Scope: proposals only (no code changes yet). Some items below may already have
partial mitigations in code; the goal is to make remaining gaps explicit and
reduce repeated inspection / misleading reporting.

Related docs:
- Tool authoring constraints: `docs/agent_skills/tool_authoring_rules.md`
- KV paging contract: `docs/gen3d/info_kv_get_paged_v1.md`
- Preserve-mode edits: `docs/gen3d/edit_preserve_existing_components.md`
- Plan-ops tool: `docs/gen3d/llm_generate_plan_ops_v1.md`
- QA tool: `docs/gen3d/qa_v1.md`
- Review-delta semantics: `docs/gen3d/review_delta_v1.md`

## Status (as of code on 2026-03-17)

This run revealed issues that are a mix of:

- *still missing* contract/tool behavior, and
- *already mitigated* by guardrails added elsewhere (no-progress budgets, QA caching, etc).

Already present (not exhaustive; listed so we don’t duplicate work):

- Deterministic no-progress guard with separate budgets for “tries” vs “inspection-only steps”.
- `qa_v1` caches by a deterministic state hash and returns `cached=true` / `no_new_information=true` on repeats.
- `info_kv_get_paged_v1` already returns bounded per-item previews (and a deterministic shape preview when an item is truncated).
- Preserve-mode regen is already QA-gated, but the gate is enforced **after** `llm_review_delta_v1` spends LLM tokens (so regen-only outputs can still be wasted).

Remaining gaps / opportunities (what this doc focuses on):

- `info_kv_get_v1` oversize reads fail with a string-only error (no bounded shape preview; no mechanical “fixits”).
- KV reads (`info_kv_get_v1` / `info_kv_get_many_v1`) do not currently surface `cached` / `no_new_information` the way `qa_v1` does.
- `done` is free-text only; the engine can’t validate evidence or force narratives to be event-true.
- Some “no heuristics” constraints are violated elsewhere (example: smoke results derive `attack_required_by_prompt` via prompt substring checks); track these in `docs/gen3d/assumptions_heuristics_todo.md`.

## Observations (from the run)

1) **Oversize KV read produced a hard error.**
   - `info_kv_get_v1` failed when called without `json_pointer` and with a too-small `max_bytes`.
   - This is recoverable but causes extra passes and wastes tokens.

2) **`llm_review_delta_v1` spent LLM tokens generating a regen action that the engine later blocked.**
   - Preserve-mode + QA-gate blocked regen of an already-generated component because QA had not been run yet.
   - The tool error was actionable, but the LLM call itself was avoidable.

3) **`llm_generate_plan_ops_v1` needed a schema repair roundtrip.**
   - The first attempt produced an unknown field (`component`) for an `add_component` op.
   - A repair prompt fixed it, but this is a common, deterministic class of mismatch.

4) **Many post-success inspection passes with no mutations.**
   - After the substantive work completed (plan ops + component generation + QA), subsequent passes mostly re-fetched the same KV slices.
   - This looks like an agent “no-progress loop” that should be cut off deterministically.

5) **Final `done` reason was not evidence-backed.**
   - The `done` narrative claimed a deterministic draft-op update + QA rerun, but the tool/event log did not show those actions in the final pass.
   - The *state* was fine; the *reporting* was misleading.

## Goals

- Reduce wasted passes/tokens due to repetitive inspection on unchanged state.
- Make Info Store inspection failures actionable without additional LLM work.
- Enforce “gates” inside tools (contract-first), avoiding agent-level “remember to do X” rules.
- Make “done” reporting verifiable against actual tool events and state.

## Non-goals

- No heuristic “intent guessing” algorithms.
- No silent auto-fixes that change content without explicit tool results.
- No backwards-compat guarantees while iterating on Gen3D contracts.

## Proposals

### 1) Improve oversize `info_kv_get_v1` ergonomics (actionable + bounded)

Problem: a call like:

```json
{ "namespace":"gen3d", "key":"ws.main.scene_graph_summary", "selector":{"kind":"latest"}, "max_bytes":12000 }
```

can fail with “KV value too large… use `json_pointer`”, forcing another pass.

Proposed contract changes (deterministic, no heuristics):

- When the selected value exceeds `max_bytes` **and** `json_pointer` is omitted, return:
  - `ok: false` plus a *structured* error payload that includes:
    - record metadata (`kv_rev`, `bytes`, `summary`, `written_by`) so the agent can pin a revision,
    - a deterministic **shape preview** of the selected JSON value (no truncation):
      - objects: sorted keys (sample + total),
      - arrays: length,
      - strings: byte length,
    - a bounded list of **fixits** that are purely mechanical next calls:
      - retry with `json_pointer` set to one top-level key (suggest a few keys from the preview),
      - or switch to `info_kv_get_paged_v1` when that key is an array.

This keeps the tool “actionable” without needing the agent to burn a full additional LLM step
to discover what pointers exist.

### 2) Prevent QA-gated regen in `llm_review_delta_v1` (reduce wasted tokens)

Problem: `llm_review_delta_v1` can call the LLM and produce `regen_component`, then the engine blocks
the regen due to preserve-mode QA-gating.

Proposed behavior (aligned with `docs/agent_skills/tool_authoring_rules.md`):

- Deterministically compute `regen_allowed` (preserve-mode + QA gate state) *before* calling the model.
- If `regen_allowed=false`, do **not** early-return the entire tool (review-delta can still produce non-regen tweak ops).
  Instead, choose one deterministic enforcement path:

  1) **Schema-variant enforcement (preferred):**
     - Use a “no-regen” ReviewDelta schema variant (or equivalent schema gate) that *cannot express* regen actions.
     - Include `regen_allowed=false` in the tool result so the agent can explain why regen wasn’t considered.

  2) **Repair-on-regen-only (fallback):**
     - If the model returns regen actions anyway and there are no non-regen actions to apply, return `ok:false` with:
       - a concise error explaining the QA gate,
       - fixits: `qa_v1`, or `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`,
       - and (optionally) one schema-repair retry prompt that explicitly instructs “return review-delta WITHOUT regen actions”.

This preserves determinism while preventing the common “regen-only output → blocked by QA gate” waste.

### 3) Reduce `llm_generate_plan_ops_v1` schema repair roundtrips

Problem: common, low-entropy schema mismatches (e.g. `add_component.component` vs `add_component.name`)
trigger an LLM “repair” call.

Options (pick one; both are deterministic and explicitly observable):

1) **Prompt/schema tightening** (preferred first step):
   - Update tool instructions/examples to emphasize exact field names.

2) **Deterministic micro-repair for known aliases** (optional):
   - If `kind="add_component"` and `name` is missing but `component` is a string:
     - map `component -> name`,
     - record the normalization explicitly in the tool result/artifacts (`repaired=true`, `repair_diff=[...]`),
     - error (do not repair) if both exist and differ.

This avoids burning a large repair completion on a tiny mechanical fix, while keeping the tool
behavior explicit and diffable.

### 4) Deterministic “no-progress loop” cutoffs for inspection-only passes

Problem: after a successful mutation + QA, the agent can continue requesting more steps that only
repeat inspection reads.

Proposed engine-side mechanisms (no heuristics):

- Note: Gen3D already has a deterministic no-progress guard and budgets. The remaining improvements here are about making inspection loops cheaper and more observable.

- Add per-pass caching for identical `info_kv_get*` calls (same selector resolved to same `kv_rev` + same `json_pointer` + same caps).
  - Return `cached=true` and `no_new_information=true` (consistent with `qa_v1`) and the previously computed result payload.

- When the no-progress guard stops the run, emit an explicit stop reason:
  - an Info Store event with `kind="budget_stop"` / `stop_reason="no_progress"` (or equivalent),
  - and a deterministic “next steps” list (fixits), e.g. `diff_snapshots_v1` if snapshots exist.

### 5) Require evidence-backed `done` (or auto-compose final summary from events)

Problem: “done” narratives can drift from reality.

Proposed contract:

- Bump the agent step protocol version and extend `done` to include evidence fields that the engine can validate (breaking change is acceptable while iterating):

```json
{
  "kind": "done",
  "reason": "…",
  "evidence": {
    "assembly_rev": 14,
    "qa": { "namespace":"gen3d", "key":"ws.main.qa", "selector":{"kind":"kv_rev","kv_rev":62} },
    "state_summary": { "namespace":"gen3d", "key":"ws.main.state_summary", "selector":{"kind":"kv_rev","kv_rev":123} },
    "mutations": [{ "tool_id":"apply_draft_ops_v1", "call_id":"call_7" }]
  }
}
```

Engine behavior:
- If evidence does not match current state/events, reject `done` as invalid (actionable error),
  or ignore the free-text narrative and render a deterministic, event-derived final summary.

This keeps “what happened” machine-true without relying on the LLM’s paraphrase.

## Acceptance criteria (what “better” looks like)

- Oversize KV reads return bounded previews + fixits (no follow-up LLM step required just to find pointers).
- When regen is deterministically disallowed (preserve-mode QA gate closed), `llm_review_delta_v1` does not produce regen actions (schema-gated or reliably repairable).
- Common plan-ops schema mismatches avoid LLM repair roundtrips (or at least become rarer).
- Runs stop quickly after successful mutation + QA (no long inspection-only tails).
- Final run summary is verifiably consistent with tool events and state.

## Open questions

- Should evidence-backed `done` be enforced for all agents/tools, or only Gen3D?
- Where should “no-progress stop” live: in the generic agent loop budgets, or specifically inside Gen3D orchestration?
- Do we want `info_kv_get_v1` to return a truncated preview (`ok:true,truncated:true`) instead of `ok:false` on oversize,
  to reduce the need for error handling logic in the agent?
