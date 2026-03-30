# Zip Import Conflict Resolution

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with [PLANS.md](/Users/flow/workspace/github/gravimera/PLANS.md).

## Purpose / Big Picture

After this change, importing Scene, 3D Model (prefab), or Terrain zip packages will no longer silently skip conflicts. When an import zip contains ids that already exist in the target realm, the app must ask the user to choose one policy for that import: `Replace`, `Keep Both`, or `Cancel`. `Replace` overwrites the conflicting destination package(s); `Keep Both` imports a second copy under newly generated ids; `Cancel` aborts the import without touching disk. The result must be observable from the rendered UI, and the core importer logic must also be covered by automated tests that prove replace and keep-both behavior on real package contents.

## Progress

- [x] (2026-03-31 16:30Z) Read `PLANS.md`, located the three zip import flows in `src/workspace_scenes_ui.rs`, `src/model_library_ui.rs`, and `src/floor_library_ui.rs`, and traced the current skip-on-conflict behavior into `src/scene_zip.rs`, `src/prefab_zip.rs`, and `src/floor_zip.rs`.
- [x] (2026-03-31 16:39Z) Confirmed `rfd = "0.15"` supports native three-button custom message dialogs, which keeps the user prompt local to the existing file-picker workflow and avoids a larger Bevy modal refactor.
- [x] (2026-03-31 16:47Z) Verified the keep-both design requirements for scenes: scene ids live in the scene folder name and `src/meta.json`, scene source JSON stores `prefab_id`, `forms`, and `destination_scene_id`, and `build/scene.grav` / `build/scene.build.grav` store prefab ids inside encoded `SceneDat` payloads.
- [x] (2026-03-31 09:55Z) Added `src/import_conflicts.rs` with `ImportConflictPolicy` and a native `Replace` / `Keep Both` / `Cancel` dialog helper, then wired the new module into `src/lib.rs`.
- [x] (2026-03-31 10:02Z) Refactored prefab and terrain zip import to split conflict scanning from policy-aware import, added replace/keep-both tests, and updated the 3D Models and Terrain panel workers to prompt once per import.
- [x] (2026-03-31 10:08Z) Refactored scene zip import to support replace and keep-both, including scene-id remapping, prefab-id remapping, scene source JSON rewrites, `SceneDat` prefab-id rewrites, and scene panel prompt/toast updates.
- [x] (2026-03-31 10:10Z) Updated `docs/scene_import_export.md`, `docs/prefab_import_export.md`, and `docs/terrain_import_export.md` to describe the new conflict prompt and outcomes.
- [x] (2026-03-31 10:13Z) Ran focused importer tests (`cargo test scene_zip`, `cargo test prefab_zip`, `cargo test floor_zip`), the full suite (`cargo test`), and the required rendered smoke test (`tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`).

## Surprises & Discoveries

- Observation: The repository already has an implementation of prefab package id remapping in `src/model_library_ui.rs::duplicate_realm_prefab_package`, including descriptor and Gen3D edit-bundle rewrites.
  Evidence: The function remaps prefab defs, copies `materials/` and `gen3d_source_v1/`, rewrites `root_prefab_id_uuid` in `gen3d_edit_bundle_v1.json`, and saves a new `<uuid>.desc.json`.

- Observation: Scene keep-both is broader than just renaming the scene folder. The scene source format derives stable ids from `meta.json.scene_id`, and source JSON also stores prefab UUIDs directly.
  Evidence: `src/scene_sources_runtime.rs` reads `meta.json.scene_id`, `prefab_id`, `forms`, and `destination_scene_id`; `src/scene_floor_selection.rs` shows scene-specific sidecar data also lives under the scene folder.

- Observation: Scene build files also reference prefab ids through `aim.components`, so scene keep-both needed a slightly broader prefab-id scan than the original export helper covered.
  Evidence: The importer tests only passed after expanding `referenced_prefab_ids_in_scene` and adding a helper that rewrites prefab ids directly in serialized `SceneDat` bytes.

## Decision Log

- Decision: Use `rfd::MessageDialog` with `YesNoCancelCustom("Replace", "Keep Both", "Cancel")` for the conflict choice instead of building a new in-game Bevy modal.
  Rationale: The app already uses `rfd` for native file selection on the same import path. Reusing it keeps the prompt synchronous with the import worker, minimizes UI churn, and still gives the user an explicit three-way choice.
  Date/Author: 2026-03-31 / Codex

- Decision: Make conflict handling explicit in the importer APIs instead of keeping “skip existing” as implicit behavior.
  Rationale: The user explicitly requested new behavior for all three import types, and the tests need to exercise replace and keep-both deterministically. An explicit policy enum keeps the control flow readable and removes hidden skip semantics from the core import paths.
  Date/Author: 2026-03-31 / Codex

- Decision: Reuse the prefab-package remap rules from duplication for keep-both import, rather than attempting a text-only rewrite of package files.
  Rationale: Prefab packages store ids in multiple places: prefab JSON filenames and contents, descriptor files, and Gen3D edit-bundle metadata. The existing duplication path already proves the required rewrite set.
  Date/Author: 2026-03-31 / Codex

## Outcomes & Retrospective

Implementation is complete. Scene, prefab, and terrain zip import now scan for conflicts first, prompt locally with `Replace`, `Keep Both`, and `Cancel`, and then import according to the selected policy. Scene keep-both rewrites both scene ids and bundled prefab ids consistently across source JSON and `SceneDat` build payloads. Prefab keep-both stages and remaps package metadata before writing the new package. Terrain keep-both imports the package under a fresh UUID.

Validation results:

- `cargo test scene_zip`
- `cargo test prefab_zip`
- `cargo test floor_zip`
- `cargo test`
- `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

The main retrospective note is that explicit policy-aware importer APIs were the right boundary. Once scan and import were separated, the UI prompt flow, test coverage, and replace/keep-both behavior all became straightforward to reason about.

## Context and Orientation

There are three separate zip import surfaces in the running game UI:

`src/workspace_scenes_ui.rs` owns the Scenes panel import button and launches `crate::scene_zip::import_scene_packages_from_zip`.

`src/model_library_ui.rs` owns the 3D Models panel import button and launches `crate::prefab_zip::import_prefab_packages_from_zip`.

`src/floor_library_ui.rs` owns the Terrain panel import button and launches `crate::floor_zip::import_floor_packages_from_zip`.

Each importer currently scans the zip, validates package layout, and writes directly into the active realm. When the destination package directory already exists, the importer increments a skipped count and leaves the existing data untouched. That skip behavior is documented today in `docs/scene_import_export.md`, `docs/prefab_import_export.md`, and `docs/terrain_import_export.md`.

For this feature, “conflict” means “the import wants to create a package whose destination id already exists in the target realm.” For scenes, that means the scene folder under `realm/<realm_id>/scenes/<scene_id>/`. For prefabs, that means the realm prefab package folder under `realm/<realm_id>/prefabs/<prefab_uuid>/`. For terrain, that means the realm terrain package folder under `realm/<realm_id>/terrain/<terrain_uuid>/`.

“Keep Both” is different per asset kind. For terrain, the new id only lives in the package folder name, so the importer can stage the package and write it under a new UUID. For prefabs, the new id also lives inside package files, so the importer must remap prefab ids consistently across the package. For scenes, the new id lives in the folder name and `src/meta.json.scene_id`, and any remapped prefab ids must also be propagated into scene build files and scene source JSON.

## Plan of Work

First, add a small shared import-conflict module, likely `src/import_conflicts.rs`, with an explicit `ImportConflictPolicy` enum and a helper that shows the native `Replace` / `Keep Both` / `Cancel` prompt using `rfd`. This helper will accept a short title/description string built by each UI caller after it scans the selected zip for conflicts.

Second, refactor `src/prefab_zip.rs` and `src/floor_zip.rs` so each module can do two separate things: inspect a zip and summarize conflicts, and then import with a chosen conflict policy. Terrain keep-both will generate a fresh UUID for each conflicting imported package and write it under the new folder. Prefab keep-both will share the existing duplication-style remap logic so the new root prefab id, internal object ids, descriptor file, and Gen3D edit bundle all stay consistent.

Third, refactor `src/scene_zip.rs` to split scanning from importing and to stage rewritten scene content when needed. The scene importer must generate a remap for conflicting scene ids, a remap for conflicting prefab ids, rewrite `build/scene.grav` and `build/scene.build.grav` through the in-repo `SceneDat` structs, rewrite scene-source JSON fields that store `scene_id`, `destination_scene_id`, `prefab_id`, and `forms`, and then copy the rewritten scene folder into the destination scene directory. Replace mode must remove any conflicting destination folder before extracting or copying the new content.

Fourth, update the three UI flows so they do not start importing immediately after the file picker returns. Instead, they will scan the selected zip, prompt if there are conflicts, abort cleanly on `Cancel`, and only then start the worker thread that performs the actual import with the chosen policy. The toast summary strings must also be updated so replace and keep-both results are visible instead of being reported as skips.

Finally, update the import/export docs and add automated tests that cover at least: prefab replace, prefab keep-both remap, terrain replace, terrain keep-both rename, scene replace, and scene keep-both with prefab-id rewriting. After that, run the required rendered smoke test from the repository root.

## Concrete Steps

Work from `/Users/flow/workspace/github/gravimera`.

1. Edit the importer modules and UI modules with `apply_patch`.
2. Run focused tests while iterating, such as:

       cargo test scene_zip
       cargo test prefab_zip
       cargo test floor_zip

3. Run the broader test suite if the focused tests pass:

       cargo test

4. Run the required rendered smoke test:

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

5. Inspect `git status --short`, update this plan’s progress and retrospective, then commit with a clear message.

## Validation and Acceptance

Acceptance is behavioral.

For scenes, importing a zip that conflicts with an existing scene id must first ask the user to choose `Replace`, `Keep Both`, or `Cancel`. `Replace` must overwrite the destination scene folder. `Keep Both` must create a second scene under a new scene id, and if the zip also contains conflicting prefab packages, the imported scene’s `scene.grav`, `scene.build.grav`, and scene source JSON must point at the newly imported prefab ids instead of the existing ones.

For prefabs, importing a zip that conflicts with an existing prefab package must ask the same three-way question. `Replace` must overwrite the destination package. `Keep Both` must create a second prefab package with a new root prefab id, remapped internal prefab ids, an updated descriptor, and an updated `gen3d_edit_bundle_v1.json` when present.

For terrain, importing a zip that conflicts with an existing terrain package must ask the same three-way question. `Replace` must overwrite the destination package. `Keep Both` must create a second terrain package under a new UUID.

Automated validation must include unit tests proving the keep-both and replace paths for all three importer modules. The rendered smoke test must still boot the app for two rendered seconds without crashing.

## Idempotence and Recovery

The importers must remain safe to rerun. `Cancel` must leave disk untouched. `Replace` must be implemented as “remove conflicting destination, then write the new package,” so retrying the same import after a partial failure restores the intended package contents. `Keep Both` must generate fresh ids until it finds ids that do not already exist in the destination realm, so rerunning the same import produces additional copies rather than corrupting earlier imports.

If an import fails after staging temporary files, the staging directory must be deleted before returning. Existing destination data must never be deleted unless the user explicitly chose `Replace`.

## Artifacts and Notes

Expected new documentation updates:

- `docs/scene_import_export.md`
- `docs/prefab_import_export.md`
- `docs/terrain_import_export.md`

Expected code areas to change:

- `src/workspace_scenes_ui.rs`
- `src/model_library_ui.rs`
- `src/floor_library_ui.rs`
- `src/scene_zip.rs`
- `src/prefab_zip.rs`
- `src/floor_zip.rs`
- a shared helper module for prompt and policy types

## Interfaces and Dependencies

The final code should expose explicit, policy-aware import entry points from the zip modules. The exact names can change during implementation, but the repository should end with functions equivalent in intent to:

    pub(crate) enum ImportConflictPolicy {
        Replace,
        KeepBoth,
    }

    pub(crate) fn summarize_scene_zip_conflicts(
        realm_id: &str,
        zip_path: &Path,
    ) -> Result<SceneZipConflictSummary, String>;

    pub(crate) fn import_scene_packages_from_zip_with_policy(
        realm_id: &str,
        zip_path: &Path,
        policy: ImportConflictPolicy,
    ) -> Result<SceneZipImportReport, String>;

    pub(crate) fn summarize_prefab_zip_conflicts(
        realm_id: &str,
        zip_path: &Path,
    ) -> Result<PrefabZipConflictSummary, String>;

    pub(crate) fn import_prefab_packages_from_zip_with_policy(
        realm_id: &str,
        zip_path: &Path,
        policy: ImportConflictPolicy,
    ) -> Result<PrefabZipImportReport, String>;

    pub(crate) fn summarize_floor_zip_conflicts(
        realm_id: &str,
        zip_path: &Path,
    ) -> Result<FloorZipConflictSummary, String>;

    pub(crate) fn import_floor_packages_from_zip_with_policy(
        realm_id: &str,
        zip_path: &Path,
        policy: ImportConflictPolicy,
    ) -> Result<FloorZipImportReport, String>;

The prompt helper should wrap `rfd::MessageDialog` and return one of three outcomes: replace, keep-both, or cancel. The importer reports should preserve invalid counts and should gain enough detail to distinguish imported packages from replaced packages and keep-both renames.

Change note: created this plan before editing so the id-remapping rules, UI prompt choice, and validation requirements are explicit and can be kept in sync with the implementation.
