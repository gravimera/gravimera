# Gen3D: Diff-first replanning via PlanOps (`llm_generate_plan_ops_v1`)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Today, even with preserve-mode “template-first” gating, replanning is still “full-plan generation”: `llm_generate_plan_v1` must emit the entire plan JSON, including many fields that are unchanged. This makes small edits expensive and brittle: the model can inadvertently restate things incorrectly, which then triggers semantic plan errors and replan loops.

After this change, preserve-mode replanning gains a diff-first option that is smaller, faster, and easier for the model to do correctly:

- A new tool `llm_generate_plan_ops_v1` asks the model to output only a bounded list of explicit **PlanOps** (a patch), instead of a full plan JSON.
- The engine applies these ops deterministically to a full-fidelity “current plan” snapshot, re-validates, and either accepts the patched plan or returns actionable diagnostics (and captures a pending attempt for `inspect_plan_v1` / `apply_plan_ops_v1`).
- The tool supports an explicit “scope” so the model can operate on a small subset of the plan without the engine guessing intent (no heuristics), enabling future “generate a subset” workflows.

User-visible outcome: preserve-mode edit requests like “add a hat” converge with fewer tokens and fewer retries, while staying consistent with this repo’s toolchain philosophy: deterministic validation, actionable errors, and no silent mutation.

## Progress

- [x] (2026-03-15) Create this ExecPlan capturing the diff-first approach, risks, and concrete implementation steps.
- [x] (2026-03-16) Define the `llm_generate_plan_ops_v1` tool contract (args/result/errors) and align prompt/tool registry/docs.
- [x] (2026-03-16) Implement `llm_generate_plan_ops_v1` end-to-end (dispatch → structured outputs schema → poll/apply → artifacts).
- [x] (2026-03-16) Add offline regression tests for scope enforcement, preserve-policy enforcement, and “no data loss” vs. lean templates.
- [x] (2026-03-16) Update `docs/gen3d/` to document the new tool and recommended flows.
- [x] (2026-03-16) Run `cargo test`.
- [ ] Run the required rendered smoke test, then commit with a clear message.

## Surprises & Discoveries

- Observation: A “lean” preserve-mode plan template can intentionally omit text-heavy fields (`assembly_notes`, `components[].modeling_notes`, `components[].contacts`) to fit a byte budget.
  Evidence: `get_plan_template_v1` supports `mode="auto"|"lean"` and can return `truncated=true` with `omitted_fields[]` (see `docs/gen3d/get_plan_template_v1.md`).

- Observation: Using a trimmed template as the *base* plan for patch application would silently erase omitted fields when accepting the patched plan.
  Evidence: `build_preserve_mode_plan_template_json_v8(...)` (in `src/gen3d/ai/plan_tools.rs`) is capable of producing a full-fidelity plan snapshot from engine state; trimming is only a transport/prompt concern, not a correctness concern.

- Observation: `apply_plan_ops_v1` already enforces preserve-mode edit policy constraints deterministically and produces actionable diagnostics, but it is scoped to “pending rejected plan attempts”.
  Evidence: `src/gen3d/ai/plan_ops.rs` `preserve_error_for_plan_apply(...)` and the tool doc `docs/gen3d/apply_plan_ops_v1.md`.

- Observation: `AiPlanJsonV1` (the internal typed plan JSON struct) is `Deserialize`-only, so writing plan snapshots as JSON artifacts is easiest when using the existing “full-fidelity plan snapshot” `serde_json::Value` produced by `build_preserve_mode_plan_template_json_v8(...)`.
  Evidence: `cargo test` initially failed with a `serde::Serialize` bound error when attempting `serde_json::to_value(&AiPlanJsonV1)`.

## Regression Risks (and mitigations)

1) **Silent data loss when templates are truncated**: if the new tool patches a plan built from a lean/truncated template, notes/contacts can be lost.
   Mitigation: always apply ops to a full-fidelity base plan reconstructed from current engine state (not the trimmed prompt template). Treat `plan_template_kv` strictly as prompt context.

2) **Accidental “wide” edits**: if the model is given too much plan context, it may generate ops that touch unrelated components.
   Mitigation: support an explicit `scope_components[]` allow-list and reject ops that touch outside the scope (plus any newly-added component names). This is deterministic and does not require intent inference.

3) **Prompt ↔ tool contract mismatch**: a new tool introduces a new JSON shape and new guardrails; if prompt/registry/docs disagree, the agent will thrash.
   Mitigation: follow `docs/agent_skills/prompt_tool_contract_review.md`; add unit tests for parsing and for contract gates; ensure `one_line_summary` mentions all “must-know” constraints.

## Decision Log

- Decision: Add a new diff-first tool (`llm_generate_plan_ops_v1`) rather than changing `llm_generate_plan_v1` to sometimes return ops.
  Rationale: Tool ids are contracts. A dedicated tool keeps behavior discoverable and avoids “mode switches” that confuse the agent/tool registry and complicate structured output schemas.
  Date/Author: 2026-03-15 / GPT-5.2

- Decision: Require explicit scope input for “subset plan” behavior; do not infer scope from the user prompt.
  Rationale: This repo’s Gen3D algorithms must be generic (no heuristics). “What subset matters” is intent; intent must come from explicit args, not engine guesses.
  Date/Author: 2026-03-15 / GPT-5.2

- Decision: Apply ops to a full-fidelity base plan snapshot derived from engine state; use `plan_template_kv` only as prompt context.
  Rationale: Prevents silent data loss when the prompt template is truncated, while still enabling bounded prompt injection and template-first correctness.
  Date/Author: 2026-03-15 / GPT-5.2

- Decision: In v1, require preserve mode for `llm_generate_plan_ops_v1` (`constraints.preserve_existing_components=true`) and require an existing accepted plan.
  Rationale: This tool is intended for seeded edit sessions; allowing non-preserve acceptance is a footgun (it can overwrite geometry) and does not match the “diff-first preserve replanning” purpose.
  Date/Author: 2026-03-16 / GPT-5.2

- Decision: Scope enforcement treats any reference to an existing component name inside ops (including `attach_to.parent`, `aim.components`, `attack.muzzle.component`, `reuse_groups` references) as “touching” that component for allow-list checks.
  Rationale: This is deterministic and generic (no intent inference), and makes the allow-list meaning unambiguous for agents: if you mention it, include it in scope.
  Date/Author: 2026-03-16 / GPT-5.2

## Outcomes & Retrospective

- (pending) This section will be updated as milestones land.

## Context and Orientation

This ExecPlan is about preserve-mode replanning and plan patching. Terms and the current system:

- “Plan”: the structured component topology the agent author produces (component names, sizes, anchors, attachments, reuse groups, and plan-level combat/mobility fields). In code, the AI JSON plan schema is `AiPlanJsonV1` and the in-memory accepted plan is `Vec<Gen3dPlannedComponent>`.
- “Preserve mode”: seeded edit/fork sessions where the engine preserves existing generated geometry/anchors and validates that planning changes obey a preserve edit policy. It is controlled by `constraints.preserve_existing_components=true` on `llm_generate_plan_v1`.
- “Plan template”: a deterministic snapshot of the current plan encoded as plan JSON v8, stored in the Info Store (KV) and referenced as `plan_template_kv`. It is produced by `get_plan_template_v1` and is now required for preserve-mode replans with an existing plan.
- “PlanOps”: explicit, deterministic patch operations over plan JSON (add component, set attachment, set anchor, etc). These are implemented by `apply_plan_ops_v1` in `src/gen3d/ai/plan_ops.rs`.
- “Pending rejected plan attempt”: when an LLM-produced plan parses but fails semantic validation, the engine stores the failed plan in `job.pending_plan_attempt` so `inspect_plan_v1` and `apply_plan_ops_v1` can operate on it.

Key files (paths from repo root):

- `src/gen3d/agent/tools.rs`: tool registry descriptors shown to the agent (schemas/examples/one-line summaries).
- `src/gen3d/ai/agent_tool_dispatch.rs`: tool dispatch; starts async LLM calls for `llm_generate_plan_v1`.
- `src/gen3d/ai/agent_tool_poll.rs`: receives LLM responses, parses structured outputs, converts plans, and mutates job state.
- `src/gen3d/ai/structured_outputs.rs`: JSON schema definitions used for model structured outputs.
- `src/gen3d/ai/prompts.rs`: plan prompt builders; preserve-mode prompt can inject a template JSON (compact).
- `src/gen3d/ai/plan_tools.rs`: deterministic plan template builder (`build_preserve_mode_plan_template_json_v8`).
- `src/gen3d/ai/plan_ops.rs`: deterministic PlanOps application (`apply_plan_ops_v1`) + preserve-policy diff validation.
- `docs/gen3d/get_plan_template_v1.md`: describes template output (`truncated`, `omitted_fields`, byte budgets).
- `docs/gen3d/apply_plan_ops_v1.md`: describes deterministic patching for rejected plan attempts.
- `docs/execplans/execplan_make_preserve-mode_replanning_template-first.md`: describes why preserve-mode replans require templates and how template size/storage risks are mitigated.

## Plan of Work

### Milestone 1 — Define the new tool contract (`llm_generate_plan_ops_v1`)

Add a new agent-facing tool with a contract optimized for “small preserve-mode edits”.

Tool intent:

- The model outputs only `{ ops: PlanOp[] }` (bounded), not a full plan.
- The engine applies ops deterministically to the current plan snapshot and validates the result under the same preserve-mode constraints/policies that `llm_generate_plan_v1` uses.
- The tool returns a diff-like summary and actionable errors; it must not require the agent to re-fetch large state to proceed.

Proposed args (v1) (exact keys must match parsing structs with `deny_unknown_fields`):

- `version?: 1`
- `prompt?: string` (defaults to the current user prompt)
- `constraints?: { preserve_existing_components?: bool, preserve_edit_policy?: "additive"|"allow_offsets"|"allow_rewire", rewire_components?: string[] }`
- `plan_template_kv?: { namespace: string, key: string, selector?: { kind:"latest"|"rev", rev?: number } }`
  - Required when `constraints.preserve_existing_components=true` and an existing plan is present (same gate as `llm_generate_plan_v1`).
- `scope_components?: string[]`
  - Optional allow-list: the engine rejects ops that touch existing components not in the scope. Newly-added component names are allowed.
- `max_ops?: number` (clamped; default 32, max 64)

Proposed result shape (v1):

- `ok: bool`
- `accepted: bool` (true iff applied and accepted)
- `ops: PlanOp[]` (the generated ops, bounded)
- `apply_summary`: counts + touched component names (bounded)
- `new_plan_summary`: bounded plan summary (root, component count, reuse groups count)
- `error?: string` (actionable; includes next-step tool ids/args)
- `new_errors?: ...` (optional bounded semantic errors/fixits from `inspect_plan_v1` style analysis when not accepted)

Error behavior requirements:

- If preserve mode requires a template and it is missing: return an immediate actionable error telling the agent to call `get_plan_template_v1` and retry with `plan_template_kv`.
- If `plan_template_kv` cannot be loaded: return an actionable error mentioning `get_plan_template_v1` (and `mode="auto"/"lean"` if the stored record is too large).
- If ops exceed scope: reject deterministically and mention how to broaden scope or fall back to `llm_generate_plan_v1` when a wide edit is intended.

### Milestone 2 — Structured outputs schema + prompt builder

Add a new structured output schema kind and a prompt that makes “diff-first” behavior the default.

1) In `src/gen3d/ai/structured_outputs.rs`:

- Add `Gen3dAiJsonSchemaKind::PlanOpsV1`.
- Define JSON schema that requires a single object with:
  - `version: 1`
  - `ops: PlanOp[]` (bounded by a max-items cap enforced post-parse; the schema can also set `maxItems`).

2) In `src/gen3d/ai/prompts.rs`:

- Add `build_gen3d_plan_ops_system_instructions()` and `build_gen3d_plan_ops_user_text(...)`.
- Include:
  - a compact injected plan template JSON (from `plan_template_kv`) as “read-only context”,
  - explicit instruction: “Output only ops; do not restate the whole plan; prefer minimal ops; do not touch components outside scope”.
- Keep it generic and contract-oriented; no object-specific heuristics.

### Milestone 3 — Tool dispatch (`agent_tool_dispatch.rs`)

In `src/gen3d/agent/tools.rs`:

- Add `TOOL_ID_LLM_GENERATE_PLAN_OPS` constant and a tool descriptor:
  - `one_line_summary` must mention: “LLM+mutates via PlanOps patch; preserve-mode requires plan_template_kv; scope enforcement; writes artifacts”.
  - `args_schema` and `args_example` must be valid.

In `src/gen3d/ai/agent_tool_dispatch.rs`:

- Implement dispatch for `llm_generate_plan_ops_v1`:
  - Parse args (`deny_unknown_fields`).
  - Enforce the preserve-mode template requirement gate (same rule as `llm_generate_plan_v1`).
  - Load `plan_template_kv` value (bounded) to inject into the plan-ops prompt.
  - Spawn an async model call using the new structured output schema kind.

### Milestone 4 — Poll/apply logic (`agent_tool_poll.rs` + `plan_ops.rs`)

Implement deterministic application of the generated ops without silent data loss.

1) Parse model output into a small struct:

    { "version": 1, "ops": [ ... ] }

2) Build a full-fidelity base plan snapshot:

- Use `build_preserve_mode_plan_template_json_v8(...)` from `src/gen3d/ai/plan_tools.rs` to serialize the current accepted plan (do not use the trimmed prompt template as the base plan).
- Parse it into `AiPlanJsonV1` for patch application.

3) Apply ops:

- Reuse the PlanOps machinery from `src/gen3d/ai/plan_ops.rs` (ideally refactor shared “apply-to-plan + validate + accept” logic into a helper that both `apply_plan_ops_v1` and `llm_generate_plan_ops_v1` can call).
- Enforce:
  - scope restrictions (`scope_components[]`) as deterministic checks over the op set,
  - preserve-mode requirements (keep all existing names, keep same root),
  - preserve edit policy diff validation (same as `apply_plan_ops_v1`).

4) Accept or reject:

- If valid: accept plan, update `job.planned_components`, bump `job.assembly_rev`, write an assembly snapshot, and return `accepted=true`.
- If invalid: do not mutate the accepted plan. Capture a `job.pending_plan_attempt` containing the patched plan + preserve constraints + error string so `inspect_plan_v1` and `apply_plan_ops_v1` work as follow-ups. Return bounded diagnostics in the tool result.

5) Artifacts:

- Under the current Gen3D `pass/` dir, write:
  - `plan_ops_generated.json` (the raw generated ops payload),
  - `plan_ops_apply_last.json` (summary: accepted/failed + diff counts + errors sample).
  - If useful for debugging, also write `plan_ops_plan_before.json` / `plan_ops_plan_after.json` (these can be large but are local artifacts, not tool outputs).

### Milestone 5 — Agent prompt + docs alignment

Update prompt and docs so the agent can discover and correctly use the new tool.

1) In `src/gen3d/ai/agent_prompt.rs`:

- Mention `llm_generate_plan_ops_v1` as an option in preserve mode for small edits:
  - “If your edit is local (add a component, adjust a single attachment), prefer `llm_generate_plan_ops_v1`.”
  - Keep it as guidance, not a heuristic rule (“local” should not gate behavior).

2) Add `docs/gen3d/llm_generate_plan_ops_v1.md`:

- Describe args/result, scope behavior, and how it relates to:
  - `get_plan_template_v1` (required context in preserve mode),
  - `inspect_plan_v1` (follow-up on failure),
  - `apply_plan_ops_v1` (explicit patching of pending attempts).

3) Update preserve-mode docs:

- `docs/gen3d/edit_preserve_existing_components.md` should mention the diff-first tool as an optional faster path for small edits, while keeping the existing template-first `llm_generate_plan_v1` flow as the general path.

Perform the prompt ↔ tool contract checklist from `docs/agent_skills/prompt_tool_contract_review.md`.

### Milestone 6 — Offline regression tests (no network / no OpenAI calls)

Add tests that validate the deterministic engine-side behavior:

- Scope enforcement:
  - Given a base plan with components `["torso","head"]`, and `scope_components=["head"]`, an op `set_attach_to` touching `torso` is rejected with an actionable error.
- Preserve policy enforcement:
  - Patch attempts that rewire disallowed components are rejected with the same style of preserve-policy error as `apply_plan_ops_v1`.
- No data loss:
  - Given a base plan where `assembly_notes` and `components[].contacts/modeling_notes` are non-empty, applying an unrelated op (e.g. `add_component`) must preserve those fields in the accepted plan snapshot.
  - This test should explicitly cover the case where the prompt template was “lean” (simulated by providing a trimmed template context) while the base plan for apply was full-fidelity.

If fixture files are needed, store them under the repo `test/` folder (per repo instruction). Prefer in-code constructed fixtures for unit tests where possible.

### Milestone 7 — Validation + commit

From repo root:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Commit with a clear message, for example:

    gen3d: diff-first preserve replanning via plan ops

## Concrete Steps

From repo root:

1) Implement tool + schema + prompt changes.
2) Run:

       cargo test -q

3) Run required smoke test (UI, not headless):

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4) Commit:

       git status
       git commit -am "gen3d: diff-first replanning via plan ops"

## Validation and Acceptance

Acceptance is met when:

1) In a seeded edit session (existing plan + preserve mode), the agent can choose `llm_generate_plan_ops_v1` and the engine applies a patch without requiring the model to re-emit the full plan JSON.

2) The tool enforces the preserve-mode template requirement (actionable error when missing), and it does not silently lose fields when templates are truncated (`truncated=true`).

3) Scope enforcement is deterministic: when `scope_components` is provided, ops that touch out-of-scope existing components are rejected with an actionable error that teaches how to proceed.

4) When the patch fails semantically, the engine captures a pending plan attempt so `inspect_plan_v1` and `apply_plan_ops_v1` can be used as follow-ups.

5) Tests pass (`cargo test`) and the rendered smoke test starts and exits cleanly.

## Idempotence and Recovery

- Tool addition is additive; it does not change existing `llm_generate_plan_v1` behavior.
- If the tool causes unexpected issues, the rollback path is a revert of the introducing commit; no data migrations are required.

## Artifacts and Notes

Keep tool outputs bounded. Prefer returning:

- counts and small samples,
- touched component name lists (bounded),
- and a short “next step” hint using concrete tool ids (`inspect_plan_v1`, `apply_plan_ops_v1`, `get_plan_template_v1`, `llm_generate_plan_v1`).

Write large debugging artifacts only under `pass/` (disk), not in tool JSON results.

## Interfaces and Dependencies

- No new external crates are required.
- Follow tool contract rules in `docs/agent_skills/tool_authoring_rules.md`:
  - versioned tool id (`*_v1`),
  - bounded results (`max_ops`, bounded diffs/summaries),
  - actionable errors with deterministic next steps,
  - no heuristics: scope is explicit and enforced; no “auto pick components to edit”.

---

Plan update note (2026-03-16): Mark milestones complete through `cargo test`, recorded implementation discoveries/decisions (preserve-mode-only gate, scope touch semantics, `AiPlanJsonV1` serialization constraint), and left the rendered smoke test + commit as the remaining work.
