# ExecPlan 03: Procedural Layers + Pinning + Deterministic Compilation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, scene sources can describe **procedural layers** and the engine can compile those layers deterministically into concrete object instances. This is the foundation for scalable scene generation (thousands of objects) without forcing agents to micromanage every placement.

The key behavior added here is regeneration with ownership:

- A layer owns its outputs unless an instance is pinned.
- Regenerating a layer updates only the instances owned by that layer.
- Pinned instances remain unchanged across regeneration.

Verification is via deterministic signatures and regression tests: compiling the same sources twice yields identical results, and regeneration respects pinning.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Confirm Milestones 1–2 landed (sources format + round-trip import/export tests).
- [ ] (2026-02-13) Extend the source format to include `src/layers/` and represent at least one minimal, generic layer kind.
- [ ] (2026-02-13) Implement deterministic instance-id derivation for layer-owned outputs (no UUID v4 in compilation paths).
- [ ] (2026-02-13) Implement compilation: sources + layers → concrete instances (ECS) with provenance (owner layer id).
- [ ] (2026-02-13) Implement regeneration: recompile one layer and update only owned instances; pinned instances are preserved.
- [ ] (2026-02-13) Add determinism tests (compile twice → identical signature; regen → expected delta).
- [ ] (2026-02-13) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Require deterministic id derivation for all compiled instances (UUID v5 or equivalent), derived from stable inputs.
  Rationale: This is mandatory for reproducible generation, stable diffs, and safe regeneration without duplicating content.
  Date/Author: 2026-02-13 / Codex

- Decision: Treat “manual placement” as just another layer kind (a layer that enumerates explicit instances).
  Rationale: This simplifies ownership semantics: everything is owned by exactly one layer unless pinned.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Design/spec references:

- Scene creation regeneration rule: `docs/gamedesign/22_scene_creation.md`
- Scene sources vs build artifacts: `docs/gamedesign/30_scene_sources_and_build_artifacts.md`

Milestones 1–2 provide:

- A canonical `src/` source layout.
- Import/export round-trip tests via automation.

This milestone is where `src/layers/` starts to matter and where we must stop relying on random ids (UUID v4) during compilation.

## Plan of Work

First, extend the `SceneSources` representation to include a `layers/` directory and define a minimal set of layer kinds that are generic and purely parameterized. This milestone must not embed domain heuristics. A minimal layer kind can be “explicit instances”, which already enables ownership/pinning semantics and deterministic compilation.

Second, implement compilation and regeneration:

- Compilation creates or updates concrete ECS instances for each layer output.
- Every compiled instance must carry provenance tying it back to its owning layer id (for blame, diffs, and regeneration).
- Regeneration of one layer must delete/update/recreate only instances owned by that layer, leaving pinned instances and other layers untouched.

Third, implement determinism gates:

- Define a “scene signature” computed from the canonical compiled instance set (ids + prefab ids + transforms + key overrides). The signature must be byte-stable across runs.
- Add tests that compile the same inputs twice and assert the same signature, and tests that regenerate a layer and assert the expected signature delta.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- Scene sources can include at least one layer under `scenes/<scene_id>/src/layers/`.
- Regeneration behavior matches the rule “layer owns outputs unless pinned”.
- Tests enforce determinism:
  - compile twice from the same sources yields identical signatures,
  - regenerating a layer updates only its owned outputs,
  - pinned instances remain unchanged across regeneration.

## Idempotence and Recovery

- Compilation and regeneration must be safe to retry. If the same compile step runs twice without source changes, it must not produce duplicates.
- If compilation fails mid-run, the system should leave the world in a recoverable state (prefer “apply after validate” semantics in later milestones).

## Interfaces and Dependencies

Use existing dependencies:

- `uuid` (v5) for deterministic id derivation.
- `sha2` for signatures.

Keep compilation logic deterministic by construction: stable ordering, stable formatting, no dependence on hash iteration order, and no use of `ObjectId::new_v4()` in compilation paths.
