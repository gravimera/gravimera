# Prompt Fixes: Gen3D Motion Authoring Convergence + Plan Join-Frame Precision

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D runs can spend most of their wall time waiting on large `/responses` calls, and they can fail to converge when the motion authoring tool does not have the information needed to satisfy motion validation (notably planted ground contacts + stance windows). Additionally, plan generation can trigger expensive schema-repair retries when attachment join-frame constraints are underspecified in prompts.

After completing this plan:

1. `llm_generate_motion_authoring_v1` requests should be smaller and faster because the model is instructed to author fewer edges and fewer keyframes (and to only replace channels it actually touches).
2. Motion authoring should be more likely to pass `smoke_check_v1` motion validation because the prompt includes ground contacts and stance schedules (and derived slip/lift tolerances).
3. `llm_generate_plan_v1` should be less likely to hit join-frame schema repair loops because the prompt makes the “anchor forward/up dot must be positive” constraint explicit.

User-visible verification is via cache artifacts: the new prompt text should appear in `tool_motion_authoring_*_system_text.txt` / `tool_motion_authoring_*_user_text.txt` and `tool_plan_*_user_text.txt` for new runs, and `agent_trace.jsonl` should show fewer tokens and fewer long motion-authoring calls.

## Progress

- [x] (2026-03-05) Create this ExecPlan.
- [x] (2026-03-05) Tighten motion-authoring system prompt to explicitly minimize output size (fewer edges, fewer keyframes, smaller `replace_channels`).
- [x] (2026-03-05) Add ground contacts + stance schedules (and derived tolerances) to motion-authoring user prompt, plus guidance to focus authored motion on contact chains.
- [x] (2026-03-05) Tighten plan prompt join-frame wording to match engine validation (require positive dot for forward/up; explain how to fix when negative).
- [x] (2026-03-05) Update docs index to include this ExecPlan (`docs/execplans/README.md`).
- [x] (2026-03-05) Run rendered smoke test and commit.
- [x] (2026-03-10) Include per-edge joint constraints (kind/axis/limits) in the motion-authoring prompt, add a hinge-axis rule to the motion-authoring system prompt, and guide the agent to re-author motion when QA reports `hinge_off_axis`.

## Surprises & Discoveries

- Observation: A single motion-authoring call can take 6–8 minutes and return ~25k tokens when it authors move+idle clips for every edge (e.g. 27 edges in an octopus).
  Evidence: cache run `~/.gravimera/cache/gen3d/8fa49547-ca9e-410e-9b8d-596e0341b4c5`, `attempt_0/pass_7/gen3d_run.log` shows `tool_motion_authoring_call_2` elapsed ~478s with 31,608 tokens.

- Observation: Plan schema repair loops were triggered by “opposing join frame axes” (dot < 0) even when the prompt only warned about “180° opposed”.
  Evidence: same run, `attempt_0/pass_0/gen3d_run.log` and `attempt_0/pass_2/gen3d_run.log` contain `tool_schema_repair_start ... opposing anchor forward vectors ... dot=-0.707`.

## Decision Log

- Decision: Reduce motion-authoring input by removing the generic “effective user prompt” boilerplate and including only the raw user prompt text.
  Rationale: Motion authoring cares about motion constraints + component graph context; modeling/style guidance is noise that increases tokens and latency.
  Date/Author: 2026-03-05 / Codex

- Decision: Fix convergence by enriching motion-authoring context (contacts/stance), rather than loosening motion validation.
  Rationale: Motion validation is the deterministic guardrail; giving the model the missing inputs is lower-risk than changing validation semantics.
  Date/Author: 2026-03-05 / Codex

## Outcomes & Retrospective

This work narrows the gap between deterministic engine validation and what the LLM sees at authoring time.

- Motion authoring is explicitly instructed to minimize output size (fewer edges, fewer keyframes, smaller `replace_channels`), so large “author everything” outputs should be rarer and faster.
- Motion authoring sees the same “ground contact + stance schedule” facts that motion validation uses, including derived slip/lift tolerances tied to `rig.move_cycle_m`, improving convergence on planted-contact locomotion.
- Motion authoring also sees per-edge joint constraints (including hinge `axis_join` and limits), and is explicitly instructed that hinge rotations must be a pure twist around `axis_join` (reducing `hinge_off_axis` QA failures).
- Plan generation is less likely to trigger join-frame schema repair loops because the prompt states the exact engine guardrail (`dot(...) > 0` for forward and up, in component-local coordinates) and provides a concrete fix strategy.

Remaining gap: cache artifacts like `tool_*_responses_raw.txt` can still be large because they store the streaming event log (SSE deltas). This plan did not change artifact logging.

Rendered smoke run result (required by repo instructions): `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2` exits 0.

## Plan Update Notes

- (2026-03-05) Mark plan complete after implementing prompt changes, updating the ExecPlan index, and running the rendered smoke test.

## Context and Orientation

Gen3D runs an “agent loop” that calls LLM-backed tools (plan, components, motion roles, motion authoring, review delta). The relevant prompt builders are in:

- `src/gen3d/ai/prompts.rs`
  - `build_gen3d_motion_authoring_system_instructions`
  - `build_gen3d_motion_authoring_user_text`
  - `build_gen3d_plan_system_instructions`
  - `build_gen3d_plan_user_text`

Engine-side join-frame validation that triggers plan schema repair lives in:

- `src/gen3d/ai/convert.rs` (join forward/up dot checks; error if dot < 0)

Motion validation that triggers `contact_stance_missing` and `contact_lift` lives in:

- `src/gen3d/ai/motion_validation.rs`

Gen3D cache artifacts are written under:

- `~/.gravimera/cache/gen3d/<run_id>/attempt_<n>/pass_<n>/`

## Plan of Work

First, tighten the motion-authoring system prompt so the model is instructed to keep the output minimal: author only the edges it needs, and use small keyframe counts. This reduces output tokens and latency.

Second, extend the motion-authoring user prompt so the model has the information motion validation uses:

1. A list of ground contacts (component/contact/anchor) including stance windows.
2. Derived lift tolerances for the current `rig_move_cycle_m`.
3. Guidance that planted contacts should remain at near-constant ground height (world Y) during stance.
4. A summary of which components are in the parent chains of ground-contact components, so the model can focus authored motion on those edges instead of animating everything.

Third, tighten the plan prompt join-frame rules to match engine validation: the plan must ensure `dot(parent_anchor.forward, child_anchor.forward) > 0` and `dot(parent_anchor.up, child_anchor.up) > 0` for every attachment (in their component-local coordinates). If negative, the plan must fix the anchor bases (or use an explicit `attach_to.offset` rotation) rather than relying on opposed anchors.

Finally, update docs if needed, run the required rendered smoke test, and commit.

## Concrete Steps

Run these commands from the repository root (`/Users/flow/workspace/github/gravimera`):

  - Rendered smoke run (required by repo instructions):
    - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

To verify prompt improvements in a new Gen3D run:

  - Start a Gen3D build from the UI with a prompt that exercises ground contacts (e.g. “octopus, wriggle move”).
  - Inspect the run cache directory for:
    - `tool_motion_authoring_*_system_text.txt` and `tool_motion_authoring_*_user_text.txt` (should include the new constraints + ground contact section).
    - `tool_plan_*_user_text.txt` (should include explicit dot>0 join-frame constraints).
    - `attempt_*/pass_*/gen3d_run.log` (look for fewer/shorter motion-authoring calls).

## Validation and Acceptance

Accept when all of the following are true:

1. A rendered smoke run starts and exits without crashing.
2. Motion-authoring system prompt includes explicit “minimize output size” rules.
3. Motion-authoring user prompt includes ground contact + stance info and derived tolerances.
4. Plan prompt includes explicit dot>0 join-frame constraints (forward and up) that match engine validation.
5. Changes are committed with a clear message.

## Idempotence and Recovery

These prompt changes are safe to iterate:

- Running Gen3D builds will generate new cache directories; no migration is required.
- If a prompt change causes regressions, revert the specific prompt-builder change and re-run the rendered smoke test.
