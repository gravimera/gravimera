# ExecPlan 04: Scene Validation + Scorecards + Evidence Reports

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, Gravimera can tell (in a machine-readable way) whether a generated scene result is “bad” according to explicit constraints, without relying on aesthetic heuristics. The output is a structured `ValidationReport` that includes evidence pointers so agents (and developers) can diagnose and repair failures.

Verification is via tests that intentionally violate constraints and assert that the report contains a stable error code plus concrete evidence (ids, file paths, and counterexamples).

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [x] (2026-02-14) Confirm Milestones 1–3 landed (sources + round-trip + layers/compilation).
- [x] (2026-02-14) Define a minimal `ScorecardSpec` subset used by the engine validators (hard gates first).
- [x] (2026-02-14) Implement a `ValidationReport` type aligned with `docs/gamedesign/27_scorecards_and_validation_reports.md`.
- [x] (2026-02-14) Implement baseline validators (referential integrity, budgets, portal validity, determinism invariants) and include evidence pointers.
- [x] (2026-02-14) Expose validation via automation endpoint(s) for integration testing.
- [x] (2026-02-14) Add failing fixtures + tests that assert specific report codes and evidence.
- [x] (2026-02-14) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Start with “hard gate” validators that are objective and deterministic (budgets, referential integrity, portal destinations, determinism checks).
  Rationale: These are necessary for safe automation and can be validated without subjective judgement.
  Date/Author: 2026-02-13 / Codex

- Decision: Require evidence pointers for every violation (not just human-readable messages).
  Rationale: Auto-repair and multi-agent debugging require concrete, machine-readable blame targets.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

- Shipped minimal scorecard + report contracts:
  - `ScorecardSpecV1` (hard gates) and `ValidationReportV1` (violations + evidence pointers).
- Added deterministic, objective scene validators (no aesthetic heuristics):
  - unknown prefab id references (`unknown_prefab_id`)
  - budget gates for predicted instance and portal counts
  - portal validity (unknown destination scenes when discoverable via `scenes/<scene_id>/src` layout)
- Exposed validation via the local Automation API:
  - `POST /v1/scene_sources/validate` returns `{ ok, report }`
- Added fixtures + an integration test asserting stable violation codes and evidence pointers.

## Context and Orientation

Validation and reports are specified here:

- `docs/gamedesign/27_scorecards_and_validation_reports.md`

Scene generation pipeline context:

- `docs/gamedesign/25_evaluation_and_auto_repair.md`
- `docs/gamedesign/26_scene_generation_agent_system.md`

This milestone focuses on engine-side validators and report contracts. Learned critics (vision models, etc.) are out of scope here; they can consume the reports later.

## Plan of Work

Implement validation as a pure function over deterministic inputs (scene sources, compiled instance set, and an explicit `ScorecardSpec`). Avoid validators that depend on frame timing, random sampling, or renderer output in this milestone.

Define a minimal set of report fields that will remain stable:

- a top-level pass/fail result for hard gates,
- a list of violations with stable `code` strings,
- an evidence section that references specific ids and source file paths,
- summary metrics (counts, budgets consumed) where they are cheap to compute deterministically.

Expose validation through an automation endpoint so integration tests can assert behavior end-to-end.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- A `ValidationReport` is produced for a scene and includes stable violation codes and evidence pointers.
- There are fixture-based tests that fail before the change and pass after, covering at least:
  - broken references (unknown prefab id / unknown portal destination),
  - budget exceeded (too many instances),
  - determinism invariant violated (non-deterministic id generation detected, if applicable).

## Idempotence and Recovery

- Validation must never mutate the scene or write to sources/build caches.
- Reports should be safe to persist as artifacts; they must not contain secrets (tokens, API keys).

## Interfaces and Dependencies

Use existing dependencies:

- `serde` / `serde_json` for report serialization (when persisted).

Keep report types “data-only” so they can be used by both automation endpoints and future `/v2/` agent APIs.
