# ExecPlan: Scene Generation Pipeline (Sources, Layers, Validation, Runs)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this work, Gravimera supports a practical, testable, multi-agent scene generation workflow where:

- Scenes have an **authoritative, git-friendly text source of truth** (split into multiple files so multiple agents can work in parallel without constant conflicts).
- Scenes can be **compiled deterministically** from those sources into optional **binary build caches** for fast loading/simulation.
- AI agents (and humans) can **create, edit, regenerate, validate, and debug** large scenes via bulk “blueprint” operations and procedural layers, with clear validation reports and durable run artifacts for crash-resume and inspection.

This is not “a town generator”. The engine remains a generic compiler + validator; agents author concrete intent using explicit parameters.

User-visible behavior we will enable (end state):

- A developer can author a scene by editing files under `scenes/<scene_id>/src/` (the “scene repo”), commit changes, and run the game to load it.
- The game can export the currently loaded scene to `src/` and import from `src/` (round-trip).
- A local automation client can:
  - validate a scene and get a structured `ValidationReport` with evidence pointers,
  - apply a blueprint patch, recompile affected layers, and get a “what changed” report,
  - persist a durable run directory (logs + artifacts) so a crash can resume from the last checkpoint.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Read and summarize the current scene save/load pipeline (`src/scene_store.rs`) and identify the minimum stable “scene core” types needed for sources.
- [ ] (2026-02-13) Define concrete JSON schemas (field names + canonical ordering rules) for `scenes/<scene_id>/src/*` that satisfy `docs/gamedesign/30_scene_sources_and_build_artifacts.md`.
- [ ] (2026-02-13) Implement `SceneSources` read/write + canonicalization utilities (serde + stable formatting) and add unit tests for canonical stability.
- [ ] (2026-02-13) Implement ECS ↔ SceneSources conversion for the currently supported object model (prefabs/instances/transforms/tint/scale) and add round-trip tests.
- [ ] (2026-02-13) Add automation endpoints for export/import of scene sources and a headless integration smoke test that:
  - starts the game with automation enabled,
  - imports a fixture scene from `tests/`,
  - exports it back,
  - and verifies canonical equality.
- [ ] (2026-02-13) Introduce procedural layer files under `src/layers/` (still data-only) and implement the regeneration rule “layer owns outputs unless pinned”.
- [ ] (2026-02-13) Implement deterministic instance-id derivation for compiled outputs (no UUID v4 in compilation paths) and add determinism regression tests (compile twice → identical signature).
- [ ] (2026-02-13) Add a minimal generic validator set (budgets, referential integrity, collision/movement blockers sanity, portal destination validity, marker reachability if navigation enabled) and expose it via automation API.
- [ ] (2026-02-13) Implement `ScorecardSpec` + `ValidationReport` JSON contracts and persist reports to disk as run artifacts.
- [ ] (2026-02-13) Implement blueprint apply in “scene sources mode”: apply mutations by editing `src/` (not binary) + recompile + invalidate/refresh build caches.
- [ ] (2026-02-13) Add durable “run directories” (artifacts + step timeline) and crash-resume semantics for long-running authoring runs.
- [ ] (2026-02-13) Add a “dev ↔ test ↔ quality” cycle: a small fixture suite + automation-driven integration tests + a deterministic signature gate that prevents accidental scene compiler changes.

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

This work is intentionally split into small, testable milestones. Each milestone must add or extend automated tests so the dev loop is:

1. Change sources/engine.
2. Run unit tests (`cargo test`).
3. Run headless automation integration tests (also under `cargo test`).
4. (Optional) Run rendered snapshot tests locally when GPU is available.
5. Commit.

### Milestone 1: Scene sources foundation (read/write/canonicalize)

Implement the concrete `src/` JSON layout and utilities to read and write it with deterministic canonical formatting. Keep the initial scope aligned with what the engine already supports (object defs + instances), but structure the sources so procedural layers and pinning can be added without a format-breaking rewrite.

Deliverables:

- A Rust module that can read and write `SceneSources` from a directory.
- A canonicalizer that rewrites sources into a stable order (keys and arrays) so diffs are meaningful.
- Unit tests that ensure canonicalization is stable and idempotent.

### Milestone 2: ECS ↔ sources round-trip + automation endpoints

Add conversion between the in-memory ECS scene state and the text sources, and expose minimal automation endpoints for export/import so we can build integration tests without interactive UI.

Deliverables:

- Export: ECS → `src/` (authoritative) + optional `build/` cache.
- Import: `src/` → ECS (spawns the same objects with stable ids).
- A new integration test that imports a fixture and exports it back, asserting canonical equality.

### Milestone 3: Procedural layers, pinning, and deterministic compilation

Introduce procedural layer files and implement deterministic compilation into concrete instances, with the regeneration rule:

- “layer owns its outputs unless pinned”.

Deliverables:

- A compilation pipeline that produces a stable “scene signature” (hash) so determinism can be regression-tested.
- Deterministic id derivation for compiled instances (no random UUIDs).
- Tests:
  - compile twice → same signature
  - regenerate a layer → owned outputs change as expected
  - pinned instances remain unchanged across regeneration

### Milestone 4: Validation + scorecards (hard gates and evidence)

Implement generic validators and the `ValidationReport` output contract so the system can objectively detect “bad results” without embedding aesthetic heuristics.

Deliverables:

- Validators for budgets, referential integrity, portal validity, and (optionally) navigation/marker reachability.
- A stable report format written to the run directory.
- Tests that intentionally violate gates and assert the report contains evidence pointers.

### Milestone 5: Blueprint apply in sources-mode + patch history

Implement blueprint application as edits to `src/` plus recompile/validate. Store patches and results as durable artifacts so the whole process is debuggable and resumable.

Deliverables:

- Blueprint validate/apply updates `src/` (authoritative) and invalidates/refreshes `build/`.
- Patch history persisted in a run directory with a timeline of steps and checkpoints.
- Integration test: apply a small blueprint patch to a fixture, revalidate, and verify only expected files changed in `src/`.

### Milestone 6: Runs, crash-resume, and the dev↔test↔quality cycle

Make long runs resilient: persist step artifacts, and ensure retries are safe (idempotent) so an agent can resume after a crash without duplicating objects.

Deliverables:

- Run directories with checkpoints after validate/apply/evaluate steps.
- A resume mechanism that picks up from the last checkpoint.
- A “quality gate” in CI: a deterministic signature suite (small fixtures) that must not change without an intentional update.

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

