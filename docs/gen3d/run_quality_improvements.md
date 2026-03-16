# Gen3D run quality improvements (inspection/KV, gating, schema, done-evidence)

This is a design note capturing improvement ideas from a real seeded-edit run
(`run_id=374aa3b9-6d79-43fb-a40f-dce8329a1463`, 2026-03-17) where the agent
performed many repeated inspection steps (`info_kv_get*`) and produced at least
one misleading final "done" narrative.

Scope: proposals only (no code changes yet).

Related docs:
- Tool authoring constraints: `docs/agent_skills/tool_authoring_rules.md`
- KV paging contract: `docs/gen3d/info_kv_get_paged_v1.md`
- Preserve-mode edits: `docs/gen3d/edit_preserve_existing_components.md`
- Plan-ops tool: `docs/gen3d/llm_generate_plan_ops_v1.md`
- QA tool: `docs/gen3d/qa_v1.md`
- Review-delta semantics: `docs/gen3d/review_delta_v1.md`

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
  - `ok: false` plus an error payload that includes:
    - the record metadata (`kv_rev`, `bytes`, `summary`) so the agent can pin the revision,
    - a deterministic **shape preview** of the top-level value:
      - for objects: sorted keys (sample + total),
      - for arrays: length,
      - for strings: byte length,
    - a short list of **suggested next calls** (fixits) that are purely mechanical, e.g.:
      - retry with `json_pointer` set to one top-level key,
      - switch to `info_kv_get_paged_v1` if that key is an array.

This keeps the tool “actionable” without needing the agent to burn a full additional LLM step
to discover what pointers exist.

### 2) Preflight QA-gate inside `llm_review_delta_v1` (avoid wasted LLM calls)

Problem: `llm_review_delta_v1` can call the LLM and produce `regen_component`, then the engine blocks
the regen due to preserve-mode QA-gating.

Proposed behavior (aligned with `docs/agent_skills/tool_authoring_rules.md`):

- Run a deterministic preflight *before* any LLM request:
  - If preserve-mode is on **and** regen of generated components is disallowed by current QA state,
    return `ok:false` with an actionable error and fixits:
    - `qa_v1` (if QA never run),
    - `apply_draft_ops_v1` / non-regen actions (if user intent is achievable via deterministic ops),
    - `llm_generate_plan_v1` with `constraints.preserve_existing_components=false` (explicit opt-out).
- Ensure the tool result makes it clear that **no LLM request was made** on this early-return path.

This preserves determinism, reduces tokens, and makes failures cheaper.

### 3) Reduce `llm_generate_plan_ops_v1` schema repair roundtrips

Problem: common, low-entropy schema mismatches (e.g. `add_component.component` vs `add_component.name`)
trigger an LLM “repair” call.

Options (pick one; both are deterministic and explicitly observable):

1) **Prompt/schema tightening** (preferred first step):
   - Update tool instructions/examples to emphasize exact field names.

2) **Deterministic micro-repair for known aliases** (optional):
   - If `kind="add_component"` and `name` is missing but `component` is a string:
     - map `component -> name`,
     - record the normalization in the tool result (`repaired=true`, `repair_diff=[...]`),
     - error (do not repair) if both exist and differ.

This avoids burning a large repair completion on a tiny mechanical fix, while keeping the tool
behavior explicit and diffable.

### 4) Deterministic “no-progress loop” cutoffs for inspection-only passes

Problem: after a successful mutation + QA, the agent can continue requesting more steps that only
repeat inspection reads.

Proposed engine-side mechanisms (no heuristics):

- Track a per-run “mutation counter” (plan/draft changes) and a “new info counter” (new KV revs read/written).
- If N consecutive passes have:
  - no mutations, and
  - no new information fetched (same KV rev + same pointers), and
  - no budgets forcing continuation,
  then stop the run deterministically with `stop_reason=no_progress` and a final status:
  - “No progress detected; last successful mutation was at pass X.”
  - Include fixits: suggest calling `diff_snapshots_v1` (if snapshots exist) or rerun with a clearer placement constraint.

Additionally (cheap win):
- Add per-pass caching for identical `info_kv_get*` calls (same selector resolved to same `kv_rev` + same `json_pointer` + same caps).
  - Return `cached=true` and the previously computed result.

### 5) Require evidence-backed `done` (or auto-compose final summary from events)

Problem: “done” narratives can drift from reality.

Proposed contract:

- Extend `done` action to include evidence fields that the engine can validate:

```json
{
  "kind": "done",
  "reason": "…",
  "evidence": {
    "assembly_rev": 14,
    "qa": { "namespace":"gen3d", "key":"ws.main.qa", "selector":{"kind":"kv_rev","kv_rev":62} },
    "mutations": [{ "tool_id":"llm_generate_plan_ops_v1", "call_id":"call_1" }]
  }
}
```

Engine behavior:
- If evidence does not match current state/events, reject `done` as invalid (actionable error),
  or ignore the free-text narrative and render a deterministic, event-derived final summary.

This keeps “what happened” machine-true without relying on the LLM’s paraphrase.

## Acceptance criteria (what “better” looks like)

- Oversize KV reads return bounded previews + fixits (no follow-up LLM step required just to find pointers).
- `llm_review_delta_v1` never spends LLM tokens on a regen plan that will be blocked by a deterministic gate.
- Common plan-ops schema mismatches avoid LLM repair roundtrips (or at least become rarer).
- Runs stop quickly after successful mutation + QA (no long inspection-only tails).
- Final run summary is verifiably consistent with tool events and state.

## Open questions

- Should evidence-backed `done` be enforced for all agents/tools, or only Gen3D?
- Where should “no-progress stop” live: in the generic agent loop budgets, or specifically inside Gen3D orchestration?
- Do we want `info_kv_get_v1` to return a truncated preview (`ok:true,truncated:true`) instead of `ok:false` on oversize,
  to reduce the need for error handling logic in the agent?

