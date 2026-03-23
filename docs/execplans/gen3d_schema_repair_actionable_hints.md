# Gen3D: Actionable schema-repair hints (error-only, no base-prompt bloat)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D relies on LLM-backed tools that return strict JSON (Plan, PlanOps, DraftOps, ComponentDraft, MotionAuthoring, ReviewDelta). When the model emits malformed JSON or the wrong field names, the engine performs a schema-repair retry (two total attempts: first + 1 repair).

Today, the repair retry prompt is generic: it repeats “return strict JSON” and includes the raw error string, but it does not provide concise, error-specific “how to fix” guidance. This causes avoidable second failures and wastes tokens/latency. A naive fix would be to keep adding more constraints/examples to the *base* prompts, but that increases context size for every successful call and does not scale.

After this change, Gen3D uses a common error-handling style for schema repair across all LLM-backed tools:

- Base prompts do not grow to cover every common mistake.
- On the repair retry only, the engine appends a compact `FIX HINTS` section derived from the actual error (unknown field, missing required field, common alias confusion, etc.).
- Hints are short, actionable, and refer to the canonical schema keys, so the model can “learn from the error” within the second attempt without blowing up normal-case prompt size.

How to see it working:

1. Trigger a tool schema failure (example: PlanOps emits `attach_to` instead of `set_attach_to`).
2. Observe that the repair retry prompt (artifact `*_user_text.txt` under the run cache) contains a `FIX HINTS:` section explaining the precise correction.
3. Observe that the second attempt succeeds without modifying the base prompt text.


## Progress

- [x] (2026-03-23) Drafted this ExecPlan (`docs/execplans/gen3d_schema_repair_actionable_hints.md`).
- [x] (2026-03-23) Implemented a shared “repair hints” builder and wired it into the schema-repair retry prompt.
- [x] (2026-03-23) Covered a small set of high-frequency schema mistakes with targeted hints (PlanOps `attach_to` vs `set_attach_to`, PlanOps `component` vs `name`, DraftOps animation-slot nesting `clip`/`clip_kind`, DraftOps `component` vs `child_component`, component-draft missing `color`).
- [x] (2026-03-23) Added unit tests for the hint builder (string-in → hint-out).
- [x] (2026-03-23) Ran `cargo test` and the rendered smoke test (`cargo run -- --rendered-seconds 2`).


## Surprises & Discoveries

- Observation: Even with “structured outputs” enabled, tool calls can still fail schema parsing (and trigger repair).
  Evidence: A real run cache contains `llm_generate_plan_ops_v1: AI JSON schema error: unknown field \`attach_to\`, expected \`component\` or \`set_attach_to\`` at `~/.gravimera/cache/gen3d/f8546966-1efe-4e90-b583-cf269102c6bc/attempt_0/pass_0/gen3d_run.log`.

- Observation: PlanOps prompts include multiple occurrences of `attach_to` (plan snapshot + plan template), while the PlanOps op for editing attachments requires the key `set_attach_to`.
  Evidence: `src/gen3d/ai/prompts.rs` includes the “Existing component snapshot” lines like `- torso attach_to ...` and the plan template JSON includes `"attach_to":{...}`.

- Observation: The schema-repair retry prompt is constructed in a single place and shared across all LLM-backed tool kinds.
  Evidence: `src/gen3d/ai/agent_tool_poll.rs::schedule_llm_tool_schema_repair(...)` is called from the error branches for GeneratePlan, GeneratePlanOps, GenerateDraftOps, GenerateComponent, GenerateMotion, and ReviewDelta.


## Decision Log

- Decision: Do not expand base prompts to enumerate specific “common mistakes”.
  Rationale: Base prompt growth increases tokens for the success path and does not scale with new tools/schemas.
  Date/Author: 2026-03-23 / user + assistant

- Decision: Improve only the repair retry prompt by appending compact, error-derived `FIX HINTS`.
  Rationale: Hints are paid only on failure; the second attempt is the only place we need extra guidance.
  Date/Author: 2026-03-23 / user + assistant

- Decision: Keep the retry budget at two total attempts (first + 1 repair).
  Rationale: Bounded retries prevent long runs; Gen3D already enforces this via `GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS = 1`.
  Date/Author: 2026-03-23 / user + assistant


## Outcomes & Retrospective

- (2026-03-23) Schema-repair retries now include an error-derived `FIX HINTS:` section, improving second-attempt success without expanding base prompts. Remaining work (future): extend mappings when new high-frequency schema mismatches show up in run caches.


## Context and Orientation

Key concepts:

- “Schema repair” is a deterministic retry: when an LLM-backed tool returns invalid JSON (or violates strict schema validation), the engine resubmits the same tool request once with an appended “REPAIR REQUEST” block that includes the error string.
- The schema-repair retry is implemented in `src/gen3d/ai/agent_tool_poll.rs::schedule_llm_tool_schema_repair(...)`.
- The retry budget is capped by `src/gen3d/ai/job.rs::GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS` (currently 1 repair attempt).

Relevant code locations:

- `src/gen3d/ai/agent_tool_poll.rs`
  - `schedule_llm_tool_schema_repair(...)` builds the repair prompt and spawns the LLM retry.
- `src/gen3d/ai/structured_outputs.rs`
  - Defines the JSON schemas (`gen3d_plan_ops_v1`, `gen3d_draft_ops_v1`, etc.) used for “structured outputs”.
- `src/gen3d/ai/plan_ops.rs`, `src/gen3d/ai/draft_ops.rs`, `src/gen3d/ai/convert.rs`
  - Contain schema parsing and semantic validation that generate the errors fed into repair.


## Plan of Work

Implement a small, shared “repair hints” module that takes:

- the tool kind / expected schema kind, and
- the error string produced by parsing/validation,

and returns a short list of hint lines.

Then, wire it into `schedule_llm_tool_schema_repair(...)` so the retry prompt becomes:

- the existing generic REPAIR REQUEST text, plus
- `FIX HINTS:` followed by 1–3 concise bullets when applicable.

Hints must be:

- deterministic (no heuristics about user intent),
- short (avoid dumping full schemas),
- actionable (tell the model exactly which key to use / where to nest a field),
- and safe (do not include the entire previous invalid output).


## Concrete Steps

1. Add a new module `src/gen3d/ai/repair_hints.rs` containing a pure function like:

    build_schema_repair_hints(expected_schema, err) -> Vec<String>

   and unit tests for common errors.

2. Update `src/gen3d/ai/mod.rs` to export the new module for internal use.

3. Update `src/gen3d/ai/agent_tool_poll.rs::schedule_llm_tool_schema_repair(...)` to append:

    FIX HINTS:
    - ...

   when the hint list is non-empty.

4. Run:

    cargo test

   and the rendered smoke test:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2


## Validation and Acceptance

Acceptance is met when:

- Unit tests cover at least one PlanOps hint and one DraftOps hint (string match on the emitted hints).
- A real schema-repair retry prompt artifact includes `FIX HINTS:` and the fix is specific to the error (for example: `set_attach_to` vs `attach_to`).
- The game starts and renders for 2 seconds without crash using the required smoke command.


## Idempotence and Recovery

- Hint generation must be purely additive and should not affect the success path (only triggers on repair retries).
- If a hint is incorrect or too noisy, remove that specific pattern mapping; the generic repair request still functions.


## Artifacts and Notes

Example target behavior for a PlanOps failure:

    Error: llm_generate_plan_ops_v1: AI JSON schema error: unknown field `attach_to`, expected `component` or `set_attach_to`
    FIX HINTS:
    - For PlanOps `kind="set_attach_to"`, use the key `set_attach_to` (not `attach_to`).
    - `attach_to` appears in full plan components; PlanOps uses `set_attach_to` for patches.


## Interfaces and Dependencies

- No new dependencies (avoid bringing in regex crates for this).
- New internal helper module: `crate::gen3d::ai::repair_hints`.
- `schedule_llm_tool_schema_repair(...)` remains the single choke point for repair prompt construction across all tool kinds.
