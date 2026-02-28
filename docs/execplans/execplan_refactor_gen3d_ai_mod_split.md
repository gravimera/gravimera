# Refactor: Reduce `src/gen3d/ai/mod.rs` “God Module” Surface

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D’s AI module (`src/gen3d/ai/mod.rs`) previously mixed many unrelated concerns in one very large file: core job state, orchestration, render capture helpers, tool feedback recording, and various utility helpers.

After this refactor, the runtime behavior should remain identical, but `mod.rs` should become a thin module boundary (submodule declarations, a few re-exports, and small glue). The heavy logic and large type blocks should move into focused submodules so the code is easier to navigate and review.

The change is “done” when:

- `src/gen3d/ai/mod.rs` is mostly module declarations + re-exports + minimal glue.
- The extracted submodules compile without widening the public surface unnecessarily (prefer `pub(super)` visibility).
- Unit tests pass (`cargo test`).
- The rendered UI smoke test starts the game and exits cleanly.

## Progress

- [x] (2026-02-28) Write this ExecPlan.
- [x] Extract Gen3D AI job state and shared types into `src/gen3d/ai/job.rs`.
- [x] Extract orchestration + helpers into `src/gen3d/ai/orchestration.rs`.
- [x] Replace `src/gen3d/ai/mod.rs` with a thin wrapper that declares submodules, re-exports the public entry points, and keeps internal `super::...` call sites working via small glue imports/wrappers.
- [x] Fix module path + visibility issues caused by the split (notably: references to Gen3D constants in `crate::gen3d`, and job/metrics helper methods that were previously same-module-private).
- [x] Run `cargo test`.
- [x] Run UI smoke test (`cargo run -- --rendered-seconds 2` with a temp `GRAVIMERA_HOME`, rendered mode).
- [x] Update `docs/refactor_todo.md` to check off the item.
- [x] Commit with a clear message.

## Surprises & Discoveries

- Some `#[cfg(test)]` modules referenced schema types via `super::AiJointJson` / `super::super::AiContactJson`, relying on older `mod.rs` imports. After the split, those names were no longer in the `gen3d::ai` module scope.
  Evidence: `cargo test` failed with unresolved `AiContactJson` / `AiJointJson` until `#[cfg(test)]` imports were restored in `src/gen3d/ai/mod.rs`.

## Decision Log

- Decision: Split `src/gen3d/ai/mod.rs` into two primary submodules (`job.rs` for state/types, `orchestration.rs` for lifecycle + helpers) and keep `mod.rs` as a thin wrapper.
  Rationale: This is a mechanical extraction that reduces file size and cognitive load without changing behavior or introducing new abstractions.
  Date/Author: 2026-02-28 / Codex

- Decision: Keep internal call sites that use `super::helper_fn` working by importing selected helpers into the `gen3d::ai` module scope (instead of rewriting all paths).
  Rationale: Minimizes churn across the many existing Gen3D agent modules while still shrinking `mod.rs`.
  Date/Author: 2026-02-28 / Codex

## Outcomes & Retrospective

- `src/gen3d/ai/mod.rs` is now a small wrapper that declares modules and re-exports the Gen3D AI entry points.
- The large type/state block lives in `src/gen3d/ai/job.rs`.
- The orchestration + helper functions live in `src/gen3d/ai/orchestration.rs`.
- `cargo test` passes and the rendered UI smoke test exits successfully.

## Context and Orientation

Gen3D code lives under `src/gen3d/`.

Key files for this refactor:

- `src/gen3d/ai/mod.rs`: The Gen3D AI module boundary (now thin glue + re-exports).
- `src/gen3d/ai/job.rs`: Job state, shared types, and internal metrics helpers.
- `src/gen3d/ai/orchestration.rs`: Build lifecycle, orchestration helpers, render capture wiring, tooling feedback recording, and API entry points.

Important constraints:

- This is a refactor: behavior should remain identical.
- Gen3D algorithm rule still applies: no heuristic algorithms; keep deterministic/schema-driven behavior.
- Required smoke test uses a real rendered UI session (no `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Concrete Steps

All commands run from the repository root.

1) Run unit tests:

    DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo test

2) Run the UI smoke test (rendered):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" DEVELOPER_DIR=/Library/Developer/CommandLineTools cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance requires:

- `cargo test` succeeds.
- The smoke run exits with code 0 and logs show “Creating new window …”.

