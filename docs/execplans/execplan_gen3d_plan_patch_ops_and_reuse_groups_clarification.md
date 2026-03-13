# Gen3D: Plan patch ops for semantic failures + reuse_groups clarification (avoid full replans)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

When `llm_generate_plan_v1` returns schema-valid JSON that is semantically invalid (unknown parent, missing anchors, missing referenced components), the agent currently has only one practical repair path: re-run `llm_generate_plan_v1` and regenerate the entire plan. This is expensive, brittle (it can “fix one thing and break another”), and makes the agent loop feel non-deterministic.

After this change:

- The plan prompt is clearer about a common pitfall: `reuse_groups` does not create components. Every `reuse_groups[].targets[]` name must also exist in `components[]` with `attach_to`.
- The engine exposes a deterministic “apply_patch-like” tool for plans: the agent can repair a rejected plan by applying explicit, validated plan operations (PlanOps) rather than regenerating the full plan.
- Inspection output for rejected plans becomes more actionable by explicitly enumerating “referenced-but-undefined” component names (including those coming from `reuse_groups`, `aim`, and `attack.muzzle`) and, when safe, returning FixIt-style suggested PlanOps.

User-visible outcome: Gen3D planning converges faster and with fewer redundant LLM calls, while staying aligned with the repository’s rule of “no heuristics”: tools provide deterministic diagnostics and deterministic application of explicit edits; the agent decides what to do next.

## Progress

- [x] (2026-03-14) Clarify `reuse_groups` semantics in the plan system prompt.
- [x] (2026-03-14) Extend plan inspection diagnostics to report missing referenced component names (not just unknown parents).
- [x] (2026-03-14) Add `apply_plan_ops_v1` (deterministic) to patch a pending rejected plan attempt and revalidate.
- [x] (2026-03-14) Add optional FixIt suggestions (bounded) to `inspect_plan_v1` for repairs that are logically forced by the rejected plan’s explicit intent.
- [x] (2026-03-14) Update agent prompt + docs so the new tool is discoverable (suggestions only; do not hardcode a flow).
- [x] (2026-03-14) Add unit tests for new inspection errors and plan-op application.
- [x] (2026-03-14) Run `cargo test` and the required rendered smoke test.
- [x] (2026-03-14) Commit with a clear message.

## Surprises & Discoveries

- Observation: The plan can include `reuse_groups` that reference mirrored target component names (e.g. `arm_lower_r`) without defining those targets as components in `components[]`. The engine does not treat `reuse_groups` as component declarations, so any later reference to a missing target (e.g. `laser_cannon.attach_to.parent = arm_lower_r`) triggers a semantic rejection.
  Evidence: In run `1468550c-5da0-482a-813d-7bc8f6332dae`, the plan defined only left-side components in `components[]` but referenced `*_r` names in `aim` and `attach_to`. It also declared mirror `reuse_groups` to produce `*_r` geometry later, but those targets were absent from `components[]`.

- Observation: The current semantic error surface (“unknown parent”) points at the symptom (`attach_to.parent` missing) but not the root cause (“you referenced a reuse target that does not exist as a component”).
  Evidence: The warning was `AI plan: component laser_cannon attach_to parent arm_lower_r not found.`; `inspect_plan_v1` reported `unknown_parent` with empty suggestions because there were no existing components (fresh plan).

- Observation: Returning `inspect_plan_v1`-style computed structural errors as part of `apply_plan_ops_v1` results makes the mutation tool actionable without forcing an extra tool call.
  Evidence: `apply_plan_ops_v1` returns `new_errors[]` that includes semantic validation errors plus bounded structural errors/fixits derived from `inspect_plan_v1`.

## Decision Log

- Decision: Provide deterministic tooling for plan patching rather than adding agent-loop “call X before Y” rules.
  Rationale: This repo prefers tool-contract enforcement and actionable tool errors over agent-level orchestration rules. The agent remains free to decide the next step (replan, patch, template-based replan), but the engine must expose safe, explicit primitives to make patching possible.
  Date/Author: 2026-03-14 / flow + agent

- Decision: Only produce FixIt-style suggested PlanOps when the repair is logically forced by the rejected plan’s explicit intent; otherwise refuse to guess and return detailed diagnostics.
  Rationale: “No heuristics” means the engine should not infer intent. Suggestions are acceptable only when they follow directly from explicit references already present in the plan (compiler-like diagnostics).
  Date/Author: 2026-03-14 / flow + agent

- Decision: Clarify prompt semantics for `reuse_groups` rather than relying on post-hoc repair.
  Rationale: Preventing a class of plan errors is cheaper than repairing them; the prompt is the first line of defense.
  Date/Author: 2026-03-14 / flow + agent

- Decision: `apply_plan_ops_v1` commits patched pending plans even when still semantically invalid (unless `dry_run=true`), and returns bounded diagnostics for iterative patching.
  Rationale: This matches the toolchain model: explicit edits + deterministic revalidation + actionable diagnostics, without needing full replans.
  Date/Author: 2026-03-14 / agent

## Outcomes & Retrospective

- Outcome (2026-03-14): Gen3D planning repairs can be expressed as explicit PlanOps instead of full replans.
  - Prompt: `reuse_groups` semantics clarified so targets/sources must also exist in `components[]`.
  - Diagnostics: `inspect_plan_v1` now reports `missing_component_reference` (attach_to / aim / muzzle / reuse_groups) and can return bounded FixIts (`analysis.fixits[]`) only when forced.
  - Mutation: `apply_plan_ops_v1` applies explicit ops to `job.pending_plan_attempt.plan`, revalidates, and accepts the plan when valid (clearing `pending_plan_attempt`), with audit artifacts and bounded diffs/errors.
  - Tests: added unit tests for the new diagnostics and plan-op application; smoke test confirmed the game starts cleanly.

## ExecPlan Change Notes

- 2026-03-14: Marked completed milestones and recorded outcomes/decisions based on the implemented tool + diagnostics changes.


## Context and Orientation

Gen3D plan generation is driven by `llm_generate_plan_v1`, which asks an LLM to output a component assembly plan as strict JSON (`AiPlanJsonV1`).

Key terms:

- Plan: the JSON object containing `components[]` (names, sizes, anchors, attachments), optional `reuse_groups`, and optional combat/aim/collider metadata.
- Semantic plan failure: the JSON parses and matches schema, but violates engine invariants (e.g., attachment parent does not exist, anchors referenced by attachments are missing, preserve-mode constraints violated).
- Pending rejected plan attempt: when the plan fails semantically, the engine stores the decoded plan as `job.pending_plan_attempt` so inspection tools can report computed constraints/errors.
- Reuse group: a plan declaration that allows the engine to generate geometry for repeated parts by deterministic copy/mirror, instead of calling the LLM for each repeated part. Reuse groups affect *geometry generation*, not the attachment tree; components and attachments must still be declared explicitly.
- PlanOps: a small, explicit edit language for plans, analogous to `apply_draft_ops_v1` for drafts. PlanOps are applied deterministically by the engine; they do not “guess”.

Relevant code (paths from repo root):

- `src/gen3d/ai/prompts.rs`: plan system prompt builder (`build_gen3d_plan_system_instructions`) and user-text prompt builders.
- `src/gen3d/ai/agent_tool_poll.rs`: parses LLM plan output and stores `pending_plan_attempt` on semantic failures.
- `src/gen3d/ai/job.rs`: `Gen3dPendingPlanAttempt` structure (`job.pending_plan_attempt`).
- `src/gen3d/ai/plan_tools.rs`: implementation of `inspect_plan_v1` (`inspect_pending_plan_attempt_v1`).
- `src/gen3d/ai/convert.rs`: plan conversion; also hydrates missing attachment anchors on reuse targets (but does not create missing components).
- `src/gen3d/agent/tools.rs`: tool registry exposed to the agent (discoverability contract).
- Docs: `docs/agent_skills/tool_authoring_rules.md` (contract-first tooling rules) and `docs/agent_skills/prompt_tool_contract_review.md` (prompt/tool alignment checklist).

## Plan of Work

### 1) Prompt clarification: `reuse_groups` does not define components

In `src/gen3d/ai/prompts.rs` (`build_gen3d_plan_system_instructions`), add an explicit rule directly under the `reuse_groups` section:

- `reuse_groups` is only an optimization for how geometry is generated.
- Every `reuse_groups[].source` and every name in `reuse_groups[].targets[]` must also appear as a component in `components[]`.
- If a component name is referenced anywhere (`attach_to.parent`, `aim.components`, `attack.muzzle.component`, `reuse_groups`), it must exist in `components[]`.

This is a clarification only; it does not change any runtime semantics.

Acceptance signal for this step: plan outputs for symmetric objects (explicitly mentioning left/right pairs) more consistently declare both sides as components rather than relying on implicit reuse targets.

### 2) Make inspection diagnostics enumerate missing referenced components

Extend `inspect_plan_v1` in `src/gen3d/ai/plan_tools.rs` to compute and report missing referenced component names beyond the existing `unknown_parent` error:

- Collect references from:
  - every `attach_to.parent` value,
  - `aim.components[]` (if present),
  - `attack.muzzle.component` (if present),
  - `reuse_groups[].source` and `reuse_groups[].targets[]` (if present).
- Compare against the plan’s declared `components[].name`.
- Emit an error kind (exact naming is up to implementation, but keep it stable and documented), for example:
  - `missing_component_reference` with fields:
    - `name` (the missing component name),
    - `referenced_by` (bounded list of reference locations such as `attach_to.parent`, `aim.components`, `attack.muzzle.component`, `reuse_groups.targets`),
    - `plan_component_names_sample` (bounded sample to aid repair).

This diagnosis must be bounded, deterministic, and purely structural (no domain heuristics).

Acceptance signal for this step: the “root cause” of a missing component is visible immediately from `inspect_plan_v1`, without making the agent infer it from `unknown_parent` alone.

### 3) Add `apply_plan_ops_v1`: deterministic plan patching for rejected plans

Add a new agent-facing tool `apply_plan_ops_v1` that mutates the current session’s “pending rejected plan attempt” (`job.pending_plan_attempt`) by applying explicit PlanOps.

This tool is analogous to `apply_draft_ops_v1`, but operates on `AiPlanJsonV1` (the plan JSON), not on draft primitives.

Tool contract:

- Mutation is explicit and observable: return `{ ok, applied_ops, rejected_ops, diff_summary, new_plan_summary, still_pending, new_errors? }`.
- Provide `dry_run?: bool` so the agent can inspect without applying.
- Enforce strict validation: reject unknown op kinds and reject ops that would make the plan invalid (or apply and then return a deterministic validation error list; pick one policy and document it).
- Re-run semantic validation after applying ops. If the plan is now valid, accept it as if `llm_generate_plan_v1` succeeded (populate `job.planned_components`, clear `pending_plan_attempt`, update `assembly_rev`, and persist snapshots/artifacts as normal for a new plan).
- Bounds: cap op count, cap diff output, cap error list sizes; return `truncated: true` flags where needed.
- No heuristics: the tool must not auto-generate ops. It only applies what the agent requested.

Minimum PlanOps to support (start small; expand later):

- `add_component`: append a new component with explicit fields (`name`, `size`, optional `anchors`, optional `attach_to`, optional `contacts`).
- `remove_component`: remove a component by name (reject if it is referenced elsewhere, unless the op also removes/replaces those references).
- `set_attach_to`: set/replace `attach_to` for a component by name.
- `set_anchor`: upsert one anchor on a component by name (or reject if anchors are omitted and this would be required).
- `set_aim_components`: replace the `aim.components` array (bounded).
- `set_attack_muzzle`: set `attack.muzzle.component` / `attack.muzzle.anchor` (bounded).
- `set_reuse_groups`: replace `reuse_groups` (bounded; optional for initial milestone).

Acceptance signal for this step: given a rejected plan attempt, an agent can patch a small mistake (missing component definition, wrong parent name, missing anchor) without re-running `llm_generate_plan_v1`.

### 4) Optional FixIts in `inspect_plan_v1` (bounded; only when forced)

Add an optional `fixits` (or `suggested_ops`) field to `inspect_plan_v1` results.

Rules:

- Only produce FixIts when the patch is logically forced from explicit plan intent. Examples:
  - If `reuse_groups` contains a target name and that name is referenced elsewhere (e.g. `aim.components` or `attach_to.parent`) but the target is missing from `components[]`, a FixIt may suggest “add component stub for target” with a minimal safe shape (and mark any required fields that must still be supplied by the agent).
- If the tool cannot produce a forced repair, return no FixIts and instead return richer error context (missing refs, available names, and which tool(s) can be used next).

Important: FixIts are suggestions only. No silent mutation; the agent must explicitly call `apply_plan_ops_v1` to apply them.

Acceptance signal for this step: common mechanical mistakes (“referenced-but-undefined component name”) produce a concrete suggested PlanOp payload that the agent can apply directly.

### 5) Discoverability: tool registry, agent prompt, and docs (suggestions only)

Update the tool registry in `src/gen3d/agent/tools.rs`:

- Add `apply_plan_ops_v1` with an accurate `args_schema`, `args_example`, and a `one_line_summary` that mentions it mutates the pending rejected plan (and revalidates).

Update agent prompt text (`src/gen3d/ai/agent_prompt.rs`) to mention, as an option:

- When `llm_generate_plan_v1` fails semantically, use `inspect_plan_v1` to get computed errors and constraints.
- If the fix is local and deterministic (rename parent, add missing component stub, add missing anchors), consider `apply_plan_ops_v1` instead of a full replan.

Do not hardcode a required sequence; keep it as guidance.

Add tool docs under `docs/gen3d/`:

- `docs/gen3d/apply_plan_ops_v1.md` describing contract, bounds, dry-run behavior, and at least one example.
- Update `docs/gen3d/inspect_plan_v1.md` to document any new error kinds and the optional FixIts field.

### 6) Tests

Add unit tests that do not require any network/LLM dependency:

- `inspect_plan_v1` reports `missing_component_reference` for:
  - references from `reuse_groups.targets`,
  - references from `aim.components`,
  - references from `attack.muzzle.component`,
  - and from `attach_to.parent`.
- `apply_plan_ops_v1`:
  - applies a small patch to a pending rejected plan attempt,
  - revalidates,
  - accepts the plan when errors are resolved,
  - rejects invalid ops deterministically.

Include a minimal JSON fixture that reproduces the “reuse target referenced but missing component” case.

### 7) Validation and commit hygiene

Run:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Commit with a clear message describing:

- prompt clarification for `reuse_groups`,
- plan inspection diagnostics improvements,
- `apply_plan_ops_v1` addition (and any docs/tests).

## Concrete Steps

From repo root:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expected:

- `cargo test` succeeds.
- The rendered smoke test starts and exits cleanly (no crash) within ~2 seconds.

## Validation and Acceptance

Acceptance is met when:

1) The plan prompt text clearly states that `reuse_groups` does not define components, and plan outputs in practice are less likely to omit reuse targets from `components[]`.

2) When a plan is rejected semantically, `inspect_plan_v1` reports (bounded) missing referenced component names with “referenced_by” context, rather than only `unknown_parent`.

3) The new tool `apply_plan_ops_v1` can repair a pending rejected plan attempt by applying explicit ops and accepting the plan without re-running `llm_generate_plan_v1`.

4) The agent is taught about the tool (registry + prompt + docs) via suggestions only; the agent remains free to replan instead of patching.

## Idempotence and Recovery

All changes are safe to apply repeatedly. If tests or smoke fail:

- Revert the most recent commit and re-run `cargo test` + smoke to confirm the regression source.
- Use `rg` to locate changes in `src/gen3d/ai/prompts.rs`, `src/gen3d/ai/plan_tools.rs`, and the new tool implementation files.

## Artifacts and Notes

`apply_plan_ops_v1` should write audit artifacts under the current pass dir (mirroring `apply_draft_ops_v1`):

- `plan_ops.jsonl` (transaction log of applied/rejected ops),
- `apply_plan_ops_last.json` (last-call summary),
- optional: `plan_before_after.json` (bounded or summarized) if useful for debugging.

## Interfaces and Dependencies

- No new external crates are required.
- Follow tool contract requirements in `docs/agent_skills/tool_authoring_rules.md`:
  - bounded outputs,
  - actionable results and errors,
  - no silent mutation.
