# ExecPlan: Per-Scene Player Character Selection and Persistence

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

After this change, each scene can designate exactly one **Player Character** (the unit the player controls). Any commandable model can be set as the Player Character per scene, the choice is saved in `scene.dat`, and it is restored on next startup. The Player Character is also selected automatically on scene load so the user immediately controls the right unit without manual selection.

You can see this working by starting the rendered game, opening the Meta panel for a commandable unit, setting it as Player Character, saving or switching scenes, restarting, and observing the same unit is selected and controlled on load.

## Progress

- [x] (2026-03-11 02:10Z) Create this ExecPlan and inventory the target files.
- [x] (2026-03-11 02:40Z) Update scene persistence to store per-instance Player Character flag, load it, and enforce a single Player Character per scene.
- [x] (2026-03-11 02:40Z) Move Player Character visuals to the prefab visual path so any model can be the Player Character.
- [x] (2026-03-11 02:45Z) Update Meta UI to show Player Character status and allow setting it.
- [x] (2026-03-11 02:45Z) Ensure the Player Character is selected on scene load when selection is empty.
- [x] (2026-03-11 02:46Z) Update docs and run the rendered smoke test.
- [x] (2026-03-11 02:50Z) Commit the change with a concise message.

## Surprises & Discoveries

- (none yet)

## Decision Log

- Decision: Store Player Character selection as a per-instance boolean in `scene.dat` (v9) and require exactly one per scene.
  Rationale: This keeps the persistence local to the scene and avoids global state or heuristics.
  Date/Author: 2026-03-11 / Codex
- Decision: If no Player Character is flagged, fall back to the hero prefab or spawn a hero instance and mark it as Player Character in memory.
  Rationale: Removing the hardcoded player spawn requires a deterministic fallback to avoid empty scenes with no controllable unit.
  Date/Author: 2026-03-11 / Codex
- Decision: Spawn Player Character visuals through `object::visuals` (hero prefab special-case) instead of hardcoding a player entity in setup.
  Rationale: This keeps visuals consistent with prefab-driven instances and allows any model to become the Player Character.
  Date/Author: 2026-03-11 / Codex

## Outcomes & Retrospective

- ✅ `scene.dat` now stores a per-instance Player Character flag and enforces a single Player Character per scene on load.
- ✅ Player Character visuals are spawned via the prefab visual path, enabling any commandable unit to become the Player Character.
- ✅ Meta UI exposes Player Character status and a “Set as Player Character” action.
- ✅ Player Character selection is automatic on load when selection is empty.
- ✅ Rendered smoke test completed successfully (2026-03-11).

## Context and Orientation

The rendered game loads scenes from `scene.dat` via `src/scene_store.rs`. Instance records are encoded in a protobuf message (`SceneDat`) and loaded into the world with prefabs from `ObjectLibrary`. Player visuals are currently spawned directly in `src/setup.rs` as a hardcoded entity, which prevents other models from becoming the player. The Meta panel is built in `src/motion_ui.rs`. Selection state is held in `SelectionState` (`src/types.rs`) and default selection on entering Play is enforced in `src/rts.rs`.

Key files:

- `src/scene_store.rs`: scene save/load and autosave logic; `SceneDat` format and versioning.
- `src/setup.rs`: startup scene creation and initial player spawn.
- `src/assets.rs`: shared meshes/materials in `SceneAssets`.
- `src/models.rs`: model-specific spawn helpers used by visuals.
- `src/object/visuals.rs`: spawns visuals for prefabs and part trees.
- `src/motion_ui.rs`: Meta panel rendering and button actions.
- `src/rts.rs`: selection rules and default selection behavior.
- `docs/controls.md`, `docs/development.md`, `docs/gamedesign/37_object_forms_and_transformations.md`: user and dev docs that describe persistence and UI.

Definitions:

- Player Character: the single unit per scene that the player controls; stored as `Player` component at runtime and persisted as `is_protagonist` in `scene.dat`.
- Commandable: a unit that can accept orders (movement/attack), required for Player Character eligibility.

## Plan of Work

First, extend the scene persistence layer in `src/scene_store.rs` by adding a boolean `is_protagonist` field to `SceneDatObjectInstance`, bumping `SCENE_DAT_VERSION` to 9, and updating save/load logic. Saving should include `Option<&Player>` in the instance query and warn if multiple Player components exist, persisting only the first. Loading should accept versions 7, 8, and 9; track the flagged Player Character, fall back to the hero prefab if none is flagged, and ensure the chosen entity receives `Player`, `Health`, `LaserDamageAccum`, and `PlayerAnimator`.

Second, move Player Character visuals into the prefab visual pipeline. Add player meshes/materials to `SceneAssets` and use them in a new `spawn_player_model` helper in `src/models.rs`. Update `src/object/visuals.rs` to special-case the hero prefab id and spawn the player model there, and remove the hardcoded player entity/children from `src/setup.rs`. Keep `PlayerMuzzles` creation in setup since it is used by combat.

Third, update the Meta panel in `src/motion_ui.rs` to show Player Character status and provide a button to set the selected commandable unit as Player Character. The button action should remove `Player` from other entities, add `Player` components to the target entity, update camera focus, and queue a `SceneSaveRequest`.

Fourth, ensure the Player Character is selected on load if selection is empty by adding a system in `src/rts.rs` that selects the first added `Player` entity.

Finally, update docs to reflect the new Player Character UI and persistence, run the rendered smoke test, and commit.

## Concrete Steps

Work in `/Users/lxl/projects/aiprojects/gravimera`.

1. Edit `src/assets.rs`, `src/setup.rs`, `src/models.rs`, and `src/object/visuals.rs` to move player visuals into the prefab visual path.
2. Edit `src/scene_store.rs` to add the `is_protagonist` field and update save/load/autosave logic.
3. Edit `src/motion_ui.rs` to add Player Character UI and button behavior; update `src/app_plugins.rs` to register new systems.
4. Edit `src/rts.rs` to add a default-selection-on-player-added system and register it.
5. Update docs and ExecPlan progress.
6. Run:

   tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

7. Commit with a concise message.

## Validation and Acceptance

Acceptance requires:

- On launch, the Player Character from `scene.dat` is selected automatically if selection is empty.
- Meta panel shows Player Character status and setting it updates the Player Character immediately.
- Reloading the scene restores the chosen Player Character from `scene.dat`.
- Rendered smoke test runs without a crash.

## Idempotence and Recovery

All edits are additive or replace existing logic; rerunning the steps is safe. If loading fails due to a format mismatch, `scene.dat` can be regenerated by deleting the file and letting the default scene scaffold recreate it.

## Artifacts and Notes

Expected runtime log snippets (examples):

    Loaded <N> scene instances from .../scene.dat.
    Saved <N> scene instances to .../scene.dat (reason).

## Interfaces and Dependencies

- `SceneDatObjectInstance` gains `is_protagonist: bool` (tag 19).
- `save_scene_dat_internal` query includes `Option<&Player>` and persists `is_protagonist`.
- `load_scene_dat_from_path` assigns `Player` components to the chosen entity.
- `models::spawn_player_model(parent: &mut ChildSpawnerCommands, assets: &SceneAssets)` spawns the Player Character model parts.
- `rts::ensure_default_selection_on_player_added` selects the added `Player` when selection is empty.

Plan update (2026-03-11 02:50Z): Marked the commit step complete and added the Outcomes & Retrospective summary now that implementation and validation are done.
