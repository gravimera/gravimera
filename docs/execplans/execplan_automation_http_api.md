# ExecPlan: Add Local HTTP Automation API (Codex/Agent-Friendly Testing)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gravimera can run an optional local HTTP service (bound to `127.0.0.1`) that exposes “game actions” (select, move, fire, build, Gen3D triggers, etc.) and observability (state snapshots, screenshots). This enables coding agents (Codex or similar) to drive the game and run automated smoke/regression checks after any change, with no developer hand-holding.

User-visible behavior:

- When automation is enabled in `config.toml` (or via CLI flags), the game logs an “Automation API listening …” line on startup.
- When `disable_local_input=true`, keyboard/mouse input does not affect the game, so an agent-run test is not disturbed by accidental clicks/keys.
- A local process can call the API to:
  - query state,
  - select units,
  - issue move/fire orders,
  - step/pause the simulation (for deterministic testing),
  - capture screenshots (rendered mode),
  - shut the game down cleanly.

## Progress

- [x] (2026-02-03) Create the initial ExecPlan.
- [ ] (2026-02-03) Add automation config parsing + CLI overrides.
- [ ] (2026-02-03) Implement the HTTP server thread and the Bevy main-thread command bridge.
- [ ] (2026-02-03) Implement minimal v1 endpoints: health, state, select-by-ids, move-to-point, fire-to-point, set mode, pause/resume/step, shutdown.
- [ ] (2026-02-03) Add a headless integration smoke test that launches the game with automation enabled and validates `/v1/health` and `/v1/shutdown`.
- [ ] (2026-02-03) Document the API + config in `README.md` and `config.example.toml`.
- [ ] (2026-02-03) Run `cargo test` and a headless smoke run; commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Expose stable instance identifiers in the API using the existing `ObjectId` (UUID v4 stored as `u128`) component on entities.
  Rationale: Bevy `Entity` ids are not stable across runs; `ObjectId` is already persisted in `scene.dat` and used for Gen3D saves.
  Date/Author: 2026-02-03 / Codex

- Decision: Use synchronous HTTP server (`tiny_http`) on a dedicated thread; communicate with Bevy via a channel and process commands on the main thread.
  Rationale: Keep dependencies and runtime complexity low (no Tokio), avoid touching ECS from other threads, and keep behavior easy to debug.
  Date/Author: 2026-02-03 / Codex

- Decision: Use blocking request semantics for “quick” actions and explicit async semantics only for long-running jobs (Gen3D).
  Rationale: Keeps the API easy to use from simple scripts; avoids needing job polling for common actions.
  Date/Author: 2026-02-03 / Codex

## Outcomes & Retrospective

(Fill in at completion: what worked, what didn’t, what to do next.)

## Context and Orientation

Important existing modules:

- `src/app.rs`: Builds the Bevy `App` for rendered/headless runs and wires systems.
- `src/rts.rs`: Selection, move order issuance, and fire targeting logic (Space + LMB).
- `src/combat.rs`: Player firing + unit attacks (driven by `FireControl` resource and selection).
- `src/build.rs`: Build object placement and deletion.
- `src/gen3d/*`: Gen3D workshop UI and AI job loop.
- `src/types.rs`: Shared ECS components/resources (`ObjectId`, `SelectionState`, `FireControl`, etc.).
- `src/config.rs`: Parses `config.toml` without a TOML parser crate (manual line scanning).

Terminology:

- “Automation API”: a local-only HTTP service used to drive the game in tests.
- “Game Action”: a semantic operation like “select these units” or “move selected units to XZ”. This is intentionally higher-level than raw keyboard/mouse input.
- “Deterministic stepping”: when enabled, the game’s `Time` is advanced by a fixed `dt` only when requested via the API, making tests repeatable.

## Plan of Work

1. Add an `automation` module (`src/automation/mod.rs`) that contains:
   - `AutomationConfig` (derived from `AppConfig`).
   - `AutomationPlugin` that starts/stops the HTTP server.
   - A thread-safe inbox of incoming HTTP requests to be processed on the main thread.
   - A system that drains the inbox each frame and applies requests, replying with JSON.

2. Extend `src/config.rs` to parse a new `[automation]` section (and top-level fallbacks) with keys:
   - `enabled` (bool)
   - `bind` (string, e.g. `127.0.0.1:8791`)
   - `disable_local_input` (bool)
   - `pause_on_start` (bool)
   - `token` (string, optional; if set, require `Authorization: Bearer <token>`)

3. Extend CLI parsing in `src/app.rs` to allow overrides (agent-friendly):
   - `--automation` (enable)
   - `--automation-bind <addr>`
   - `--automation-token <token>`
   - `--automation-disable-local-input`
   - `--automation-pause-on-start`

4. Disable local keyboard/mouse input when automation requires it:
   - Add a run condition `automation::local_input_enabled()` and apply it to systems that read `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>` (selection, move command input, build placement/deletion hotkeys, weapon switching, etc.).
   - Important: keep simulation systems (movement execution, combat, AI, autosave) running normally so the agent can drive them via API actions.

5. Implement v1 endpoints (all JSON):
   - `GET /v1/health`
   - `GET /v1/state` (mode, selection, units list with `instance_id_uuid`, `prefab_id_uuid`, `pos`, `yaw`, basic flags)
   - `POST /v1/select` (select by instance id UUID list)
   - `POST /v1/move` (issue move order for currently selected units to a world point)
   - `POST /v1/fire` (set `FireControl` active + target point or enemy id)
   - `POST /v1/mode` (Build/Play/Gen3D)
   - `POST /v1/pause`, `POST /v1/resume`, `POST /v1/step` (deterministic time control)
   - `POST /v1/shutdown`

6. Add a smoke test:
   - Add `tests/automation_api_smoke.rs` that spawns `target/debug/gravimera` with `--headless` and automation enabled on a fixed port, calls `/v1/health`, then calls `/v1/shutdown`, and asserts the process exits.
   - Keep it minimal: don’t require a GPU or a rendered window.

7. Update documentation:
   - `README.md`: Add a short “Automation API” section with example commands.
   - `config.example.toml`: Add an `[automation]` section.

## Concrete Steps

All commands should be run from the repo root.

1. Build and test:

    cargo test

2. Headless smoke run:

    cargo run -- --headless --headless-seconds 1

3. Automation smoke run (example):

    cargo run -- --headless --headless-seconds 0 --automation --automation-bind 127.0.0.1:8791

   In another terminal:

    curl -s http://127.0.0.1:8791/v1/health
    curl -s -X POST http://127.0.0.1:8791/v1/shutdown

## Validation and Acceptance

This work is accepted when:

- With automation disabled (default), the game behaves exactly as before.
- With automation enabled, the game logs the listen address and responds to `/v1/health`.
- With `disable_local_input=true`, pressing keys/mouse does not change selection/movement/fire state.
- The smoke test passes reliably on machines without a GPU (`cargo test` passes in headless mode).

## Idempotence and Recovery

- The automation server binds to `127.0.0.1` only. If the port is in use, the game should log an error and continue running without automation.
- The test runner should use a dedicated port; if a test fails and leaves a process behind, rerun the test after killing the leftover process.

## Artifacts and Notes

(Fill in with any notable transcripts, logs, or edge cases encountered.)

## Interfaces and Dependencies

Dependencies to add:

- `tiny_http` for a minimal HTTP server.

Core internal interfaces to implement:

- In `src/automation/mod.rs`, define:

    - `enum AutomationRequestKind { ... }`
    - `struct AutomationRequest { id: u64, kind: AutomationRequestKind, reply: Sender<AutomationReply> }`
    - `struct AutomationReply { status: u16, body_json: String }`

And expose:

- `pub(crate) fn local_input_enabled(...) -> bool` (run condition).
