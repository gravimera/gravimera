# Prefab Bulk Import/Export (Zip)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `/Users/lxl/projects/aiprojects/gravimera/PLANS.md`.

## Purpose / Big Picture

Enable users to batch export and import prefab packages directly from the Prefabs panel. Users can toggle multi-select with Shift, export the selected packages into a zip with a standard directory layout, and import a zip to add prefab packages into the active realm. This makes sharing and moving prefabs between realms or machines straightforward from the UI.

## Progress

- [x] (2026-03-24 17:10 CST) Draft plan, identify UI/state changes, zip tooling, and docs updates.
- [x] (2026-03-24 17:18 CST) Implement Prefabs UI multi-select, buttons, and toasts.
- [x] (2026-03-24 17:20 CST) Implement prefab zip import/export module and wire it into UI.
- [x] (2026-03-24 17:24 CST) Update documentation and run smoke test.
- [x] (2026-03-24 17:35 CST) Move export work onto a background thread to prevent UI stalls.
- [x] (2026-03-24 17:48 CST) Move import/export file dialogs and import work onto background threads.
- [x] (2026-03-24 17:56 CST) Make Import/Export buttons mutually exclusive and non-layout affecting.
- [x] (2026-03-24 18:00 CST) Fix import/export button layout system to avoid conflicting queries.

## Surprises & Discoveries

- Observation: The smoke test build surfaced pre-existing warnings in unrelated modules.
  Evidence: `gen3d/ai/draft_ops.rs` unused assignment, `automation/mod.rs` unused fields, plus other unused code warnings.
- Observation: Exporting large prefab packages can stall the UI if done on the main thread.
  Evidence: User report of UI freeze when clicking Export.
- Observation: Blocking file dialogs stall the UI when opened on the main thread.
  Evidence: User report of UI freeze when clicking Import/Export without dialog showing.
- Observation: Initial button display system caused a Bevy query conflict on `Node`.
  Evidence: Bevy error `B0001` about conflicting `Query<&mut Node>` parameters.

## Decision Log

- Decision: Use zip layout `prefabs/<uuid>/...` for export/import.
  Rationale: Matches the on-disk realm prefab package layout and the confirmed requirement.
  Date/Author: 2026-03-24 / Codex
- Decision: Multi-select is toggled by Shift only when the Prefabs panel is visible and topmost (BuildScene=Realm), without requiring mouse position.
  Rationale: Matches the clarified UI requirement about panel stacking and avoids accidental toggles.
  Date/Author: 2026-03-24 / Codex
- Decision: Import conflicts are skipped with a warning summary.
  Rationale: Keeps existing prefabs safe and aligns with the confirmed conflict policy.
  Date/Author: 2026-03-24 / Codex
- Decision: Entering multi-select seeds the selection with the current single-selected prefab (if any).
  Rationale: Preserves the current selection when toggling into multi-select mode.
  Date/Author: 2026-03-24 / Codex
- Decision: Run prefab export work on a background thread and surface completion via toast polling.
  Rationale: Avoids blocking the UI during zip generation.
  Date/Author: 2026-03-24 / Codex
- Decision: Run file dialog selection in a background thread and then queue import/export work on completion.
  Rationale: Prevents main-thread stalls when opening native file dialogs.
  Date/Author: 2026-03-24 / Codex
- Decision: Hide Import/Export via `Display::None` so hidden buttons do not consume layout space.
  Rationale: Fixes panel overflow and matches the requested mutually exclusive UI.
  Date/Author: 2026-03-24 / Codex

## Outcomes & Retrospective

- Delivered batch prefab import/export with zip layout enforcement, Shift multi-select mode, and updated docs. Smoke test passed; compile warnings were pre-existing.

## Context and Orientation

The Prefabs panel lives in `/Users/lxl/projects/aiprojects/gravimera/src/model_library_ui.rs`. It builds the list UI, handles input, and provides preview/drag behaviors. Prefab packages for a realm are stored under `~/.gravimera/realm/<realm_id>/prefabs/<root_prefab_uuid>/` with the layout defined in `/Users/lxl/projects/aiprojects/gravimera/docs/gamedesign/39_realm_prefab_packages_v1.md`. The panel is only visible in Build mode when `BuildScene::Realm` is active.

The new functionality requires:

- UI state for multi-select and selection set.
- Import/Export buttons with file dialogs.
- A zip helper module that reads and writes prefab package directories.
- Toast notifications using `UiToastCommand`.
- Documentation updates in `/Users/lxl/projects/aiprojects/gravimera/docs/`.

## Plan of Work

Add multi-select state to `ModelLibraryUiState` in `src/model_library_ui.rs`, and a system that toggles it when Shift is pressed while the Prefabs panel is visible and topmost. In multi-select mode, list clicks should toggle selection membership and avoid drag/preview. Entering multi-select should close any preview and clear drag state. Exiting multi-select clears the selection set.

Extend the Prefabs panel header to include a persistent Import button and a conditional Export button. The Export button is only visible when multi-select mode is active. Add interaction systems that open file dialogs via `rfd`, then call a new zip module to export or import packages. Use `UiToastCommand::Show` to display warnings and summaries.

Add a new module (e.g. `src/prefab_zip.rs`) that implements `export_prefab_packages_to_zip` and `import_prefab_packages_from_zip`. Export should write each selected prefab package under `prefabs/<uuid>/...` in the zip. Import should only accept entries under that root, reject path traversal, require at least one `prefabs/*.json` per package, and skip conflicts when the target directory already exists.

Update `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md` and create a new doc (e.g. `/Users/lxl/projects/aiprojects/gravimera/docs/prefab_import_export.md`) describing the workflow and zip layout. Do not modify README.

## Concrete Steps

1. Create `src/prefab_zip.rs` with export/import helpers and a small report struct.
2. Add dependencies `zip` and `rfd` to `/Users/lxl/projects/aiprojects/gravimera/Cargo.toml`, and add `mod prefab_zip;` in `/Users/lxl/projects/aiprojects/gravimera/src/lib.rs`.
3. Update `src/model_library_ui.rs`:
   - Add multi-select state and components for Import/Export buttons.
   - Add systems for Shift toggle, list item multi-select behavior, and button interactions.
   - Ensure export is hidden outside multi-select and list highlighting uses the multi-select set.
4. Register new systems in `/Users/lxl/projects/aiprojects/gravimera/src/app_plugins.rs`.
5. Update documentation files in `/Users/lxl/projects/aiprojects/gravimera/docs/`.
6. Run the required smoke test and capture the result for validation.

## Validation and Acceptance

- Open Prefabs panel in Build mode (Realm) and press Shift: multi-select toggles on and Export appears.
- With Gen3D panel active (BuildScene != Realm), pressing Shift does not toggle multi-select.
- Selecting multiple prefabs highlights all selected entries.
- Export with no selection shows a toast prompting selection.
- Export with selection writes a zip that contains `prefabs/<uuid>/...` directories.
- Importing that zip adds the prefabs to the active realm; conflicts are skipped with a summary toast.
- Run the smoke test:

  tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expect the game to launch and exit without crashing.

## Idempotence and Recovery

Zip import is safe to re-run; existing prefab packages are skipped and reported. Export is non-destructive. If any import fails due to invalid zip structure, no partial extraction should be left behind for skipped packages; users can re-run after fixing the zip.

## Artifacts and Notes

Include a short note in the final update summarizing export/import counts and any warnings from the smoke test, if present.

## Interfaces and Dependencies

- `src/prefab_zip.rs`:

  pub(crate) struct PrefabZipImportReport {
      pub(crate) imported: usize,
      pub(crate) skipped: usize,
      pub(crate) invalid: usize,
  }

  pub(crate) fn export_prefab_packages_to_zip(
      realm_id: &str,
      prefab_ids: &[u128],
      zip_path: &std::path::Path,
  ) -> Result<usize, String>;

  pub(crate) fn import_prefab_packages_from_zip(
      realm_id: &str,
      zip_path: &std::path::Path,
  ) -> Result<PrefabZipImportReport, String>;

- Dependencies in `Cargo.toml`: `zip` for archive handling, `rfd` for file dialogs.

Plan update (2026-03-24): Marked implementation complete, recorded smoke test warnings, added multi-select seeding decision, and documented background threading for export plus file dialogs to address UI stalls.
