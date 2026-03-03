# ExecPlan: Standalone Intelligence Service (Standalone Brains)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Gravimera needs a way to run “brain code” for units in a separate process (a standalone **intelligence service**) so creators can ship advanced AI logic without embedding arbitrary code into the simulation host process.

After this change, you can:

- run a local intelligence service sidecar binary,
- enable the host integration in Gravimera,
- attach a standalone brain to a unit, and
- see that unit move using **host-authoritative** commands produced by the service (the service requests actions; the host validates and applies them).

The proof that it works is an observable behavior: with the feature enabled, a debug unit visibly moves under service control in rendered mode, and the sidecar protocol can be exercised by automated tests.

## Progress

- [x] (2026-03-04) Write and check in this ExecPlan.
- [x] (2026-03-04) Add protocol types (JSON) and budget limits with unit tests.
- [x] (2026-03-04) Implement `gravimera_intelligence_service` sidecar with demo brain module(s) and `/v1/*` endpoints.
- [x] (2026-03-04) Implement host client + Bevy plugin to tick brains and apply outputs as `MoveOrder` (host-authoritative).
- [x] (2026-03-04) Add an automation-friendly way to attach/detach brains (config `debug_spawn_unit`) and a regression test.
- [x] (2026-03-04) Run `cargo test` + rendered smoke test; record key transcripts here.

## Surprises & Discoveries

- Observation: The repo was a binary-only crate; adding a sidecar binary that shares protocol types required introducing a library target (`src/lib.rs`) and turning `src/main.rs` into a thin wrapper.
  Evidence: `src/lib.rs` now exists and `src/main.rs` calls `gravimera::run()`.

## Decision Log

- Decision: Use HTTP/1.1 + JSON over localhost for the initial sidecar transport, implemented with `std::net::TcpStream` (client) and `tiny_http` (server).
  Rationale: The repo already depends on `tiny_http` for the Automation API; avoiding new dependencies keeps iteration fast while still matching the spec’s “transport-agnostic” requirement (the message schema is the real contract).
  Date/Author: 2026-03-04 / Codex

- Decision: Implement an end-to-end “movement only” MVP first (service outputs `move_to`, host applies as `MoveOrder`), then expand to additional command types (combat/interact/speech) and perception events.
  Rationale: Movement provides a clear, low-risk, observable behavior that validates the lifecycle + tick pipeline without requiring the full gameplay command surface immediately.
  Date/Author: 2026-03-04 / Codex

- Decision: Introduce a library target to share the protocol between the game binary and the sidecar binary.
  Rationale: Separate Cargo binary targets are separate crates; without a library, the sidecar would need to duplicate the protocol types and risk drift.
  Date/Author: 2026-03-04 / Codex

## Outcomes & Retrospective

(Fill in at completion.)

## Context and Orientation

### Source of truth

The final-target design is specified in:

- `docs/gamedesign/06_brains_and_ai.md`
- `docs/gamedesign/38_intelligence_service_spec.md`

Key design points to uphold:

- The intelligence service is **untrusted** by default; it can only request actions.
- The simulation host is always authoritative and must validate and budget-limit requests.
- Brains run as instances managed by the service and are ticked via a bounded `TickInput` / `TickOutput` contract.
- The host should support batching (`tick_many`) and (eventually) async/latency-tolerant control (action horizons).

### Relevant code today

Gravimera currently has “commandable” units and host-side movement orders:

- `src/types.rs`: `Commandable`, `MoveOrder` (path/target).
- `src/rts.rs`: `execute_move_orders` applies `MoveOrder` to unit `Transform` each frame in rendered mode.
- `src/automation/mod.rs`: local Automation HTTP API (uses `tiny_http`).

Standalone brain MVP implementation added by this ExecPlan:

- `src/intelligence/protocol.rs`: protocol types + clamps for budgets.
- `src/bin/gravimera_intelligence_service.rs`: sidecar process implementing `/v1/health`, `/v1/load_module`, `/v1/spawn`, `/v1/tick_many`, `/v1/despawn`.
- `src/intelligence/host_plugin.rs`: rendered-mode Bevy plugin that can spawn a debug brain-driven unit and apply `move_to` as `MoveOrder`.

## Plan of Work

Implement the feature as a small set of incremental milestones that keep the game bootable at every step.

First, define the protocol types as Rust structs with `serde` and hard caps for all variable-length lists (events, nearby entities, commands). Include a `protocol_version` field and stable ids as strings (UUIDs) so the contract is explicit and easy to debug.

Second, add a new binary `gravimera_intelligence_service` that implements the service side of the lifecycle:

- `GET /v1/health`
- `POST /v1/load_module` (optional; initially supports only built-in demo module ids)
- `POST /v1/spawn`
- `POST /v1/tick_many`
- `POST /v1/despawn`

The service stores per-brain-instance state in memory and returns bounded `TickOutput`. For the demo module, implement a deterministic “orbit” or “square patrol” behavior based on the provided RNG seed and tick index.

Third, implement the host integration:

- A Bevy plugin that, when enabled by config, connects to the sidecar, spawns brain instances for entities tagged with a new component (e.g., `StandaloneBrain`), and on each frame produces `TickInput` and consumes `TickOutput`.
- Apply `move_to` outputs by inserting/updating `MoveOrder` on the controlled unit (the host remains authoritative; we clamp and validate).
- Enforce budgets and capabilities at the host boundary (for MVP: only allow `brain.move`).

Fourth, make the system testable:

- Provide a way to attach a standalone brain for a unit without manual UI work. Prefer a config-driven “spawn a debug unit with brain attached” in rendered mode, and optionally add an Automation API endpoint for attaching/detaching brains.
- Add integration tests that spawn the sidecar binary and exercise `/v1/health` and `/v1/tick_many` deterministically.

Throughout, keep `docs/gamedesign/38_intelligence_service_spec.md` as the design reference, and update any implementation-facing docs under `docs/` (not the root `README.md`) when behavior or configuration changes.

## Concrete Steps

All commands are run from the repo root (`C:\\Users\\flow\\github\\gravimera`).

1) Implement protocol + service + host integration in small commits:

    - Edit/add Rust files under `src/` and new binary under `src/bin/`.
    - Add tests under `tests/` for sidecar protocol.

2) Run unit/integration tests:

    - `cargo test`

3) Run the required rendered smoke test (no `--headless`):

    - In PowerShell:
        - `$tmpdir = Join-Path $env:TEMP (\"gravimera_smoke_\" + [guid]::NewGuid().ToString())`
        - `New-Item -ItemType Directory -Force -Path $tmpdir | Out-Null`
        - `$env:GRAVIMERA_HOME = Join-Path $tmpdir \".gravimera\"`
        - `cargo run -- --rendered-seconds 2`

## Validation and Acceptance

Acceptance is behavior-focused:

- With the feature disabled (default), the game still starts in rendered mode and exits cleanly with `--rendered-seconds 2`.
- `cargo test` passes.
- The sidecar binary starts and responds to `GET /v1/health` with `ok=true` and the expected `protocol_version`.
- When the feature is enabled and a unit has a standalone brain attached, the unit moves in a deterministic pattern driven by the service’s `TickOutput` (as observed in rendered mode or via a state snapshot endpoint if added).

## Idempotence and Recovery

- The sidecar is designed to be restartable: if the service is unavailable, the host plugin should fail closed (disable standalone brains and log a clear error) rather than crashing the game.
- The protocol is versioned; incompatible versions should be rejected with an explicit error.

## Artifacts and Notes

- `cargo test` (2026-03-04): PASS (includes `tests/intelligence_service_smoke.rs`).
- Rendered smoke test (2026-03-04): PASS (`cargo run -- --rendered-seconds 2` with a fresh `GRAVIMERA_HOME`).

Quick manual demo (rendered mode):

- Terminal A: `cargo run --bin gravimera_intelligence_service`
- Terminal B: create `~/.gravimera/config.toml`:

    [intelligence_service]
    enabled = true
    addr = "127.0.0.1:8792"
    debug_spawn_unit = true

  Then run: `cargo run`

## Interfaces and Dependencies

### New Rust modules

- `crate::intelligence::protocol`: serde types for `TickInput`, `TickOutput`, lifecycle requests/replies, errors, and budget caps.
- `crate::intelligence::sidecar_client`: host-side HTTP client for the intelligence service (local sidecar).
- `crate::intelligence::host_plugin`: rendered-mode Bevy plugin that produces tick inputs and applies outputs as host-authoritative commands.

### New binaries

- `src/bin/gravimera_intelligence_service.rs`: standalone service process implementing the `/v1/*` endpoints and demo brain modules.

---

Plan revision note (2026-03-04): Updated Progress/Context/Artifacts to reflect the MVP implementation and documented the library-target change needed to share protocol types across binaries.
