# Prefab Import/Export (Zip)

This document describes the prefab package import/export workflow from the 3D Models panel.

## UI Workflow

- In normal mode, the 3D Models panel shows **Import** (and **Generate**) below the title. **Import** prompts for a zip file and imports any valid prefab packages into the active realm.
- If the zip conflicts with existing prefab package ids, the app opens a native local conflict dialog with `Replace`, `Keep Both`, and `Cancel`.
- Click **Manage** to enter manage mode (multi-select). In manage mode the panel shows **Export** (and **Delete**) plus **All**/**None**.
- In manage mode, select prefabs by clicking list items; `Shift`+click selects a contiguous range.
- **Export** saves the selected prefab packages into a zip.
- If no prefabs are selected when exporting, the UI shows a warning toast.

## Zip Layout

Exports always use the following layout:

- `prefabs/<root_prefab_uuid>/...`

Each prefab package directory is copied as-is from the realm prefab store, including:

- `prefabs/` (published prefab JSON files)
- `materials/` (prefab-scoped assets)
- `gen3d_source_v1/` and `gen3d_edit_bundle_v1.json` when present
- `thumbnail.png` when present

## Import Rules

- Only entries under `prefabs/<uuid>/...` are accepted.
- Paths are validated to prevent traversal or absolute paths.
- A package must include at least one `prefabs/*.json` (excluding `*.desc.json`) to be considered valid.
- Conflicts are detected against `realm/<realm_id>/prefabs/<prefab_uuid>/`.
- If the zip has no conflicts, import proceeds immediately.

## Conflict Policy

- `Replace` removes the conflicting destination package and imports the zip package in its place.
- `Keep Both` imports a second copy under a fresh root prefab id.
- `Cancel` aborts the import without changing disk.

For `Keep Both`, the importer stages the package and rewrites the package metadata so the new copy stays internally consistent:

- the package folder name changes to the new root prefab UUID
- published prefab JSON ids are remapped
- the descriptor file name and contents are updated
- `gen3d_edit_bundle_v1.json` is updated when present

The toast summary now reports imported, replaced, kept-both, and invalid package counts separately.
