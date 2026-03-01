# Refactor: Modularize `src/app.rs` system wiring into plugins

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

`src/app.rs` currently wires most gameplay, UI, and Gen3D systems directly via many `app.add_systems(...)` calls, making it hard to see feature boundaries or safely edit scheduling without missing a call site.

After this refactor, runtime behavior should remain identical, but the wiring should be split into small Bevy `Plugin` types grouped by feature area (startup, scene runtime, UI, Gen3D, gameplay). `run_rendered(...)` should read as high-level composition (resources + base Bevy plugins + feature plugins), rather than a long list of system registrations.

The change is “done” when:

- `run_rendered(...)` delegates most system wiring to plugins.
- Plugins are grouped by feature area and live outside `src/app.rs`.
- `cargo test` passes.
- The rendered UI smoke test starts and exits cleanly.

## Progress

- [x] (2026-03-01) Write this ExecPlan.
- [x] (2026-03-01) Extract rendered-mode system wiring into `Plugin` types (startup, scene runtime, UI, Gen3D, gameplay).
- [x] (2026-03-01) Replace the large `run_rendered(...)` `add_systems` list with `add_plugins(...)` calls to those new plugins.
- [x] (2026-03-01) Run `cargo test`.
- [x] (2026-03-01) Run UI smoke test (`cargo run -- --rendered-seconds 2` with a temp `GRAVIMERA_HOME`, rendered mode).
- [x] (2026-03-01) Update `docs/refactor_todo.md` to check off the item.
- [x] (2026-03-01) Commit with a clear message.

## Surprises & Discoveries

- Extracting the wiring required removing a very large contiguous `add_systems(...)` block from `src/app.rs`.
  Evidence: `cargo test` + rendered smoke test still passed after replacing it with plugin composition.

## Decision Log

- Decision: Implement rendered-mode wiring plugins in a new module (`src/app_plugins.rs`) and keep `run_rendered(...)` focused on high-level composition.
  Rationale: This minimizes churn while still making scheduling boundaries explicit and testable.
  Date/Author: 2026-03-01 / Codex

## Outcomes & Retrospective

- `run_rendered(...)` now delegates most system wiring to a small set of feature plugins, making `src/app.rs` substantially easier to scan.
- The new plugins live in `src/app_plugins.rs` and preserve the original scheduling constraints (`after(...)`, `before(...)`, `run_if(...)`).

## Context and Orientation

Rendered mode is built in `src/app.rs` in `run_rendered(...)`.

Today, `run_rendered(...)`:

- Initializes resources/state for gameplay + UI + Gen3D.
- Adds Bevy `DefaultPlugins` and platform-specific render setup.
- Registers a large number of startup/update systems for gameplay, UI, scene authoring, Gen3D, and save/autosave.

The goal is to move most of those `add_systems` calls into feature plugins so the high-level wiring is easier to read and maintain.

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
