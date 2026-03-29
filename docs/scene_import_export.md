# Scene Import/Export (Zip)

This document describes the scene package import/export workflow from the Scenes panel.

## UI Workflow

- In normal mode, the Scenes panel shows **Add Scene**, **Manage**, and **Import**.
- **Import** prompts for a zip file and imports any valid scene packages into the active realm.
- Click **Manage** to enter manage mode (multi-select). In manage mode the panel keeps **Import** visible and adds **Export**, **Delete**, **All**, and **None**.
- In manage mode, click scene rows to toggle selection.
- **Export** saves the selected scene folders plus any referenced prefab packages into one zip.
- **Delete** removes the selected scene folders from disk, but skips the currently active scene.
- If no scenes are selected when exporting or deleting, the UI shows a warning toast.

## Zip Layout

Exports always use this layout:

- `scenes/<scene_id>/...`
- `prefabs/<prefab_uuid>/...`

Each selected scene is exported by copying its scene folder as-is, including `build/` and `src/`.

Prefab packages are exported by scanning the selected scenes for referenced prefab ids and then copying each matching realm prefab package directory into `prefabs/<prefab_uuid>/...`.

## Import Rules

- Paths are validated to prevent traversal or absolute paths.
- A scene package is considered valid when it contains at least one of:
  - `build/scene.grav`
  - `build/scene.build.grav`
  - `src/index.json`
- A prefab package is considered valid when it contains a JSON prefab definition under `prefabs/`.
- Import never replaces existing scene folders or prefab package folders.
- If a scene id already exists in the target realm, that scene is skipped.
- If a prefab UUID already exists in the target realm, that prefab package is skipped.

## Conflict Policy

Conflicts are **skipped** to avoid overwriting the destination realm. The UI reports imported, skipped, and invalid scene/prefab counts in a toast summary.
