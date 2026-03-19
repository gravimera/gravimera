# Gen3D: Actionable root-field QA fixits + PlanOps support for mobility/attack/collider

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D runs can get stuck in repeated passes when QA fails for a *root-field* reason (for example: the prompt requires attacks but the draft root has no attack profile). The agent often keeps re-authoring motion or re-running QA because the QA output does not include an actionable “what to do next” tool path.

After this change, root-field QA failures become **actionable** and **diffable**:

- `qa_v1` will return fixits that point to the right next tool(s) (for example: preserve-mode plan ops, or review-delta) instead of only an opaque `blocked_reason`.
- `apply_plan_ops_v1` (and therefore `llm_generate_plan_ops_v1`) will support new PlanOps operations to set **plan-level root fields**: `mobility`, `attack`, and `collider`. This gives seeded edits a preserve-mode, diff-first way to add an attack profile without regenerating geometry.

How to see it working:

1. Start a seeded Edit/Fork Gen3D run on a movable unit prefab that has `attack=null` at the root, and use a prompt that clearly requests attacks (any language).
2. Run `qa_v1`. It should fail with a root attack profile error and include fixits that route to preserve-mode plan ops.
3. Apply the suggested fixits (`get_plan_template_v1` → `llm_generate_plan_ops_v1`, or `apply_plan_ops_v1` directly) and re-run `qa_v1`.
4. Observe that `qa_v1.ok=true` (warnings may remain), and `smoke.attack_present=true`.


## Progress

- [x] (2026-03-20) Drafted this ExecPlan.
- [x] (2026-03-20) Implemented root-field PlanOps (`set_mobility`, `set_attack`, `set_collider`) in `apply_plan_ops_v1` + structured-output schema.
- [x] (2026-03-20) Made `qa_v1` root-field capability gaps include actionable fixits (prefer preserve-mode plan ops; fallback to replanning when no plan exists).
- [x] (2026-03-20) Updated tool documentation (`src/gen3d/agent/tools.rs`) and user-facing docs (`gen_3d.md`) to reflect new PlanOps kinds and root-field remediation guidance.
- [ ] Add unit/regression tests for the new PlanOps operations and QA fixit generation.
- [ ] Run `cargo test` and the rendered smoke test (`cargo run -- --rendered-seconds 2`).


## Surprises & Discoveries

- Observation: `smoke.attack_present` means “root attack profile exists” (draft root `attack != null`), not “attack animation exists”.
  Evidence: `src/gen3d/ai/orchestration.rs::build_gen3d_smoke_results` sets `attack_present = draft.root_def().attack.is_some()`.

- Observation: PlanOps schema is duplicated in two places: the LLM structured-output schema (`src/gen3d/ai/structured_outputs.rs`) and the human-facing tool descriptor (`src/gen3d/agent/tools.rs`).
  Evidence: `schema_plan_op()` (structured outputs) vs. `TOOL_ID_APPLY_PLAN_OPS.args_schema` (tool descriptor text).

- Observation: `llm_review_delta_v1` can set root `mobility`/`attack` but cannot set root `collider` today; collider repairs require plan changes.
  Evidence: `src/gen3d/ai/structured_outputs.rs` includes `tweak_mobility` and `tweak_attack`, but has no `tweak_collider`.

- Observation: `plan.attack` is ignored unless required subfields are present (melee requires `range`/`radius`/`arc_degrees`; ranged_projectile requires BOTH `muzzle` and `projectile`).
  Evidence: `src/gen3d/ai/convert.rs` warns and skips the attack profile when these fields are missing.


## Decision Log

- Decision: Implement root-field edits as PlanOps (`apply_plan_ops_v1`) rather than a new bespoke “root patch” tool.
  Rationale: PlanOps already provide deterministic diffs, preserve-mode policy checks, and are the preferred “diff-first” mechanism for seeded edits.
  Date/Author: 2026-03-20 / user + assistant

- Decision: Make QA fixits prefer preserve-mode plan ops (`get_plan_template_v1` → `llm_generate_plan_ops_v1`) for root-field gaps when a plan exists.
  Rationale: This creates a consistent and diffable remediation path that does not consume the limited `llm_review_delta_v1` budget and does not require geometry regeneration.
  Date/Author: 2026-03-20 / user + assistant


## Outcomes & Retrospective

- (TBD) Summarize which loops were eliminated and which root-field gaps are now automatically actionable from QA outputs.


## Context and Orientation

Key concepts:

- “Root fields” are the top-level gameplay fields on the Gen3D draft root object definition (`draft.root_def()`), such as:
  - `mobility` (static/ground/air),
  - `attack` (none/melee/ranged_projectile),
  - `collider` (selection/click hit area).
- “Smoke check” (`smoke_check_v1`) is a deterministic check that reports:
  - whether required capabilities are present (`attack_present`, `mobility_present`),
  - motion validation issues,
  - and an overall `ok` boolean.
- “QA” (`qa_v1`) runs `validate_v1` + `smoke_check_v1`, then aggregates issues into:
  - `errors` (blocking),
  - `warnings` (non-blocking),
  - and `capability_gaps` (intended to be actionable blockers with fixits).

Relevant code locations:

- `src/gen3d/ai/orchestration.rs`
  - Builds smoke results; defines `attack_present`/`mobility_present`.
- `src/gen3d/ai/agent_tool_dispatch.rs`
  - Implements `qa_v1` and constructs `capability_gaps` via `build_capability_gaps_from_smoke_v1`.
- `src/gen3d/ai/plan_ops.rs`
  - Implements PlanOps application (`apply_plan_ops_v1`) and the PlanOp enum.
- `src/gen3d/ai/structured_outputs.rs`
  - Defines the JSON schema for `llm_generate_plan_ops_v1` structured output (PlanOps).
- `src/gen3d/agent/tools.rs`
  - Human-facing tool descriptor strings shown to the LLM (args schema summaries).


## Plan of Work

### 1) Extend PlanOps to support root fields

Update PlanOps so a preserve-mode plan patch can set the plan root fields (not just components/attachments):

- Add new PlanOp kinds in `src/gen3d/ai/plan_ops.rs`:
  - `set_mobility` with payload `mobility` (same schema as plan.mobility).
  - `set_attack` with payload `attack` (nullable; same schema as plan.attack).
  - `set_collider` with payload `collider` (nullable; same schema as plan.collider).

Implementation notes:

- These ops mutate the plan object fields (`plan.mobility`, `plan.attack`, `plan.collider`) before conversion to planned components/draft defs.
- They must produce deterministic diffs in `apply_plan_ops_v1` output (`applied_ops[].diff`).
- Preserve-mode diff policy should still only validate component attachment/offset diffs; root-field ops must be allowed.

Update the structured-output schema in `src/gen3d/ai/structured_outputs.rs::schema_plan_op()` to include the new PlanOp variants so `llm_generate_plan_ops_v1` can emit them.

Update the tool descriptor in `src/gen3d/agent/tools.rs` so the LLM sees the new PlanOp shapes in the abbreviated args schema string.


### 2) Make QA root-field gaps actionable

Update `src/gen3d/ai/agent_tool_dispatch.rs::build_capability_gaps_from_smoke_v1`:

- For root attack/mobility inconsistencies (attack required but missing; attack present but mobility missing), include fixits that point to a working remediation path:
  - Prefer preserve-mode plan ops fixits when a plan exists:
    - `get_plan_template_v1` (mode="auto")
    - `llm_generate_plan_ops_v1` with a prompt override that explicitly says to set the missing root fields (using the new PlanOps kinds).
  - If preserve-mode plan ops are not applicable (no plan yet), suggest `llm_generate_plan_v1` or `llm_review_delta_v1` as appropriate.

- For root collider missing (`collider.kind="none"` on a movable unit), update guidance to avoid suggesting tools that cannot fix collider. Prefer plan ops / replan guidance.

- Make the gap evidence explicit (for example, include `field="attack"` vs `field="collider"`) so the agent can choose the correct tool instead of re-authoring motion blindly.

The goal is that a single QA error is enough to tell an agent/pipeline the correct next tool(s) to call without additional KV inspection loops.


### 3) Update docs and add tests

Docs:

- Update `gen_3d.md` to mention that preserve-mode plan ops can now set root `mobility`/`attack`/`collider` (and that this is the recommended remediation path for root-field QA gaps).

Tests:

- Add unit tests in `src/gen3d/ai/plan_ops.rs` to verify:
  - `apply_plan_ops_v1` accepts and applies the new PlanOp kinds.
  - Root fields change as expected in the accepted plan and in the draft root def.
- Add a unit test around `build_capability_gaps_from_smoke_v1` to verify:
  - When `attack_required_by_prompt=true` and `attack_present=false`, the resulting capability gap includes fixits referencing `get_plan_template_v1` and `llm_generate_plan_ops_v1` (or the chosen policy).


## Concrete Steps

All commands below run from the repository root (`/Users/flow/workspace/github/gravimera`).

1. Implement code changes and tests.
2. Run unit tests:

    cargo test

3. Run the required rendered smoke test (UI, non-headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2


## Validation and Acceptance

This change is accepted when:

- `cargo test` passes.
- The rendered smoke test starts and exits without crashing.
- A seeded edit run that previously looped due to missing root attack profile now has an actionable QA output and can converge by applying the suggested fixits (plan ops / review delta), resulting in `qa_v1.ok=true`.


## Idempotence and Recovery

- The PlanOps changes are additive. If a new PlanOp kind is malformed, `apply_plan_ops_v1` must reject it with an actionable error without mutating state (`dry_run=true` can be used to validate a patch safely).
- If QA fixits are too aggressive or incorrect, revert the fixit changes without reverting the PlanOps extensions (PlanOps are still useful as a standalone capability).


## Artifacts and Notes

Run caches for debugging live under:

    ~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_N/

Key files:

- `qa.json`, `smoke_results.json` (what QA saw)
- `tool_calls.jsonl`, `tool_results.jsonl` (which tools were invoked)
- `gen3d_run.log` (high-level orchestration log)


## Interfaces and Dependencies

PlanOps JSON interface additions:

- `set_mobility`:
  - `{ "kind": "set_mobility", "mobility": { "kind": "static" | "ground" | "air", ... } }`
- `set_attack`:
  - `{ "kind": "set_attack", "attack": null | { "kind": "none" | "melee" | "ranged_projectile", ... } }`
- `set_collider`:
  - `{ "kind": "set_collider", "collider": null | { "kind": "none" | "circle_xz" | "aabb_xz", ... } }`

These must be accepted by:

- `apply_plan_ops_v1` (deterministic tool)
- `llm_generate_plan_ops_v1` (LLM tool structured output schema)
