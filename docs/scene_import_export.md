# Scene Import/Export (Zip)

This document describes the scene package import/export workflow from the Scenes panel.

## UI Workflow

- In normal mode, the Scenes panel shows **Add Scene**, **Manage**, and **Import**.
- **Import** prompts for a zip file and imports any valid scene packages into the active realm.
- If the zip conflicts with existing scene ids or bundled prefab package ids, the app opens a native local conflict dialog with `Replace`, `Keep Both`, and `Cancel`.
- Click **Manage** to enter manage mode (multi-select). In manage mode the panel keeps **Import** visible and adds **Export**, **Delete**, **All**, and **None**.
- In manage mode, click scene rows to toggle selection.
- **Export** saves the selected scene folders plus any referenced prefab packages into one zip.
- **Delete** removes the selected scene folders from disk, but skips the currently active scene.
- If no scenes are selected when exporting or deleting, the UI shows a warning toast.

## Zip Layout

Exports always use this layout:

- `scenes/<scene_id>/...`
- `prefabs/<prefab_uuid>/...`
- `terrain/<terrain_uuid>/...`

Each selected scene is exported by copying its scene folder as-is, including `build/` and `src/`.

Prefab packages are exported by scanning the selected scenes for referenced prefab ids and then copying each matching realm prefab package directory into `prefabs/<prefab_uuid>/...`.

Terrain packages are exported by reading each selected scene’s terrain selection (`build/terrain.grav` / legacy `build/floor_selection.json`) and then copying each matching realm terrain package directory into `terrain/<terrain_uuid>/...`.

## Import Rules

- Paths are validated to prevent traversal or absolute paths.
- A scene package is considered valid when it contains at least one of:
  - `build/scene.grav`
  - `build/scene.build.grav`
  - `src/index.json`
- A prefab package is considered valid when it contains a JSON prefab definition under `prefabs/`.
- A terrain package is considered valid when it contains a terrain def:
  - `terrain_def_v1.json` (legacy `floor_def_v1.json` is accepted and migrated)
- Scene ids conflict on `realm/<realm_id>/scenes/<scene_id>/`.
- Bundled prefab package ids conflict on `realm/<realm_id>/prefabs/<prefab_uuid>/`.
- Bundled terrain package ids conflict on `realm/<realm_id>/terrain/<terrain_uuid>/`.
- If the zip has no conflicts, import proceeds immediately.

## Conflict Policy

- `Replace` removes the conflicting destination scene folder and/or bundled prefab/terrain package folder, then imports the zip contents.
- `Keep Both` imports a second copy under new ids.
- `Cancel` aborts the import without changing disk.

For `Keep Both`:

- Conflicting scene folders are imported under fresh scene ids.
- Conflicting bundled prefab packages are imported under fresh prefab ids.
- Conflicting bundled terrain packages are imported under fresh terrain ids.
- Imported scene files are rewritten so `build/scene.grav`, `build/scene.build.grav`, `src/meta.json`, portal targets, pinned prefab references, and form prefab references all point at the new ids.
- Imported scene terrain selection (`build/terrain.grav`) is rewritten to point at any newly imported terrain ids.

The toast summary now reports imported, replaced, kept-both, and invalid counts separately for scenes, prefabs, and terrain.
