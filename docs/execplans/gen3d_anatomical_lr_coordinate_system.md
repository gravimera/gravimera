# Gen3D: Anatomical Left/Right Semantics (+X = left)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `PLANS.md` (repository root).

## Purpose / Big Picture

Historically, some Gen3D prompts and docs described the coordinate system as `+X` = “right”. When a user asks for a humanoid unit to “hold a sword in its right hand”, they mean **anatomical** right (the unit’s right side). But when a unit is viewed from the front, the unit’s anatomical right appears on the viewer’s left. This caused repeated “left/right swapped” reports where the plan data was internally consistent but the naming and the user’s mental model disagreed.

After this change (implemented 2026-04-14), Gen3D uses an explicit **anatomical** axis convention for authoring and naming:

- `+Z` is the unit’s front (where it faces)
- `+Y` is up
- `+X` is the unit’s anatomical left (so anatomical right is `-X`)

Prompts and docs will explicitly distinguish **anatomical left/right** from **viewer/screen left/right**, and will stop using “viewer-left” as an instruction target. This makes left/right naming stable and matches how people describe a character facing them.

You can see it working by generating a simple humanoid with “right hand holding a sword”: `right_*` limb components resolve to negative X in the assembled rest pose (anatomical right), and the sword attachment points at the `right_*` chain. In a front view, that will naturally appear on the viewer-left side, which is expected.

## Progress

- [x] (2026-04-14) Draft ExecPlan with definitions, scope, and acceptance checks.
- [x] (2026-04-14) Update Gen3D plan/system prompts to use anatomical axes (`+X` = left).
- [x] (2026-04-14) Remove ambiguous “right/left” wording in prompts/tool outputs; prefer axis-based phrasing (`+X/-X`, `join_x_world`) when describing math frames (JOIN frame, component-local basis).
- [x] (2026-04-14) Add a lightweight validation that flags `left_*` parts placed on `-X` (and `right_*` on `+X`) in the assembled rest pose.
- [x] (2026-04-14) Update docs describing coordinate conventions.
- [x] (2026-04-14) Run the rendered smoke start; commit.

## Surprises & Discoveries

- Observation: A reported “right hand sword shows on left arm” case was not an engine-side attachment swap. The saved prefab graph attaches the sword under the component named `right_forearm`, but `right_forearm` is positioned on the `+X` side in the assembled pose, which is anatomically “left” under the user’s convention.
  Evidence: Affected prefab edit bundle shows `right_forearm.pos[0] = +4.550124`, `left_forearm.pos[0] = -4.550124`, and `sword.attach_to.parent = "right_forearm"`.

- Observation: Prior to this change, multiple Gen3D prompt strings embedded the sentence “`+X` is right”, causing the LLM to systematically produce `right_*` components on `+X`.
  Status: Fixed (2026-04-14) by updating Gen3D prompts to define `+X` as anatomical left and by adding a validation gate that errors on `left_*`/`right_*` placement mismatches.

## Decision Log

- Decision: Define Gen3D authoring axes anatomically: `+Z` front, `+Y` up, `+X` anatomical left (therefore anatomical right is `-X`).
  Rationale: Matches user intent for “left/right arm” on a character and removes the recurring confusion between anatomical vs viewer-relative left/right when looking at the unit’s front.
  Date/Author: 2026-04-14 / Codex

- Decision: Rename join-frame “right” vectors to axis-based naming (`join_x_world`) in Gen3D scene-graph summaries shown to the model.
  Rationale: The join frame’s `+X` axis is not “right” under the `+X=left` convention, and the raw JSON appears in prompts (so the key name itself can mislead). Axis-based naming keeps the math-frame contract correct and reduces left/right confusion.
  Date/Author: 2026-04-14 / Codex

- Decision: Avoid using “viewer-left/right” as plan requirements. Only mention viewer/screen mapping as explanatory text and always keep the authoritative requirement in anatomical terms or explicit axis signs.
  Rationale: Viewer/screen left-right depends on the camera and is not stable across contexts; anatomical axes are stable by definition.
  Date/Author: 2026-04-14 / Codex

## Outcomes & Retrospective

- Outcome: Gen3D prompts, tool guidance strings, docs, and validation now consistently treat `+X` as anatomical left (therefore `-X` is anatomical right).
- Outcome: Rendered smoke start verified and changes committed.

## Context and Orientation

Terminology used in this plan:

- **Anatomical left/right:** Left/right from the unit’s perspective, defined relative to the unit’s front direction. This is what users mean when they say “right hand”, “left shoulder”, etc.
- **Viewer/screen left/right:** Left/right on the screen. In a front view, anatomical right typically appears on viewer-left. This is a property of the camera view, not of the unit.
- **Front view:** A view where the camera is in front of the unit and the unit faces the camera.

Key files/modules:

- `src/gen3d/ai/prompts.rs`: Gen3D LLM system prompts, including the coordinate system statement used for planning.
- `docs/gamedesign/34_realm_prefabs_v1.md`: Prefab format spec including a “Coordinate System” section. (This plan expects us to update docs so they don’t contradict Gen3D’s authoring convention.)
- `src/gen3d/ai/orchestration.rs` (`build_gen3d_validate_results`): A convenient place to add validation warnings/errors about draft/plan mismatches.

Current state summary:

- Gen3D planning prompts define `+X` as anatomical left (therefore anatomical right is `-X`).
- Validation errors when a component name implies left/right (e.g. `left_*`, `*_right`) but its assembled `pos.x` has the wrong sign.

## Plan of Work

Make the change as a prompt/docs/validation update only. Avoid engine-wide coordinate changes or “camera-dependent” heuristics.

1. Update Gen3D planning prompt coordinate system wording.

   In `src/gen3d/ai/prompts.rs` inside `build_gen3d_plan_system_instructions()`, replace the current “Coordinate system” bullets with an anatomical definition:

   - State the axes explicitly: `+Z` front, `+Y` up, `+X` anatomical left.
   - Add an explicit warning that viewer-left/right is not the same as anatomical left/right, and that front-view appearance is expected to be mirrored.

   Keep the rest of the plan schema unchanged. The goal is to change how the LLM reasons about left/right, not how the engine stores numbers.

2. Remove ambiguous left/right words from any math-frame instructions in Gen3D prompts.

   Search `src/gen3d/ai/prompts.rs` for phrases like `+X is right`, `join_right_world`, or “right axis”, and rewrite them to be axis-based (use `+X` / `-X`, or `join_x_world`) so the text remains correct under the anatomical convention.

   Note: the scene-graph summary JSON is embedded directly into some prompts (for DraftOps / review delta), so “prompt clarity” includes the field names used in that JSON.

3. Add a validation gate for naming vs placement.

   Add a check that catches the specific class of regressions that caused this bug report:

   - If a planned component name starts with `left_`, then its assembled `pos.x` must be `> 0`.
   - If a planned component name starts with `right_`, then its assembled `pos.x` must be `< 0`.

   The exact source of “assembled pos” should be the same one used to populate the Gen3D edit bundle / preview overlay (do not invent a second placement model). When this rule is violated, emit a `severity=error` issue in validation so the pipeline cannot silently ship a swapped model.

4. Update docs (minimal).

   Add a short Gen3D-facing note (or a new small doc) that defines the anatomical coordinate convention and the anatomy vs viewer-left/right distinction. Keep it concise and do not attempt to rewrite unrelated coordinate discussions.

5. Validate and commit.

   Run the required rendered smoke start and commit with a message that mentions anatomical left/right semantics.

## Concrete Steps

All commands run from the repo root:

    cargo test

Smoke start (rendered; do NOT use headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Manual acceptance checks (user-visible):

- Generate a simple humanoid unit with a sword “held in its right hand”.
- In the Gen3D preview front view, the sword may appear on the viewer-left (this is expected); the internal naming must still be `right_*`.
- In the “explode/components” overlay, the sword attachment parent must be a `right_*` component (example: `right_forearm`).

Data acceptance checks (mechanical):

- The assembled `right_*` chain resolves to negative X positions and the `left_*` chain resolves to positive X positions.
- If any `left_*` component resolves to `x < 0` (or `right_*` to `x > 0`), validation reports a `severity=error` issue.

## Idempotence and Recovery

- Prompt and doc edits are safe to re-run and re-apply.
- If the validation is too strict for a non-humanoid object, the fix should be to stop using `left_*`/`right_*` names in that plan, not to weaken the gate based on heuristics.

## Artifacts and Notes

Example `jq` snippets for inspecting an edit bundle (paths vary per prefab id):

    jq -r '.planned_components[] | select(.name=="left_forearm")  | .pos[0]'  gen3d_edit_bundle_v1.json
    jq -r '.planned_components[] | select(.name=="right_forearm") | .pos[0]'  gen3d_edit_bundle_v1.json
    jq -r '.planned_components[] | select(.name=="sword") | .attach_to.parent' gen3d_edit_bundle_v1.json

## Interfaces and Dependencies

- No external dependencies required.
- Implementation should be limited to Gen3D prompt text, Gen3D validation, and small docs updates.
