# Gen3D: Preserve attachment joints in preserve-mode replans, and reduce inspection-loop thrash with actionable tool hints

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Preserve-mode Gen3D edits (calling `llm_generate_plan_v1` with `constraints.preserve_existing_components=true`) are meant to let a user make small, safe changes to an already-generated prefab (for example: “add a blue hat”), without accidentally changing unrelated parts of the model or its behavior.

Today, preserve-mode replans can unintentionally drop `attach_to.joint` (articulation metadata) from existing components because joint data is not preserved during plan acceptance. In a real run (cache: `~/.gravimera/cache/gen3d/6689cf3e-9c92-47e4-86ac-fb1b18f4e5c0`), this manifested as the tail articulation changing when the user only requested a hat. The same run also burned many “inspection” steps (repeated `get_scene_graph_summary_v1` / `query_component_parts_v1`) and ended via the no-progress guard, because the agent kept re-inspecting instead of taking an actionable deterministic edit path when regen was QA-gated.

After this change:

1) Preserve-mode replans do not change existing attachment joints unless an explicit, allowed mutating tool call changes them. Adding a hat must not alter the tail’s joint behavior.

2) When the agent is stuck in read-only inspection loops, tools and state summaries provide actionable, bounded “next-step payloads” and hints (per `docs/agent_skills/tool_authoring_rules.md`) rather than relying on prompt-level “do X then Y” micromanagement.

User-visible outcome: A user can run a preserve-mode edit like “add a hat” and observe that tail/wing/neck articulation is unchanged (no surprise behavioral drift), and the agent converges without wasting the no-progress inspection budget on repeated read-only queries.

## Progress

- [x] (2026-03-14 04:33Z) Write this ExecPlan based on investigation of run `6689cf3e-9c92-47e4-86ac-fb1b18f4e5c0`.
- [x] (2026-03-14) Preserve existing `attach_to.joint` during preserve-mode plan acceptance (fix root cause).
- [x] (2026-03-14) Improve preserve-mode planning context so the LLM sees joint metadata.
- [x] (2026-03-14) Make “regen-only but QA-gated” review deltas return an actionable tool error (avoid deadlocks).
- [x] (2026-03-14) Make read-only inspection tools return bounded “next-step payloads” (recipes/templates) for common deterministic fixes (recolor, nudge attachment).
- [x] (2026-03-14) Add focused unit tests for joint preservation and QA-gated regen diagnostics.
- [x] (2026-03-14) Update docs under `docs/gen3d/` to match the new semantics and tool result shapes.
- [x] (2026-03-14) Run `cargo test` and the required rendered smoke test.
- [x] (2026-03-14) Commit with a clear message.

## Surprises & Discoveries

- Observation: The preserve-mode replanning prompt snapshot does not include existing `attach_to.joint` fields, so the planner is not reminded to preserve joint metadata.
  Evidence: `~/.gravimera/cache/gen3d/6689cf3e-9c92-47e4-86ac-fb1b18f4e5c0/attempt_0/pass_2/tool_plan_call_1_user_text.txt` lists only parent/anchors/offset/size; no joint data for any component.

- Observation: Plan acceptance claims to “Preserve … motion metadata”, but only preserves `attach_to.animations` (and not `attach_to.joint`) when an attachment interface is unchanged, allowing joint drops on replans.
  Evidence: `src/gen3d/ai/plan_ops.rs` in `apply_plan_acceptance()` copies `new_att.animations = old_att.animations.clone()` under `same_interface`, but does not copy `old_att.joint`.

- Observation: The agent can enter an inspection loop when `llm_review_delta_v1` proposes only a regeneration that is blocked by the QA gate, because the resulting tool outcome is not strongly “actionable” (it returns “ok” with blocked indices), and the agent keeps trying to gather more read-only info.
  Evidence: `~/.gravimera/cache/gen3d/6689cf3e-9c92-47e4-86ac-fb1b18f4e5c0/attempt_0/pass_16/gen3d_run.log` ends with `no_progress_guard_stop tries=1 inspection_steps=12` after repeated `get_scene_graph_summary_v1` / `query_component_parts_v1`.

- Observation: `query_component_parts_v1` previously returned primitive mesh names in a debug format (`UnitCube`) that was not directly usable as `apply_draft_ops_v1` input (which expects canonical strings like `cube` / `unit_cube`).
  Outcome: The tool now includes `primitive.mesh_apply` and bounded `recipes` to make deterministic edits copy/pasteable.

## Decision Log

- Decision: Treat `attach_to.joint` as “motion metadata” that must be preserved across preserve-mode replans when the attachment interface (parent + anchors) is unchanged.
  Rationale: Joint metadata is not part of “add a hat” intent. Preserving it makes preserve-mode safe by default and matches the existing behavior for `attach_to.animations`.
  Date/Author: 2026-03-14 / flow + agent

- Decision: Avoid agent-level “call X after Y” enforcement. Prefer improving tool contracts and tool results to be compiler-like: deterministic, bounded, and actionable, and add soft “no-progress” hints computed from state.
  Rationale: This aligns with `docs/agent_skills/tool_authoring_rules.md` (especially “Prefer tool-contract enforcement over prompt micromanagement” and “QA gates and budgets: make them teachable”) and avoids brittle prompt sequencing rules.
  Date/Author: 2026-03-14 / flow + agent

- Decision: In “regen-only but QA-gated” review-delta situations, return an explicit actionable error (or a structured warning + required next steps) rather than a seemingly-successful result that only lists blocked indices.
  Rationale: When the only requested work is impossible due to a gate, the tool must teach the recovery path (deterministic edits, disable preserve mode, or finish best-effort) to prevent inspection loops.
  Date/Author: 2026-03-14 / flow + agent

## Outcomes & Retrospective

- Implemented (2026-03-14): preserve-mode edits no longer drop joints; QA-gated regen-only review deltas return an actionable error; `query_component_parts_v1` returns bounded copy/pasteable next-step payloads (`recipes`) and canonical primitive mesh strings (`mesh_apply`).

## Context and Orientation

Key concepts (plain language):

- “Preserve mode” means Gen3D is editing an already-generated draft. The user wants small changes while keeping existing geometry and behavior. This is triggered by calling `llm_generate_plan_v1` with `constraints.preserve_existing_components=true`.

- An “attachment joint” (`attach_to.joint`) is articulation metadata stored on a component attachment (hinge/ball/fixed/free) that affects motion/rig behavior. In this repo it is represented as `AiJointJson` and stored in `Gen3dPlannedAttachment.joint`.

- An “inspection loop” means the agent keeps calling read-only tools (scene graph summary, component parts queries, KV reads) without performing a mutating edit, consuming the no-progress inspection budget.

Relevant code locations (paths from repo root):

- `src/gen3d/ai/plan_ops.rs`: accepts/rejects plan updates and merges preserve-mode replans with existing planned components and draft defs.
- `src/gen3d/ai/prompts.rs`: builds preserve-mode plan prompts, including the “Existing component snapshot”.
- `src/gen3d/ai/agent_tool_poll.rs`: applies `llm_review_delta_v1` outputs, enforces QA gates and regen budgets, and decides whether to treat outcomes as errors.
- `src/gen3d/ai/draft_ops.rs` and `src/gen3d/ai/agent_tool_dispatch.rs`: implement `query_component_parts_v1` and attach Info Store refs; this is a good place to add bounded “next-step payloads”.
- `src/gen3d/ai/agent_step.rs` + `src/gen3d/ai/agent_utils.rs`: implements the no-progress guard (tries vs inspection steps) and the state hash.

Tool contract philosophy (must follow):

- `docs/agent_skills/tool_authoring_rules.md` is the source of truth for tool changes. Especially: bounded outputs, actionable results/errors, no silent mutation, and prefer tool-level teachable gates over prompt micromanagement.

## Plan of Work

### 1) Preserve `attach_to.joint` during preserve-mode plan acceptance (root-cause fix)

In `src/gen3d/ai/plan_ops.rs`, within `apply_plan_acceptance()` under the `can_preserve_geometry` branch, extend the existing “motion metadata preservation” merge:

- Today: when `same_interface` (same parent + parent_anchor + child_anchor), the code copies `old_att.animations` onto `new_att.animations`.
- Change: also copy `old_att.joint` onto `new_att.joint` under the same condition.

This ensures that preserve-mode replans that omit joint metadata do not clear it. It also makes behavior consistent with the existing handling of `attach_to.animations`.

Implementation note (important for safety): only preserve for `same_interface`. For new components, or for explicit rewires (when `same_interface` is false), keep whatever the new plan declares so that future allow-rewire workflows can still evolve (and so we avoid silently “guessing” intent).

### 2) Improve preserve-mode planning context to include joint metadata (recommended)

In `src/gen3d/ai/prompts.rs` (`build_gen3d_plan_user_text_preserve_existing_components()`), extend the “Existing component snapshot” lines for components with `attach_to` to include joint metadata if present.

Keep it compact and deterministic:

- If a component’s attachment has `joint=None`, omit the joint section.
- If `joint=Some`, append a short summary like: `joint.kind=hinge axis_join=[...] limits_degrees=[...]` with missing optional fields omitted.

Also add one sentence in the preserve-mode prompt text stating that existing joints are considered part of “motion metadata” and should not be changed in additive preserve mode unless the user explicitly asked for articulation changes.

This is not a hard “call X then Y” rule; it is context that reduces accidental omissions and makes planner output closer to the actual preserved state.

### 3) Make QA-gated “regen-only” review deltas actionable (avoid deadlocks)

In `src/gen3d/ai/agent_tool_poll.rs`, in the `llm_review_delta_v1` application path, tighten the detection of “no actionable actions were applied” when the delta requests only regeneration but:

- every requested regen component is blocked due to the QA gate, and
- there are no non-regen actions, and
- there is no `replan_reason`.

When this happens, return a tool error (or a structured warning field) that is machine-readable and includes explicit recovery options, for example:

- “Regen blocked by QA gate because QA is clean/unknown; prefer deterministic edits via `apply_draft_ops_v1` (recolor primitives / tweak attachment offset), or disable preserve mode and rebuild, or end best-effort if requested work is impossible.”

The important change is to avoid returning a superficially “ok” tool result that provides only blocked indices and relies on the agent to infer that it must switch strategies.

### 4) Make inspection tools return bounded “next-step payloads” (recipes/templates)

Add bounded, copy/pasteable templates to read-only inspection tool results, starting with `query_component_parts_v1`.

In `src/gen3d/ai/draft_ops.rs` (or in `src/gen3d/ai/agent_tool_dispatch.rs` after the Info Store KV ref is added), extend the returned JSON with a small field like `recipes` (name bikesheddable) containing:

- A recolor template: an `apply_draft_ops_v1` payload showing how to set `set_primitive.color_rgba` for a small sample of `part_id_uuid`s (first N primitives, N bounded like 8–16), plus a note that it is a sample and the agent should replicate for all parts if needed.
- An attachment nudge template: an `apply_draft_ops_v1` payload showing `set_attachment_offset` for the component (when applicable), with a placeholder delta.

Constraints:

- Must be deterministic and generic (no “hat placement heuristics”).
- Must be bounded (small samples, explicit `truncated` handling).
- Must not silently mutate state (templates only; actual mutation requires calling a mutation tool).

If the current `apply_draft_ops_v1` ergonomics make “recolor all parts” too verbose for components with many primitives, consider (as a separate, explicitly versioned milestone) adding a new dedicated deterministic mutation tool (or a new `apply_draft_ops_v1` op kind) that recolors all primitive parts in a component in one call, returning an applied-count diff. This should follow contract-first tool authoring rules (registry entry, schema, examples, bounded results).

### 5) Tests and docs

Add focused unit tests:

- In `src/gen3d/ai/plan_ops.rs` tests, construct a minimal preserve-mode scenario where an old planned component has `attach_to.joint=Some(hinge)` and the incoming preserve-mode plan omits the joint (None) with an unchanged interface. Assert that after acceptance the job’s planned components still have the hinge joint.
- In `src/gen3d/ai/agent_tool_poll.rs` (or wherever existing tests live for tool gating), add a unit test for the “regen-only but QA-gated” path to assert the tool result is an error and contains the recovery hint.

Update docs (keep `README.md` clean; put detail in `docs/`):

- Update `docs/gen3d/edit_preserve_existing_components.md` to document that preserve-mode replans preserve existing attachment joints for unchanged interfaces.
- Update the tool docs for any modified tool result shape (for example, `docs/gen3d/query_component_parts_v1.md` if it exists, or add one under `docs/gen3d/`).
- Ensure tool registry summaries (`src/gen3d/agent/tools.rs`) mention any new fields/gates (especially if new tools are added).

## Concrete Steps

From repo root:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

During development, for quick verification of the original bug, you can re-run a preserve-mode edit on a prefab with known joints (tail/neck) and confirm joint metadata in the saved edit bundle remains unchanged after adding a new component (hat).

## Validation and Acceptance

Acceptance is met when all of the following are true:

1) Preserve-mode replan that adds a new component (hat) does not change existing joints.

   Practical check: in the resulting `gen3d_edit_bundle_v1.json`, components like `tail_base` retain their prior `attach_to.joint` values (hinge axis/limits) instead of becoming `null`.

2) `llm_review_delta_v1` “regen-only but QA-gated” outcomes are returned as an actionable tool error (or at minimum an explicit, machine-readable warning + recovery options), and the agent does not get stuck in repeated `get_scene_graph_summary_v1` / `query_component_parts_v1` loops after such an outcome.

3) `query_component_parts_v1` results include a bounded next-step payload (recipes/templates) so an agent can apply deterministic recolor/offset edits without needing multiple extra inspection steps.

4) `cargo test` passes and the rendered smoke test starts and exits cleanly within ~2 seconds.

## Idempotence and Recovery

These changes should be safe to apply incrementally and repeatedly:

- If joint preservation causes unexpected behavior, revert only the `apply_plan_acceptance` merge change and rerun `cargo test` + smoke. This isolates whether the behavioral change is caused by joint handling or by later hint/tool-result work.

- If “regen-only but QA-gated” changes cause regressions in agent behavior, treat them as a contract change: keep the tool result shape stable, add tests, and prefer introducing a new explicit warning field rather than changing existing fields silently.

## Artifacts and Notes

Investigation artifacts (not to be checked in; local cache only):

- Run cache: `~/.gravimera/cache/gen3d/6689cf3e-9c92-47e4-86ac-fb1b18f4e5c0`
- The preserve-mode plan produced there: `.../attempt_0/pass_2/plan_raw.txt`
- The no-progress stop log: `.../attempt_0/pass_16/gen3d_run.log`

## Interfaces and Dependencies

- No new external crates are required.
- Tool changes must follow `docs/agent_skills/tool_authoring_rules.md` (deterministic, versioned contracts, bounded outputs, actionable results/errors, no silent mutation).
