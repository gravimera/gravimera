# Tool Authoring Rules (Agent-Facing Contracts)

These rules apply when adding or modifying **agent-facing tools** (Gen3D tools, scene generation tools, HTTP APIs treated as tools, etc).

Goal: tools should feel like a **compiler toolchain** for agents:

- predictable and deterministic,
- generic (no domain heuristics),
- easy to discover and use correctly,
- results are **actionable** (the agent can take the next step without guesswork),
- errors are **actionable** (the agent learns what to do next),
- no “silent” state changes without an explicit mutation tool call.

---

## 1) Non‑negotiables

1) **Deterministic and generic**
- Tools must not encode hidden, domain-specific heuristics (“town rules”, “hat placement nudges”, etc).
- If the user/agent can ask for *any* object/system, defaults must be generic and explicit.
- If inputs are ambiguous or missing, prefer **deterministic error → agent decides next step** (or ask for explicit params) over silent “best guesses”.

2) **Versioned contracts**
- Tool ids are versioned (`*_v1`) and schemas are treated as **contracts**.
- If you must break behavior/shape, bump the version.

3) **Bounded by design**
- Any tool that can return “a lot” must have explicit bounds (`max_items`, `max_bytes`, etc) and return `truncated: true/false`.

4) **Observable side effects**
- Mutation tools must clearly report what changed.
- If a tool writes artifacts, say so and return refs/paths when possible.

---

## 2) Discovery: registry + schema + examples

When you add/change a tool:

- Update `src/gen3d/agent/tools.rs` (or the equivalent registry) with:
  - `one_line_summary`: include **must-know constraints** (gates, budgets, side effects, artifacts written).
  - `args_schema`: list only accepted keys (many args structs use `deny_unknown_fields`).
  - `args_example`: must be valid and runnable; avoid pseudo-keys.
- Keep the tool list self-sufficient: the agent should not have to “guess” that a gate exists.

Rule of thumb:
- If a constraint can cause a tool to no-op or error, it belongs in `one_line_summary` and the tool’s error message.

---

## 3) Tool results must be actionable

### 3.1 Provide the “next-step payload”

A tool result is actionable if the agent can decide what to do next **without**:

- dumping huge state blobs,
- calling multiple extra inspection tools,
- or inventing heuristics.

Prefer including small, targeted summaries:

- counts (`components_total`, `parts_total`, `warnings_count`),
- a few **examples** (first N items) that include the fields needed to edit correctly,
- stable identifiers for mutation (`part_id_uuid`, `component_index`, `component_name`),
- explicit “skipped/blocked” lists with reasons.

Bad (non-actionable):
- `ok keys=["parts","truncated"]`

Good (actionable, bounded):
- `ok parts=3 part_examples=[{part_id_uuid, mesh, color_rgba, pos, scale}, …]`

### 3.2 Prefer diffs over dumps

If a tool mutates state, it should return either:

- a structured `applied[] / rejected[]` list, or
- a compact “diff-like” summary (what changed, for which ids).

Do not make the agent infer changes by re-querying the entire world.

### 3.3 Make gating explicit in the result shape

When work is skipped/blocked, return machine-readable fields:

- `skipped_due_to_*` (budget, preserve mode, missing capability, etc)
- `blocked_due_to_*` (QA gate, policy gate, etc)

Include `index` + `name` pairs where possible (agents reason in names; engines act on indices/ids).

---

## 4) Errors must be actionable (compiler-style diagnostics)

Tool errors should include:

1) **What failed** (which constraint/gate/policy was violated).
2) **What the tool expected** (the rule, or the missing arg).
3) **What to do next** (the recovery path) using concrete tool ids/args.

Prefer errors that are specific and lead to a next step:

- Good: “Refusing force regeneration because QA is clean; run `qa_v1` and only use `force=true` when there are errors; otherwise use `apply_draft_ops_v1` / non-regen `llm_review_delta_v1` actions.”
- Bad: “Invalid request.”

Avoid silent fallbacks:
- If the tool had to ignore inputs (unknown key, invalid component id), return an error (or a structured warning field) rather than silently dropping work.

---

## 5) No silent mutation without explicit agent confirmation

Principle: **suggest → confirm → apply**.

- Read-only tools may propose changes (FixIts, suggestions).
- Mutation must happen only via explicit mutation tool calls.

If a tool *must* do deterministic auto-repair (rare), it must:

- be documented in `one_line_summary`,
- be explicitly reported in the result (`applied=true`, list of edits),
- write an artifact for auditability,
- ideally support `dry_run=true` so the agent can inspect before applying.

This prevents “the engine changed something but the agent didn’t ask”.

---

## 6) Prefer tool-contract enforcement over prompt micromanagement

Avoid adding agent-level “call X before Y” rules or heuristic-looking guidance in prompts.

Instead:

- enforce preconditions inside the tool implementation,
- return an actionable error when a precondition isn’t met,
- expose state flags in `get_state_summary_v1` (or equivalent),
- provide deterministic suggestion tools (`suggest_*`) that return explicit patches.

Prompts should focus on:

- strict output formatting (JSON-only, one object),
- universal safety/determinism constraints,
- high-level “compiler philosophy” (structure over cosmetics).

Tool-specific sequencing is better expressed as:

- tool contract + schema,
- tool result fields (`pending_*`, `skipped_*`, `blocked_*`),
- tool errors with concrete recovery options.

---

## 7) QA gates and budgets: make them teachable

If a gate exists (QA gate, capability gate, regen budget, no-progress guard):

- surface it in state summary (`budgets.*`, `pending_*`, `blocked_*`),
- surface it in tool summaries,
- surface it in tool errors.

Important: if the only remaining requested work is impossible due to a gate,
return an error that explains **why** and **what alternatives are allowed** (deterministic edits, disable preserve mode, finish best-effort).

This prevents deadlocks and “inspection loops” that burn the no-progress budget.

---

## 8) Minimum checklist (authoring)

Before merging a tool change:

- Registry updated (`one_line_summary`, `args_schema`, `args_example`).
- Tool results include a bounded “next-step payload” (examples + ids + counts).
- Tool errors explain cause + fix (with concrete tool ids/args).
- Any automatic behavior is explicit (no silent apply).
- Docs updated under `docs/` (keep `README.md` clean).
- Run the rendered smoke test:
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

