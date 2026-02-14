# ExecPlan 02: Scene Sources Round-Trip + Automation Verification

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, Gravimera can **import** a scene from `scenes/<scene_id>/src/` sources into the running ECS world and **export** the currently loaded scene back into canonical `src/` sources. This enables a real dev↔test loop: tests can round-trip a fixture scene and assert byte-identical canonical output.

Verification is via an automation-driven integration test (similar to `tests/automation_api_smoke.rs`) that launches the game in headless automation mode, imports a fixture, exports it to a temporary directory, canonicalizes it, and compares it to the fixture.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [x] (2026-02-14) Confirm Milestone 1 landed (scene sources module + canonicalization + minimal fixture).
- [x] (2026-02-14) Implement ECS→sources export for the current object model (instances + ids + transforms + tint/scale as supported).
- [x] (2026-02-14) Implement sources→ECS import for the same subset, ensuring stable ids are respected.
- [x] (2026-02-14) Add local Automation API endpoints to import/export scene sources directories.
- [x] (2026-02-14) Add `tests/scene_sources_roundtrip.rs` integration test that exercises import→export→compare.
- [x] (2026-02-14) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Verify round-trip via the existing local Automation API rather than adding a new CLI tool first.
  Rationale: The Automation API is already the repo’s primary mechanism for headless integration tests.
  Date/Author: 2026-02-13 / Codex

- Decision: Keep the initial import/export scope aligned with what `scene.dat` can currently represent.
  Rationale: A correct, testable subset is better than an unstable “complete” exporter; later milestones can extend the format and tests.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

- Shipped scene sources ↔ ECS round-trip in headless automation mode:
  - Import: `POST /v1/scene_sources/import` loads `src/` and replaces non-player scene instances with pinned instances.
  - Export: `POST /v1/scene_sources/export` writes canonical `src/` sources (preserving metadata from the last import).
- Added a pinned-instance fixture and an integration test that round-trips `tests/scene_generation/fixtures/minimal/src/` via the Automation API and asserts byte-identical canonical output.
- Limitations (expected for this milestone):
  - Only pinned instances are materialized as ECS entities; layers/portals are retained for round-trip but not applied/compiled yet.

## Context and Orientation

Milestone 1 defines the on-disk source format and canonicalization utilities:

- `docs/execplans/execplan_scene_01_sources_foundation.md`

Existing automation test patterns:

- `tests/automation_api_smoke.rs` spawns the binary with automation enabled and performs HTTP calls over a local port.

The current save/load pipeline:

- `src/scene_store.rs` currently persists the world to `scene.dat` (protobuf). Import/export of sources in this milestone should not break `scene.dat` behavior.

## Plan of Work

Add two data-paths in the engine:

1) Export path: ECS world state → `SceneSources` → write to a directory (canonical).
2) Import path: read `SceneSources` from a directory → spawn/update ECS world state to match (using stable ids when provided).

Then expose those operations via the local Automation API so tests can drive them without UI. The API should accept explicit paths and must be safe to call repeatedly (idempotent where possible). Because this is a dev-only API, the endpoint family can live under `/v1/` with the rest of automation endpoints.

Finally, add an integration test that performs a full round-trip against the fixture created in Milestone 1.

## Concrete Steps

Run from the repo root:

1) Integration tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- A new automation integration test passes that:
  - launches the game with automation enabled,
  - imports `tests/scene_generation/fixtures/minimal/src/` into the running world,
  - exports the world back to a temp directory as canonical sources,
  - and verifies the exported `src/` tree is byte-identical to the fixture after canonicalization.
- Re-running the import/export cycle does not duplicate instances (stable ids are respected).

## Idempotence and Recovery

- Import must not create duplicates when stable instance ids are present in sources.
- Export must be canonical and stable (running export twice without mutations produces identical bytes).
- If import encounters unknown fields, it must ignore them but preserve them on re-export when possible (by storing them in `extras` fields in `SceneSources`).

## Interfaces and Dependencies

Use existing dependencies only:

- `serde` / `serde_json` for source parsing and writing.
- Existing HTTP automation infrastructure in `src/automation/mod.rs`.

Do not add external test harness tooling in this milestone; keep tests in Rust under `tests/`.
