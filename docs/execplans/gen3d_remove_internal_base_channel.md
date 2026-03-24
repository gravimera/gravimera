# Gen3D: Remove internal `__base` animation channel (replace with explicit fallback basis)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has `PLANS.md` at the repo root; this document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Today, attachment edges in Gen3D may include an internal reserved animation channel named `__base`. It is not user-authored motion; it is an implementation detail used as a last-priority fallback when a gameplay channel (idle/move/action/attack) has no authored slot, while still preserving the “rest pose” under preserve-mode offset edits.

After this change, the engine no longer uses (or generates) a reserved `__base` animation channel. Instead, each animated edge carries an explicit `fallback_basis` transform that is applied only when no channel slot matches. This keeps preserve-mode behavior stable (no pops) without encoding “base” as a fake animation slot. The game must still start and run, and Gen3D must keep the same preserve-mode stability guarantees.

You can see it working by running the UI smoke test:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

and by running unit tests that exercise motion selection and offset rebasing.

## Progress

- [x] (2026-03-24) Write this ExecPlan and identify all `__base` call sites.
- [x] Implement explicit per-edge `fallback_basis` storage and persistence.
- [x] Update runtime motion selection to use `fallback_basis` when no channel slot matches.
- [x] Remove Gen3D `__base` normalization/gating and port rebasing to `fallback_basis`.
- [x] Update docs (`docs/gen3d/README.md`) to describe the new mechanism.
- [x] Update/replace unit tests that referenced `__base`.
- [x] Run `cargo test`, then run the rendered smoke test, then commit.

## Surprises & Discoveries

- Legacy saves/prefabs can still contain a synthetic `__base` animation slot; migrating it into
  `fallback_basis` on load avoids regressions.

## Decision Log

- Decision: Replace `__base` slot with an explicit per-edge `fallback_basis` transform.
  Rationale: Preserves the same visual stability under offset rebasing without encoding it as a fake animation channel, and keeps authored channels clean.
  Date/Author: 2026-03-24 / Codex

## Outcomes & Retrospective

- Removed the reserved `__base` animation channel from runtime selection and Gen3D authoring.
- Added explicit per-edge `fallback_basis` persistence and application (used only when no channel slot matches).
- Preserved preserve-mode stability by rebasing both slot bases and `fallback_basis` across offset edits.
- Added legacy migration: synthetic `__base` slots are converted into `fallback_basis` on load.

## Context and Orientation

Key concepts (as implemented in this repo):

- A “part edge” is an `ObjectPartDef` inside an `ObjectDef` (`src/object/registry.rs`). For attachment edges (child object refs), the part has `attachment: Some(AttachmentDef)` and a base transform `transform` which is the join-frame offset.
- A “slot” is a `PartAnimationSlot` (`src/object/registry.rs`) with a `channel` string (e.g. `move`, `idle`) and a `PartAnimationSpec` containing:
  - `basis`: a constant transform applied between the base offset and the clip delta.
  - `clip`: a time-varying delta transform.
- Runtime selection and application lives in `src/object/visuals.rs:update_part_animations`.
- Gen3D preserve-mode can change `attach_to.offset` and rebases slot bases so `new_offset * basis_new == old_offset * basis_old`. Previously, channels with no authored slot relied on an internal `__base` slot to carry this rebased basis.

Current `__base` call sites to remove/replace:

- Runtime applies `fallback_basis` when no channel slot matches: `src/object/visuals.rs`.
- Gen3D rebases both slot bases and `fallback_basis` under offset changes: `src/gen3d/ai/attachment_motion_basis.rs` and callers in plan/draft ops and motion application.
- Gen3D prompts/summaries/validation simulation no longer mention or filter `__base`: `src/gen3d/ai/prompts.rs`, `src/gen3d/ai/orchestration.rs`, `src/gen3d/ai/agent_prompt.rs`, `src/gen3d/ai/agent_step.rs`, and `src/gen3d/ai/motion_validation.rs`.
- Docs updated: `docs/gen3d/README.md` documents `fallback_basis`, and `docs/execplans/gen3d_per_slot_animation_basis.md` is marked as superseded.

Persistence to update:

- Prefab JSON serialization/deserialization: `src/realm_prefabs.rs` (`ObjectPartDefJson`).
- Scene `.dat` serialization: `src/scene_store.rs` (`SceneDatPartDef` and conversions).
- Gen3D edit bundle format: `src/gen3d/ai/edit_bundle.rs` (`Gen3dPlannedAttachmentBundleV1`).

## Plan of Work

Implement the refactor in three layers, keeping behavior identical:

1. Data model changes:
   - Add `fallback_basis: Transform` to runtime `ObjectPartDef` so the engine can persist and apply a per-edge fallback basis.
   - Add `fallback_basis: Transform` to `Gen3dPlannedAttachment` so Gen3D can preserve/rebase it during preserve-mode offset edits.
   - Initialize `fallback_basis` to identity for new edges, and reset it to identity when an edge has no animation slots (matching the previous behavior where `__base` was removed when it was the only slot).

2. Runtime behavior:
   - Remove the reserved `__base` channel and selection logic.
   - When no channel slot matches, apply `fallback_basis` (sanitized) with identity delta:
     `animated_offset = base_offset * fallback_basis`.

3. Gen3D behavior:
   - Remove all logic that creates/canonicalizes a `__base` slot.
   - When rebasing due to `attach_to.offset` changes, rebase both:
     - every slot’s `spec.basis`
     - the edge’s `fallback_basis`
     using the same delta matrix `inv(new_offset) * old_offset`.
   - Update prompts/summaries/validation simulation to no longer mention or filter `__base`.

Then update docs and tests, run tests, run smoke test, and commit.

## Concrete Steps

All commands run from the repo root.

1. Locate all references:

    rg -n "__base|fallback_basis" src docs

2. Implement data model + persistence:

    cargo test -q

3. Implement runtime selection change and Gen3D rebasing changes.

4. Update docs:

    rg -n "__base" docs/gen3d/README.md docs/execplans

5. Validate:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

6. Commit:

    git status
    git commit -am "gen3d: remove __base channel; add per-edge fallback basis"

## Validation and Acceptance

Acceptance means:

- `cargo test` passes.
- The rendered smoke test runs for 2 seconds without crashing.
- The runtime no longer references a reserved channel named `__base`.
- Gen3D preserve-mode offset rebasing still keeps authored channels visually stable, and missing-channel cases use the edge’s `fallback_basis` instead of a fake `__base` slot.

## Idempotence and Recovery

Re-running the tests and smoke test is safe and should be done after any change to motion selection or persistence.

If something breaks, rollback by `git reset --hard HEAD~1` (after confirming no uncommitted changes you care about), then re-apply the plan steps more incrementally.

## Artifacts and Notes

- Keep this file updated with any additional call sites discovered, test output that explains failures, and any design changes made during implementation.

## Interfaces and Dependencies

At the end of the change, these interfaces must exist:

- `crate::object::registry::ObjectPartDef` includes `fallback_basis: Transform`.
- `crate::object::visuals::PartAnimationPlayer` includes `fallback_basis: Transform`.
- Gen3D attachment representation (`Gen3dPlannedAttachment`) includes `fallback_basis: Transform`.
- Prefab JSON (`src/realm_prefabs.rs`) and scene.dat (`src/scene_store.rs`) round-trip `fallback_basis` (optional when identity).
