# ExecPlan: Scene Generation Pipeline (Roadmap + Verification Cycle)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Scene generation is a large system. This file is the **roadmap** plus the shared **dev↔test/quality verification cycle**. The implementation work is split into milestone ExecPlans so each step delivers something that can be verified in automation.

After completing the milestone ExecPlans linked in the `Plan of Work` section, Gravimera supports a practical, testable, multi-agent scene generation workflow where scenes are authored as git-friendly text sources under `scenes/<scene_id>/src/`, deterministically compiled into optional build caches under `scenes/<scene_id>/build/`, regenerated via procedural layers (“layer owns outputs unless pinned”), and verified via structured validation reports and durable run artifacts (checkpoints + crash-resume).

The engine remains a generic compiler + validator (no domain-specific “town generator” heuristics); agents and humans supply explicit intent via sources and bulk blueprint operations.

End-state behaviors:

1) A developer can edit `scenes/<scene_id>/src/`, commit, and run the game to load the scene.
2) The game can export/import scene sources (round-trip), and tests can enforce canonical equality.
3) Automation can validate/apply/regenerate and persist a run directory with artifacts for debugging and resume.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Execute Milestone 1: `docs/execplans/execplan_scene_01_sources_foundation.md`.
- [ ] (2026-02-13) Execute Milestone 2: `docs/execplans/execplan_scene_02_sources_roundtrip_automation.md`.
- [ ] (2026-02-13) Execute Milestone 3: `docs/execplans/execplan_scene_03_layers_and_compilation.md`.
- [ ] (2026-02-13) Execute Milestone 4: `docs/execplans/execplan_scene_04_validation_scorecards.md`.
- [ ] (2026-02-13) Execute Milestone 5: `docs/execplans/execplan_scene_05_blueprint_apply_sources.md`.
- [ ] (2026-02-13) Execute Milestone 6: `docs/execplans/execplan_scene_06_runs_resume_quality_gate.md`.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Treat protobuf `scene.dat` and other binary encodings as **build artifacts**, not the authoritative source for multi-agent authoring.
  Rationale: Binary formats are not diff/merge friendly; git-driven process management requires canonical text sources.
  Date/Author: 2026-02-13 / Codex

- Decision: Store authoritative scene sources under `scenes/<scene_id>/src/` and optional derived caches under `scenes/<scene_id>/build/`.
  Rationale: Matches `docs/gamedesign/30_scene_sources_and_build_artifacts.md` and allows fast runtime without sacrificing VCS workflows.
  Date/Author: 2026-02-13 / Codex

- Decision: Keep the engine generic: procedural layers are explicit primitives + parameters; the engine provides compilation and validation, not domain “town rules”.
  Rationale: The same pipeline must support “anything”, not only towns.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

(Fill in at completion: what shipped, what remains, lessons learned, and next ExecPlans.)

## Context and Orientation

### Existing code and behavior (today)

Gravimera currently persists the world primarily via a protobuf file:

- `src/scene_store.rs` reads/writes `scene.dat` using `prost::Message`.
- `src/paths.rs` defines the default `scene.dat` path under `GRAVIMERA_HOME`.

This is a good runtime format, but it is not compatible with git-based process management because it is hard to diff/merge, and it produces frequent conflicts when multiple agents edit in parallel.

The repository already has a local HTTP Automation API suitable for development-time integration tests:

- `src/automation/mod.rs` provides `/v1/*` endpoints and supports deterministic stepping in headless mode.
- `tests/automation_api_smoke.rs` shows how to start Gravimera headless, call the API, and shut it down in a test.

### Target design/spec references (source of truth)

Read these docs before implementing anything in this plan:

- Scene creation goals + regeneration rule: `docs/gamedesign/22_scene_creation.md`
- Multi-agent pipeline overview: `docs/gamedesign/26_scene_generation_agent_system.md`
- Observability + resumability goals: `docs/gamedesign/29_observability_and_resumability.md`
- Spec for VCS-friendly scene sources vs build artifacts: `docs/gamedesign/30_scene_sources_and_build_artifacts.md`
- Blueprint bulk authoring: `docs/gamedesign/19_blueprint_spec.md`
- Validation report contracts: `docs/gamedesign/27_scorecards_and_validation_reports.md`

### Terms used in this plan

- “Scene sources”: a set of canonical JSON files under `scenes/<scene_id>/src/` that are the authoritative representation of a scene.
- “Build cache / build artifacts”: optional derived files under `scenes/<scene_id>/build/` (binary instance tables, nav caches) that can be regenerated.
- “Procedural layer”: a source file that describes how to generate many instances deterministically from explicit parameters (roads, scatter, parcels, etc.).
- “Pinned instance”: a single instance file that is not owned by a procedural layer and therefore survives regeneration unchanged.
- “ValidationReport”: a structured report describing which hard gates passed/failed and including evidence pointers for debugging.
- “Run directory”: a durable directory containing logs, specs, patches, and validation reports for one long-running authoring attempt.

## Plan of Work

This work is intentionally split into multiple ExecPlans (one per milestone) so each milestone delivers something that can be verified automatically.

Execute these milestone plans in order:

1) `docs/execplans/execplan_scene_01_sources_foundation.md` — define `scenes/<scene_id>/src/` sources, implement canonical read/write, and add unit tests for idempotence.
2) `docs/execplans/execplan_scene_02_sources_roundtrip_automation.md` — add import/export automation endpoints and an integration test that round-trips a fixture scene.
3) `docs/execplans/execplan_scene_03_layers_and_compilation.md` — add procedural layers + pinning semantics and deterministic compilation/signatures with regression tests.
4) `docs/execplans/execplan_scene_04_validation_scorecards.md` — add validators + scorecards and write structured `ValidationReport` outputs with evidence pointers.
5) `docs/execplans/execplan_scene_05_blueprint_apply_sources.md` — implement blueprint validate/apply that edits sources, recompiles, and revalidates, with patch-history artifacts.
6) `docs/execplans/execplan_scene_06_runs_resume_quality_gate.md` — add run directories, checkpoints/resume, and a deterministic signature quality gate over fixtures.

All milestones share the same dev↔test cycle: make a small change, add/update a test, run `cargo test`, run a headless smoke boot (`cargo run -- --headless --headless-seconds 1`), then commit.

## Concrete Steps

All commands are run from the repository root.

Baseline checks (before starting a milestone):

  cargo test
  cargo run -- --headless --headless-seconds 1

During development, prefer adding fixtures under `tests/`:

- `tests/scene_generation/fixtures/<name>/...` for text scene sources and expected reports/signatures.
- Any temporary configs or cache files must live under `tests/` (never in the repo root).

## Validation and Acceptance

Acceptance is met when the following are true:

1) `cargo test` passes on a machine without a GPU (headless).

2) There is at least one end-to-end integration test that:

- launches the game with automation enabled,
- imports a scene from `tests/scene_generation/fixtures/.../src`,
- validates it (gets `ok=true`),
- exports it back to a temp directory,
- and verifies the exported sources match the canonical fixture.

3) Determinism is enforced by tests:

- compiling the same sources twice yields the same scene signature,
- regeneration respects “layer owns outputs unless pinned”.

4) The system produces actionable debugging artifacts:

- failing validation returns a `ValidationReport` with concrete evidence pointers (file path + id references),
- a run directory exists on disk that contains inputs, patches, and reports sufficient to reproduce failures.

## Idempotence and Recovery

- Export and canonicalization must be idempotent: running them twice produces byte-identical outputs.
- Import must be safe to retry: if the same sources are imported twice with the same stable ids, the result must not duplicate instances or diverge.
- Blueprint apply must be idempotent under `request_id` semantics (see `docs/gamedesign/19_blueprint_spec.md`): repeating the same apply should return the same ids and not duplicate content.

If a new format or migration breaks, provide a safe rollback:

- keep reading legacy `scene.dat` for at least one compatibility window,
- provide an explicit conversion/export tool so users can migrate intentionally,
- never silently discard unknown fields (preserve round-trip where possible).

## Artifacts and Notes

Keep example fixtures small. Prefer “toy scenes” that exercise:

- a few object defs and instances,
- at least one procedural layer,
- at least one pinned instance,
- a portal and a marker (even if the game remains single-scene internally).

For every fixture, store:

- the `src/` tree,
- an expected deterministic signature file (text),
- and (if useful) an expected validation report snapshot.

## Interfaces and Dependencies

Use the existing Rust dependencies:

- `serde` + `serde_json` for sources and reports.
- `sha2` for deterministic signatures.
- Keep `prost`/protobuf support for legacy `scene.dat` and/or derived build caches.

Prefer to reuse the existing Automation API for integration tests before exposing new public APIs. If a new HTTP endpoint is added, it must be:

- capability-gated in hosted contexts (future), but at minimum token-protected in local automation mode,
- deterministic when deterministic stepping is enabled,
- safe to call repeatedly (idempotent) where applicable.

At the end of Milestone 2, the codebase should contain (names can vary, but responsibilities must exist):

- a `SceneSources` in-memory representation (pure data),
- a `scene_sources` module for read/write/canonicalize,
- a conversion layer between `SceneSources` and ECS world state,
- automation endpoints to import/export/validate scenes for tests.
