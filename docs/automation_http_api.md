# Gravimera Local Automation HTTP API (v1)

Gravimera can expose a **local-only** HTTP API that lets external tools/agents drive the game (select/move/fire/mode/Gen3D/screenshot) for automated testing.

The API is “semantic” (game actions). It intentionally does **not** expose raw keyboard/mouse event injection endpoints; use `/v1/select`, `/v1/move`, `/v1/fire`, `/v1/mode`, etc.

## Enable the server

### CLI

```bash
cargo run -- \
  --automation \
  --automation-bind 127.0.0.1:8791 \
  --automation-disable-local-input \
  --automation-pause-on-start
```

### config.toml

```toml
[automation]
enabled = true
bind = "127.0.0.1:8791"
disable_local_input = true
pause_on_start = true
# token = "CHANGE_ME" # optional; enables Authorization check
```

On startup, the game prints a line like:

```
Automation API listening on http://127.0.0.1:8791
```

Note (Windows): some port ranges may be excluded/reserved by the OS. If the game logs a bind failure like
`os error 10013`, pick a different port (e.g. `127.0.0.1:18791`).

## Authentication (optional)

If `[automation].token` (or `--automation-token`) is set, **every request** must include:

```bash
-H "Authorization: Bearer <token>"
```

Otherwise the API returns `401`:

```json
{"ok":false,"error":"Unauthorized"}
```

## Conventions

- All endpoints return JSON.
- Success responses include `"ok": true`.
- Error responses include `"ok": false` and an `"error"` string (and use appropriate HTTP status codes).
- Positions:
  - Most gameplay actions operate on the XZ plane (`x`, `z`).
  - `y` is vertical and is optional in `/v1/move` (used as “goal ground height” for height-aware pathing).
- IDs:
  - `instance_id_uuid` is a stable UUID for a spawned object (player/unit/building/enemy).
  - `prefab_id_uuid` identifies the prefab definition.

Local input masking notes:

- When `[automation].disable_local_input=true`, Gravimera drains platform keyboard/mouse events so they do not affect gameplay.
  - This is useful for automation rigs where you don’t want the developer’s local input to interfere.

## Time control (important for tests)

`POST /v1/step` is **synchronous**: the HTTP request blocks until the requested frames finish stepping.

Practical guidance:

- Keep `frames` small (e.g. `<= 10`) during heavy operations like Gen3D builds.
- Don’t send another `/v1/step` until the previous call has returned (otherwise it may queue and your HTTP client may time out).
- After `/v1/step`, time remains paused until you call `/v1/resume`.

## Endpoints

### `GET /v1/health`

Check server status.

```bash
curl -s http://127.0.0.1:8791/v1/health
```

Response:

```json
{
  "ok": true,
  "name": "gravimera",
  "version": "0.0.0",
  "automation": {
    "disable_local_input": true,
    "pause_on_start": true,
    "paused": true,
    "listen_addr": "http://127.0.0.1:8791"
  }
}
```

### `GET /v1/window`

Fetch primary window dimensions and cursor position (if available).

```bash
curl -s http://127.0.0.1:8791/v1/window
```

Response (shape):

```json
{
  "ok": true,
  "window_entity": "Entity(0v0)",
  "width": 1280.0,
  "height": 720.0,
  "scale_factor": 2.0,
  "cursor": [640.0, 360.0]
}
```

Notes:

- Returns `501` in headless mode (no window).

### `GET /v1/state`

Fetch a state snapshot: current mode, selection, and object list.

```bash
curl -s http://127.0.0.1:8791/v1/state
```

Response (shape):

```json
{
  "ok": true,
  "mode": "build",
  "build_scene": "realm",
  "selected_instance_ids": ["..."],
  "objects": [
    {
      "instance_id_uuid": "...",
      "prefab_id_uuid": "...",
      "pos": [0.0, 1.0, 0.0],
      "scale": [1.0, 1.0, 1.0],
      "yaw": 0.0,
      "is_player": true,
      "is_enemy": false,
      "is_build_object": false,
      "is_commandable": true,
      "has_attack": true,
      "attack_kind": "ranged_projectile"
    }
  ]
}
```

Notes:

- `build_scene` is only meaningful when `mode="build"` and is one of: `realm`, `preview`.

### `POST /v1/scene_sources/import`

Import a VCS-friendly **scene sources** directory (`src/`) into the running ECS world.

This is intended for automated tests and scene generation tooling.

Request body:

```json
{"src_dir":"/abs/path/to/scenes/<scene_id>/src"}
```

Example:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene_sources/import \
  -H 'Content-Type: application/json' \
  -d '{"src_dir":"/tmp/my_scene/src"}'
```

Response (shape):

```json
{"ok":true,"imported_instances":1,"src_dir":"/tmp/my_scene/src"}
```

Notes:

- Import currently **replaces** all non-player `BuildObject` + `Commandable` instances with the scene’s pinned instances.
- Procedural layers under `src/layers/` are loaded into memory but not compiled until you call `POST /v1/scene_sources/compile`.

### `POST /v1/scene_sources/export`

Export the currently loaded scene back into canonical `src/` sources.

Request body:

```json
{"out_dir":"/abs/path/to/write/src"}
```

Example:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene_sources/export \
  -H 'Content-Type: application/json' \
  -d '{"out_dir":"/tmp/exported_scene/src"}'
```

Response (shape):

```json
{"ok":true,"exported_instances":1,"out_dir":"/tmp/exported_scene/src"}
```

Notes:

- Export requires a prior import in the current session so metadata files (`index.json`, `meta.json`, etc.) can be preserved.
- Returns `409` if no scene sources have been imported yet.
- Export writes **only pinned/unowned** instances. Layer-owned outputs are treated as derived and are not exported as pinned instances.

### `POST /v1/scene_sources/reload`

Reload the last imported `src_dir` from disk into the current session.

This is useful when you:

- edit `src/` files externally (editor/agent), then
- want the running game to pick up the changes without re-importing the world.

Request body:

```json
{}
```

Response:

```json
{"ok":true}
```

Errors:

- `409` if no scene sources directory has been imported yet.

### `POST /v1/scene_sources/compile`

Compile **all** procedural layers from the currently loaded scene sources into concrete ECS instances.

Request body:

```json
{}
```

Response (shape):

```json
{
  "ok": true,
  "spawned": 3,
  "updated": 1,
  "despawned": 0,
  "layers_compiled": 2,
  "pinned_upserts": 1
}
```

Notes:

- Requires a prior `POST /v1/scene_sources/import`.
- If you changed any files on disk, call `POST /v1/scene_sources/reload` first.

### `POST /v1/scene_sources/regenerate_layer`

Regenerate (recompile) a **single** layer by id. This updates only the instances owned by that layer
and leaves pinned instances and other layers untouched.

Request body:

```json
{"layer_id":"layer_a"}
```

Response (shape):

```json
{"ok":true,"layer_id":"layer_a","spawned":0,"updated":1,"despawned":1}
```

### `GET /v1/scene_sources/signature`

Fetch a deterministic signature summary of the current compiled scene instance set.

This is intended for regression tests and determinism gates.

```bash
curl -s http://127.0.0.1:8791/v1/scene_sources/signature
```

Response (shape):

```json
{
  "ok": true,
  "overall_sig": "…",
  "pinned_sig": "…",
  "layer_sigs": { "layer_a": "…", "layer_b": "…" },
  "total_instances": 4,
  "pinned_instances": 1,
  "layer_instance_counts": { "layer_a": 2, "layer_b": 1 }
}
```

### `POST /v1/scene_sources/validate`

Validate the currently loaded scene sources against a **ScorecardSpec** and return a structured
`ValidationReport`.

Request body (ScorecardSpec shape, minimal):

```json
{
  "format_version": 1,
  "hard_gates": [
    { "kind": "schema" },
    { "kind": "budget", "max_instances": 40000, "max_portals": 2000 }
  ]
}
```

Response (shape):

```json
{
  "ok": true,
  "report": {
    "format_version": 1,
    "tick": 0,
    "event_id": 0,
    "scene_id": "hub",
    "hard_gates_passed": false,
    "metrics": { "predicted_total_instances": 123 },
    "violations": [
      {
        "code": "unknown_prefab_id",
        "message": "…",
        "severity": "error",
        "evidence": { "source_path": "pinned_instances/…", "prefab_id": "…" }
      }
    ]
  }
}
```

Notes:

- Requires a prior `POST /v1/scene_sources/import`.
- Validation does not mutate the world or the on-disk sources.

### `POST /v1/scene_sources/patch_validate`

Validate a patch (dry-run) against the currently loaded scene sources.

Request body:

```json
{
  "scorecard": { "format_version": 1, "hard_gates": [{ "kind": "schema" }] },
  "patch": {
    "format_version": 1,
    "request_id": "req_123",
    "ops": [
      {
        "kind": "upsert_layer",
        "layer_id": "layer_a",
        "doc": { "kind": "explicit_instances", "instances": [] }
      }
    ]
  }
}
```

Response (shape):

```json
{
  "ok": true,
  "patch_summary": { "changed_paths": ["layers/layer_a.json"], "derived_instance_ids": {} },
  "validation_report": { "hard_gates_passed": true, "violations": [] }
}
```

Notes:

- Dry-run only: does not write to disk and does not recompile.

### `POST /v1/scene_sources/patch_apply`

Apply a patch by mutating the authoritative `src/` files, then recompiling all layers.

Request body: same as `patch_validate`.

Response (shape):

```json
{
  "ok": true,
  "applied": true,
  "patch_summary": { "changed_paths": ["layers/layer_a.json"], "derived_instance_ids": {} },
  "compile_report": { "spawned": 0, "updated": 1, "despawned": 1, "layers_compiled": 2, "pinned_upserts": 1 },
  "validation_report": { "hard_gates_passed": true, "violations": [] }
}
```

Notes:

- If validation fails, the response has `"applied": false` and includes the `validation_report`.

### `POST /v1/scene_sources/run_status`

Return the run status for a given `run_id` in the current scene workspace.

Request body:

```json
{ "run_id": "run_01" }
```

Response (shape):

```json
{
  "ok": true,
  "status": {
    "format_version": 1,
    "run_id": "run_01",
    "last_complete_step": 3,
    "next_step": 4
  }
}
```

### `POST /v1/scene_sources/run_apply_patch`

Apply a patch as part of a durable **run step**. This persists artifacts on disk under
`runs/<run_id>/steps/<step>/` and supports crash-resume by replaying completed steps.

Request body:

```json
{
  "run_id": "run_01",
  "step": 1,
  "scorecard": { "format_version": 1, "hard_gates": [{ "kind": "schema" }] },
  "patch": { "format_version": 1, "request_id": "req_123", "ops": [] }
}
```

Response (shape):

```json
{
  "ok": true,
  "run_id": "run_01",
  "step": 1,
  "mode": "executed",
  "result": { "applied": true, "patch_summary": { "changed_paths": [] }, "validation_report": { "hard_gates_passed": true } }
}
```

Notes:

- If `steps/<step>/complete.json` already exists, the response uses `mode = "replayed"` and returns
  the stored `apply_result.json` without reapplying.

### `POST /v1/mode`

Switch game mode: `build`, `play`.

To enter the Gen3D workshop (Build Preview scene), set `mode` to `preview` / `build_preview`.
For legacy compatibility, `gen3d` (alias `gen3d_workshop`) maps to the same behavior.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/mode \
  -H 'Content-Type: application/json' \
  -d '{"mode":"build_preview"}'
```

Response:

```json
{"ok":true}
```

Note: mode switching applies on the next frame; step a few frames after switching.

### `POST /v1/spawn`

Spawn a prefab instance into the world (unit or build object depending on whether the prefab has
mobility).

If `x`/`z` are omitted, the object spawns in front of the hero.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/spawn \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"6573e745-043f-4036-8b7d-020354cbe730"}'
```

Optionally place it explicitly:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/spawn \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"6573e745-043f-4036-8b7d-020354cbe730","x":2.0,"z":-1.0,"yaw":1.57}'
```

Response:

```json
{"ok":true,"instance_id_uuid":"...","prefab_id_uuid":"...","mobility":true,"pos":[...]}
```

Notes:

- Spawning uses deferred ECS commands; step 1–2 frames after spawning before attempting to
  `/v1/select` the new instance.

### `POST /v1/select`

Replace the current selection with a list of `instance_id_uuid`s.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/select \
  -H 'Content-Type: application/json' \
  -d '{"instance_ids":["52904bf2-0855-4796-bac9-fdd3d39ac3a0"]}'
```

Response:

```json
{"ok":true,"selected":1}
```

Notes:

- Invalid/missing UUIDs are ignored (selection may become empty).
- Only selectable objects can be selected (units/build objects/enemies).

### `POST /v1/move`

Issue a move order to the destination (`x`, `z`) for currently selected units that can move.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/move \
  -H 'Content-Type: application/json' \
  -d '{"x":10.0,"z":-2.0}'
```

Optionally include `y` as the intended ground height:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/move \
  -H 'Content-Type: application/json' \
  -d '{"x":10.0,"z":-2.0,"y":0.0}'
```

Response:

```json
{"ok":true}
```

Errors:

- `400` if no selection.
- `409` if no move orders could be issued (e.g. selected objects aren’t movable or no path found).

### `POST /v1/fire`

Enable/disable firing for currently selected units + set a fire target.

Start firing at a world point:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/fire \
  -H 'Content-Type: application/json' \
  -d '{"active":true,"target":{"kind":"point","x":5.0,"z":12.0}}'
```

Start firing at a specific enemy by UUID:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/fire \
  -H 'Content-Type: application/json' \
  -d '{"active":true,"target":{"kind":"enemy","instance_id":"<enemy_instance_id_uuid>"}}'
```

Stop firing:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/fire \
  -H 'Content-Type: application/json' \
  -d '{"active":false}'
```

Response:

```json
{"ok":true}
```

### `POST /v1/pause`

Pause simulation time (cannot be used while a `/v1/step` is in progress).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/pause -H 'Content-Type: application/json' -d '{}'
```

Response:

```json
{"ok":true,"paused":true}
```

### `POST /v1/resume`

Resume simulation time (cannot be used while a `/v1/step` is in progress).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/resume -H 'Content-Type: application/json' -d '{}'
```

Response:

```json
{"ok":true,"paused":false}
```

### `POST /v1/step`

Advance the simulation by N frames with a fixed `dt` (synchronous).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/step \
  -H 'Content-Type: application/json' \
  -d '{"frames":10,"dt_secs":0.0166667}'
```

Response:

```json
{"ok":true}
```

Notes:

- `frames` is clamped to `1..=1200`.
- `dt_secs` defaults to `1/60` and is clamped to `0.001..=0.1`.
- The game stays paused after stepping; call `/v1/resume` to run in real time.

### `POST /v1/screenshot`

Capture a screenshot from the primary window (rendered mode only).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/screenshot \
  -H 'Content-Type: application/json' \
  -d '{"path":"./shots/frame.png"}'
```

Response:

```json
{"ok":true,"path":"./shots/frame.png"}
```

Notes:

- Screenshot saving is **asynchronous** (it is written on a later frame). If you run with
  `--automation-pause-on-start`, call `/v1/step` for 1–2 frames after `/v1/screenshot` before you
  read the file.

Limitations:

- Not available in headless mode.
- `include_ui=false` is not supported yet.

## Gen3D endpoints

Gen3D build/save require rendered mode and the Build Preview scene (`mode=build`, `build_scene=preview`).

### `GET /v1/gen3d/status`

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/status
```

Response (shape):

```json
{
  "ok": true,
  "running": true,
  "build_complete": false,
  "draft_ready": false,
  "run_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
  "attempt": 0,
  "pass": 3,
  "status": "Generating component wheel_fr...",
  "error": null,
  "run_dir": "/abs/path/to/.gravimera/cache/gen3d/<run_id>"
}
```

### `POST /v1/gen3d/prompt`

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/prompt \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"A warcar with a cannon as weapon"}'
```

Response:

```json
{"ok":true}
```

### `POST /v1/gen3d/build`

Start a new build.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/build \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{"ok":true,"run_id":"1e973ac3-ce48-4319-9582-cabf9c929598"}
```

Notes:

- Returns `409` if a build is already running.
- Poll `/v1/gen3d/status` while stepping frames to drive progress.

### `POST /v1/gen3d/stop`

Cancel the current build.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response:

```json
{"ok":true}
```

### `POST /v1/gen3d/save`

Save the current draft into the world:

- saves prefab defs (and an optional descriptor) into the local model depot under `~/.gravimera/depot/models/<model_uuid>/prefabs/`, and
- spawns an instance near the hero and persists to the active scene’s `scene.dat` under `~/.gravimera/realm/.../build/scene.dat` (unless overridden by `scene.scene_dat_path` in config).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/save \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{
  "ok": true,
  "instance_id_uuid": "52904bf2-0855-4796-bac9-fdd3d39ac3a0",
  "prefab_id_uuid": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
  "mobility": true,
  "pos": [2.35, 1.02, 0.70]
}
```

## Scene Build endpoints

Scene Build runs the **Scene Builder** (AI-driven scene generation) for the active realm/scene.

Requirements:

- Rendered mode (not headless).
- Build Realm scene (not Preview).
- OpenAI config in `config.toml`.

### `GET /v1/scene_build/status`

Fetch current build status (or the last build summary).

```bash
curl -s http://127.0.0.1:8791/v1/scene_build/status
```

Response (shape):

```json
{
  "ok": true,
  "status": {
    "running": true,
    "run_id": "scene_build_...",
    "message": "Step 2/5 done: ...",
    "run_dir": "/abs/path/to/.gravimera/realm/<realm_id>/scenes/<scene_id>/runs/<run_id>",
    "phase": "step_request",
    "step_index": 2,
    "total_steps": 5
  }
}
```

### `POST /v1/scene_build/start`

Start a new Scene Build from a text description.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene_build/start \
  -H 'Content-Type: application/json' \
  -d '{"description":"A small garden with a cottage, trees, flowers, and a path."}'
```

Response:

```json
{"ok":true,"run_id":"scene_build_..."}
```

### `POST /v1/scene_build/stop`

Cancel the current Scene Build (best-effort; in-flight LLM calls may still finish but are ignored).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene_build/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{"ok":true,"canceled":true,"run_id":"scene_build_..."}
```

## `POST /v1/shutdown`

Request a clean shutdown.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/shutdown -H 'Content-Type: application/json' -d '{}'
```

Response:

```json
{"ok":true}
```

## End-to-end example: Gen3D warcar + save

This is the minimal flow for a deterministic script (pseudo-code):

1. `POST /v1/mode {"mode":"build_preview"}`
2. Step a few frames: `POST /v1/step {"frames":3}`
3. `POST /v1/gen3d/prompt {"prompt":"A warcar with a cannon as weapon"}`
4. `POST /v1/gen3d/build {}`
5. Loop:
   - `POST /v1/step {"frames":10}`
   - `GET /v1/gen3d/status`
   - stop when `build_complete=true` and `draft_ready=true`
6. `POST /v1/gen3d/save {}`
7. Switch back: `POST /v1/mode {"mode":"build"}`
