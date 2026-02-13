# ExecPlan: Runs + Checkpoints/Resume + Determinism Quality Gate

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, long-running scene generation/editing becomes **debuggable and resilient**. The system persists a durable “run directory” containing inputs, patches, validation reports, and compilation signatures. If the process crashes, it can resume from the last checkpoint without duplicating work.

This milestone also establishes a “quality gate” for determinism: a small suite of fixture scenes must compile to known signatures, and CI fails if signatures change unexpectedly. This is essential when multiple agents are iterating on the compiler and generators in parallel.

Verification is via tests that:

- create a run directory and ensure required artifacts are written,
- simulate resume behavior,
- and enforce deterministic compilation signatures for fixture scenes.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Confirm Milestones 1–5 landed (sources + compilation + validation + blueprint apply).
- [ ] (2026-02-13) Define the on-disk run directory layout and which artifacts are required at each checkpoint.
- [ ] (2026-02-13) Implement run step tracking and checkpoint writes (spec inputs, patch history, reports, signatures).
- [ ] (2026-02-13) Implement resume semantics (detect last completed checkpoint, retry safely using idempotent request ids).
- [ ] (2026-02-13) Add fixture compilation signature tests (a deterministic “golden” suite).
- [ ] (2026-02-13) Add a documented workflow to intentionally update (“bless”) signatures when a change is intended.
- [ ] (2026-02-13) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Persist run artifacts to local disk by default for dev (under `runs/`), treating them like build/test artifacts.
  Rationale: Scene generation is long-running; durable artifacts are required for debugging, resume, and offline analysis.
  Date/Author: 2026-02-13 / Codex

- Decision: Enforce deterministic signatures in tests as a quality gate.
  Rationale: Without a gate, compiler changes will silently break reproducibility and make multi-agent iteration chaotic.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Observability and resumability goals:

- `docs/gamedesign/29_observability_and_resumability.md`

Run concepts in the agent system design:

- `docs/gamedesign/26_scene_generation_agent_system.md` (SceneGenRun, artifacts, patches)

This milestone turns those goals into concrete, testable engine behavior and locks determinism in place via tests.

## Plan of Work

Define a run directory layout that is stable and intentionally boring. It should be easy for both humans and agents to inspect. A run directory must contain, at minimum:

- the input specs used for the run (scene intent, seed policy, scorecard),
- the ordered patch history applied,
- validation reports for each iteration,
- a compilation signature and any “what changed” summaries,
- and a step timeline with explicit checkpoint markers.

Implement a small run manager that writes these artifacts at well-defined points. Every write must be safe (write-then-rename) so a crash does not corrupt the last good checkpoint.

Add resume logic that:

- finds the last completed checkpoint,
- restores enough state to continue (sources + patch history),
- and retries incomplete steps safely using idempotent `request_id` semantics.

Finally, add a deterministic signature test suite using small fixtures under `tests/scene_generation/fixtures/`. Tests should fail loudly if signatures drift, and the repo should provide a clear, intentional workflow for updating signatures when changes are expected.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- Run directories are created and contain the required artifacts (validated by tests).
- Resume behavior works for at least one simulated crash scenario (validated by tests).
- Deterministic signature fixtures exist and CI enforces them (tests fail on unexpected signature changes).
- The repository documents how to intentionally update fixture signatures when needed.

## Idempotence and Recovery

- Every run step write must be atomic at the file level (write temp file → rename).
- Resume must not duplicate instances or apply patches twice. Request idempotency is required for retry safety.

## Interfaces and Dependencies

Use existing dependencies only. If signatures are already implemented in Milestone 3, reuse the exact signature algorithm and ensure it is stable and documented.

