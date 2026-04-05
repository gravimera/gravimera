# Build a Utopian Chrome Showcase Scene (Automation API)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Create a large, visually dense, **future-fiction interstellar** city showcase scene in the user’s **default** Gravimera data directory (`~/.gravimera`). The scene should be usable later from the in-game Scenes list without requiring this repo’s `test/` artifacts.

The scene theme:

- Clean, utopian chrome architecture with subtle “interstellar travel era” cues.
- Multiple planetary species living together: varied robots, aliens, civilians.
- A reasonable city layout: streets + plazas, tall + small buildings, ground + air vehicles.
- Units should have existing built-in brains attached (no new brains are added in this plan).

This plan also produces a resumable automation driver and a small set of new Automation HTTP API endpoints to make the build controllable (terrain selection + camera control + prefab grounding metadata).

## Progress

- [x] (2026-04-05) Write this ExecPlan and commit it.
- [x] (2026-04-05) Add missing Automation HTTP API endpoints (scene terrain select, camera, prefab grounding metadata) and update `docs/automation_http_api.md`.
- [x] (2026-04-05) Add a resumable scene builder tool under `tools/` that runs the game in **release** with Automation enabled and generates the showcase scene in `~/.gravimera`.
- [x] (2026-04-05) Run the rendered smoke test (`--rendered-seconds 2`) and commit code+docs changes.
- [ ] (2026-04-05) Run the scene builder, produce screenshots under `test/run_1/...`, and verify the scene exists under `~/.gravimera/realm/default/scenes/<scene_id>/`.

## Surprises & Discoveries

- GenFloor may generate a small terrain by default (observed 60m x 60m), which can cause large city layouts to spill into the void; the builder should enforce a minimum terrain size and/or scale placements to the chosen terrain.

## Decision Log

- Decision: Use a **versioned** scene id derived from date, e.g. `showcase_scene_20260405_v1`.
  Rationale: Avoid collisions with existing user scenes; allow multiple iterations.
  Date/Author: 2026-04-05 / Codex.

- Decision: Use `cargo run --release` for long automation sessions.
  Rationale: User explicitly requested smooth runtime; Gen3D/GenFloor runs are heavy.
  Date/Author: 2026-04-05 / Codex.

- Decision: Persist generation progress using two independent mechanisms:
  1) a repo-local run directory under `test/run_1/...` (logs, screenshots, manifest), and
  2) the engine’s durable scene run step artifacts under `~/.gravimera/realm/default/scenes/<scene_id>/runs/<run_id>/steps/...`.
  Rationale: Resumability across crashes/outages without requiring the running game process to survive.
  Date/Author: 2026-04-05 / Codex.

## Outcomes & Retrospective

- (Fill in once the scene is generated and verified.)

## Context and Orientation

Key repo docs:

- Automation HTTP API reference: `docs/automation_http_api.md`
- Scene sources and deterministic layers: `docs/gamedesign/30_scene_sources_and_build_artifacts.md`
- Scene sources patch format: `docs/gamedesign/31_scene_sources_patch_v1.md`

Key engine concepts used by the builder:

- A “realm” is the top-level world package under `~/.gravimera/realm/<realm_id>/`.
- A “scene” lives at `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/`.
- Authoritative **scene sources** live under `src/` inside that scene folder.
- Derived build outputs live under `build/`.
- The Automation API can import scene sources (`POST /v1/scene_sources/import`) and then apply durable “run steps”
  (`POST /v1/scene_sources/run_apply_patch`) that write artifacts under `scenes/<scene_id>/runs/<run_id>/steps/...`.

Why new Automation endpoints are needed:

- Terrain selection is stored as a scene build artifact (`build/terrain.grav`) and needs a semantic “set active scene terrain” endpoint.
- The existing screenshot endpoint captures the current camera; we need camera control to produce useful overview and detail shots without manual input.
- Scene sources layers require correct `translation.y` grounding; exposing `ground_origin_y` per prefab allows the external builder to write grounded transforms deterministically.

## Plan of Work

### 1) Extend Automation HTTP API (small, targeted additions)

Implement:

1. `POST /v1/scene/terrain/select`
   - Request: `{ "floor_id_uuid": "<uuid>" }` or `{ "floor_id_uuid": null }`
   - Behavior:
     - Persist the selection for the **active** realm+scene (same data the UI uses).
     - Apply it immediately to the running world (updates active floor and the Terrain list selection).
   - Errors:
     - `400` invalid UUID
     - `404` floor id not found in realm floor library
     - `501` not available (headless / missing resources)

2. `GET /v1/camera`
   - Return current camera focus + yaw/pitch/zoom (enough to reproduce a shot).

3. `POST /v1/camera`
   - Request: `{ "focus":[x,y,z]?, "yaw": f32?, "pitch": f32?, "zoom_t": f32? }`
   - Behavior: update the corresponding camera resources; changes apply on the next frame.

4. Extend `GET /v1/prefabs`
   - Add `ground_origin_y` field (float) so external tools can compute grounded `translation.y`.

Update `docs/automation_http_api.md` to document these endpoints.

### 2) Add a resumable builder tool (`tools/showcase_scene_builder.py`)

The tool is an “external agent” that:

- starts the game in rendered mode via `cargo run --release` with Automation enabled (bind `127.0.0.1:0`),
- creates a versioned scene id under the default `~/.gravimera`,
- builds a flat chrome plaza terrain via GenFloor,
- generates many prefabs via Gen3D task queue (buildings, roads, vehicles, robots, aliens, props),
- applies placements as `scene_sources` layers using durable `run_apply_patch` steps,
- switches to Play mode to let built-in brains attach to commandable units,
- captures periodic screenshots (overview + detail) into the run dir under `test/run_1/...`,
- writes a resumable `manifest.json` so reruns can skip completed assets/steps.

The tool must never print secrets. It should not modify `~/.gravimera/config.toml`.

### 3) Validate and commit

After code changes:

- Run the required rendered smoke test (isolated home):

      tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

- Commit with a clear message, for example:
  - `automation: add terrain select + camera endpoints for scene builder`

### 4) Run the builder and verify the scene

Run the builder (example):

    python3 tools/showcase_scene_builder.py --run-dir test/run_1/showcase_scene_20260405_v1

Verify outputs:

- `test/run_1/showcase_scene_*/shots/*.png` exist and show a utopian chrome city.
- `~/.gravimera/realm/default/scenes/<scene_id>/` exists.
- The scene appears in the in-game Scenes list and loads without crashing.

## Concrete Steps

1) Build + run the game in release with automation (the tool does this).

2) If running manually for debugging:

   - Start:

         cargo run --release -- \
           --automation --automation-bind 127.0.0.1:8791 --automation-pause-on-start

   - Health:

         curl -s http://127.0.0.1:8791/v1/health

3) Run the tool:

       python3 tools/showcase_scene_builder.py --run-dir test/run_1/showcase_scene_run_01

4) If interrupted (network outage / crash):

   - Re-run the same command with the same `--run-dir`. The tool should read its manifest and continue.

## Validation and Acceptance

Minimum acceptance:

- A new scene exists under `~/.gravimera/realm/default/scenes/showcase_scene_*/`.
- The scene contains many buildings, props, units, and vehicles consistent with the theme.
- Units have brains attached by existing engine logic (verified by switching to Play mode and observing movement/behavior).
- Screenshots captured during the build exist under `test/run_1/...` and show both an overview and several detail shots.

## Idempotence and Recovery

- Scene source mutations must be applied via `POST /v1/scene_sources/run_apply_patch` with a stable `run_id` and monotonically increasing `step`.
- If the tool is re-run with an existing `run_id`, it must query `POST /v1/scene_sources/run_status` and continue from `next_step`.
- Gen3D asset generation must be tracked in `manifest.json` so completed prefab ids are reused across restarts.
