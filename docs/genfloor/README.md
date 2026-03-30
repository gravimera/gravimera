# GenFloor

## Entry Points

- Top bar: click `Terrain` to open the Terrain panel.
- Terrain panel: click `Generate` to enter GenFloor (Terrain Preview).
- Terrain panel: click a completed terrain item to open the Terrain Preview panel.
- Terrain Preview panel: click **Apply** to switch the active scene terrain, **Edit** to fork a new terrain from that base, **Delete** to remove it, or **Exit** to close (Default Terrain only shows **Apply**/**Exit**).
- Exiting GenFloor (Save or Cancel) returns to the Terrain panel.

## Storage Layout

Terrain packages live under the active realm:

```
realm/<realm_id>/terrain/<terrain_id>/
  terrain_def_v1.json
  thumbnail.png (optional)
  materials/
  genfloor_source_v1/
    prompt.txt
```

- `terrain_def_v1.json` is the canonical definition written by GenFloor.
- `thumbnail.png` is optional; if missing, the list shows an empty placeholder until a capture runs.
- Thumbnails are auto-generated after saves and backfilled when the Terrain list opens.

Per-scene terrain selection is stored here:

```
realm/<realm_id>/scenes/<scene_id>/build/terrain.grav
```

## Terrain Definition (`FloorDefV1`)

`terrain_def_v1.json` is a JSON object with these top-level fields:

- `format_version`: number (currently `1`).
- `label`: optional string label shown in the Terrain list.
- `mesh`: grid parameters (`size_m`, `subdiv`, `thickness_m`, `uv_tiling`).
- `material`: `base_color_rgba`, `metallic`, `roughness`, `unlit`.
- `coloring`: color pattern definition (`mode`, `palette`, `scale`, `angle_deg`, `noise`).
- `relief`: static height variation (`mode`, `amplitude`, `noise`).
- `animation`: `mode` (`none`, `cpu`, `gpu`), `waves`, `normal_strength`.

The runtime canonicalizes and clamps values on load.

Note: `mesh.size_m` is clamped to at least the default terrain size so generated terrain always covers the scene.

## Runtime + Preview

- `ActiveWorldFloor` stores the active terrain package and is applied to the live world terrain.
- Terrain Preview uses the same `ActiveWorldFloor` data so the preview and runtime match.
- Terrain height sampling for grounding/pathing uses relief only (ignores animated waves).
- Grounding uses the maximum relief height under an instance footprint, then applies a 0.02m sink
  via `apply_floor_sink` (skip the sink when the height is exactly 0).
- Relief heights below 0 are tagged as water for sampling/diagnostics only; placement and pathing
  do not block on water by default.
- Movement/placement/camera bounds clamp to the active floor mesh size (per-axis half size).

## Notes

- Clicking `Build` sends the prompt to the GenFloor AI and returns a `FloorDefV1` terrain draft.
- GenFloor auto-saves the draft on completion and switches the Build button to Edit (subsequent runs overwrite the same terrain id in that session).
- Clicking `Generate` from the Terrain panel starts a fresh GenFloor session (clears the previous edit-overwrite terrain id), unless a build is currently running (in which case it resumes the active session).
- Terrain can be switched at runtime from the Terrain Preview panel's `Apply` button.
- Terrain Preview `Edit` starts a forked session: the first run saves a new terrain id, then subsequent runs overwrite that new id.
- The Terrain list always includes a built-in `Default Terrain` entry.
- GenFloor uses the same AI service selection as Gen3D (`[gen3d].ai_service`).
- The Terrain list shows placeholder rows for active GenFloor work (`Generating`, `Editing`, `Queued`).
- If a scene has no stored terrain selection, Default Terrain is used.
- Import/export details live in `docs/terrain_import_export.md`.

## Automation API

See `docs/automation_http_api.md`:

- Enter GenFloor: `POST /v1/mode {"mode":"genfloor"}`
- Reset to a fresh session: `POST /v1/genfloor/new {}`
- Set prompt: `POST /v1/genfloor/prompt {"prompt":"..."}`
- Build: `POST /v1/genfloor/build {}`
- Status: `GET /v1/genfloor/status`

Test runners:

- Mock/offline API regression: `python3 test/run_1/genfloor_api/run.py`
- Real provider (uses your `~/.gravimera/config.toml`): `python3 tools/genfloor_real_test.py`
