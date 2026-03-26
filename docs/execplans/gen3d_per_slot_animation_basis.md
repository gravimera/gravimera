# Gen3D: Per-slot animation basis + internal `__base` fallback channel

> Status (2026-03-24): Superseded by `docs/execplans/gen3d_remove_internal_base_channel.md`.
> The reserved `__base` channel described here has been removed and replaced by a per-edge
> `fallback_basis` transform (applied only when no channel slot matches).

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

Gen3D edit runs currently allow PlanOps to change `attach_to.offset` for existing components (when `constraints.preserve_edit_policy="allow_offsets"`). Even when we preserve the existing animation slots, changing the attachment offset changes the coordinate basis used to apply animation deltas, so existing channels (idle/move/action/etc) can look “wrong” after an edit that only intended to add new motions.

After this change:

1. Each animation slot carries its own constant “basis” transform, applied between the attachment offset and the time-varying delta.
2. Preserve-mode offset edits rebase that per-slot basis so that existing animations (and missing-channel fallbacks) remain visually stable.
3. A reserved internal channel `__base` acts as a last-priority fallback slot on an attachment edge so the system can preserve the “rest pose” for channels that have no authored slot.

You can see it working by reproducing an edit run that changes an existing edge’s `attach_to.offset` rotation in preserve mode: after the change, existing idle/move/action poses remain the same, while newly authored channels adopt the new offset basis.

## Progress

- [x] (2026-03-24) Write this ExecPlan and identify all call sites.
- [x] Implement per-slot basis in core animation types and runtime composition.
- [x] Add internal `__base` fallback selection and normalization rules.
- [x] In preserve-mode plan acceptance, rebase preserved slot bases when `attach_to.offset` changes.
- [x] Update Gen3D motion validation and scene serialization to match the new semantics.
- [x] Add focused unit tests for basis rebasing and fallback behavior.
- [ ] Run smoke test (`cargo run -- --rendered-seconds 2`) and commit.

## Surprises & Discoveries

- Observation: Gen3D attachments are emitted as `ObjectPartDef::object_ref(child_id, att.offset)` and attachment-edge animations are stored on the parent part (`part.animations.extend(att.animations.clone())`).
  Evidence: `src/gen3d/ai/convert.rs` in `sync_attachment_tree_to_defs`.

- Observation: Runtime applies animation deltas in the “offset slot” via `animated_base = base * delta(t)` and then resolves attachment transforms as `parent_anchor * offset * inv(child_anchor)`.
  Evidence: `src/object/visuals.rs` in `update_part_animations` and `resolve_attachment_transform_with_offset`.

## Decision Log

- Decision: Store basis per slot (`PartAnimationSpec.basis: Transform`) rather than introducing a separate per-edge basis.
  Rationale: Allows different channels on the same edge to be preserved independently (e.g., preserve old `idle` while regenerating `move` to adopt the updated rig).
  Date/Author: 2026-03-24 / agent

- Decision: Use a reserved internal channel name `__base` as the last-priority fallback slot for attachment edges.
  Rationale: Keeps the selection model “channels choose a slot” while allowing a deterministic fallback that can carry a basis, without changing the external “motion channel” API.
  Date/Author: 2026-03-24 / agent

## Outcomes & Retrospective

- Existing animation channels stay visually stable across preserve-mode offset edits (and DraftOps `set_attachment_offset`) by rebasing per-slot `basis` instead of rewriting keyframes.
- Channels with no authored slot fall back to an internal `__base` slot, keeping the rest pose stable under the same rebasing logic.
- `basis` is persisted through prefab JSON, `scene.grav`, and edit bundles (missing basis defaults to identity).

## Context and Orientation

Key concepts:

- Attachment edge: In object defs, a child component is referenced from its parent via an `ObjectPartDef` with `kind=ObjectRef` and `attachment=Some(AttachmentDef)`. The part’s `transform` is the authored `attach_to.offset` in the parent anchor’s join frame.
- Animation slot: `PartAnimationSlot { channel, spec }` stored on a part edge; runtime selects the best slot based on active animation channel state.
- Current composition (pre-change): `animated_offset = offset * delta(t)`.

Relevant files:

- `src/object/registry.rs`: Core types `PartAnimationSlot`, `PartAnimationSpec`, and `PartAnimationDef`.
- `src/object/visuals.rs`: Runtime animation selection and transform application (`update_part_animations`).
- `src/gen3d/ai/plan_ops.rs`: Preserve-mode plan acceptance merge (`apply_plan_acceptance`), currently preserves old slots but does not compensate for offset changes.
- `src/gen3d/ai/agent_motion_batch.rs`: Motion authoring applies channel replacements.
- `src/gen3d/ai/motion_validation.rs`: QA validation that simulates runtime animation application.
- `src/realm_prefabs.rs` and `src/scene_store.rs`: Serialization of animation specs to JSON and `scene.grav`.
- `docs/gen3d/README.md`: Gen3D documentation; needs to describe new semantics and reserved channel.

## Plan of Work

### 1) Add per-slot basis to core animation spec

In `src/object/registry.rs`, extend:

  - `pub(crate) struct PartAnimationSpec` to include `basis: Transform` (default identity).

Update all constructors/call sites that build a `PartAnimationSpec` to set `basis: Transform::IDENTITY`.

### 2) Update runtime composition and selection order

In `src/object/visuals.rs`:

  - Apply animations as `animated_base = base * spec.basis * delta(t)` (or equivalent with `mul_transform`).
  - Extend the channel selection order to include `__base` as the last fallback (after `ambient`).
  - Keep existing channel priority for `attack`, `action`, `move`, `idle`, `ambient`.

### 3) Normalize/ensure internal `__base` slot

Define a small helper (in Gen3D code) that enforces:

  - If an attachment edge has at least one non-`__base` slot, ensure exactly one `__base` slot exists.
  - If an attachment edge has no non-`__base` slots, ensure there is no `__base` slot (avoid creating animation players on fully static edges).

This helper is called whenever we mutate attachment-edge slots.

### 4) Preserve-mode offset changes rebase preserved bases

In `src/gen3d/ai/plan_ops.rs` within `apply_plan_acceptance`, when `preserve_existing_components=true` and an existing component keeps the same attachment interface (same `parent`, `parent_anchor`, `child_anchor`), preserve old slots and joint metadata as today, but:

  - If `old_att.offset != new_att.offset`, update every preserved slot’s basis:

        basis_new = inv(new_offset) * old_offset * basis_old

  - Ensure `__base` exists after preserving slots, so missing-channel fallbacks are also stable.

### 5) Update Gen3D motion validation and serialization formats

In `src/gen3d/ai/motion_validation.rs`:

  - Mirror runtime: compose animated offset as `offset * basis * delta(t)`.
  - Update any “choose slot” logic to consider `__base` as last fallback when no other channel slot matches.

In `src/realm_prefabs.rs`:

  - Extend `PartAnimationSpecJson` to include optional `basis` (serde default → identity).

In `src/scene_store.rs`:

  - Extend `SceneDatPartAnimation` with optional `basis` field (new protobuf tag).
  - Encode basis when non-identity; decode missing basis as identity.

In `src/gen3d/ai/edit_bundle.rs`:

  - Extend `PartAnimationSpecBundleV1` to include `basis` (serde default).

### 6) Tests

Add focused unit tests (prefer fast unit tests) that cover:

  - Rebasing: if `old_offset` differs from `new_offset`, and a preserved slot has `basis_old=identity`, then `new_offset * basis_new == old_offset` (within float epsilon).
  - Fallback: when an attachment has `move` only (plus `__base`), “idle” selection in runtime/validation falls back to `__base`.

### 7) Docs

Update `docs/gen3d/README.md` to document:

  - The new composition formula (`offset * basis * delta(t)`).
  - The meaning and reservation of the `__base` channel.
  - Preserve-mode behavior: offset changes rebase existing slot bases to keep existing channels stable.
  - How to intentionally change existing channels (regenerate/replace that channel).

## Concrete Steps

Run from repository root (`/Users/flow/workspace/github/gravimera`):

1. Edit code and run unit tests:

   - `cargo test`

2. Run the required smoke test (rendered; non-headless):

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`

3. Commit changes:

   - `git status`
   - `git commit -am \"gen3d: add per-slot animation basis\"` (or similar; include docs + validation updates)

## Validation and Acceptance

Acceptance is satisfied when:

1. The project builds and unit tests pass.
2. The smoke test starts and runs for 2 rendered seconds without crashing.
3. In a preserve-mode edit run where PlanOps changes `attach_to.offset` for an existing edge, existing channels (idle/move/action) remain visually stable after acceptance, while new channels can be added without disturbing old ones.

## Idempotence and Recovery

- The schema changes (JSON + scene.grav) are backward compatible by defaulting `basis` to identity when absent. Re-running serialization should not corrupt data.
- If the `scene.grav` tag change causes decode issues, temporarily gate writes of `basis` behind “non-identity only” and keep decode tolerant.

## Artifacts and Notes

- Reserved channel: `__base` (internal; last-priority fallback).
- Composition formula on attachment edges:

    offset_animated(t) = attach_to.offset * slot.spec.basis * delta(t)

## Interfaces and Dependencies

No new external dependencies are required.

At end-state, these type/interface expectations must hold:

- `crate::object::registry::PartAnimationSpec` includes a `basis: Transform`.
- Runtime `update_part_animations` applies `basis` and falls back to `__base`.
- Gen3D preserve-mode plan acceptance rebases preserved slot bases when offsets change.
