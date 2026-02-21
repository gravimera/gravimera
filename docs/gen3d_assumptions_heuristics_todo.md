# Gen3D Assumptions / Heuristics TODOs

This document is a living TODO list of engine-side **assumptions**, **silent defaults**, and **heuristics** in/around Gen3D that can:

- Assemble parts incorrectly (wrong rotation / mirrored results / unexpected “auto-fixes”).
- Violate the Gen3D rule: **no heuristic algorithms** (a user could ask for *any* object; the engine must not guess intent).
- Limit animation expressiveness (a user could ask for *any* animation; the engine should not silently rewrite/disable motion).

Each item below is intended to be resolved one-by-one later. Keep this list updated as issues are fixed or new ones are discovered.

## 1) Heuristic decisions (engine “guesses intent”) — remove or replace with explicit plan/tool inputs

- [x] **Require explicit mirror-vs-rotate alignment during reuse/copy (remove Auto)**
  - Fixed (2026-02-21): plan-level `reuse_groups[].alignment` is **required** (`rotation` | `mirror_mount_x`) and is used deterministically by auto-copy (no scoring-based selection).
  - Missing/unknown alignment is now treated as a schema error (plan must be regenerated), rather than silently guessing.
  - Code: `src/gen3d/ai/schema.rs` (adds `reuse_groups[].alignment`), `src/gen3d/ai/structured_outputs.rs` (requires it in the JSON schema), `src/gen3d/ai/reuse_groups.rs` (passes alignment through), `src/gen3d/ai/copy_component.rs` (removes `Gen3dCopyAlignmentMode::Auto` and heuristic scoring/tie-break).

- [x] **Remove spinner auto-alignment to spin axis**
  - Fixed (2026-02-21): the engine no longer rotates component geometry based on inferred axial symmetry to “help” spinners.
  - Result: spinners can intentionally tumble/off-axis spin; if you want clean spinning, the AI must rotate primitives explicitly (the component prompt now includes a spin-axis hint).
  - Code: `src/gen3d/ai/convert.rs` (removed `maybe_align_axially_symmetric_spinner_to_spin_axis`).

- [ ] **Axis-permutation “fix” based on matching planned size**
  - Problem: If measured AABB size looks like the plan’s size with axes permuted, the engine may apply one of 24 axis-aligned rotations to “match”.
  - Impact: Can rotate a correct component into an incorrect one when plan sizes are wrong/ambiguous; heuristic correction.
  - Code: `src/gen3d/ai/convert.rs` `maybe_fix_component_axis_permutation`.
  - Direction: Replace with explicit authored orientation constraints (plan fields), or treat mismatch as a validation error and request regeneration.

- [ ] **Runtime heuristic flip of wheel/roller spin direction**
  - Problem: Runtime may flip `move_distance` spin sign when the spin axis points toward `-X` in the parent frame (to make “left wheels” spin forward).
  - Impact: Engine changes motion semantics; breaks intentional reverse spins; not generic.
  - Code: `src/object/visuals.rs` `signed_move_distance_for_spin_axis`.
  - Direction: Make spin direction fully data-driven (author the desired sign/axis), or compute from explicit “ground travel direction” metadata instead of a heuristic.

- [ ] **Auto-disable animation channels after repeated motion validation errors**
  - Problem: After repeated motion-validation errors, the engine replaces the failing channel with an identity loop (“disables” it) to keep the model usable.
  - Impact: Silently removes motion; limits “any animation” and can hide the real issue.
  - Code: `src/gen3d/ai/agent_loop.rs` (fallback policy) + `src/gen3d/ai/convert.rs` `disable_attachment_animation_channel_identity_loop`.
  - Direction: Prefer explicit repair actions (LLM delta, user prompt) or mark channel as failed without mutating authored motion; if fallback exists, require an explicit opt-in policy.

- [ ] **Heuristic parsing that changes semantic meaning**
  - Problem: Some fields accept “near miss” strings and are normalized (drivers, attack kind, clip kind, etc.); colors can be guessed from material-ish words.
  - Impact: The engine can interpret ambiguous text differently than intended; “it worked but wrong”.
  - Code: `src/gen3d/ai/parse.rs` (driver/clip/attack normalization, key alias rewrites), heuristic colors in `src/gen3d/ai/parse.rs` (material-name → RGBA).
  - Direction: Restrict to strictly versioned enums in Gen3D outputs; treat unknown values as errors (or surface explicit warnings + structured “normalization applied” artifacts).

## 2) Silent defaults / hidden assumptions — make explicit, validate hard, or error early

- [ ] **Hard-coded coordinate conventions (forward = +Z, up = +Y)**
  - Problem: The entire system assumes these conventions; if an AI uses a different convention (e.g., +X forward), rotations will be “wrong”.
  - Code: `src/gen3d/ai/convert.rs` `plan_rotation_from_forward_up` and all callers.
  - Direction: Make the convention a first-class contract in specs/prompts; consider stricter validation (require non-degenerate forward+up when rotation matters).

- [ ] **Forward/up degeneracy fallbacks can create arbitrary roll**
  - Problem: If `up` is missing/parallel to `forward`, we pick a fallback `up`. That produces an arbitrary roll that might be inconsistent across parts.
  - Code: `src/gen3d/ai/convert.rs` `plan_rotation_from_forward_up`.
  - Direction: Require `up` whenever roll matters, or require a quaternion; treat degenerate bases as errors.

- [ ] **`attach_to.offset` rotation frame defaults to JOIN frame**
  - Problem: If rotation is provided but `rot_frame` is omitted, the engine assumes join-frame vectors/quats (legacy). Authors often think in the parent/component frame.
  - Code: `src/gen3d/ai/schema.rs` (`AiAttachmentOffsetJson.rot_frame`) + `src/gen3d/ai/convert.rs` `attachment_offset_from_ai`.
  - Direction: Require `rot_frame` whenever a rotation is present, or adopt a safer default (and warn loudly when converting).

- [ ] **Missing anchors sometimes silently become identity**
  - Problem: In several places, missing anchors resolve to `Transform::IDENTITY` and assembly proceeds.
  - Impact: Broken joins/axes show up as “weird rotation” rather than a clear error.
  - Code: multiple `anchor_transform*_…unwrap_or(Transform::IDENTITY)` paths (Gen3D plan conversion and runtime visuals).
  - Direction: For Gen3D-generated content, missing anchors should be a hard error (plan/draft validation) rather than a silent fallback.

- [ ] **Join-frame alignment assumptions (forward must match; up opposition is only a warning)**
  - Problem: Plan validation errors on opposing forward but only warns on opposing up. Opposing up can still cause roll flips that look like “incorrect rotation”.
  - Code: `src/gen3d/ai/convert.rs` (attachment join-frame validation).
  - Direction: Decide strictness: either require up alignment too (error), or require explicit `offset` roll when up differs (and document it).

## 3) “Any animation” constraints — where current behavior limits what can be expressed

- [ ] **Channel model is state-based (limited automatic playback)**
  - Problem: Runtime prioritizes a fixed set of channels (`attack_primary`, `move`, `idle`, `ambient`) unless forcibly overridden.
  - Impact: Nonstandard channels may exist but won’t play automatically (limits “any animation” without extra control plumbing).
  - Code: `src/object/visuals.rs` `update_part_animations`.
  - Direction: Make channel selection data-driven (priorities/conditions in plan) or expose explicit per-instance channel control.

- [ ] **Only two clip kinds for attachments: `loop` and `spin`**
  - Problem: No higher-level motion constructs (curves/easing/events/noise constraints); “any animation” must be approximated via many keyframes or a single axis spin.
  - Code: `src/object/registry.rs` `PartAnimationDef` + Gen3D schema/conversion.
  - Direction: If “any animation” is a goal, extend the animation schema with more generic primitives (without engine guesses).

- [ ] **Fixed joints sanitize away rotation**
  - Problem: Declaring a joint `fixed` removes `spin` and forces loop rotation deltas to identity.
  - Impact: Prevents “visual-only” motion on attachments that are logically fixed.
  - Code: `src/gen3d/ai/convert.rs` `sanitize_fixed_joint_attachment_animations`.
  - Direction: Separate “physics constraint” from “visual animation allowed”, or require explicit opt-in for rotation on fixed joints.

- [ ] **Scale sanitization blocks negative/zero scale effects**
  - Problem: Gen3D sanitizes scales to positive minimums and treats primitive scale as a size vector.
  - Impact: Limits mirroring, squash-to-zero, and other stylized effects.
  - Code: `src/gen3d/ai/convert.rs` `sanitize_component_part_transforms` and `attachment_offset_from_ai`.
  - Direction: Introduce explicit support for mirroring/visibility/fade rather than relying on negative/zero scale.

- [ ] **Motion validation assumes specific rig geometry semantics**
  - Problem: Validation like `chain_axis_mismatch` assumes intermediate chain segments are oriented along join +Z between joints.
  - Impact: Unusual but valid rigs/animations can be flagged as “errors” and (today) can trigger channel-disable fallback.
  - Code: `src/gen3d/ai/motion_validation.rs` (e.g. `chain_axis_mismatch`).
  - Direction: Make validation profiles configurable/optional per rig style, and avoid mutating authored motion as a fallback.

## 4) Diagnostics / guardrails to prevent “wrong result but no clue why”

- [ ] **Add structured “heuristic/applied-default” artifacts**
  - Track when any heuristic/default/sanitization was applied (what changed, why, before/after summary) so bugs are debuggable.
  - Likely touchpoints: component conversion pipeline, reuse/copy pipeline, motion fallback pipeline.

- [ ] **Add regression tests for each item above (in `tests/gen3d/` and/or `test/`)**
  - Each fix should come with a minimal fixture that reproduces the failure and asserts the corrected behavior.
