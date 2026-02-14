# ExecPlan 07: Additional Procedural Layer Kinds (Grid + Polyline)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, scene sources support more scalable, generic authoring primitives than
`explicit_instances`, without introducing any domain heuristics.

Concretely, authors (humans or AI agents) can represent “lots of objects” compactly by using:

- a `grid_instances` layer for regular repeated placement, and
- a `polyline_instances` layer for repeated placement along a path.

Both compile deterministically into layer-owned concrete instances with stable ids, participate in
validation, and support scoped regeneration (“layer owns outputs unless pinned”).

Verification is via fixture scenes and tests (including the deterministic golden signatures gate).

## Progress

- [x] (2026-02-14) Create the initial ExecPlan.
- [x] (2026-02-14) Define a spec for additional layer kinds (grid + polyline) and link it from the specs index.
- [x] (2026-02-14) Implement parsing + deterministic compilation for the new layer kinds.
- [x] (2026-02-14) Add fixture scenes that exercise the new kinds and extend the golden signature suite.
- [x] (2026-02-14) Add an integration test that validates deterministic compile + scoped regeneration for the new kinds.
- [x] (2026-02-14) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Ship `grid_instances` and `polyline_instances` first as minimal, deterministic primitives.
  Rationale: They are generic, parameterized, and immediately unlock “many objects” authoring without requiring a UI or embedding domain rules.
  Date/Author: 2026-02-14 / Codex

- Decision: In v1, `polyline_instances` does not auto-align rotations to the path tangent.
  Rationale: Orientation conventions differ across prefabs; forcing an implicit “forward axis” would be a hidden assumption. Authors can use explicit instances if per-instance rotation is required.
  Date/Author: 2026-02-14 / Codex

## Outcomes & Retrospective

- Shipped two additional generic procedural layer kinds:
  - `grid_instances`
  - `polyline_instances`
- Added a v1 layer-kinds spec and indexed it:
  - `docs/gamedesign/33_scene_layer_kinds_v1.md`
- Extended determinism coverage:
  - Added fixture `tests/scene_generation/fixtures/procedural_layers_v1/src`
  - Added integration test `tests/scene_layer_kinds_v1_compile_regen.rs`
  - Extended + blessed `tests/scene_generation/golden_signatures.json`

What remains:

- Add additional generic primitives (e.g., scatter, region filters, instanced variations) as new milestones while keeping deterministic compilation and scorecard-driven validation.

## Context and Orientation

Relevant design/spec references:

- Scene creation goals + regeneration rule: `docs/gamedesign/22_scene_creation.md`
- Scene sources layout (and `explicit_instances`): `docs/gamedesign/30_scene_sources_and_build_artifacts.md`
- New layer kinds spec (to be added in this milestone): `docs/gamedesign/33_scene_layer_kinds_v1.md`

Relevant code:

- Scene sources parsing/canonicalization: `src/scene_sources.rs`
- Layer parsing + deterministic compilation: `src/scene_sources_runtime.rs`
- Determinism quality gate: `src/scene_sources_runtime.rs` test module `golden_scene_signatures`
- Existing fixtures: `tests/scene_generation/fixtures/*`

## Plan of Work

1) Spec: define the new layer kinds and their deterministic semantics.

   - Add `docs/gamedesign/33_scene_layer_kinds_v1.md`.
   - Update `docs/gamedesign/specs.md` to index it.
   - Link to it from `docs/gamedesign/30_scene_sources_and_build_artifacts.md` so readers can discover it from the core sources spec.

2) Engine support: implement parsing + compilation in `src/scene_sources_runtime.rs`.

   - Extend the internal `SceneLayer` enum with:
     - `GridInstances`
     - `PolylineInstances`
   - Implement strict parsing/validation:
     - `grid_instances`: finite origin, finite non-zero step, non-negative counts.
     - `polyline_instances`: at least 2 points, no zero-length segments, finite spacing > 0, finite start_offset >= 0.
   - Extend deterministic compilation by implementing `desired_instances_for_layer(...)` for the new variants.
   - Ensure `validate_scene_sources_impl(...)` continues to:
     - predict instance counts per layer,
     - validate prefab references,
     - detect pinned-vs-layer id conflicts for derived ids.

3) Fixtures + tests:

   - Add a new fixture directory under `tests/scene_generation/fixtures/` that includes:
     - one `grid_instances` layer
     - one `polyline_instances` layer
     - at least one pinned instance (to ensure ownership boundaries still behave)
   - Extend the golden signatures gate to include the new fixture.
   - Add an integration test similar to `tests/scene_layers_compile_regen.rs` that:
     - imports the fixture sources,
     - compiles twice and asserts signatures match,
     - edits one layer on disk, reloads, regenerates only that layer,
     - and asserts other layer + pinned signatures remain unchanged.

4) Validation + commit:

   - Run:

       cargo test
       cargo run -- --headless --headless-seconds 1

   - Commit with a message indicating Milestone 07 completion.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- Scene source layers support `grid_instances` and `polyline_instances` as specified in `docs/gamedesign/33_scene_layer_kinds_v1.md`.
- Deterministic compilation produces stable signatures for a fixture that uses the new kinds.
- Scoped regeneration works for the new kinds: regenerating one layer does not change pinned outputs or other layers (validated by an integration test).
- The deterministic golden signature gate includes at least one fixture using the new kinds.

## Idempotence and Recovery

- Compiling the same sources twice without changes must not produce duplicates and must yield identical signatures.
- Regeneration of a single layer must be safe to retry (idempotent with deterministic ids).

## Interfaces and Dependencies

Use existing dependencies only:

- `serde_json::Value` parsing as used in `src/scene_sources_runtime.rs`
- `uuid` v5 for deterministic ids (reuse `derived_layer_instance_id(...)`)

Do not introduce domain-specific defaults. All behavior must be driven by explicit parameters in the layer docs.
