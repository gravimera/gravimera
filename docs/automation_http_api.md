# Gravimera Local Automation HTTP API (v1)

Gravimera can expose a **local-only** HTTP API that lets external tools/agents drive the game (select/move/fire/mode/Gen3D/GenScene/screenshot) for automated testing.

The API is “semantic” (game actions). It intentionally does **not** expose raw keyboard/mouse event injection endpoints; use `/v1/select`, `/v1/move`, `/v1/fire`, `/v1/mode`, etc.

## Enable the server

### CLI

```bash
cargo run -- \
  --automation \
  --automation-bind 127.0.0.1:8791
```

### config.toml

```toml
[automation]
enabled = true
bind = "127.0.0.1:8791"
# Optional: local UI becomes read-only (camera + browsing allowed; mutations via API).
# monitor_mode = true
#
# Optional: ignore keyboard/mouse input while automation is enabled (default: false).
# disable_local_input = true
#
# Optional: start with time paused (default: false). Use the API to resume/step.
# pause_on_start = true
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
- When `[automation].monitor_mode=true`, Gravimera leaves camera + browsing input enabled, but blocks **local world mutations**
  (Gen3D/build placement/moving instances/etc). The Automation API still has full control.

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
    "disable_local_input": false,
    "pause_on_start": false,
    "monitor_mode": false,
    "paused": false,
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

- `build_scene` is only meaningful when `mode="build"` and is one of: `realm`, `preview`, `floor_preview`, `scene_preview`.

### `GET /v1/discovery`

Return a machine-readable discovery payload: supported features and a list of commonly used endpoints.

```bash
curl -s http://127.0.0.1:8791/v1/discovery
```

Response (shape):

```json
{
  "ok": true,
  "name": "gravimera",
  "version": "0.0.0",
  "api": { "version": 1, "base_path": "/v1" },
  "active": { "realm_id": "default", "scene_id": "default" },
  "features": { "ui_toast": true, "object_status_bar": true, "speech_bubble": true, "tts": true, "realm_scene_switch": true, "monitor_mode": false },
  "endpoints": [{ "method": "GET", "path": "/v1/health" }]
}
```

Notes:

- `features.ui_toast` and `features.speech_bubble` are `false` in headless mode (no window).
- The `endpoints` list is intended as a “starter index”, not a complete schema registry; refer to this doc for full details.

### `GET /v1/prefabs`

List known prefab definitions available to the running world (builtins + any loaded/saved prefabs).

```bash
curl -s http://127.0.0.1:8791/v1/prefabs
```

Response (shape):

```json
{
  "ok": true,
  "prefabs": [
    {
      "prefab_id_uuid": "...",
      "label": "Human",
      "mobility": true,
      "size": [1.0, 2.0, 1.0],
      "ground_origin_y": 0.0,
      "tags": [],
      "roles": [],
      "provenance_source": "gen3d"
    }
  ]
}
```

Notes:

- `ground_origin_y` is the prefab’s “ground contact” offset (in meters) used to ground instances onto terrain when authoring scene sources. Multiply by the instance Y scale.

### `POST /v1/prefabs/reload_realm`

Load all **realm prefab packages** from disk into the running world’s prefab library (builtins + realm prefabs).

This is useful when you:

- restart the game (realm prefabs exist on disk, but may not be loaded yet)
- switch realm/scene (the in-memory prefab library resets to builtins)
- want `scene_sources` patch validation to recognize Gen3D-saved prefabs

Request:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/prefabs/reload_realm \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{
  "ok": true,
  "realm_id": "default",
  "found_packages": 12,
  "loaded_packages": 12,
  "loaded_defs": 36,
  "loaded_descriptors": 12,
  "descriptor_error": null,
  "failed_packages": []
}
```

Notes:

- The endpoint is idempotent; it can be called repeatedly.
- `loaded_defs` is the total number of object defs loaded across all realm prefab packages.

### `POST /v1/prefabs/duplicate`

Duplicate a **realm prefab package** into a new prefab id (new package folder) in the active realm.

Request:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/prefabs/duplicate \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"..."}'
```

Response (shape):

```json
{
  "ok": true,
  "src_prefab_id_uuid": "...",
  "new_prefab_id_uuid": "..."
}
```

Notes:

- This duplicates on-disk packages under `realm/<realm_id>/prefabs/<prefab_id_uuid>/` (the “realm prefabs” system).
- Built-in prefabs that are not backed by a realm prefab package cannot be duplicated via this endpoint.

### `POST /v1/prefabs/export_gltf_glb`

Export one or more prefab definitions to **glTF 2.0** files (Blender-friendly), written into `out_dir`.

This endpoint writes **both**:

- `.glb` (binary container)
- `.gltf` + `.bin` (JSON + external buffer)

Notes:

- `POST /v1/prefabs/export_glb` currently aliases to the same exporter.

Request:

```bash
curl -s -X POST http://127.0.0.1:8791/v1/prefabs/export_gltf_glb \
  -H 'Content-Type: application/json' \
  -d '{
    "out_dir":"./out/prefabs_gltf_glb",
    "prefab_id_uuids":["..."],
    "fps": 30,
    "move_units_per_sec": 1.0
  }'
```

Response (shape):

```json
{
  "ok": true,
  "exported": 1,
  "out_dir": "./out/prefabs_gltf_glb",
  "out_paths": [
    "./out/prefabs_gltf_glb/TreeSmall_<uuid>.glb",
    "./out/prefabs_gltf_glb/TreeSmall_<uuid>.gltf",
    "./out/prefabs_gltf_glb/TreeSmall_<uuid>.bin"
  ]
}
```

Notes:

- `fps` and `move_units_per_sec` are optional.
- Three files are written per prefab id (`.glb`, `.gltf`, `.bin`).
- Current limitations are documented in `docs/prefab_export_gltf_glb.md`.

### `GET /v1/realm_scene/active`

Return the currently active realm/scene selection plus on-disk directories.

```bash
curl -s http://127.0.0.1:8791/v1/realm_scene/active
```

Response (shape):

```json
{
  "ok": true,
  "realm_id": "default",
  "scene_id": "default",
  "scene_dir": "/abs/path/to/.gravimera/realm/default/scenes/default",
  "scene_src_dir": "/abs/path/to/.gravimera/realm/default/scenes/default/src",
  "scene_build_dir": "/abs/path/to/.gravimera/realm/default/scenes/default/build"
}
```

### `GET /v1/realm_scene/list`

List realms and their scenes found on disk.

```bash
curl -s http://127.0.0.1:8791/v1/realm_scene/list
```

Response (shape):

```json
{ "ok": true, "realms": [{ "realm_id": "default", "scenes": ["default", "OpenClaw"] }] }
```

Notes:

- Only ids matching `[A-Za-z0-9._-]` are returned.

### `POST /v1/realm_scene/create`

Create a realm/scene scaffold on disk (scene sources + build dir). Optionally schedule a switch to it.

Request body:

```json
{
  "realm_id": "default",
  "scene_id": "OpenClaw",
  "label": "OpenClaw",
  "description": "A monitoring scene for an external agent run.",
  "switch_to": true
}
```

Response (shape):

```json
{
  "ok": true,
  "realm_id": "default",
  "scene_id": "OpenClaw",
  "scheduled_switch": true,
  "scene_dir": "/abs/path/to/.../scenes/OpenClaw",
  "scene_src_dir": "/abs/path/to/.../scenes/OpenClaw/src",
  "scene_build_dir": "/abs/path/to/.../scenes/OpenClaw/build"
}
```

Notes:

- `realm_id` defaults to the currently active realm.
- IDs must match `[A-Za-z0-9._-]` (no slashes).
- Switching is deferred; step a few frames after scheduling the switch.

### `POST /v1/realm_scene/switch`

Schedule a realm/scene switch (applies after a few frames).

Request body:

```json
{ "realm_id": "default", "scene_id": "OpenClaw" }
```

Response:

```json
{ "ok": true, "realm_id": "default", "scene_id": "OpenClaw", "scheduled_switch": true }
```

Notes:

- After calling, step 2–5 frames via `POST /v1/step` to let the switch apply.

### `POST /v1/ui/toast`

Show a non-modal “popup message box” toast in rendered mode.

Request body:

```json
{ "text": "Searching files… 🔍", "kind": "info", "ttl_secs": 3.5 }
```

Response:

```json
{ "ok": true }
```

Notes:

- Requires rendered mode (returns `501` in headless).
- `kind` is one of: `info`, `warn`, `error` (default: `info`).
- `ttl_secs` is clamped to `0.2..=120.0`.

### `POST /v1/ui/object_status_bar`

Set (or clear) an ObjectStatusBar under an object's speech bubble.

Request body:

```json
{ "instance_id_uuid": "...", "text": "Thinking…" }
```

Response (shape):

```json
{ "ok": true, "instance_id_uuid": "...", "text": "Thinking…" }
```

Notes:

- If `text` is empty/whitespace, the status bar is cleared (hidden).
- `text` is capped at 400 characters. The rendered UI may truncate further.
- The UI is only visible in rendered mode, but the endpoint still works in headless mode.

### `GET /v1/ui/object_status_bar/<instance_id_uuid>`

Fetch the current ObjectStatusBar text for an object.

```bash
curl -s http://127.0.0.1:8791/v1/ui/object_status_bar/<instance_id_uuid>
```

Response (shape):

```json
{ "ok": true, "instance_id_uuid": "...", "text": "Thinking…" }
```

Notes:

- If no status is set, `text` is an empty string.

### `POST /v1/speak`

Speak text via built-in TTS (ONNX if available, else system TTS) and optionally show a speech bubble above an object.

Request body:

```json
{
  "content": "Collecting materials.",
  "voice": "dog",
  "volume": 1.0,
  "instance_id_uuid": "...",
  "bubble": true
}
```

Response (shape):

```json
{ "ok": true, "speech_id": 1, "voice": "dog", "bubble": true }
```

Notes:

- Speaking is asynchronous; the response indicates the request was queued.
- `voice` is one of: `dog`, `cow`, `dragon` (default: `dog`).
- `content` is capped at 800 characters.
- `bubble=true` requires `instance_id_uuid` and rendered mode (returns `501` otherwise).

### `POST /v1/scene/save`

Force a `scene.grav` save (async; performed by the scene store systems).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene/save -H 'Content-Type: application/json' -d '{}'
```

Response:

```json
{ "ok": true }
```

### `POST /v1/scene/terrain/select`

Persist and immediately apply a terrain selection for the **active** realm+scene.

This is useful for tooling that generates a floor via GenFloor and then wants the scene to use that floor
as its active terrain (so spawns, screenshots, and later loads match).

Request body:

- `floor_id_uuid`:
  - UUID string: select that floor id (must exist in the active realm floor library)
  - `null` (or empty string): clear the scene’s terrain selection

```bash
curl -s -X POST http://127.0.0.1:8791/v1/scene/terrain/select \
  -H 'Content-Type: application/json' \
  -d '{"floor_id_uuid":"..."}'
```

Response (shape):

```json
{
  "ok": true,
  "realm_id": "default",
  "scene_id": "default",
  "floor_id_uuid": "..."
}
```

Notes:

- Requires rendered mode (returns `501` in headless mode).
- Validates that the floor id exists on disk in the active realm (`404` if not found).

### `POST /v1/despawn`

Despawn a world instance by id (for cleanup of props/units).

Request body:

```json
{ "instance_id_uuid": "..." }
```

Response:

```json
{ "ok": true, "despawned": true }
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

To enter the GenFloor workshop (Terrain Preview scene), set `mode` to `floor_preview`.
For legacy compatibility, `genfloor` maps to the same behavior.

To enter GenScene (Scene Preview), set `mode` to `scene_preview`.
For legacy compatibility, `gen_scene` / `genscene` map to the same behavior.

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

Issue a move order to the destination (`x`, `z`) for currently selected **commandable units** that can move.

If the selection includes **build objects**, they are repositioned instantly (teleport) to the destination.

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

Notes:

- For build objects, `y` (when provided) is interpreted as the desired **ground** height; Gravimera keeps the object’s origin offset.
- If `y` is omitted, build objects keep their current `translation.y`.

Response:

```json
{"ok":true}
```

Errors:

- `400` if no selection.
- `409` if no move actions could be applied (e.g. selected objects aren’t movable or no path found).

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

### `GET /v1/camera`

Fetch a small set of camera parameters useful for reproducible screenshots.

```bash
curl -s http://127.0.0.1:8791/v1/camera
```

Response (shape):

```json
{
  "ok": true,
  "focus": [0.0, 2.0, 0.0],
  "focus_initialized": true,
  "yaw": 1.2,
  "yaw_initialized": true,
  "pitch": -0.2,
  "zoom_t": 0.0
}
```

Notes:

- Requires rendered mode (returns `501` in headless mode).

### `POST /v1/camera`

Set camera parameters for a predictable viewpoint (useful before `/v1/screenshot`).

All fields are optional; omitted fields are unchanged.

Request body (shape):

```json
{ "focus": [0.0, 2.0, 0.0], "yaw": 1.2, "pitch": -0.2, "zoom_t": 0.0 }
```

Notes:

- Requires rendered mode (returns `501` in headless mode).
- `pitch` and `zoom_t` are clamped to the app’s allowed ranges.

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

Gen3D requires rendered mode (no `--headless`).

There are two ways to drive Gen3D via the API:

- **Workshop endpoints** (`/v1/gen3d/status`, `/v1/gen3d/build`, `/v1/gen3d/save`, …) operate on the active Gen3D workshop session and require the Build Preview scene (`mode=build`, `build_scene=preview`).
- **Task queue endpoints** (`/v1/gen3d/tasks*`) enqueue background Gen3D work without switching scenes; tasks run FIFO and only one task runs at a time.

### `GET /v1/gen3d/tasks`

List all non-idle Gen3D tasks (waiting/running/done/failed/canceled).

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/tasks
```

Response (shape):

```json
{
  "ok": true,
  "tasks": [
    {
      "task_id": "f6fb3f8a-0d66-4c0f-8bf8-66db7f8bda41",
      "kind": "build",
      "prefab_id_uuid": null,
      "state": "waiting",
      "run_id": null,
      "status": "",
      "error": null,
      "result_prefab_id_uuid": null
    }
  ]
}
```

### `POST /v1/gen3d/tasks/enqueue`

Enqueue a Gen3D task that will run in the background (no Build Preview scene switch required).

Request:

- `kind`: `build` | `edit_from_prefab` | `fork_from_prefab`
- `prompt`:
  - required for `kind=build`
  - optional for seeded kinds; when present, overrides the session prompt used by `Resume`
- `prefab_id_uuid`: required for `kind=edit_from_prefab|fork_from_prefab`

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/tasks/enqueue \
  -H 'Content-Type: application/json' \
  -d '{"kind":"build","prompt":"A warcar with a cannon as weapon"}'
```

Response:

```json
{"ok":true,"task_id":"f6fb3f8a-0d66-4c0f-8bf8-66db7f8bda41"}
```

Notes:

- Tasks are serialized: only one task runs at a time; additional tasks wait (FIFO).
- Step frames (via `/v1/step`) while polling `/v1/gen3d/tasks` to drive progress.
- A real-provider regression runner is available at `tools/gen3d_task_queue_suite_real_test.py` (artifacts under `test/run_1/...`).

### `GET /v1/gen3d/tasks/{task_id}`

Fetch one task.

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/tasks/f6fb3f8a-0d66-4c0f-8bf8-66db7f8bda41
```

Response (shape):

```json
{
  "ok": true,
  "task": {
    "task_id": "f6fb3f8a-0d66-4c0f-8bf8-66db7f8bda41",
    "kind": "build",
    "prefab_id_uuid": null,
    "state": "running",
    "run_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
    "status": "Planning components…",
    "error": null,
    "result_prefab_id_uuid": null
  }
}
```

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
  "can_resume": false,
  "edit_base_prefab_id": null,
  "save_overwrite_prefab_id": null,
  "draft_ready": false,
  "run_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
  "attempt": 0,
  "pass": 3,
  "status": "Generating component wheel_fr...",
  "error": null,
  "run_dir": "/abs/path/to/.gravimera/cache/gen3d/<run_id>"
}
```

Notes:

- `edit_base_prefab_id` / `save_overwrite_prefab_id` are present when the current Gen3D session was
  seeded from a saved prefab via `/v1/gen3d/edit_from_prefab` or `/v1/gen3d/fork_from_prefab`.
- When `save_overwrite_prefab_id` is non-null, `POST /v1/gen3d/save` overwrites that prefab id
  instead of generating a new root prefab id.

### `GET /v1/gen3d/preview`

Debug endpoint to inspect the Gen3D preview camera and whether the app is currently hiding a
background-running session from the user-facing preview (e.g. when opening a fresh Gen3D build while
another Gen3D task is running).

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/preview
```

Response (shape):

```json
{
  "ok": true,
  "active_session_id": "f6fb3f8a-0d66-4c0f-8bf8-66db7f8bda41",
  "active_kind": "build",
  "running_session_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
  "active_draft_empty": true,
  "should_hide_running_preview": true,
  "preview_state": {
    "explode_components": false,
    "camera_focus": [0.0, 1.2, 0.0],
    "draft_focus": [0.0, 1.2, 0.0],
    "exploded_component_center": null,
    "view_pan": [0.0, 0.0, 0.0],
    "yaw": 0.0,
    "pitch": -0.45,
    "distance": 6.0
  },
  "preview_camera": {
    "present": true,
    "render_layers": []
  }
}
```

Notes:

- `preview_camera.render_layers=[]` means the camera renders **no** layers (the preview is visually blank).
- `preview_state.camera_focus` is the effective orbit target after explode-centering and user pan
  are applied.
- `preview_state.draft_focus` is the stable assembled-object center used for deterministic explode
  direction math.
- `preview_state.exploded_component_center` is present when the visible preview component tree can
  be measured; in explode mode, `camera_focus` should track this value plus `view_pan`.

### `GET /v1/gen3d/preview/components`

Debug endpoint to inspect the currently visible Gen3D preview components, including projected panel
frames and current explode offsets.

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/preview/components
```

Response (shape):

```json
{
  "ok": true,
  "panel_size_logical": [960.0, 540.0],
  "probe_panel_logical": null,
  "probe_target_px": null,
  "hovered": null,
  "components": [
    {
      "entity_bits": 12345,
      "object_id_uuid": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
      "label": "torso",
      "depth": 2,
      "order": 0,
      "stable_order": 7,
      "ray_t": null,
      "explode_offset_local": [0.0, 0.0, 0.0],
      "projected": {
        "frame_panel_logical": {
          "min": [120.0, 80.0],
          "max": [340.0, 420.0]
        },
        "label_anchor_panel_logical": [230.0, 250.0]
      }
    }
  ]
}
```

Notes:

- `depth` is the actual object-ref depth inside the preview hierarchy. Root child components start
  at `depth=1`.
- `explode_offset_local` is the preview-only offset currently applied in the component’s parent
  local space.

### `GET /v1/gen3d/preview/export`

Inspect the current Gen3D preview export runtime status.

```bash
curl -s http://127.0.0.1:8791/v1/gen3d/preview/export
```

Response (shape):

```json
{
  "ok": true,
  "export": {
    "phase": "completed",
    "run_id": 1,
    "out_dir": "/Users/me/.gravimera/cache/gen3d_preview_exports/<id>_20260331_131800",
    "manifest_path": "/Users/me/.gravimera/cache/gen3d_preview_exports/<id>_20260331_131800/manifest.json",
    "total_channels": 4,
    "completed_channels": 4,
    "current_channel": null,
    "message": "Preview export completed: /Users/me/.gravimera/cache/gen3d_preview_exports/<id>_20260331_131800/manifest.json",
    "error": null
  }
}
```

Notes:

- `phase` is one of `idle`, `running`, `completed`, or `failed`.
- `completed_channels` counts finished channel bundles (motion channels only). Each bundle writes
  one `*_still.png` and one `*_anim.gif`.
- Every export also writes three static angle images: `angle_front.png`, `angle_left_front.png`,
  and `angle_right_front.png`.
- `manifest_path` is present after a successful export.

### `POST /v1/gen3d/preview/export`

Start a preview export for the current Gen3D draft. The request is asynchronous; poll
`GET /v1/gen3d/preview/export` until `phase` becomes `completed` or `failed`.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/preview/export \
  -H 'Content-Type: application/json' \
  -d '{
    "out_dir": "./test/run_1/preview_export",
    "channels": ["idle", "attack"]
  }'
```

Request fields:

- `out_dir` optional string path (parent directory). `~` is expanded. When omitted, Gravimera uses
  `GRAVIMERA_HOME/cache/gen3d_preview_exports/`.
  - The exporter always creates a new folder named `<id>_<YYYYMMDD_HHMMSS>` inside `out_dir` and
    writes all files into that folder.
  - `<id>` is the current Gen3D prefab id when available (seeded sessions or after saving),
    otherwise the current Gen3D run id, otherwise the preview export run id.
- `channels` optional array of exact preview channel names. When omitted, the export uses the
  authored preview channel order for the current draft (falling back to `idle` if the draft has no
  explicit channels yet).

Response (shape):

```json
{
  "ok": true,
  "export": {
    "phase": "running",
    "run_id": 2,
    "out_dir": "./test/run_1/preview_export/<id>_20260331_131800",
    "manifest_path": null,
    "total_channels": 2,
    "completed_channels": 0,
    "current_channel": "idle",
    "message": "Queued preview export for 2 channel(s).",
    "error": null
  }
}
```

Notes:

- The route requires `BuildScene::Preview` and an assembled Gen3D draft preview.
- Invalid channel names return `409` and list the available preview channels.
- Output filenames are informative and ordered, for example:
  `01_idle_still.png`, `01_idle_anim.gif`, `02_attack_still.png`, `02_attack_anim.gif`.
- Static angle images are also written: `angle_front.png`, `angle_left_front.png`,
  `angle_right_front.png`.
- `manifest.json` records the export camera state plus the static `angles` images and the
  per-channel still+gif bundles.

### `POST /v1/gen3d/preview/explode`

Enable or disable preview-only component explosion.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/preview/explode \
  -H 'Content-Type: application/json' \
  -d '{"enabled":true}'
```

Response:

```json
{"ok":true,"explode_components":true}
```

### `POST /v1/gen3d/preview/pan`

Apply a semantic preview pan in preview-screen axes. This exists for rendered automation and reuses
the same view-pan math as the keyboard preview controls.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/preview/pan \
  -H 'Content-Type: application/json' \
  -d '{"dx":2.0,"dy":-1.5}'
```

Response (shape):

```json
{
  "ok": true,
  "preview_state": {
    "explode_components": true,
    "camera_focus": [1.2, 2.4, -0.8],
    "draft_focus": [0.0, 1.2, 0.0],
    "exploded_component_center": [0.5, 1.4, 0.2],
    "view_pan": [0.7, 1.0, -1.0],
    "yaw": 0.0,
    "pitch": -0.45,
    "distance": 6.0
  }
}
```

Notes:

- `dx` moves the preview right/left in screen space; `dy` moves it up/down.
- The endpoint is additive. Send negative deltas to pan back.

### `POST /v1/gen3d/preview/probe`

Probe the preview picking logic at a logical point inside the preview panel. This reuses the same
component ranking as the UI hover overlay.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/preview/probe \
  -H 'Content-Type: application/json' \
  -d '{"x":230.0,"y":250.0}'
```

Response (shape):

```json
{
  "ok": true,
  "panel_size_logical": [960.0, 540.0],
  "probe_panel_logical": [230.0, 250.0],
  "probe_target_px": [402.0, 271.0],
  "hovered": {
    "index": 7,
    "entity_bits": 12345,
    "object_id_uuid": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
    "label": "torso",
    "depth": 2,
    "order": 0,
    "stable_order": 7,
    "projected": {
      "frame_panel_logical": {
        "min": [120.0, 80.0],
        "max": [340.0, 420.0]
      },
      "label_anchor_panel_logical": [230.0, 250.0]
    }
  },
  "components": []
}
```

Notes:

- `hovered=null` means the probe point was outside the visible preview image or did not resolve to a
  component.
- The endpoint uses the same nested-component preference as the UI: when multiple projected frames
  contain the probe point, deeper/smaller components beat enclosing parents.

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

Notes:

- The prompt is validated server-side and must be ≤ 250 whitespace-separated words and ≤ 2000 characters.
  - Over-limit prompts return `400` with an actionable error string (same limits as the UI).

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
- Starting a build resets the Gen3D session (new `run_id`, fresh draft).
- Poll `/v1/gen3d/status` while stepping frames to drive progress.

### `POST /v1/gen3d/edit_from_prefab`

Seed a Gen3D edit session from a **Gen3D-saved** prefab id.

Save semantics:

- `POST /v1/gen3d/save` overwrites the same `prefab_id_uuid` (future spawns use the updated prefab; respawn to see changes).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/edit_from_prefab \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"41d2d0fb-24ff-498f-ad05-c0884aa620ba"}'
```

Response (shape):

```json
{
  "ok": true,
  "run_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
  "can_resume": true,
  "edit_base_prefab_id": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
  "save_overwrite_prefab_id": "41d2d0fb-24ff-498f-ad05-c0884aa620ba"
}
```

Notes:

- Returns `400` if the prefab is not Gen3D-saved (descriptor provenance gate) or cannot be seeded.
- After seeding, use `/v1/gen3d/resume` to continue generation, or `/v1/gen3d/save` to save immediately.
- Overwrite sessions are QA-gated for safety: if the agent requests `done` while QA reports explicit errors, the engine will ignore `done` up to 2 times to encourage applying QA fixits. Engine auto-save skips overwriting when the latest QA indicates errors.

### `POST /v1/gen3d/fork_from_prefab`

Seed a Gen3D fork session from a **Gen3D-saved** prefab id.

Save semantics:

- `POST /v1/gen3d/save` writes a new `prefab_id_uuid` (different from the base).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/fork_from_prefab \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"41d2d0fb-24ff-498f-ad05-c0884aa620ba"}'
```

Response (shape):

```json
{
  "ok": true,
  "run_id": "1e973ac3-ce48-4319-9582-cabf9c929598",
  "can_resume": true,
  "edit_base_prefab_id": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
  "save_overwrite_prefab_id": null
}
```

### `POST /v1/gen3d/stop`

Stop (pause) the current build.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response:

```json
{"ok":true}
```

Notes:

- Stop cancels in-flight work but preserves the session context (draft + artifacts) so it can be resumed.
- Use `/v1/gen3d/resume` to continue the same `run_id`.

### `POST /v1/gen3d/resume`

Resume a stopped Gen3D session (same `run_id`, new `pass`).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/resume \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{"ok":true,"run_id":"1e973ac3-ce48-4319-9582-cabf9c929598","pass":4}
```

Notes:

- Returns `409` if a build is already running.
- Returns `400` if there is no prior Gen3D session to resume.

### `POST /v1/gen3d/apply_draft_ops`

Apply deterministic DraftOps to the active Gen3D draft.

Requirements:

- Rendered mode.
- Build Preview scene.
- An active Gen3D session (build or seeded edit).

This is the same deterministic edit surface used by the seeded-edit pipeline. It supports primitive
part edits, anchor edits, attachment offset/joint edits, animation-slot edits, and articulation-node
rig edits such as `upsert_articulation_node`, `remove_articulation_node`, and
`rebind_articulation_node_parts`.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/apply_draft_ops \
  -H 'Content-Type: application/json' \
  -d '{
    "version": 1,
    "atomic": true,
    "if_assembly_rev": 1,
    "ops": [
      {
        "kind": "upsert_articulation_node",
        "component": "head",
        "node_id": "jaw_hinge",
        "parent_node_id": "head_core",
        "set_transform": {
          "pos": [0.0, -0.12, 0.16],
          "rot_quat_xyzw": [0.0, 0.0, 0.0, 1.0]
        },
        "bound_part_id_uuids": ["2f6b7d3e-4f16-4a2b-8d35-0e15a8c4d3b0"]
      }
    ]
  }'
```

Response (shape):

```json
{
  "ok": true,
  "version": 1,
  "atomic": true,
  "committed": true,
  "assembly_rev_before": 1,
  "new_assembly_rev": 2,
  "applied_ops": [
    {
      "index": 0,
      "kind": "upsert_articulation_node",
      "diff": {
        "node_id": "jaw_hinge",
        "added": true
      }
    }
  ],
  "rejected_ops": [],
  "diff_summary": {
    "articulation_nodes": {
      "upserted": 1,
      "removed": 0,
      "rebound": 0
    }
  }
}
```

Notes:

- `atomic=true` is strongly recommended so invalid ops do not partially apply.
- `if_assembly_rev` is strongly recommended so stale DraftOps cannot apply to a changed draft.
- `remove_articulation_node` is rejected for non-leaf nodes.
- `remove_primitive_part` is rejected while an articulation node still binds that part.

### `POST /v1/gen3d/save`

Save the current draft to prefabs:

- saves prefab defs (and an optional descriptor) into the active realm’s prefab store under `<root_dir>/realm/<realm_id>/prefabs/<prefab_uuid>/prefabs/`.
- for Gen3D-saved prefabs, persists `gen3d_source_v1/` + `gen3d_edit_bundle_v1.json` (provenance).
- best-effort writes `thumbnail.png` at `<root_dir>/realm/<realm_id>/prefabs/<prefab_uuid>/thumbnail.png`.

Notes:

- Does **not** spawn an instance into the world. Use the 3D Models UI or `POST /v1/spawn` to spawn.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen3d/save \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{
  "ok": true,
  "prefab_id_uuid": "41d2d0fb-24ff-498f-ad05-c0884aa620ba",
  "mobility": true
}
```

## GenFloor endpoints

GenFloor requires rendered mode (no `--headless`).

Workshop endpoints (`/v1/genfloor/*`) operate on the active GenFloor session and require the Terrain Preview scene (`mode=build`, `build_scene=floor_preview`).

### `GET /v1/genfloor/status`

```bash
curl -s http://127.0.0.1:8791/v1/genfloor/status
```

Response (shape):

```json
{
  "ok": true,
  "running": true,
  "draft_ready": false,
  "edit_base_floor_id_uuid": null,
  "last_saved_floor_id_uuid": null,
  "prompt": "…",
  "status": "Building terrain…",
  "error": null
}
```

### `POST /v1/genfloor/new`

Reset to a fresh GenFloor session (clears prompt/draft and resets edit-overwrite state).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/genfloor/new \
  -H 'Content-Type: application/json' \
  -d '{}'
```

### `POST /v1/genfloor/prompt`

```bash
curl -s -X POST http://127.0.0.1:8791/v1/genfloor/prompt \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"A subtle stone floor with gentle variation."}'
```

### `POST /v1/genfloor/build`

Start (or edit-overwrite) a build using the current prompt.

Notes:

- On success, GenFloor auto-saves the generated floor and sets `edit_base_floor_id_uuid`.
- Subsequent builds overwrite the same `edit_base_floor_id_uuid` until you call `/v1/genfloor/new` (or start a fresh session from the Terrain panel).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/genfloor/build \
  -H 'Content-Type: application/json' \
  -d '{}'
```

### `POST /v1/genfloor/stop`

Request cancellation of the active build (best-effort).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/genfloor/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

## GenScene endpoints

GenScene runs the AI-driven scene generator. It creates a new scene immediately, then selects or
generates terrain + prefabs and applies placements to build the scene.

Requirements:

- Rendered mode (not headless).
- Build mode.
- OpenAI config in `config.toml`.

### `GET /v1/gen_scene/status`

Fetch current build status (or the last build summary).

```bash
curl -s http://127.0.0.1:8791/v1/gen_scene/status
```

Response (shape):

```json
{
  "ok": true,
  "status": {
    "running": true,
    "run_id": "gen_scene_...",
    "phase": "planning",
    "message": "Planning scene…",
    "scene_id": "NewScene_...",
    "error": null
  }
}
```

### `POST /v1/gen_scene/prompt`

Set the GenScene prompt (required before Build).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen_scene/prompt \
  -H 'Content-Type: application/json' \
  -d '{"prompt":"A small garden with a cottage, trees, flowers, and a path."}'
```

Response:

```json
{"ok":true}
```

### `POST /v1/gen_scene/build`

Start a new GenScene build using the current prompt.

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen_scene/build \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response:

```json
{"ok":true,"run_id":"gen_scene_..."}
```

### `POST /v1/gen_scene/stop`

Cancel the current GenScene build (best-effort; in-flight LLM calls may still finish but are ignored).

```bash
curl -s -X POST http://127.0.0.1:8791/v1/gen_scene/stop \
  -H 'Content-Type: application/json' \
  -d '{}'
```

Response (shape):

```json
{"ok":true,"canceled":true,"run_id":"gen_scene_..."}
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
