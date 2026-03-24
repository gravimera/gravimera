# Prefab Import/Export (Zip)

This document describes the prefab package import/export workflow from the Prefabs panel.

## UI Workflow

- **Import** is always available in the Prefabs panel header. It prompts for a zip file and imports any valid prefab packages into the active realm.
- **Export** appears only when multi-select mode is enabled (toggle with `Shift`). It saves the selected prefab packages into a zip.
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
- If a prefab UUID already exists in the target realm, that package is skipped.

## Conflict Policy

Conflicts are **skipped** to avoid overwriting existing prefab packages. The UI reports imported, skipped, and invalid package counts in a toast summary.
