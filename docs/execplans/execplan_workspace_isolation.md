# Isolate Object Preview and Scene Build Workspaces

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `/Users/lxl/projects/aiprojects/gravimera/PLANS.md`. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

Creators need “Object Preview” and “Scene Build” to behave as isolated workspaces, so that switching tabs swaps the visible world instead of mixing objects. After this change, adding objects in Object Preview saves to `scene.dat`, switching to Scene Build loads a separate `scene.build.dat`, and switching back restores the original Object Preview content. The effect is observable by adding different objects in each workspace and seeing them appear/disappear as you toggle the top-left workspace dropdown.

## Progress

- [x] (2026-03-02 10:20Z) Capture current behavior, design decisions, and acceptance checks in this ExecPlan.
- [x] (2026-03-02 10:40Z) Add workspace-specific scene.dat path selection and update load/save logic to depend on the active workspace tab.
- [x] (2026-03-02 10:45Z) Add a workspace switch flow that saves the current workspace, clears selection/drag state, swaps scene entities, reloads the other workspace, and resets autosave state.
- [x] (2026-03-02 10:48Z) Update documentation to describe workspace isolation and file naming.
- [x] (2026-03-02 10:36Z) Run the rendered smoke test and record results.
- [x] (2026-03-02 10:52Z) Commit changes with a clear message.
- [ ] Add workspace-isolated camera state and update docs.
- [ ] Run the rendered smoke test and commit the camera isolation change.

## Surprises & Discoveries

- Observation: Bevy system functions hit the 16-parameter limit; the workspace switch system needed a `SystemParam` wrapper to compile.
  Evidence: `error[E0599]: no method named after found for fn item ...` traced to the oversized system signature.
- Observation: Camera state lives in `CameraZoom`, `CameraYaw`, `CameraPitch`, and `CameraFocus` resources and must be captured/restored for isolation.
  Evidence: `/Users/lxl/projects/aiprojects/gravimera/src/types.rs` defines these as resources and player camera systems mutate them.

## Decision Log

- Decision: Object Preview continues to use `scene.dat`, while Scene Build uses `scene.build.dat` in the same build directory.
  Rationale: Matches the user request and preserves existing Object Preview persistence.
  Date/Author: 2026-03-02 (Codex).

- Decision: Play mode continues to use the Object Preview workspace for now.
  Rationale: The user explicitly deferred deciding how Scene Build should play.
  Date/Author: 2026-03-02 (Codex).

- Decision: Workspace switching saves the current workspace immediately before swapping.
  Rationale: Ensures no data loss even if autosave has not run yet.
  Date/Author: 2026-03-02 (Codex).

## Outcomes & Retrospective

- Workspace switching now isolates Object Preview (`scene.dat`) from Scene Build (`scene.build.dat`), and load/save paths respect the active workspace.
- Documentation updated to explain workspace isolation and file naming.
- Rendered smoke test passes and changes are committed.
- Camera isolation work in progress.

## Context and Orientation

Gravimera’s build mode currently persists all in-world instances to a single protobuf file `scene.dat` per realm/scene, stored under `~/.gravimera/realm/<realm>/scenes/<scene>/build/scene.dat` (or overridden by `scene.scene_dat_path` in config). The workspace dropdown in the top-left UI (`Object Preview` / `Scene Build`) is purely visual today: it does not swap the world data. The persistence logic lives in `/Users/lxl/projects/aiprojects/gravimera/src/scene_store.rs`, and the workspace UI state lives in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs`. The scene load/save entrypoints are `load_scene_dat`, `scene_save_requests`, and `scene_autosave_tick`. Realm/scene switches are handled by `apply_pending_realm_scene_switch` in the same module.

“Workspace” in this change means: the saved world state file that should be shown when the user is on a given top-left workspace tab. Object Preview should map to `scene.dat`, and Scene Build should map to `scene.build.dat` in the same directory. Switching tabs should save the current workspace file, swap all current in-world entities, and load the other file so the world content appears isolated between tabs.

## Plan of Work

First, add a helper in `/Users/lxl/projects/aiprojects/gravimera/src/scene_store.rs` that maps `WorkspaceTab` to the correct scene data path. Use the existing `scene_dat_path` (Object Preview) and derive `scene.build.dat` in the same directory for Scene Build. Update `load_scene_dat`, `scene_save_requests`, `scene_autosave_tick`, and `apply_pending_realm_scene_switch` to use this workspace-aware path.

Next, add a new resource in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs` to record pending workspace switches. When the user clicks a different workspace tab in the dropdown, populate this resource with `from` and `to` tabs.

Implement a new system in `/Users/lxl/projects/aiprojects/gravimera/src/scene_store.rs` that consumes the pending switch and applies it. The system must:

- Save the current workspace to its file path before any world changes.
- Clear selection state and world drag state to avoid referencing despawned entities.
- Despawn existing BuildObject/Commandable entities.
- Reset the ObjectLibrary and prefab descriptor library to builtins + depot + realm prefabs (same logic as existing scene loads).
- Load the target workspace file and spawn its instances.
- Reset autosave timers (`dirty = false`, `primed = false`) so switching doesn’t trigger immediate autosave.

Wire the new system into `/Users/lxl/projects/aiprojects/gravimera/src/app_plugins.rs` so it runs after the workspace dropdown interaction system and before autosave detection.

Finally, update documentation in `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md`, `/Users/lxl/projects/aiprojects/gravimera/docs/development.md`, and `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md` to describe the two workspace files and the isolation behavior. Add a new short doc if a clearer place to explain file naming is needed.

## Concrete Steps

All commands run from `/Users/lxl/projects/aiprojects/gravimera`.

1) Edit `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs` to add a pending workspace switch resource and set it when the dropdown changes tabs.

2) Edit `/Users/lxl/projects/aiprojects/gravimera/src/scene_store.rs`:

   - Add a `workspace_scene_dat_path` helper.
   - Update load/save paths to use the active workspace.
   - Add the workspace switch system that saves, clears state, despawns, and reloads.

3) Edit `/Users/lxl/projects/aiprojects/gravimera/src/app.rs` to initialize the new resource.

4) Edit `/Users/lxl/projects/aiprojects/gravimera/src/app_plugins.rs` to register the workspace switch system with appropriate ordering.

5) Update docs:

   - `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md`
   - `/Users/lxl/projects/aiprojects/gravimera/docs/development.md`
   - `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md`
   - Add `/Users/lxl/projects/aiprojects/gravimera/docs/workspaces.md` if needed for a fuller explanation.

6) Run the rendered smoke test:

   tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

7) Commit with a clear message.

## Validation and Acceptance

In Build mode, add a few objects while in Object Preview. Switch the top-left workspace dropdown to Scene Build. The Object Preview objects should disappear, and if `scene.build.dat` is empty it should show an empty world. Add different objects in Scene Build, then switch back to Object Preview; the original objects should reappear. On disk, under `~/.gravimera/realm/<realm>/scenes/<scene>/build/`, both `scene.dat` and `scene.build.dat` should exist after you have created objects in both workspaces.

Run the rendered smoke test command listed above and confirm the app starts and exits without crashing.

## Idempotence and Recovery

Re-running the changes is safe: the workspace switch system always saves before swap and always reloads from the current workspace file. If a workspace file is missing or unreadable, the system should log a warning and load an empty world rather than crashing. If anything goes wrong, deleting `scene.dat` or `scene.build.dat` in the build directory is a safe reset for that workspace.

## Artifacts and Notes

- Expected log examples (not exhaustive):

  - Saved <N> scene instances to <...>/scene.dat (workspace switch).
  - Loaded <N> scene instances from <...>/scene.build.dat.

## Interfaces and Dependencies

This change uses existing types and modules:

- `crate::workspace_ui::WorkspaceTab` and a new `crate::workspace_ui::PendingWorkspaceSwitch` resource.
- `crate::scene_store::load_scene_dat`, `scene_save_requests`, `scene_autosave_tick`, and a new workspace switch system.
- `crate::scene_store::save_scene_dat_internal` and `load_scene_dat_from_path` for persistence.
- `crate::object::registry::ObjectLibrary` and `crate::prefab_descriptors::PrefabDescriptorLibrary` for prefab definitions.

No new external dependencies are required.

## Plan Revision Notes

- 2026-03-02 (Codex): Initial plan created to implement workspace isolation with `scene.dat` vs `scene.build.dat`.
- 2026-03-02 (Codex): Updated progress, recorded the SystemParam workaround, and noted current outcomes after implementation.
- 2026-03-02 (Codex): Marked commit completion and updated outcomes.
- 2026-03-02 (Codex): Added camera isolation tasks after user request.
