# ExecPlan: Gen3D Strict Structured Outputs + Agent Guardrails (Stop, Regen, No-Progress)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D runs should end promptly once the requested change is complete and the draft is usable. Today, a run can burn many extra LLM calls on repeated “inspection” steps when:

1) the AI backend does not truly enforce Structured Outputs (strict JSON schema) and returns multiple candidate JSON objects, and/or
2) the agent emits both a “done” step and a nearly identical non-done step (causing the engine to ignore the early “done”), and/or
3) the agent spends regeneration budget on `force:true` regenerations even though validate/smoke has no errors, and/or
4) the engine waits too long before applying the no-progress stop condition.

After this change:

- Gen3D will request strict Structured Outputs for all schema-constrained calls (plan / component draft / agent step / review delta). If a backend violates schema enforcement (for example returning multiple JSON objects), Gen3D will warn and attempt a best-effort coercion to a single JSON object; it only fails if it cannot recover.
- When the model returns multiple `gen3d_agent_step_v1` JSON objects where the only meaningful difference is the presence of a trailing `done` action, Gen3D will accept the `done` step (ending the run promptly instead of continuing to “inspect”).
- Gen3D will block `force:true` regenerations unless there are actual validate/smoke errors, preventing wasted regen budget and “thrashing” on already-OK drafts.
- Gen3D will auto-finish runs that are “complete enough” (QA is OK, motion requirements satisfied, no pending work) instead of requesting additional agent steps; and the no-progress guard will stop after 3 no-progress steps by default.

User-visible success is: a small edit like “add a red hat on top” finishes in a handful of passes instead of continuing for dozens of inspection-only passes.

## Progress

- [x] (2026-03-08) Create this ExecPlan and check it in.
- [x] (2026-03-08) Add a config switch to require strict structured outputs for Gen3D schema-constrained calls; warn + recover when an OpenAI-compatible gateway ignores schema enforcement; fail with clear error messaging when the backend does not support schema-constrained outputs at all.
- [x] (2026-03-08) Update `parse_agent_step()` to accept `done` when the only difference between candidates is the presence/absence of `done`.
- [x] (2026-03-08) Track validate/smoke “last OK” state and block `force:true` regen unless validate/smoke has errors.
- [x] (2026-03-08) Implement “auto-finish when complete enough” and reduce default no-progress max steps to 3; update docs/config example accordingly.
- [x] (2026-03-08) Add unit tests for the new parsing/guardrail behavior.
- [x] (2026-03-08) Run `cargo test` and the rendered smoke start.
- [x] (2026-03-08) Commit changes.

## Surprises & Discoveries

- Observation: Some OpenAI-compatible gateways accept `text.format` / `response_format` fields but silently ignore strict schema enforcement, returning multiple message outputs in a single response.
  Evidence: cached Gen3D run `~/.gravimera/cache/gen3d/5bbe3851-e03e-48c3-b81e-010d44f8d731/attempt_0/pass_4/agent_step_responses.json` contains multiple `"type":"message"` outputs.

## Decision Log

- Decision: Add an explicit “require structured outputs” config flag for Gen3D schema-constrained calls.
  Rationale: “Best effort” fallback is helpful for compatibility, but it can hide misconfigured backends and lead to long runs with ambiguous agent outputs. We still prefer strict schema enforcement, but for OpenAI-compatible gateways that accept schema fields yet emit multiple message outputs, we recover by coercing a single JSON object and logging a warning.
  Date/Author: 2026-03-08 / Codex

- Decision: Accept `done` when it is a strict superset of a non-done candidate (same tool calls + trailing `done`).
  Rationale: This fixes the common failure mode where the model emits “done” then repeats the same step without “done”, and the current parser always prefers the non-done step (causing unnecessary extra passes).
  Date/Author: 2026-03-08 / Codex

- Decision: Block `force:true` regen unless validate/smoke has errors (not warnings).
  Rationale: Regeneration is expensive and consumes limited budgets. If the draft is already structurally OK (no validate/smoke errors), forced regen is usually churn and often causes long inspection loops.
  Date/Author: 2026-03-08 / Codex

- Decision: Auto-finish when the run is complete enough, and set default no-progress guard to 3 steps.
  Rationale: If the agent refuses to emit `done`, the engine should still stop quickly once the draft is acceptable and no required work remains.
  Date/Author: 2026-03-08 / Codex

## Outcomes & Retrospective

- Added `gen3d.require_structured_outputs` (default true). Gen3D requests strict schemas for all schema-constrained calls, warns when a backend violates enforcement, and attempts to coerce outputs back to a single JSON object.
- Fixed `parse_agent_step()` to prefer a `done` step when it is a strict superset of the non-done candidate (same tool calls + trailing `done`).
- Blocked `force:true` regen unless validate/smoke indicates errors (prevents churn after QA is clean).
- Tightened the no-progress guard: defaults `no_progress_tries_max = 3` and `inspection_steps_max = 12`, and only auto-finishes when the run is objectively “complete enough”.

## Context and Orientation

Gen3D’s “agent loop” is in `src/gen3d/ai/` and is driven by:

- `src/gen3d/ai/agent_loop/mod.rs`: spawns the next agent step request.
- `src/gen3d/ai/agent_step.rs`: parses agent JSON steps, executes tool calls, and decides when to stop a run.
- `src/gen3d/ai/agent_parsing.rs`: parses agent step JSON; today it prefers the last step that does not include `done`.
- `src/gen3d/ai/agent_tool_dispatch.rs`: executes tool calls like `llm_generate_component_v1` and `llm_generate_components_v1`.
- `src/gen3d/ai/openai.rs` (and `src/gen3d/ai/gemini.rs`, `src/gen3d/ai/claude.rs`): builds HTTP requests to AI providers. Structured outputs (strict JSON schema) are requested when an `expected_schema` is provided.

Terminology used in this plan:

- “Structured Outputs”: asking the AI endpoint to constrain output to a strict JSON schema. In this repo it means:
  - OpenAI `/responses`: `text.format = {"type":"json_schema","schema":...,"strict":true}`.
  - OpenAI `/chat/completions`: `response_format = {"type":"json_schema","json_schema":...,"strict":true}`.
  - Equivalent provider-specific schema-constrained JSON outputs for Gemini and Claude.
- “Schema-constrained calls”: any Gen3D AI call where we already pass `expected_schema` (agent step, plan, component draft, review delta, etc.).
- “No-progress guard”: a safety stop when many steps occur with no changes to the assembled draft state.
- “Force regen”: calling a generation tool with `force:true` / `regen:true` to regenerate an already-generated component, consuming regen budget.

## Plan of Work

1) Add a Gen3D config flag that requires structured outputs.

In `src/config.rs`, introduce a new `AppConfig` field under the existing Gen3D section. Parse it from `[gen3d]` in `config.toml`. Default should be enabled (true), since backwards compatibility is not required.

Thread this flag into the Gen3D AI service calls so that when `expected_schema` is present:

- if the backend rejects structured outputs entirely (unsupported parameters / unknown fields / not supported), the build errors immediately with a clear message that the configured backend is not suitable, and
- if the backend appears to ignore enforcement (for example returning multiple JSON objects), Gen3D logs a warning and attempts to coerce a single JSON object; the build only errors if it cannot recover.

Update `config.example.toml` with the new flag, explaining that it should remain enabled for reliable Gen3D.

2) Fix agent-step selection when `done` is the only difference.

In `src/gen3d/ai/agent_parsing.rs`:

- Keep the existing defense against “simulated tool results” and accidental early termination.
- Add a new rule: if there exists a `done` candidate whose actions are exactly equal to some non-done candidate’s actions plus a trailing `done`, prefer the `done` candidate.
- Add unit tests covering:
  - current behavior when the only done candidate is “done-only” (still prefer tool-call step),
  - new behavior for “identical tool calls + trailing done” (prefer the done step).

3) Gate `force:true` regen on validate/smoke errors.

In `src/gen3d/ai/job.rs`, track the last known validate OK status (parallel to existing `last_smoke_ok`).

In `src/gen3d/ai/agent_tool_dispatch.rs`, in both:

- `llm_generate_component_v1` (single component), and
- `llm_generate_components_v1` (batch),

reject `force:true` regeneration when:

- the target includes any already-generated component, AND
- validate + smoke have been run at least once and both are OK (no errors), OR validate/smoke has not been run yet (force regen requires running QA first).

The tool result should be an error with a clear message so the agent can pivot to running `qa_v1` (or stop).

4) Auto-finish when complete enough, and set default no-progress budgets.

In `src/gen3d/ai/agent_step.rs`, add an “auto-finish” check at the end of a step (before requesting a new step) that stops the run when all of the following are true:

- All planned components are generated.
- No pending regen is queued.
- Validate + smoke have been run on the current assembled state, and both are OK.
- If the draft is movable, motion requirements are satisfied for the current assembled state (either runtime motion candidate is available, or authored move clips exist).
- If appearance review is enabled, the latest renders have been reviewed.

In `src/config.rs`, set default no-progress budgets to `gen3d_no_progress_tries_max = 3` and `gen3d_inspection_steps_max = 12`, and update `config.example.toml` commentary to match.

5) Validation, smoke start, and commit.

Run `cargo test` and the required rendered smoke start command from `AGENTS.md`. Commit all changes with a clear message.

## Concrete Steps

All commands run from the repo root:

1) `cargo fmt`
2) `cargo test`
3) Rendered smoke start (per `AGENTS.md`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance is satisfied when:

- `cargo test` passes and new unit tests cover `parse_agent_step()` done-selection and force-regen gating conditions.
- A Gen3D edit run that produces both a “done” step and an identical non-done step terminates at the done step (no long inspection tail).
- A Gen3D run cannot spend regen budget on `force:true` regen when validate/smoke has no errors.
- A run that becomes “complete enough” finishes promptly (without needing 10+ extra agent steps), and the no-progress guard defaults to 3 steps.

## Idempotence and Recovery

All changes are code-level and safe to repeat. If a strict structured output backend is not available, the new strict mode will error with a message explaining how to configure an appropriate AI endpoint.

## Artifacts and Notes

When testing against a real run, record:

- the `run_id` cache folder under `~/.gravimera/cache/gen3d/`,
- the request JSON artifacts (`*_responses_request.json` / `*_chat_request.json`),
- the agent step raw text (`agent_step_raw.txt`),
- whether the run ends quickly (no large number of “inspection-only” passes).
