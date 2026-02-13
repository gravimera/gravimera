# ExecPlan: Blueprint Apply in Sources-Mode (Patch → Recompile → Revalidate)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, agents (and tools) can apply bulk scene edits safely by submitting a blueprint patch that mutates the **authoritative scene sources** (`src/`), then deterministically recompiles layers, and finally revalidates using the scorecard/validators. This is the core “agent authoring loop”: validate → apply patch → recompile → validate → accept/reject.

Verification is via an automation-driven integration test that applies a small patch to a fixture scene and asserts that:

- only the expected `src/` files changed,
- compilation is deterministic,
- and validation passes (or fails with the expected report) after the patch.

## Progress

- [x] (2026-02-13) Create the initial ExecPlan.
- [ ] (2026-02-13) Confirm Milestones 1–4 landed (sources + round-trip + compilation + validation reports).
- [ ] (2026-02-13) Define the minimal blueprint patch operations needed for scene generation iteration (spawn/edit/destroy, layer edits, pin/unpin).
- [ ] (2026-02-13) Implement blueprint validate/apply that edits `src/` (not binary caches) and returns a structured “what changed” summary.
- [ ] (2026-02-13) Ensure apply is idempotent under `request_id` semantics (repeat apply does not duplicate content).
- [ ] (2026-02-13) Expose validate/apply via automation endpoint(s) for integration testing.
- [ ] (2026-02-13) Add fixtures + integration tests for patch apply.
- [ ] (2026-02-13) Run `cargo test` + headless smoke boot and commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Apply blueprints by mutating authoritative sources (`src/`) and treating build caches as derived outputs.
  Rationale: Git/process workflows and multi-agent parallel edits require a text source of truth. Binary caches must never be the only editable representation.
  Date/Author: 2026-02-13 / Codex

- Decision: Require validate-before-apply and return a structured diff summary.
  Rationale: Agents need predictable failure modes and must be able to reason about the effect of a patch without “guessing”.
  Date/Author: 2026-02-13 / Codex

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

Blueprint goals and semantics:

- `docs/gamedesign/19_blueprint_spec.md`

Scene sources vs build caches:

- `docs/gamedesign/30_scene_sources_and_build_artifacts.md`

Validation reports:

- `docs/gamedesign/27_scorecards_and_validation_reports.md`

This milestone is intentionally minimal: it establishes a safe patch/apply loop. Higher-level “agents that decide what patch to apply” are out of scope here; those can be built on top once apply/validate is reliable.

## Plan of Work

Define a minimal, explicit patch language for scenes (either as a JSON document or a subset of the existing blueprint spec) that supports the operations needed for iterative generation:

- add/remove/edit a pinned instance file,
- edit a layer definition file,
- (optionally) add/remove portals and markers, if already represented in sources.

Implement two steps:

1) Validate: parse the patch, compute a predicted impact (budgets, references), and fail fast with a `ValidationReport` if it violates hard gates.
2) Apply: write the patch into the authoritative `src/` tree, canonicalize changed files, recompile layers, and re-run validation. Return a structured summary of what changed.

Expose validate/apply via automation endpoints so integration tests can confirm behavior end-to-end.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

This milestone is accepted when:

- There is an automation integration test that applies a patch to a fixture and verifies:
  - deterministic compilation signature after apply,
  - expected `src/` file diffs only (no unrelated churn),
  - validation results match expectations.
- Applying the same patch twice with the same `request_id` is idempotent (no duplicate instances).

## Idempotence and Recovery

- Apply must be retry-safe under `request_id`.
- If apply fails mid-way (crash), the next milestone will introduce run/checkpoint semantics; for now, keep apply as small and atomic as possible (write to a temp dir then rename, or equivalent).

## Interfaces and Dependencies

Use existing dependencies:

- `serde` / `serde_json` for patch documents and responses.

Prefer to implement patch validation/apply as pure operations over `SceneSources` + patch, with IO at the boundary. This makes it easier to unit test and to reuse later for a hosted `/v2/` API.

