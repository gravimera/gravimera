# Terrain Naming Migration And Scene Package Management

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

After this change, scene-facing UI and on-disk storage use the term `terrain` instead of `floor` for the reusable world-surface package concept. Existing installs should migrate automatically on startup so old `floors/` folders and per-scene `floor_selection.json` files are renamed into the new terrain layout without requiring manual cleanup.

The Scenes panel will also gain manage-mode import, export, and delete actions. Scene export must package the whole scene folder and any realm prefab packages referenced by that scene so importing the zip into another realm preserves scene-local data plus editable prefab packages. Scene import must never replace an existing scene or prefab package.

## Progress

- [x] (2026-03-29 14:05 CST) Mapped the current scene panel, scene persistence, floor package storage, prefab package storage, and startup migration entrypoints.
- [x] (2026-03-29 14:18 CST) Drafted this ExecPlan with the concrete migration and scene-package scope.
- [x] (2026-03-29 14:54 CST) Implemented terrain naming migration in storage paths, runtime migration helpers, and per-scene protobuf terrain selection persistence.
- [x] (2026-03-29 15:18 CST) Implemented scene zip export/import helpers that package selected scenes plus referenced prefab packages and skip existing destinations on import.
- [x] (2026-03-29 15:41 CST) Extended the Scenes panel with manage mode, import/export/delete actions, native file dialog jobs, and import/export/delete toasts.
- [x] (2026-03-29 15:47 CST) Updated docs and terminology, ran focused tests, fixed a Bevy runtime query conflict found by the rendered smoke test, and reran the smoke test successfully.

## Surprises & Discoveries

- Observation: Current scene load does not depend on prefab package folders because `scene.grav` already embeds every object definition needed to spawn the scene.
  Evidence: `src/scene_store.rs` resets `ObjectLibrary` to builtins and comments that `scene.grav` must contain all defs needed to spawn.

- Observation: The existing Scenes panel is much smaller in scope than the Floors and 3D Models panels. It only supports add and switch, and it has no job resources or native-file-dialog flow yet.
  Evidence: `src/workspace_scenes_ui.rs` currently defines add/select behavior only, while import/export job patterns live in `src/floor_library_ui.rs` and `src/model_library_ui.rs`.

- Observation: Bevy runtime query validation caught a mutable `Node` access conflict in the new Scenes panel visibility system even though the code compiled cleanly.
  Evidence: The required rendered smoke test panicked with `error[B0001]` in `workspace_scenes_ui::scenes_panel_update_action_visibility` until the three `Query<&mut Node, ...>` parameters were merged into a `ParamSet`.

## Decision Log

- Decision: Keep existing internal Rust module names such as `floor_library_ui`, `scene_floor_selection`, and `BuildScene::FloorPreview` for this change, while renaming user-facing labels and persisted terrain file/folder names.
  Rationale: The user asked for the concept and storage to become `terrain`, but renaming every internal Rust symbol would add broad churn unrelated to behavior. This plan prioritizes visible behavior, saved-data migration, and scene management.
  Date/Author: 2026-03-29 / Codex

- Decision: Scene export will zip `scenes/<scene_id>/...` plus deduplicated `prefabs/<prefab_uuid>/...` package folders referenced by the selected scenes.
  Rationale: Exporting the raw scene folder preserves build/src data exactly, while bundling referenced realm prefab packages preserves editable prefab metadata that is not required for scene load but is required for later prefab management.
  Date/Author: 2026-03-29 / Codex

- Decision: Scene import will skip existing scene ids and existing prefab package ids instead of replacing them.
  Rationale: The request explicitly says to avoid replace during import. Skipping conflicting destinations preserves the destination realm and still allows importing new scenes that reuse already-existing prefab ids.
  Date/Author: 2026-03-29 / Codex

## Outcomes & Retrospective

Terrain is now the canonical on-disk and user-facing term for realm surface packages. Existing `floors/` roots, `floor_def_v1.json` package files, and per-scene `floor_selection.json` files are migrated automatically to `terrain/`, `terrain_def_v1.json`, and protobuf `terrain.grav`.

The Scenes panel now supports manage-mode export/delete plus import in both normal and manage modes. Scene export writes `scenes/<scene_id>/...` together with referenced `prefabs/<prefab_uuid>/...` packages, and import skips existing scene/prefab destinations instead of replacing them.

Validation completed with focused tests plus the required rendered smoke test. The smoke test also exposed a Bevy query conflict in the new scene panel code; fixing that before closeout materially improved runtime confidence.

## Context and Orientation

The current scene list UI lives in `src/workspace_scenes_ui.rs`, and the panel layout is built in `src/workspace_ui.rs`. Scene switching and scene save/load live in `src/scene_store.rs`. Realm and scene scaffolding plus startup migration hooks live in `src/realm.rs`.

The current reusable surface-package storage uses the term `floor`. Realm packages live under `realm/<realm_id>/floors/<floor_uuid>/` via `src/paths.rs` and `src/realm_floor_packages.rs`. A scene’s selected floor is stored separately in `realm/<realm_id>/scenes/<scene_id>/build/floor_selection.json` via `src/scene_floor_selection.rs`. The actual scene contents are already protobuf files named `scene.grav` and `scene.build.grav` handled by `src/scene_store.rs`.

Prefab package zip import/export already exists. The file-copying and native dialog patterns live in `src/prefab_zip.rs`, `src/floor_zip.rs`, `src/model_library_ui.rs`, and `src/floor_library_ui.rs`. The new scene package flow should follow those patterns rather than inventing a one-off mechanism.

## Plan of Work

First, update storage-path helpers so the canonical on-disk terrain package root becomes `realm/<realm_id>/terrain/`, the canonical package file becomes `terrain_def_v1.json`, and the per-scene terrain selection file becomes protobuf `terrain.grav`. Add migration helpers that scan startup realms/scenes, rename old `floors/` directories where possible, rename `floor_def_v1.json` files in package directories, and convert old `floor_selection.json` JSON files into the new protobuf file. Scene load and floor-selection save/load callsites should continue to work through the existing helper module names so higher-level code keeps its shape.

Second, add a new scene zip helper module that exports selected scene folders plus referenced realm prefab packages and imports them back into a realm without replacement. The export path should gather referenced prefab ids from both `scene.grav` and `scene.build.grav` when present, deduplicate package ids across all selected scenes, and write a single zip with `scenes/` and `prefabs/` roots. The import path should validate zip layout, reject path traversal, skip existing scene directories and prefab package directories, and report imported/skipped/invalid counts.

Third, expand the Scenes panel state and layout to support a manage mode. Normal mode should keep add/switch behavior and expose import. Manage mode should let the user select multiple scenes, export them, delete them, select all, clear selection, and exit manage mode. Export and delete should require at least one selected scene. Delete should be conservative about the active scene if that becomes risky during implementation; the final behavior must be explicit in the UI toast and docs.

Finally, rename the user-facing panel/button/toast labels from `Floor`/`Floors` to `Terrain`, update the docs that explain storage paths and panel behavior, run tests, run the required rendered smoke test, and commit the finished changes.

## Concrete Steps

From `/Users/flow/workspace/github/gravimera`:

1. Edit `src/paths.rs`, `src/realm.rs`, `src/realm_floor_packages.rs`, and `src/scene_floor_selection.rs` to switch canonical terrain paths and add migration helpers.
2. Add scene package helpers in a new source file and wire it through `src/lib.rs`.
3. Extend `src/workspace_scenes_ui.rs`, `src/workspace_ui.rs`, `src/app.rs`, and `src/app_plugins.rs` with scene manage/import/export/delete state, resources, and systems.
4. Update docs under `docs/` and any user-visible strings that still say `Floor` where the feature now means `Terrain`.
5. Run targeted tests and the required smoke test:

       cargo test scene_floor_selection
       cargo test scene_zip
       cargo test realm_floor_packages
       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

6. Commit all changes with a clear message.

This section will be revised with exact commands and observed results as work lands.

## Validation and Acceptance

Acceptance is reached when all of the following are observable:

- Starting the app against an old data layout renames or converts legacy floor storage into the new terrain layout, and the active scene still restores the same selected terrain.
- The top toolbar and terrain-management UI use terrain wording in user-visible places.
- Exporting scenes from the Scenes panel writes a zip containing the selected scene folders plus referenced prefab packages.
- Importing that zip into another realm creates only missing scenes/prefabs and does not replace an existing scene or prefab package.
- Deleting selected scenes from the Scenes panel removes them from the list according to the implemented safety rules.
- The required rendered smoke test launches and exits without a startup crash.

## Idempotence and Recovery

The startup migration must be safe to run multiple times. If a terrain destination already exists, the migration should preserve the new destination and avoid overwriting it. Scene import must skip existing destinations instead of replacing them. Export should create parent directories as needed and overwrite only the selected zip output path the user explicitly chose.

## Artifacts and Notes

- `cargo test scene_floor_selection -- --nocapture` ✅
- `cargo test realm_floor_packages -- --nocapture` ✅
- `cargo test scene_zip -- --nocapture` ✅
- `cargo test referenced_prefab_ids_in_scene_includes_scene_defs_and_instance_ids -- --nocapture` ✅
- `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2` ✅

## Interfaces and Dependencies

The implementation should end with these repository-local interfaces:

- In `src/scene_floor_selection.rs`, helper functions that still provide:

      pub(crate) fn load_scene_floor_selection(realm_id: &str, scene_id: &str) -> Result<Option<u128>, String>;
      pub(crate) fn save_scene_floor_selection(realm_id: &str, scene_id: &str, floor_id: Option<u128>) -> Result<(), String>;

  but use protobuf `terrain.grav` storage internally and perform legacy JSON migration.

- In a new scene package module, helpers shaped like:

      pub(crate) fn export_scene_packages_to_zip(realm_id: &str, scene_ids: &[String], zip_path: &Path) -> Result<SceneZipExportReport, String>;
      pub(crate) fn import_scene_packages_from_zip(realm_id: &str, zip_path: &Path) -> Result<SceneZipImportReport, String>;

  where the report types expose imported/exported/skipped/invalid counts needed for UI toasts.

- In `src/workspace_scenes_ui.rs`, resource types parallel to the existing floor/prefab dialog jobs so the scene panel can launch native file dialogs and poll worker threads without blocking the main Bevy update loop.

Revision note: 2026-03-29 / Codex. Initial ExecPlan draft after codebase inspection so implementation can proceed against a written migration and UI contract.
