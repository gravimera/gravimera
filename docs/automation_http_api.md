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
- Only pinned instances are applied as ECS entities in this milestone; other source files (layers/portals) are loaded and retained for round-trip export.

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

### `POST /v1/mode`

Switch game mode: `build`, `play`, `gen3d` (alias `gen3d_workshop`).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/mode \
  -H 'Content-Type: application/json' \
  -d '{"mode":"gen3d"}'
```

Response:

```json
{"ok":true}
```

Note: mode switching applies on the next frame; step a few frames after switching.

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

Limitations:

- Not available in headless mode.
- `include_ui=false` is not supported yet.

## Gen3D endpoints

Gen3D build/save require rendered mode and `mode=gen3d`.

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

Save the current draft into the world (spawns it near the hero and persists to `scene.dat`).

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

1. `POST /v1/mode {"mode":"gen3d"}`
2. Step a few frames: `POST /v1/step {"frames":3}`
3. `POST /v1/gen3d/prompt {"prompt":"A warcar with a cannon as weapon"}`
4. `POST /v1/gen3d/build {}`
5. Loop:
   - `POST /v1/step {"frames":10}`
   - `GET /v1/gen3d/status`
   - stop when `build_complete=true` and `draft_ready=true`
6. `POST /v1/gen3d/save {}`
7. Switch back: `POST /v1/mode {"mode":"build"}`
