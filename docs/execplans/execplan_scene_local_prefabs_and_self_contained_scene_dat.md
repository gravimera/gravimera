# Scenes: Remove Depot, Make `scene.dat` Self-Contained, and Store Prefabs Locally per Scene

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

After this change, a **scene** becomes the single unit of storage and reliability:

- The runtime world is fully defined by `scene.dat` alone (no external prefab stores required to load and render the scene).
- “Prefabs on disk” still exist, but they are **scene-local** and are used mainly for **editing, copying, and tooling** (including Gen3D edit sessions). Prefabs are no longer shared across scenes in a realm.
- The global “depot” is removed. There is no `~/.gravimera/depot/...` content store anymore.
- When the user saves a Gen3D model, it is saved into the currently open scene’s local prefab store, and we also persist the **minimum edit bundle** required to reliably resume Gen3D edits after restarting the game (even if `~/.gravimera/cache/` is deleted).
- Any non-built-in “material assets” (textures/meshes, etc.) associated with a prefab are stored under that prefab’s own folder so assets from different prefabs do not intermix.

You can see this working by:

1. Running the rendered game (`cargo run`).
2. Entering Gen3D Preview, generating a model, and clicking Save.
3. Observing new files appear under the current scene directory (not under a depot directory).
4. Closing the game, deleting `~/.gravimera/cache/`, restarting, and applying an edit/fix to the previously saved model successfully (the Gen3D agent sees a loaded draft with planned components and can apply `apply_draft_ops_v1`).
5. Temporarily renaming/removing the scene’s `prefabs/` directory and observing that the scene still loads and renders from `scene.dat` alone (prefabs are “edit-only”, not “runtime required”).

## Progress

- [x] (2026-03-08 03:59Z) Draft this ExecPlan and inventory relevant code/docs.
- [ ] Define the new on-disk scene layout (scene-local prefab packages, per-prefab materials) in `docs/gamedesign/` and update `docs/gamedesign/specs.md`.
- [ ] Remove depot: delete/retire `src/model_depot.rs`, depot path helpers, depot loaders, and depot UI.
- [ ] Remove realm-shared prefabs: stop loading `realm/<realm_id>/prefabs/packs/...` on scene load; implement scene-local prefab loading for edit tools only.
- [ ] Implement scene-local prefab store module (read/write prefab defs + descriptors per scene).
- [ ] Change Gen3D Save to write prefab packages into the current scene, including a persisted Gen3D edit bundle.
- [ ] Change Gen3D Edit/Fork seeding to hydrate session state from the persisted edit bundle (no cache dependency).
- [ ] Make “materials per prefab folder” concrete for any existing exported/imported asset flows (or explicitly define it as a reserved folder when only primitives are used).
- [ ] Update UI/tooling surfaces to browse/copy scene-local prefabs (replacing the old depot model library panel).
- [ ] Run `cargo test`, run the rendered smoke test, and perform the manual restart+edit scenario.
- [ ] Commit a single “big change” commit (or a small series) with clear messages and doc updates.

## Surprises & Discoveries

- Observation: `scene.dat` already has a self-describing shape: it stores both object definitions (`defs`) and instances (`instances`) in a protobuf message (`SceneDat`), and load code upserts the defs into the in-memory `ObjectLibrary` before spawning instances.
  Evidence (code pointers): `src/scene_store.rs` defines `SceneDat` with `defs` + `instances` and in `load_scene_dat_from_path` it loops over `scene.defs` and `library.upsert(def)`.
- Observation: `save_scene_dat_internal` already tries to include all referenced object defs by walking object references starting from the instances’ prefab ids (and their forms). This is exactly the behavior we want to rely on for “runtime loads without prefabs”.
  Evidence: `src/scene_store.rs` `save_scene_dat_internal` gathers `root_defs`, calls `gather_referenced_defs(library, root_defs)`, then encodes those defs into `SceneDat`.
- Observation: The current Gen3D edit/fork seed path restores `draft.defs` but does not restore Gen3D session state (planned components, workspaces, etc.), which causes “empty workspace” behavior after restart even when the source bundle exists.
  Evidence: `src/gen3d/ai/orchestration.rs` `gen3d_start_seeded_session_from_prefab_id_from_api` sets `draft.defs = seeded_defs`, but then clears `job.planned_components` and resets `job.agent` to default; the agent prompt state summary is derived from `job.planned_components` and `job.agent.workspaces`, not from `draft.defs`.

## Decision Log

- Decision: Treat `scene.dat` as the sole runtime dependency for loading a scene; scene-local prefab packages are “edit/copy” artifacts that are not required at runtime.
  Rationale: This meets the “self described scene” goal and avoids cross-directory precedence/override problems.
  Date/Author: 2026-03-08 / Codex
- Decision: Remove the depot entirely and remove realm-shared prefab packs; all user-created prefabs live under the scene they were created in.
  Rationale: This removes the biggest source of confusion (“where is my model stored?”) and avoids global caches/state affecting editability.
  Date/Author: 2026-03-08 / Codex
- Decision: Persist a dedicated Gen3D “edit bundle” alongside the scene-local prefab package for Gen3D-saved prefabs, and hydrate Gen3D session state from it during Edit/Fork seeding.
  Rationale: Fixes the restart/edit failure without relying on `~/.gravimera/cache/` and without requiring heuristic reconstruction of the plan from geometry.
  Date/Author: 2026-03-08 / Codex

## Outcomes & Retrospective

(TBD once implemented.)

## Context and Orientation

This section describes the current (pre-change) repository behavior and the target end state.

### Current storage model (pre-change)

- There is a global model depot under `~/.gravimera/depot/models/<uuid>/...` described by `docs/gamedesign/36_model_depot_v1.md` and implemented in `src/model_depot.rs`.
- The in-memory `ObjectLibrary` is populated on scene load by:
  - loading depot prefab defs, then
  - loading realm prefab defs (`realm/<realm_id>/prefabs/packs/...`), then
  - upserting object defs contained in `scene.dat` itself.
  This happens in `src/scene_store.rs` in the `load_scene_dat` flow.
- Gen3D Save writes prefab defs to the depot and writes a “source bundle” (`gen3d_source_v1`) under the depot model directory, then spawns the instance into the world and triggers a scene save. This is implemented in `src/gen3d/save.rs`.
- Gen3D Edit/Fork seeding reads from the depot’s model folder and uses either the `gen3d_source_v1` bundle or reconstructs from saved prefabs. This is implemented in `src/gen3d/ai/orchestration.rs`.

Key terms (plain language):

- **Scene**: the currently loaded world state, persisted to a single file `scene.dat` under `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/build/scene.dat`. Code: `src/scene_store.rs`, paths: `src/paths.rs`.
- **Prefab**: a reusable object definition (`ObjectDef`) that can be instanced into the world. Object defs can reference other object defs by id via `ObjectPartKind::ObjectRef`. Code: `src/object/registry.rs` and serialization in `src/realm_prefabs.rs`.
- **Prefab descriptor**: a JSON sidecar (`*.desc.json`) with human/agent-facing metadata and provenance. Code: `src/prefab_descriptors.rs`.
- **Gen3D source bundle**: an editable JSON dump of the Gen3D draft and its component defs, currently written to `<model_dir>/gen3d_source_v1/`. Code: `src/gen3d/save.rs`.
- **Gen3D edit bundle (new in this plan)**: a small persisted JSON file that stores the Gen3D session state needed to perform deterministic edits (planned components + attachments + anchors + assembly revision metadata) after restarting.

### Target storage model (post-change)

We remove the global depot and realm-shared prefab packs. Each scene directory owns its prefabs.

Proposed on-disk layout under `GRAVIMERA_HOME` (default `~/.gravimera`):

    realm/<realm_id>/scenes/<scene_id>/
      build/
        scene.dat
      prefabs/
        <root_prefab_uuid>/
          prefabs/                      # prefab defs for this root package (PrefabFileV1 JSON)
            <any_object_uuid>.json
            <root_prefab_uuid>.desc.json
          gen3d_source_v1/              # present only for Gen3D-saved prefabs (editable defs bundle)
            <any_object_uuid>.json
          gen3d_edit_bundle_v1.json     # present only for Gen3D-saved prefabs (new; required for restart edit)
          materials/                    # prefab-scoped external assets (textures/meshes/etc)
            ... files ...

Notes:

- The directory name `<root_prefab_uuid>` is the stable identifier for the prefab package (it is also the root object id).
- `prefabs/` holds the runtime prefab defs in the same JSON schema currently used by `src/realm_prefabs.rs` (`PrefabFileV1`).
- `gen3d_source_v1/` holds the Gen3D draft bundle (the source of truth for edit geometry and anchors).
- `gen3d_edit_bundle_v1.json` is a deterministic “session hydration” file. It must be sufficient to re-create a Gen3D edit session after restart without reading `~/.gravimera/cache/`.
- `materials/` exists for all prefab packages, even if empty today (primitives-only models).

## Plan of Work

This plan is intentionally incremental but aims to land as a cohesive refactor (the current system is already complex; half-migrations increase confusion).

### 1) Update design docs and declare the new storage contract

Before code changes, write/update the relevant specs under `docs/gamedesign/` so the code has a clear “source of truth”:

- Deprecate/remove `docs/gamedesign/36_model_depot_v1.md` (or rewrite it as “removed in v2” and link to the new scene-local spec).
- Deprecate/remove `docs/gamedesign/34_realm_prefabs_v1.md` as “realm-shared packs are removed; prefabs are scene-local now”, or split into a generic prefab JSON spec plus a scene-local storage spec.
- Add a new spec doc describing the scene-local prefab package layout and the Gen3D edit bundle file (`gen3d_edit_bundle_v1.json`).
- Update `docs/gamedesign/specs.md` to point at the new/updated docs.

This doc work must define:

- Exactly where prefab packages live (paths and naming rules).
- The edit bundle JSON schema (fields and invariants).
- What counts as “runtime required” (only `scene.dat` + any referenced external material/mesh files) vs “edit required” (prefab packages).

### 2) Remove depot and realm-shared prefab loading from runtime

In code, make runtime scene load depend only on:

- built-in object defs (the ones compiled into the game), and
- `scene.dat` defs embedded in the scene file.

Concretely:

- Remove `src/model_depot.rs` and its `lib.rs` module wiring.
- Remove the depot directory creation from `src/paths.rs` (`ensure_default_dirs`).
- Remove the “load depot prefabs/descriptors” steps from the scene load pipeline (`src/scene_store.rs`).
- Remove the “load realm prefab packs” step from the scene load pipeline (`src/scene_store.rs`).

At this point, `scene.dat` must contain all object defs needed to spawn every instance in the scene, or scene load will log warnings and skip instances. If any current workflows produce instances whose defs never make it into `scene.dat`, fix that as part of this milestone (by ensuring the `ObjectLibrary` is always populated with any prefab ids referenced by world entities before saving).

### 3) Introduce a scene-local prefab store module

Create a new module responsible for scene-local prefab packages, something like:

- `src/scene_prefabs.rs` (new), or
- refactor `src/realm_prefabs.rs` into a generic `src/prefabs.rs` plus a scene-local wrapper.

It must support:

- Writing a prefab package for a given root prefab id into the current scene directory:
  - write prefab defs under `.../prefabs/<root_uuid>/prefabs/*.json`
  - write descriptor under `.../prefabs/<root_uuid>/prefabs/<root_uuid>.desc.json` (or next to the root json; keep existing `*.desc.json` rule)
  - ensure `materials/` exists
  - for Gen3D, write `gen3d_source_v1/` and `gen3d_edit_bundle_v1.json`
- Enumerating prefab packages in the current scene (for UI/tooling).
- Loading a prefab package’s defs/descriptor/edit bundle on demand (for edit/copy tools).

Important: runtime scene load should not depend on this module; it is for edit/copy tools only.

### 4) Change Gen3D Save to write to the current scene

Update `src/gen3d/save.rs` so “Save” writes a scene-local prefab package instead of writing to the depot:

- Locate the current realm/scene id (likely via `crate::realm::ActiveRealmScene` which already exists in the scene store).
- Choose the scene prefab package directory for the saved root id.
- Write the saved prefab defs into that package’s `prefabs/` directory (this is the “published” runtime defs).
- Write `gen3d_source_v1/` (the editable bundle) into that package.
- Write `gen3d_edit_bundle_v1.json` into that package (the new persisted edit state).

Ensure the in-memory `ObjectLibrary` is upserted with the saved defs so:

- the newly spawned instance renders immediately, and
- the next autosave writes a fully self-contained `scene.dat`.

### 5) Change Gen3D Edit/Fork seeding to hydrate session state (no cache dependency)

Update `src/gen3d/ai/orchestration.rs` seeded-session startup so that editing a previously saved Gen3D model works after restart:

- Instead of reading from the depot model directory, read from the current scene’s prefab package directory for the selected prefab id.
- Load `gen3d_source_v1/` defs into `draft.defs`.
- Load `gen3d_edit_bundle_v1.json` and hydrate:
  - `job.planned_components`
  - `job.plan_hash`
  - `job.assembly_rev`
  - `job.assembly_notes`
  - `job.plan_collider` (if needed for validation)
  - `job.motion_roles` / `job.motion_authoring` (optional but recommended)
  - `job.reuse_groups` and warnings (optional)
- Ensure the agent “has a workspace”: initialize `job.agent.workspaces` with a `main` workspace clone and set `job.agent.active_workspace_id = "main"` so the agent prompt state summary is not empty.

If `gen3d_edit_bundle_v1.json` is missing, return a clear user-facing error like “This prefab can’t be edited because it’s missing Gen3D edit metadata (gen3d_edit_bundle_v1.json).” We explicitly do not rely on `~/.gravimera/cache/`.

### 6) Materials are prefab-scoped

Make the “materials per prefab folder” rule real for any existing flows that emit or consume external assets:

- If Gen3D (or other tools) export GLB/texture assets, write them under `.../prefabs/<root_uuid>/materials/`.
- Ensure any `ObjectPartKind::Model { scene }` references point into a scene-local path (not a repo `assets/` path and not a depot path).
- If Bevy’s AssetServer cannot load from a scene-local absolute path today, introduce a small abstraction (an “asset root” under `GRAVIMERA_HOME`) so scene-local assets can be loaded by relative paths.

If the current game only uses primitive colors and built-in `MaterialKey`, implement this milestone as:

- Create the `materials/` directory during prefab package write, and
- Document that it is reserved for future non-primitive assets.

### 7) UI and tooling: replace the depot model library with a scene-local prefab browser

The old Models panel (`src/model_library_ui.rs`) lists depot models. Replace it with a scene-local prefab browser:

- Enumerate prefab packages under the current scene.
- Allow spawning a prefab into the world (optional; if not implemented, ensure Gen3D Save still spawns so users can at least place what they create).
- Allow copying a prefab package within the same scene (duplicate prefab ids if needed) and/or exporting/importing between scenes (explicit copy tool).

Keep scope minimal: the key user-visible requirement is that “restart + edit” works. Spawning/browsing is nice, but do not let UI scope block the storage refactor.

### 8) Validation, acceptance, and cleanup

At the end:

- Ensure `cargo test` passes.
- Run the rendered smoke test described in `AGENTS.md` (see “Concrete Steps”).
- Manually verify: Gen3D save-to-scene, restart, delete cache, edit works.
- Remove dead docs and update remaining docs so they match the new architecture.

## Concrete Steps

Run these from the repo root (`/Users/flow/workspace/github/gravimera` in this environment):

1. Code search inventory:

   - `rg -n "model_depot|depot/models" src`
   - `rg -n "realm_prefabs" src`
   - `rg -n "prefab_descriptors" src`

2. After implementing, run tests:

   - `cargo test`

3. Rendered smoke start (mandatory; do not use `--headless`):

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

4. Manual restart+edit scenario (example checklist):

   - Start the game with a fresh `GRAVIMERA_HOME`.
   - Enter Gen3D Preview, generate a simple model, click Save.
   - Verify files exist under `GRAVIMERA_HOME/realm/<realm>/scenes/<scene>/prefabs/<prefab_uuid>/...`.
   - Quit the game.
   - `rm -rf "$GRAVIMERA_HOME/cache"` (or delete only `cache/gen3d`).
   - Restart the game, select the saved instance, run the Gen3D edit/fix workflow, and verify edits apply.

## Validation and Acceptance

Acceptance is the following observable behavior:

- **No depot**: the directory `GRAVIMERA_HOME/depot/` is no longer created or used, and all references to `src/model_depot.rs` are gone.
- **No realm-shared prefab packs**: scenes do not load from `realm/<realm_id>/prefabs/packs/...` anymore.
- **Self-contained runtime**: loading a scene spawns all instances using only builtins + the `defs` embedded in `scene.dat`.
- **Scene-local Gen3D save**: saving a Gen3D model writes a prefab package under the current scene directory and does not write to any global store.
- **Restart-safe Gen3D edit**: after restart (and after deleting `GRAVIMERA_HOME/cache/`), the user can edit a previously saved Gen3D model and the Gen3D agent can operate on a loaded session (planned components present, `apply_draft_ops_v1` can modify attachments/anchors/parts).
- **Prefab-scoped materials**: any external assets associated with a prefab are stored under that prefab package’s `materials/` directory, or the directory exists as reserved empty structure if no external assets exist yet.

## Idempotence and Recovery

- Most changes here are destructive refactors. During implementation, keep a backup of any existing `~/.gravimera` data you care about.
- Provide a one-time “migration” helper only if it’s cheap; per `AGENTS.md`, backwards compatibility is not required.
- Ensure repeated saves overwrite the same prefab package folder when doing in-place edits (`edit_overwrite`), and create a new folder when doing forks.

## Artifacts and Notes

- Keep a short “before/after” directory tree in the doc updates under `docs/gamedesign/` so users can find their data on disk.
- When debugging restart/edit issues, capture:
  - the saved `gen3d_edit_bundle_v1.json`,
  - the scene’s `scene.dat`,
  - and the relevant `gravimera.log` lines indicating where data was loaded from.

## Interfaces and Dependencies

New/updated APIs (names are suggestions; pick stable names and keep them consistent):

- In `src/paths.rs`, add:
  - `pub(crate) fn scene_prefabs_dir(realm_id: &str, scene_id: &str) -> PathBuf`
  - `pub(crate) fn scene_prefab_package_dir(realm_id: &str, scene_id: &str, root_prefab_id: u128) -> PathBuf`
- Add a “scene-local prefab store” module:
  - `pub(crate) fn save_scene_prefab_package(...) -> Result<(), String>`
  - `pub(crate) fn load_scene_prefab_package_defs(...) -> Result<Vec<ObjectDef>, String>`
  - `pub(crate) fn load_scene_prefab_descriptor(...) -> Result<PrefabDescriptorFileV1, String>`
  - `pub(crate) fn load_gen3d_edit_bundle_v1(...) -> Result<Gen3dEditBundleV1, String>`

Define `gen3d_edit_bundle_v1.json` schema explicitly in the docs. At minimum it must contain:

- `version` (u32)
- `root_prefab_id` (UUID string)
- `created_at_ms` (u128 or u64)
- `plan_hash` (string)
- `assembly_rev` (u32)
- `planned_components` (full deterministic data to rebuild attachment tree and allow patch ops)
- `assembly_notes` (string; optional but useful)
- any other fields required so seeded edit sessions do not start “empty”.
