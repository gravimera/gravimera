# ExecPlan: Automation Input Events + Gen3D Regression Tests

> Status note (2026-02-07): The low-level input injection endpoints (`/v1/input/*`) described in this plan were removed. Prefer the semantic Automation API endpoints (`/v1/select`, `/v1/move`, `/v1/fire`, `/v1/mode`, `/v1/gen3d/*`, etc.).

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gravimera’s local Automation HTTP API can inject synthetic keyboard and mouse input (cursor move, button press/release, wheel scroll, key press/release). This enables a local AI agent to drive the game “like a human” (click UI, drag-select units, issue RMB move orders, aim/fire) while the real keyboard/mouse input is ignored so the developer can keep using the computer normally.

In addition, we keep a “real rendered Gen3D” regression test plan and scripts that repeatedly:

1) build a model via Gen3D from a prompt,
2) save it into the world,
3) move it around and attack,
4) capture screenshots into the run’s `gen3d_cache/<run_id>/…` directory for review.

User-visible behavior:

- When `automation.disable_local_input=true`, real OS keyboard/mouse input no longer affects the game, but injected input via HTTP does.
- A local process can query the window size/cursor state and send synthetic input events to interact with UI and gameplay.
- The existing semantic endpoints (`/v1/select`, `/v1/move`, `/v1/fire`, `/v1/gen3d/*`) continue to work.
- Gen3D LLM reasoning effort defaults to `high` for all steps unless explicitly overridden, to improve consistency.

## Progress

- [ ] (2026-02-06) Write and check in the ExecPlan.
- [ ] (2026-02-06) Add Automation input injection data model and API endpoints.
- [ ] (2026-02-06) Mask local OS input when `disable_local_input=true` while still running normal input-driven systems.
- [ ] (2026-02-06) Update `docs/automation_http_api.md` with the new endpoints + examples.
- [ ] (2026-02-06) Add/refresh a living Gen3D regression test plan doc and update `tools/gen3d_real_test.py` to be robust to long builds.
- [ ] (2026-02-06) Run real rendered Gen3D regressions for: warcar, soldier, horse, knight on horse. Record run ids and any issues.
- [ ] (2026-02-06) Run `cargo test` and a short smoke run; commit.

## Surprises & Discoveries

- Observation: (fill in during implementation)
  Evidence: (paths / short transcripts)

## Decision Log

- Decision: Use an “input masking + injection” system that runs in `PreUpdate` after Bevy’s `InputSystems`.
  Rationale: Bevy already converts platform events into `ButtonInput<…>` resources and `KeyboardInput`/`MouseWheel` messages. Masking after `InputSystems` lets us reliably ignore OS input (and drain OS input messages) while applying injected input in the same frame before gameplay systems run.
  Date/Author: 2026-02-06 / Codex

- Decision: Keep existing semantic automation endpoints.
  Rationale: Semantic endpoints are faster and less brittle for most tests; injected input is for UI-like flows and parity with human control, not a replacement.
  Date/Author: 2026-02-06 / Codex

- Decision: Default Gen3D reasoning effort to `high` for all Gen3D steps (plan/agent-step/component/review/repair) and default `openai.model_reasoning_effort` to `high`.
  Rationale: The system is currently more sensitive to incomplete / under-reasoned outputs than to token spend; consistent `high` reasoning reduces schema drift and improves assembly/animation consistency.
  Date/Author: 2026-02-06 / Codex

## Outcomes & Retrospective

(Fill in at completion: what worked, what didn’t, what to do next.)

## Context and Orientation

Key code locations:

- `src/automation/mod.rs`: The Automation HTTP server thread and the main-thread request handler (`handle_request_main_thread`).
- `src/app.rs`: Wires systems and currently gates many input-driven systems with `automation::local_input_enabled`.
- `src/player.rs`, `src/rts.rs`, `src/build.rs`, `src/gen3d/*`: Input-driven systems that should become controllable via injected input.
- `tools/gen3d_real_test.py`: A rendered end-to-end Gen3D driver that uses the Automation API.
- `docs/automation_http_api.md`: The human-readable API reference that must be kept in sync.

Terminology:

- “Injected input”: synthetic key/mouse events sent by HTTP that are applied to Bevy input state.
- “Masked input”: real OS keyboard/mouse input is ignored (it does not affect gameplay) when automation is running with `disable_local_input=true`.
- “Deterministic stepping”: in automation mode, `/v1/step` can advance time for a fixed number of frames while virtual time is paused otherwise.

## Plan of Work

First, extend Automation’s runtime state to store an injected input state:

- A persistent cursor position in window logical pixels (`Vec2`), with a remembered last position so we can compute `CursorMoved.delta`.
- A persistent set of “held” keys and “held” mouse buttons.
- A queue of one-shot events (key down/up, mouse down/up, mouse wheel, cursor move) to be applied on the next frame.

Then, implement new Automation endpoints:

- `GET /v1/window`: return window width/height/scale_factor and the engine’s current cursor position (useful for scripting).
- `GET /v1/input/state`: return injected cursor position, held keys/buttons, and queue length.
- `POST /v1/input/reset`: clear injected state (no keys/buttons held, cursor set to window center).
- `POST /v1/input/events`: enqueue one or more input events. This is the main entry point for agents.

Next, mask OS input + apply injected input:

- Add a `PreUpdate` system in `AutomationPlugin` that runs after Bevy’s `InputSystems`. If `automation.enabled && automation.disable_local_input`, it drains OS input messages that gameplay systems would read (`KeyboardInput`, `MouseButtonInput`, `MouseWheel`, `CursorMoved`, `FileDragAndDrop`) and forces `ButtonInput` pressed sets to match the injected held sets (releasing any “OS-held” keys/buttons).
- Apply queued injected events by mutating `ButtonInput<KeyCode>` / `ButtonInput<MouseButton>`, by setting the primary `Window` cursor position, and by emitting `KeyboardInput` / `MouseWheel` / `CursorMoved` messages where needed.

Then, remove the current gating approach:

- Change `automation::local_input_enabled` to return `true` always (or remove the run conditions) and rely on the masking system to prevent local OS input from affecting the game. This allows the injected input to drive the same normal systems that a human would.

Finally, build and run regression tests:

- Keep a living test plan doc under `docs/testplans/` explaining how to run the real rendered Gen3D regression prompts and what to visually inspect (wheel spin axis/direction, leg swing, assembly symmetry, aim vs move facing).
- Update the existing python driver(s) to optionally use injected input for a “human-like” control demo (drag-select + RMB move + Space+LMB fire), while still keeping semantic endpoints for the high-level Gen3D loop.
- Run the 4 prompts and record run ids + any observed issues in `docs/gen3d_real_test_issues.md`.

## Concrete Steps

All commands should be run from the repo root.

1) Build + unit tests:

    cargo fmt
    cargo test

2) Smoke start (headless):

    cargo run -- --headless --headless-seconds 1 --config /Users/flow/.gravimera/config.toml

3) Run rendered Gen3D regressions (writes cache artifacts under `target/debug/gen3d_cache/<run_id>/…`):

    python3 tools/gen3d_real_test.py --config /Users/flow/.gravimera/config.toml --reset-scene \
      --prompt "A warcar with a cannon as weapon" \
      --prompt "A soldier with a gun" \
      --prompt "A horse" \
      --prompt "A knight on a horse"

4) Manual review:

    - Open each run_dir and check `attempt_0/pass_*/render_*.png` for obvious assembly issues.
    - Check `external_screenshots_world/*.png` (spawn + movement) and `external_screenshots_anim/*.png` (attack frames).
    - If wheels/legs are wrong, record the run_id and the specific symptom in `docs/gen3d_real_test_issues.md`.

## Validation and Acceptance

Automation input injection is accepted when:

- With `automation.disable_local_input=true`, typing/clicking in the OS does not affect the game, but:
- A script can:
  - move the cursor and click UI buttons,
  - drag-select units,
  - issue RMB move orders,
  - hold Space and click to aim/fire,
  - and capture screenshots via `/v1/screenshot`.

Gen3D regressions are accepted when:

- For the 4 prompts above, `tools/gen3d_real_test.py` completes without crashing and produces a saved instance for each prompt.
- The captured images show wheels spin around axles and legs swing plausibly, without obvious detached blocks.

## Idempotence and Recovery

- All new endpoints are additive and can be tested independently.
- If a Gen3D build takes too long, `tools/gen3d_real_test.py` should stop the run and attempt a best-effort Save (so we still get artifacts to debug).

## Artifacts and Notes

- `docs/automation_http_api.md`: API reference (must be updated).
- `docs/testplans/gen3d_real_test_plan.md`: living test plan.
- `docs/gen3d_real_test_issues.md`: known failure modes and run ids.
- `target/debug/gen3d_cache/<run_id>/agent_trace.jsonl`: step-by-step agent trace and tool calls.
