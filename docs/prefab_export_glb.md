# Prefab Export (GLB / Blender)

Gravimera can export prefab definitions from the **Prefabs** panel as **glTF 2.0 binary** (`.glb`) files for Blender and other DCC tools.

## UI workflow

1. Open **Prefabs** panel.
2. Click **Manage** (multi-select mode).
3. Select one or more prefabs.
4. Click **Export GLB** and choose an output folder.

The exporter writes one `.glb` per selected prefab.

## Output layout

Files are written to the chosen folder:

- `<out_dir>/<Label>_<prefab_uuid>.glb`

`Label` is sanitized for filesystem safety (non-alphanumeric characters become `_`), and `prefab_uuid` is the prefab id.

## What is exported

- **Hierarchy:** each prefab part becomes a glTF node; `ObjectRef` parts are expanded recursively.
- **Geometry:** procedural meshes from Gravimera’s prefab system (`MeshKey` + optional `PrimitiveParams`).
- **Materials:** best-effort glTF PBR factors (base color + metallic/roughness) and `KHR_materials_unlit` when `unlit=true`.
- **Animations:** baked TRS animation curves at a fixed FPS (default `30`) for each animation channel name found in the prefab.

### Animation notes

- Each exported channel becomes a glTF animation named after the channel (`idle`, `move`, `action`, `attack`, plus any custom channels).
- Drivers are baked into standard timelines:
  - `Always` / `AttackTime` / `ActionTime`: time is seconds.
  - `MovePhase` / `MoveDistance`: time is seconds * `move_units_per_sec` (default `1.0`).

## Limitations

- `ObjectPartKind::Model { scene }` parts are **not supported** yet (export fails with an actionable error).
- No texture images are exported (material factors only).
- Physics/colliders are not exported.

## Automation API

See `POST /v1/prefabs/export_glb` in `docs/automation_http_api.md`.

