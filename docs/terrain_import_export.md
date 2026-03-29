# Terrain Import/Export (Zip)

This document describes the terrain package import/export workflow from the Terrain panel.

## UI Workflow

- In normal mode, the Terrain panel shows **Import** (and **Generate**) below the title. **Import** prompts for a zip file and imports any valid terrain packages into the active realm.
- Click **Manage** to enter manage mode (multi-select). In manage mode the panel shows **Export** (and **Delete**) plus **All**/**None**.
- In manage mode, select terrain packages by clicking list items; `Shift`+click selects a contiguous range.
- The **Default Terrain** row is not selectable in manage mode and is never exported/deleted.
- **Export** saves the selected terrain packages into a zip.
- If no terrain packages are selected when exporting, the UI shows a warning toast.

## Zip Layout

Exports always use the following layout:

- `terrain/<terrain_uuid>/...`

Each terrain package directory is copied as-is from the realm terrain store, including:

- `terrain_def_v1.json` (required)
- `thumbnail.png` when present
- `materials/` when present
- `genfloor_source_v1/` when present

## Import Rules

- New exports write `terrain/<uuid>/...`.
- Import also accepts legacy `floors/<uuid>/...` archives for migration convenience.
- Paths are validated to prevent traversal or absolute paths.
- A package must include `terrain_def_v1.json` or legacy `floor_def_v1.json` to be considered valid.
- If a terrain UUID already exists in the target realm, that package is skipped.

## Conflict Policy

Conflicts are **skipped** to avoid overwriting existing terrain packages. The UI reports imported, skipped, and invalid package counts in a toast summary.
