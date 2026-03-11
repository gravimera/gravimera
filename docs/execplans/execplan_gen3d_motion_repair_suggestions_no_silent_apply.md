# Gen3D: Motion repair suggestions (no silent apply)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D motion validation can fail with `hinge_limit_exceeded` when a hinge joint’s authored motion exceeds its declared limits by a small amount (for example, by ~1 degree). Today, the only practical repair path is to ask the LLM to re-author the motion; the engine does not provide a deterministic, tool-driven way to propose and apply minimal constraint- or amplitude-based fixes.

After this change, the Gen3D agent (or a human operator) can call a **read-only** tool to get deterministic **repair suggestions** in the form of concrete `apply_draft_ops_v1` operations (diff-like patches). Nothing is applied automatically: the draft changes only if the agent explicitly calls a mutation tool (typically `apply_draft_ops_v1`) with the chosen ops. This keeps the engine non-heuristic and avoids silent “auto-fix” behavior, while making small constraint mismatches easy to resolve.

## Progress

- [x] (2026-03-12 06:19Z) Write ExecPlan and confirm current code touch points.
- [x] (2026-03-12 06:33Z) Implement `suggest_motion_repairs_v1` tool (read-only suggestions).
- [x] (2026-03-12 06:38Z) Implement deterministic animation rotation scaling draft-op.
- [x] (2026-03-12 06:43Z) Update agent prompt + docs to encourage explicit confirmation before apply.
- [x] (2026-03-12 06:49Z) Add unit tests for scaling + suggestions.
- [x] (2026-03-12 06:54Z) Run smoke test (rendered UI) and commit changes.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (TBD)

## Decision Log

- Decision: Add a new read-only tool `suggest_motion_repairs_v1` that returns candidate `apply_draft_ops_v1` ops for motion-validation errors (starting with `hinge_limit_exceeded`).
  Rationale: The engine should be able to deterministically propose minimal fixes (relax joint limits, or scale motion amplitude) without guessing intent, while keeping mutation explicit and reviewable.
  Date/Author: 2026-03-12 / Codex CLI agent

- Decision: Add a new deterministic `apply_draft_ops_v1` op to scale animation slot rotation deltas (rather than emitting full rewritten keyframes from the suggestion tool).
  Rationale: Returning entire animation clips as patches can be large and error-prone; a scale op is small, generic, and deterministic, and provides the missing “animation amplitude” affordance.
  Date/Author: 2026-03-12 / Codex CLI agent

- Decision: Never auto-apply repairs as part of QA/smoke/validation.
  Rationale: Enforces the “no silent apply without AI confirm” rule: the engine may suggest, but only explicit mutation tools apply changes.
  Date/Author: 2026-03-12 / Codex CLI agent

## Outcomes & Retrospective

- Outcome: Added an explicit “suggest then apply” path for `hinge_limit_exceeded` via `suggest_motion_repairs_v1`, with concrete patches for relaxing joint limits or scaling animation rotation.
- Outcome: Added `apply_draft_ops_v1` op `scale_animation_slot_rotation` to provide a generic animation amplitude affordance without requiring keyframe rewrites from the LLM.
- Lesson: Keeping repair tools read-only and emitting ready-to-apply draft ops preserves “no silent apply” while still enabling deterministic fixes.

## Context and Orientation

Gen3D is a tool-driven agent loop. The agent calls tools such as:

- `qa_v1`: composed `validate_v1` + `smoke_check_v1`, where `smoke_check_v1` includes `motion_validation` results.
- `apply_draft_ops_v1`: deterministic mutations to anchors/attachments/joints and animation slots.

Motion validation currently reports `hinge_limit_exceeded` with evidence fields including:

- `limits_degrees`: the declared hinge limits
- `hinge_angle_degrees`: observed signed hinge angle at the worst sample
- `max_exceed_degrees`: magnitude exceeded
- `at_phase_01`: phase of the worst sample

Relevant implementation files:

- Tool registry: `src/gen3d/agent/tools.rs`
- Tool dispatch: `src/gen3d/ai/agent_tool_dispatch.rs`
- Motion validation (source of `hinge_limit_exceeded`): `src/gen3d/ai/motion_validation.rs`
- Draft mutation ops: `src/gen3d/ai/draft_ops.rs`
- Agent system instructions: `src/gen3d/ai/agent_prompt.rs`
- Tool docs live under: `docs/gen3d/`

## Plan of Work

1) Add a new Gen3D agent tool `suggest_motion_repairs_v1`.

The tool is read-only: it computes the current `motion_validation` report (same as smoke-check) and returns a bounded list of repair suggestions. Each suggestion contains:

- the triggering issue identifier (kind/component/channel),
- a short human-readable explanation,
- a ready-to-apply `apply_draft_ops_v1` patch (list of draft ops),
- basic impact metrics (for example, degrees of limit relaxation, or scale factor).

Initial supported repair: `hinge_limit_exceeded` on hinge joints.

2) Add a deterministic `apply_draft_ops_v1` op to scale animation slot rotation amplitude.

Add a new draft op kind that:

- targets one attachment animation slot by `(child_component, channel)`,
- scales the delta rotation angle for each keyframe (or `spin` radians-per-unit) by a factor in `(0, 1]`,
- does not change translation or scale.

This op is generic (works for any object/clip) and deterministic. It supplies the missing “animation amplitude” control without requiring the LLM to rewrite keyframes.

3) Update agent prompt and docs to use the new tool without silent apply.

Update `src/gen3d/ai/agent_prompt.rs` guidance so that when `qa_v1` reports `hinge_limit_exceeded`, the agent prefers:

- calling `suggest_motion_repairs_v1` to get concrete patch options,
- explicitly deciding whether to apply one via `apply_draft_ops_v1`,
- only falling back to `llm_generate_motion_authoring_v1` when suggestions are unsuitable (for example, off-axis hinge, missing axis, or unacceptable scale factor).

Add `docs/gen3d/suggest_motion_repairs_v1.md` describing args/output and the “no silent mutation” guarantee.

4) Add unit tests.

Add Rust unit tests that:

- validate quaternion angle scaling behavior for keyframes (`scale_animation_slot_rotation` op),
- construct a small synthetic planned-component attachment with a hinge + move loop that exceeds limits and verify:
  - motion validation reports `hinge_limit_exceeded`,
  - `suggest_motion_repairs_v1` returns at least one suggestion,
  - applying the suggested op(s) removes the error (re-run validation in test).

These tests must not call any network-backed LLM tools.

## Concrete Steps

All commands are run from the repository root (`/Users/flow/workspace/github/gravimera`).

1) Run targeted unit tests:

    cargo test -p gravimera gen3d -- --nocapture

2) Run the required smoke test with rendered UI (no headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expected behavior: the game starts, renders for ~2 seconds, then exits without crash.

## Validation and Acceptance

Acceptance criteria:

- A new tool `suggest_motion_repairs_v1` appears in the tool registry and is callable by the Gen3D agent.
- The tool is read-only (does not mutate the draft).
- For a `hinge_limit_exceeded` issue, the tool returns at least:
  - one “relax limits” suggestion (patch uses `set_attachment_joint`), and
  - one “scale animation rotation” suggestion (patch uses the new scale op).
- Applying either suggestion via an explicit mutation tool call (typically `apply_draft_ops_v1`) fixes the corresponding `hinge_limit_exceeded` in `qa_v1` / motion validation, when the fix is mathematically sufficient.
- Unit tests for the new behavior pass.
- The required rendered smoke test runs without crash.

## Idempotence and Recovery

- `suggest_motion_repairs_v1` is safe to call repeatedly; it has no side effects.
- `apply_draft_ops_v1` remains the only way to apply changes; use `atomic=true` and/or `if_assembly_rev` to avoid accidental races.
- If a suggested patch is undesirable, the agent can ignore it and choose to re-author motion via `llm_generate_motion_authoring_v1`.

## Artifacts and Notes

- Tool suggestions should be small and bounded (avoid dumping full keyframe arrays unless explicitly requested by a caller).
- Suggested patches should reference components and channels by explicit strings that already exist in `planned_components`.

## Interfaces and Dependencies

New or updated interfaces to exist at the end:

- In `src/gen3d/agent/tools.rs`:
  - `TOOL_ID_SUGGEST_MOTION_REPAIRS: &str = "suggest_motion_repairs_v1"`
  - tool descriptor entry describing args + output.

- In `src/gen3d/ai/agent_tool_dispatch.rs`:
  - dispatch branch for `suggest_motion_repairs_v1`.

- In `src/gen3d/ai/draft_ops.rs`:
  - new `DraftOpJsonV1` variant for scaling rotation amplitude on an animation slot,
  - implementation that mutates `Gen3dPlannedAttachment.animations` deterministically.
