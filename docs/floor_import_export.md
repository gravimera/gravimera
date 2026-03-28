# Floor Import/Export (Zip)

This document describes the floor package import/export workflow from the Floors panel.

## UI Workflow

- In normal mode, the Floors panel shows **Import** (and **Generate**) below the title. **Import** prompts for a zip file and imports any valid floor packages into the active realm.
- Click **Manage** to enter manage mode (multi-select). In manage mode the panel shows **Export** (and **Delete**) plus **All**/**None**.
- In manage mode, select floors by clicking list items; `Shift`+click selects a contiguous range.
- The **Default Floor** row is not selectable in manage mode and is never exported/deleted.
- **Export** saves the selected floor packages into a zip.
- If no floors are selected when exporting, the UI shows a warning toast.

## Zip Layout

Exports always use the following layout:

- `floors/<floor_uuid>/...`

Each floor package directory is copied as-is from the realm floor store, including:

- `floor_def_v1.json` (required)
- `thumbnail.png` when present
- `materials/` when present
- `genfloor_source_v1/` when present

## Import Rules

- Only entries under `floors/<uuid>/...` are accepted.
- Paths are validated to prevent traversal or absolute paths.
- A package must include `floor_def_v1.json` to be considered valid.
- If a floor UUID already exists in the target realm, that package is skipped.

## Conflict Policy

Conflicts are **skipped** to avoid overwriting existing floor packages. The UI reports imported, skipped, and invalid package counts in a toast summary.

