# GenFloor

## Entry Points

- Top bar: click `Floors` to open the Floors panel.
- Floors panel: click `Generate` to enter GenFloor (Floor Preview).
- Floors panel: click a completed floor item to open the Floor Preview panel.
- Exiting GenFloor (Save or Cancel) returns to the Floors panel.

## Storage Layout

Floors live under the active realm:

```
realm/<realm_id>/floors/<floor_id>/
  floor_def_v1.json
  thumbnail.png (optional)
  materials/
  genfloor_source_v1/
    prompt.txt
```

- `floor_def_v1.json` is the canonical definition written by GenFloor.
- `thumbnail.png` is optional; if missing, the list shows an empty placeholder.

Per-scene floor selection is stored here:

```
realm/<realm_id>/scenes/<scene_id>/build/floor_selection.json
```

## Floor Definition (FloorDefV1)

`floor_def_v1.json` is a JSON object with these top-level fields:

- `format_version`: number (currently `1`).
- `label`: optional string label shown in the Floors list.
- `mesh`: grid parameters (`size_m`, `subdiv`, `thickness_m`, `uv_tiling`).
- `material`: `base_color_rgba`, `metallic`, `roughness`, `unlit`.
- `coloring`: color pattern definition (`mode`, `palette`, `scale`, `angle_deg`, `noise`).
- `relief`: static height variation (`mode`, `amplitude`, `noise`).
- `animation`: `mode` (`none`, `cpu`, `gpu`), `waves`, `normal_strength`.

The runtime canonicalizes and clamps values on load.

Note: `mesh.size_m` is clamped to at least the default floor size so generated floors always cover the scene.

## Runtime + Preview

- `ActiveWorldFloor` stores the active floor and is applied to the live world floor.
- Floor Preview uses the same `ActiveWorldFloor` data so the preview and runtime match.

## Notes

- Clicking `Build` sends the prompt to the GenFloor AI and returns a `FloorDefV1` draft.
- GenFloor auto-saves the draft on completion and switches the Build button to Edit (subsequent runs overwrite the same floor id).
- Floors can be switched at runtime by clicking items in the Floors list.
- Floors list always includes a built-in “Default Floor” entry (legacy plain floor).
- GenFloor uses the same AI service selection as Gen3D (`[gen3d].ai_service`).
- Floors list shows placeholder rows for active GenFloor work (`Generating`, `Editing`, `Queued`).
- If a scene has no stored floor selection, Default Floor is used.
