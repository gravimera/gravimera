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

- [x] **Remove axis-permutation auto-fix (treat as error → regenerate)**
  - Fixed (2026-02-21): removed the “try 24 rotations to match planned size” behavior.
  - Result: if the component’s measured local AABB looks like a permuted version of `target_size`, conversion fails with an explicit error so the AI regenerates with correct local axes (no silent rotation).
  - Code: `src/gen3d/ai/convert.rs` (axis-permutation detection + hard error).

- [x] **Remove runtime wheel/roller spin sign flip (make spin direction data-driven)**
  - Fixed (2026-02-22): the runtime no longer flips `move_distance` spin sign based on whether the spin axis points toward `-X` in the parent frame.
  - Result: spin direction is now fully authored by data (`axis` direction and/or `radians_per_unit` sign). For mirrored wheels/rollers you may need to flip the axis (or the sign of `radians_per_unit`) on one side; the engine will not guess.
  - Code: `src/object/visuals.rs` (removed `signed_move_distance_for_spin_axis`; `move_distance` spin uses `LocomotionClock.signed_distance_m` directly).

- [x] **Stop auto-disabling animation channels after motion validation errors**
  - Fixed (2026-02-22): removed the Gen3D smoke-check fallback that silently replaced failing channels with an identity loop after repeated motion validation errors.
  - Result: motion validation failures are now surfaced as errors and must be repaired (plan/regen/authoring). The engine does not mutate authored motion as a “make it usable” guess.
  - Code: `src/gen3d/ai/agent_loop.rs` (removed fallback policy), `src/gen3d/ai/convert.rs` (removed `disable_attachment_animation_channel_identity_loop` helper).

- [x] **Make Gen3D JSON parsing strict (remove semantic-normalization heuristics)**
  - Fixed (2026-02-22): removed “near miss” normalization for enums/fields (attack kinds, animation drivers/clips, key alias rewrites) and removed named/material color guessing.
  - Result: Gen3D outputs must follow the exact schema (explicit `version`, canonical enums, explicit `projectile.color` RGBA array). Nonconforming outputs fail fast and are handled via regeneration / schema repair rather than engine-side guessing.
  - Code: `src/gen3d/ai/parse.rs` (now strict; no normalization), `src/gen3d/ai/schema.rs` (stricter types; fewer aliases), `src/gen3d/ai/structured_outputs.rs` (projectile color requires RGBA), `src/gen3d/ai/convert.rs` (validates projectile color range).

## 2) Silent defaults / hidden assumptions — make explicit, validate hard, or error early

- [x] **Hard-coded coordinate conventions (forward = +Z, up = +Y)**
  - Fixed (2026-02-22): documented the coordinate system contract (+X right, +Y up, +Z forward) and tightened Gen3D conversion to require explicit anchor frames.
  - Code/docs: `docs/gamedesign/34_realm_prefabs_v1.md` (coordinate system), `src/gen3d/ai/prompts.rs` (explicit conventions), `src/gen3d/ai/schema.rs` + `src/gen3d/ai/structured_outputs.rs` (anchors require `forward`+`up`).

- [x] **Forward/up degeneracy fallbacks can create arbitrary roll**
  - Fixed (2026-02-22): added strict basis handling and removed “pick a fallback up” behavior from Gen3D conversion paths.
  - Result: if a part/offset basis is partial or degenerate (missing vectors or `up` nearly parallel to `forward`), conversion fails with an explicit error so the AI regenerates with a valid basis.
  - Code: `src/gen3d/ai/convert.rs` (`plan_rotation_from_forward_up_strict`, strict validation in `anchors_from_ai`, `quat_from_forward_up_or_identity`, `attachment_offset_from_ai`).

- [x] **`attach_to.offset` rotation frame defaults to JOIN frame**
  - Fixed (2026-02-22): if `offset.forward`/`offset.up` or `offset.rot_quat_xyzw` are present and `offset.rot_frame` is missing, conversion fails with an explicit error so the AI regenerates (no silent join-frame default).
  - Result: rotation frames are always explicit (`join` vs `parent`) and deterministic; no more “it rotated wrong because the engine assumed join-frame”.
  - Code/docs: `src/gen3d/ai/convert.rs` (`attachment_offset_from_ai`), `src/gen3d/ai/structured_outputs.rs` (requires `rot_frame`), `src/gen3d/ai/prompts.rs` + `gen_3d.md` (updated contract text).

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
