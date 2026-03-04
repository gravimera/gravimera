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
  - Code: `src/gen3d/ai/agent_tool_dispatch.rs` (removed fallback policy), `src/gen3d/ai/convert.rs` (removed `disable_attachment_animation_channel_identity_loop` helper).

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

- [x] **Missing anchors sometimes silently become identity**
  - Fixed (2026-02-23): Gen3D now treats missing referenced anchors as a hard error instead of silently using `Transform::IDENTITY`.
  - Key fixes:
    - `resolve_planned_component_transforms` errors if an attachment references a missing `parent_anchor` / `child_anchor` (no silent identity).
    - Review-delta `tweak_attachment` validates that the specified parent/child anchors exist (errors early instead of producing a broken assembly).
    - Copy/mirror helpers no longer treat missing anchors as identity (errors instead of guessing).
    - Update (2026-03-05): plan conversion **hydrates missing required attachment anchors for `reuse_groups` targets only** (e.g. `attach_to.child_anchor`) using deterministic copy-friendly defaults, so reuse targets may omit duplicate anchors without blocking plan validation/conversion. Non-reuse missing anchors still error.
  - Code: `src/gen3d/ai/convert.rs` (anchor lookup + review-delta validation), `src/gen3d/ai/copy_component.rs` (anchor lookup).
  - Test: `src/gen3d/ai/convert.rs` `review_delta_tweak_attachment_errors_on_missing_anchors`, `reuse_target_hydrates_missing_child_anchor_for_plan_conversion`.

- [x] **Join-frame alignment assumptions (forward opposition errors; up opposition must error too)**
  - Fixed (2026-02-23): plan validation now errors on **opposing up** as well as opposing forward so 180° join-frame roll flips never happen silently.
  - Result: if you want a 180° flip/roll at a join, it must be authored explicitly via `attach_to.offset` rotation (with `rot_frame` set), not by opposing anchors.
  - Code: `src/gen3d/ai/convert.rs` (attachment join-frame validation).

## 3) “Any animation” constraints — where current behavior limits what can be expressed

Status (2026-02-24):

- Gen3D plans are now **static-only** and do not include AI-authored `attach_to.animations` clips.
- The items below apply primarily to runtime/prefab-authored animation channels and forced playback, not Gen3D plan generation.

- [x] **Channel model: add explicit per-instance control for “any channel” playback**
  - Fixed (2026-02-23): runtime still auto-selects canonical channels for gameplay (`idle`/`move`/`attack_primary`/`ambient`), but **any channel** can be played via explicit overrides.
  - Key capabilities:
    - Gen3D preview UI lists available channels and can force-play any of them.
    - Gameplay hotkeys `1..9/0` force-play the selected unit’s channels (ordered; up to 10).
    - Automation HTTP API: `POST /v1/animation/force_channel` sets/clears `ForcedAnimationChannel`.
  - Code: `src/object/visuals.rs` (`update_part_animations` forced override), `src/types.rs` (`ForcedAnimationChannel`), `src/rts.rs` (digit hotkeys), `src/automation/mod.rs` (`/v1/animation/force_channel`), `src/gen3d/ui.rs` + `src/gen3d/preview.rs` (data-driven dropdown).

- [x] **Support more generic clip kinds (keyframed `once` / `ping_pong` in addition to `loop` / `spin`)**
  - Fixed (2026-02-23): runtime supports `once` and `ping_pong` keyframed clips (generic, non-heuristic building blocks).
  - Code: `src/object/registry.rs` (`PartAnimationDef`), `src/object/visuals.rs` (`sample_part_animation`).

- [x] **Fixed joints no longer sanitize away rotation (allow visual-only motion)**
  - Fixed (2026-02-23): fixed joints may still have rotating animations; the engine does not rewrite rotation deltas to identity.
  - Motion validation reports `fixed_joint_rotates` as a warning (diagnostic) rather than silently mutating motion.
  - Code: `src/gen3d/ai/motion_validation.rs` (`fixed_joint_rotates` warn).

- [x] **Allow negative/zero scale effects (no engine-side “positive minimum” sanitization for tool transforms)**
  - Fixed (2026-02-23): Gen3D tool-call transform parsing preserves negative and zero scale (enables mirroring, squash-to-zero, stylized effects).
  - Runtime transform math uses safe decomposition that supports degenerate and mirrored transforms.
  - Code: `src/gen3d/ai/agent_parsing.rs` (`parse_delta_transform`), `src/geometry.rs` (`mat4_to_transform_allow_degenerate_scale`).
  - Test: `src/gen3d/ai/agent_loop/mod.rs` `gen3d_tool_transform_parsing_preserves_negative_and_zero_scale`.

- [x] **Motion validation: keep semantics-based checks non-blocking; remove channel-mutation fallback**
  - Fixed (2026-02-23):
    - `chain_axis_mismatch` is **warn-only** and is scoped to a narrow “limb link” shape to reduce false positives.
    - The engine does not disable/mutate authored motion channels as a fallback when validation finds issues.
  - Code: `src/gen3d/ai/motion_validation.rs` (`chain_axis_mismatch` severity), `src/gen3d/ai/agent_tool_dispatch.rs` (no channel-disable fallback policy).

## 4) Diagnostics / guardrails to prevent “wrong result but no clue why”

- [x] **Review-delta prompt/schema drift prevents automatic repairs**
  - Fixed (2026-02-23): updated the Gen3D review-delta system prompt to match the strict JSON schema so the agent can actually apply anchor/animation fixes.
  - Key fixes:
    - `tweak_anchor.set` uses `forward` + `up` directly (no `set.rot` field).
    - Identity-loop example uses the canonical animation schema (`spec.clip.kind="loop"`, `duration_secs`, `keyframes[].time_secs`, `delta=null`).
  - Code: `src/gen3d/ai/prompts.rs` (`build_gen3d_review_delta_system_instructions`).

- [x] **Add structured “applied-default” artifacts**
  - Fixed (2026-02-23): whenever the engine applies a deterministic non-authoring adjustment (recentering geometry, overriding anchor rotation to plan, rebasing offsets when anchors change), it appends a structured record to `applied_defaults.jsonl` in the run cache directory.
  - Code: `src/gen3d/ai/convert.rs` (uses `append_gen3d_jsonl_artifact` → `applied_defaults.jsonl`), `src/gen3d/ai/artifacts.rs`.

- [x] **Add regression tests for the items above**
  - Fixed (2026-02-23): Gen3D has a growing set of offline regression tests across the Gen3D pipeline and helpers (plan conversion, copy/mirror, parsing strictness, motion validation).
  - Key locations: `src/gen3d/ai/convert.rs`, `src/gen3d/ai/copy_component.rs`, `src/gen3d/ai/reuse_groups.rs`, `src/gen3d/ai/motion_validation.rs`, `src/gen3d/ai/parse.rs`, `src/gen3d/ai/regression_tests.rs`.
