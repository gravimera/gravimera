# Refactor: Split `src/gen3d/ai/agent_loop/mod.rs` Into Focused Modules

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D’s agent loop code previously lived in one very large file: `src/gen3d/ai/agent_loop.rs` (~6k+ lines). This made changes risky (hard to review) and slowed down navigation and debugging.

After this refactor, the Gen3D agent loop should behave identically, but the code should be split into smaller, focused modules (prompt building, parsing helpers, tool dispatch, render capture, etc.). A developer should be able to find and edit an area without scrolling thousands of lines.

The change is “done” when:

- `src/gen3d/ai/agent_loop/mod.rs` no longer contains the bulk of the logic (moved into focused modules).
- The resulting module layout is clear and minimal, and the public surface of `agent_loop` remains small (still primarily `poll_gen3d_agent` and `spawn_agent_step_request`).
- The game starts successfully with the UI smoke test.
- Unit tests compile and run (at least `cargo test`).

## Progress

- [x] (2026-02-28) Write this ExecPlan.
- [x] Move `src/gen3d/ai/agent_loop.rs` to `src/gen3d/ai/agent_loop/mod.rs` (no behavior change).
- [x] Extract prompt + state-summary helpers into `src/gen3d/ai/agent_prompt.rs`.
- [x] Extract agent-step parsing helpers into `src/gen3d/ai/agent_parsing.rs`.
- [x] Extract review image selection + sizing helpers into `src/gen3d/ai/agent_review_images.rs`.
- [x] Extract regen-budget helpers into `src/gen3d/ai/agent_regen_budget.rs`.
- [x] Extract step polling/execution state machine into `src/gen3d/ai/agent_step.rs`.
- [x] Extract tool dispatch into `src/gen3d/ai/agent_tool_dispatch.rs`.
- [x] Extract tool polling into `src/gen3d/ai/agent_tool_poll.rs`.
- [x] Extract component-batch handling into `src/gen3d/ai/agent_component_batch.rs`.
- [x] Extract render/snapshot capture helpers into `src/gen3d/ai/agent_render_capture.rs`.
- [x] Extract review-delta call wiring into `src/gen3d/ai/agent_review_delta.rs`.
- [x] Extract misc agent helpers into `src/gen3d/ai/agent_utils.rs`.
- [x] Keep / relocate `#[cfg(test)]` tests so they still cover parsing and image-selection logic.
- [x] Run `cargo test`.
- [x] Run UI smoke test (`cargo run -- --rendered-seconds 2` with a temp `GRAVIMERA_HOME`).
- [x] Update `docs/refactor_todo.md` to check off the item.
- [ ] Commit with a clear message.

## Surprises & Discoveries

- On macOS, linking can fail if the active Xcode install has not had its license accepted.
  Workaround for local runs: `DEVELOPER_DIR=/Library/Developer/CommandLineTools` (uses CLT SDK).

## Decision Log

- Decision: Keep `agent_loop` as a directory module (`src/gen3d/ai/agent_loop/mod.rs`) with a small public surface, but extract most logic into focused sibling modules under `src/gen3d/ai/agent_*.rs`.
  Rationale: The extracted pieces are used across tool dispatch + tool polling + render capture; keeping them as sibling modules avoids deep module nesting while still shrinking `agent_loop/mod.rs`.
  Date/Author: 2026-02-28 / Codex

## Outcomes & Retrospective

- `agent_loop` entry points remain `poll_gen3d_agent` and `spawn_agent_step_request`.
- The original ~6k line file is reduced to ~700 lines in `src/gen3d/ai/agent_loop/mod.rs`.
- `cargo test` passes and the rendered UI smoke test exits successfully.

## Context and Orientation

Gen3D AI code lives under `src/gen3d/ai/`.

Key files for this refactor:

- `src/gen3d/ai/mod.rs`: Owns the overall Gen3D job state and calls `agent_loop::poll_gen3d_agent`.
- `src/gen3d/ai/agent_loop/mod.rs`: The agent-loop entry point (orchestration and wiring).
- `src/gen3d/ai/agent_*.rs`: Extracted logic (prompting, parsing, tool dispatch/polling, render capture, etc.).

Important behavior constraints:

- This refactor must not change Gen3D agent behavior. It should be a mechanical move/extraction only.
- The repo’s required smoke test uses a real rendered UI session:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Plan of Work

First, convert `agent_loop` into a directory module (`agent_loop/mod.rs`) without changing any code behavior. Then, extract cohesive chunks into focused modules by moving code (not rewriting it) and adjusting imports/paths.

Keep `poll_gen3d_agent` and `spawn_agent_step_request` in `agent_loop/mod.rs` as the entry points. All extracted helpers should be `pub(super)` only when needed by `mod.rs` or sibling modules.

Keep the existing unit tests, but relocate them to remain under the `agent_loop` module so they can access internal helpers via `pub(super)` where required.

## Concrete Steps

All commands run from the repository root.

1) Move the module to a directory module:

    mkdir -p src/gen3d/ai/agent_loop
    git mv src/gen3d/ai/agent_loop.rs src/gen3d/ai/agent_loop/mod.rs

2) Add submodules in `src/gen3d/ai/agent_loop/mod.rs` and create new files:

    src/gen3d/ai/agent_prompt.rs
    src/gen3d/ai/agent_parsing.rs
    src/gen3d/ai/agent_review_images.rs
    src/gen3d/ai/agent_regen_budget.rs
    src/gen3d/ai/agent_step.rs
    src/gen3d/ai/agent_tool_dispatch.rs
    src/gen3d/ai/agent_tool_poll.rs
    src/gen3d/ai/agent_component_batch.rs
    src/gen3d/ai/agent_render_capture.rs
    src/gen3d/ai/agent_review_delta.rs
    src/gen3d/ai/agent_utils.rs

3) Compile and validate:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance requires:

- `cargo test` succeeds.
- The UI smoke run exits with code 0 and creates a window (logs show “Creating new window …”).
- There are no behavior changes expected; success is demonstrated by compilation + tests + smoke run.

## Idempotence and Recovery

This refactor is intended to be mechanical and reversible:

- If a split step breaks compilation, revert to the last passing commit and extract fewer functions per step.
- Avoid renaming types/fields or changing logic; prefer pure moves and import-path updates.
