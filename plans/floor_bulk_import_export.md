# Floor Library Manage Mode + Bulk Import/Export (Zip)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

Enable the Floors panel to match the Prefabs panel interaction model: a distinct Manage mode that supports multi-select with Shift range selection, bulk Export/Delete actions, and convenient All/None selection buttons. In normal mode the panel should show Import + Generate below the title. In Manage mode the panel should show Export + Delete + All + None below the title, disable open/preview behavior, and widen the panel so rows are not truncated.

Users can verify this works by opening the Floors panel in Build → Realm, clicking Manage to enter Manage mode (preview closes and list clicks only change selection), selecting multiple floors (including Shift range selection), exporting them to a zip, importing that zip back into the realm, and deleting selected floors with a confirmation modal. The Default Floor entry must not be selectable in Manage mode (radio hidden/disabled) and must never be exported/deleted.

## Progress

- [x] (2026-03-28 17:18 CST) Add floor zip helper module (`src/floor_zip.rs`) for export/import.
- [x] (2026-03-28 17:26 CST) Extend Floors panel UI layout: title Manage/Done button, normal/manage action rows, manage-mode width.
- [x] (2026-03-28 17:30 CST) Implement Floors manage mode interactions: multi-select + Shift range, All/None, disable preview/open, close preview on entry.
- [x] (2026-03-28 17:33 CST) Implement Floors Import/Export UI actions with background threads + toasts.
- [x] (2026-03-28 17:36 CST) Implement Floors bulk Delete with confirmation modal and list refresh.
- [x] (2026-03-28 17:38 CST) Update docs in `docs/` to describe Floors manage mode and zip layout.
- [x] (2026-03-28 17:40 CST) Run UI smoke test and commit changes.

## Surprises & Discoveries

- Observation: (none yet)

## Decision Log

- Decision: Use zip layout `floors/<uuid>/...` for floor package export/import.
  Rationale: Mirrors the on-disk realm floor package layout and matches the prefab zip pattern.
  Date/Author: 2026-03-28 / Codex
- Decision: In Manage mode, the Default Floor row is not selectable (radio hidden/disabled) and is excluded from All/None.
  Rationale: Default Floor is not a disk-backed package and should not be exported/deleted; this matches the confirmed requirement.
  Date/Author: 2026-03-28 / Codex

## Outcomes & Retrospective

- Delivered Floors panel Manage mode with the same interaction model as Prefabs: Manage/Done toggle, action row swap, multi-select with Shift range selection, and All/None. Manage mode disables applying/opening and preview; entering Manage mode closes any open preview.
- Added floor zip import/export support with layout enforcement `floors/<uuid>/...`, background-thread dialogs/work, and toast reporting.
- Added Delete confirmation modal and guarded Default Floor from selection/export/delete.
- Updated docs and ran the required rendered smoke test successfully.

## Context and Orientation

The Floors panel is implemented in `src/floor_library_ui.rs`. It is visible in Build mode when `BuildScene::Realm` is active. The panel contains:

1. A title row with a distinct `Manage/Done` toggle button.
2. An action row under the title (Import+Generate in normal mode; Export+Delete+All+None in Manage mode).
3. A search field.
4. A scrollable list of floors (including a special `Default Floor` entry with id `DEFAULT_FLOOR_ID = 0`).
5. A preview overlay panel rendered via an offscreen scene (disabled in Manage mode).

Realm floor packages live on disk under the realm’s floors directory and are managed by helper functions in `src/realm_floor_packages.rs` (list, load, save, delete). The scene’s selected floor is persisted by `src/scene_floor_selection.rs`, and applied on scene load by `apply_scene_floor_selection` in `src/scene_store.rs`.

The Prefabs panel already implements the desired Manage mode behavior in `src/model_library_ui.rs`. This work copies that interaction model into the Floors panel.

## Plan of Work

Add a new zip helper module `src/floor_zip.rs` analogous to `src/prefab_zip.rs`, supporting:

- Export of selected floor package directories into a zip under `floors/<uuid>/...`.
- Import from such a zip into the active realm, with path traversal protection and a minimal validity check (each package must contain `floor_def_v1.json`). Conflicts where the destination package dir already exists are skipped and reported.

Refactor `setup_floor_library_ui` in `src/floor_library_ui.rs` to match Prefabs panel layout:

- Title row: `Floors` + a distinct `Manage/Done` toggle button.
- Under title: normal actions row (Import + Generate) and manage actions row (Export + Delete + All + None). Only one row is visible at a time.
- When in Manage mode, widen the panel.

Extend `FloorLibraryUiState` to track manage mode and multi-selection, plus pending state for dialogs/modals. Implement interaction systems and style updates mirroring the prefabs implementation:

- Manage toggle: closes any preview when entering, and disables open/preview behavior while active.
- Multi-select list behavior: toggles membership; Shift selects a contiguous range by list order; Default Floor is not selectable.
- All/None buttons: select all listed non-default floors, or clear selection.
- Delete: opens a confirmation modal; confirm deletes selected floor packages from disk, updates list and selection, and if the active floor was deleted, clears scene selection and switches to Default Floor.
- Import/Export: use background threads for file dialogs and zip work; report outcomes via toasts and refresh list.

Update `src/app.rs` to initialize new job resources and `src/app_plugins.rs` to schedule the new systems.

Update docs in `docs/` to document Floors manage mode and the zip layout.

## Concrete Steps

1. Implement `src/floor_zip.rs` (export/import + report struct) and register it in `src/lib.rs`.
2. Update `src/floor_library_ui.rs` UI tree to add Manage/Done and the two action rows, and add new components/state fields.
3. Add interaction systems for manage toggle, All/None, Import/Export (dialog + job polling), Delete modal, and list multi-select + Shift range behavior.
4. Update system scheduling in `src/app_plugins.rs` to run the new systems with appropriate `run_if` gates and ordering.
5. Update docs in `docs/controls.md` and add `docs/floor_import_export.md`.
6. Run the required UI smoke test:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

7. Commit the change with a clear message.

## Validation and Acceptance

In Build → Realm:

- Floors panel title shows `Manage`. Under title shows `Import` and `Generate`.
- Clicking `Manage` enters Manage mode: title button becomes `Done`, under title shows `Export`, `Delete`, `All`, `None`, the panel widens, and any open floor preview closes.
- In Manage mode, clicking list items toggles selection only; it does not apply floors to the world and does not open preview. Shift-click selects a contiguous range.
- The `Default Floor` row shows no radio indicator and cannot be selected.
- `All` selects all non-default floors; `None` clears the selection.
- `Export` with no selection shows a warning toast; with selection it writes a zip containing `floors/<uuid>/...`.
- `Import` in normal mode imports a zip and refreshes the list with a summary toast.
- `Delete` opens a confirmation modal; confirming deletes selected floor packages and refreshes the list; canceling closes the modal.

Finally, run the smoke test command and confirm the app starts rendered and exits without crashing.

## Idempotence and Recovery

- Import is safe to rerun: existing floor package directories are skipped and reported.
- Export is non-destructive.
- If import fails due to invalid zip structure, it should not extract path-traversal entries and should not partially create packages for entries that are invalid.

## Artifacts and Notes

- Smoke test (rendered) succeeded:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Interfaces and Dependencies

Add `src/floor_zip.rs`:

    pub(crate) struct FloorZipImportReport {
        pub(crate) imported: usize,
        pub(crate) skipped: usize,
        pub(crate) invalid: usize,
    }

    pub(crate) fn export_floor_packages_to_zip(
        realm_id: &str,
        floor_ids: &[u128],
        zip_path: &std::path::Path,
    ) -> Result<usize, String>;

    pub(crate) fn import_floor_packages_from_zip(
        realm_id: &str,
        zip_path: &std::path::Path,
    ) -> Result<FloorZipImportReport, String>;
