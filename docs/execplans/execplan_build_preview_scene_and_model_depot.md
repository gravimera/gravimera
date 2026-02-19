# Build Mode: Realm vs Preview Scene + Model Depot Panel

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with that file.

## Purpose / Big Picture

After this change, the game exposes only two top-level modes: **Build** and **Play**. While in **Build** mode, the user can switch between:

- the **Realm scene** (normal world view + build tools), and
- the **3D Preview scene** (Gen3D workshop UI for generating + previewing models).

Gen3D exists only in the 3D Preview scene. A **Models** side panel on the left lists all available depot models and is visible in both Build scenes. Users can click a model to spawn it near the hero, or drag it to place it at a picked location with a dashed placement frame. In Build mode, selected scene instances (units/build objects) can be dragged to reposition them.

You can see this working by:

1. Running the rendered game (`cargo run`).
2. Pressing `Tab` to switch between Build and Play.
3. In Build mode, using the scene switch button to enter the 3D Preview scene and back to the Realm scene.
4. In the Models panel, clicking to spawn a depot model near the hero, or dragging to place it (dashed frame preview).
5. Selecting a spawned instance and dragging it to move it.
6. In the 3D Preview scene, generating and saving a Gen3D draft and then observing the saved model appears in the Models list (loaded from the depot).

## Progress

- [x] (2026-02-19) Create this ExecPlan and inventory relevant code/docs.
- [x] (2026-02-19) Refactor game state to `GameMode::{Build,Play}` plus `BuildScene::{Realm,Preview}`.
- [x] (2026-02-19) Replace the Gen3D mode toggle with a Build scene switch button (Build only).
- [x] (2026-02-19) Gate Gen3D systems and “world interaction” systems by `BuildScene` (Preview disables world input like before).
- [x] (2026-02-19) Update Automation HTTP API mode surface to reflect `build_scene` while keeping best-effort compatibility with `"gen3d"`.
- [x] (2026-02-19) Update docs to match: depot layout + UI/mode behavior.
- [x] (2026-02-19) Run `cargo test` and a headless smoke start (`cargo run -- --headless --headless-seconds 1`).
- [ ] Commit with a clear message.

## Surprises & Discoveries

- Observation: Gen3D is currently implemented as `GameMode::Gen3D` with many `run_if(in_state(GameMode::Gen3D))` / `not(in_state(GameMode::Gen3D))` gates in `src/app.rs`.
  Evidence: `src/app.rs` schedules Gen3D systems under `in_state(GameMode::Gen3D)` and disables camera/selection/build inputs with `not(in_state(GameMode::Gen3D))`.
- Observation: When Preview is a Build sub-scene, any systems that run in Build mode must also be explicitly gated by `BuildScene::Realm` to avoid “world interaction” while the Gen3D overlay is open.
  Evidence: Build placement and combat/bullet systems required `run_if(in_state(BuildScene::Realm))` after removing `GameMode::Gen3D`.

## Decision Log

- Decision: Represent “3D Preview scene” as a separate Bevy `State` (`BuildScene`) instead of a third `GameMode`.
  Rationale: Matches the requested UX (“2 modes”) while preserving the existing “enter/exit Gen3D workshop overlay” behavior via state transitions.
  Date/Author: 2026-02-19 / Codex

## Outcomes & Retrospective

Shipped a two-mode (Build/Play) state model with a Build sub-scene switch (Realm/Preview) that hosts the Gen3D workshop. The Models panel remains available in both Build scenes, and world interaction stays disabled during Preview to match the prior Gen3D-mode behavior.

## Context and Orientation

Key modules involved:

- `src/types.rs`: defines `GameMode` as a Bevy `States` enum.
- `src/app.rs`: wires Bevy schedules, including per-mode gates and `OnEnter/OnExit` for Gen3D.
- `src/setup.rs`: spawns the top-left Gen3D toggle UI button.
- `src/gen3d/`: Gen3D workshop UI, preview rendering, AI orchestration, and save-to-depot behavior.
- `src/model_depot.rs`: local depot layout for generated models under `~/.gravimera/depot/models/<uuid>/prefabs/`.
- `src/model_library_ui.rs`: left “Models” panel listing depot models and supporting click/drag spawn with dashed placement preview.
- `src/world_drag.rs`: drag-to-move for selected units/build objects in Build mode.
- `src/automation/mod.rs`: local Automation HTTP API that currently exposes `mode=build/play/gen3d`.

Terms used here:

- **Realm scene**: the normal in-world 3D camera view where instances live and are persisted in `scene.dat`.
- **3D Preview scene**: the Gen3D workshop overlay + preview render (model turntable render-to-texture) used for asset creation.
- **Depot model**: a generated model stored outside any realm, keyed by a UUID. Its prefab JSON (and descriptor JSON) live under `~/.gravimera/depot/models/<model_uuid>/prefabs/`.

## Plan of Work

### 1) Introduce a `BuildScene` state and remove `GameMode::Gen3D`

In `src/types.rs`:

- Change `GameMode` to contain only `Build` and `Play`.
- Add a new `States` enum `BuildScene` with `Realm` (default) and `Preview`.

In `src/app.rs`:

- Initialize `BuildScene` state.
- Replace `OnEnter(GameMode::Gen3D)` / `OnExit(GameMode::Gen3D)` with `OnEnter(BuildScene::Preview)` / `OnExit(BuildScene::Preview)`.
- Replace all `run_if(in_state(GameMode::Gen3D))` with `run_if(in_state(BuildScene::Preview))`.
- Replace all `run_if(not(in_state(GameMode::Gen3D)))` with `run_if(not(in_state(BuildScene::Preview)))`.
- Ensure world interaction systems (selection, build placement, camera controls, world dragging, etc.) remain disabled while in Preview (same behavior as the old Gen3D mode).

Add a small safety system: when entering `GameMode::Play`, force `BuildScene` back to `Realm` so Preview never “sticks” into Play mode.

### 2) Replace the Gen3D toggle button with a Build scene switch button

In `src/setup.rs`, replace the existing Gen3D toggle button UI with a “Scene” switch button that is visible only in Build mode.

In `src/gen3d/state.rs` and `src/gen3d/ui.rs`:

- Remove `Gen3dReturnMode` and any logic that stores a return `GameMode`.
- Update the toggle handler to set `NextState<BuildScene>` between `Realm` and `Preview`.
- Update the button label to reflect the current `BuildScene` (for example, `Preview` when in Realm, and `Realm` when in Preview).

### 3) Update mode-conditional behavior elsewhere

- `src/ui.rs`: window title should show Build/Play, and in Build include whether the user is in Preview (e.g., “BUILD (Preview)”).
- `src/build.rs`: `toggle_game_mode` (Tab) should remain disabled while in Preview (to match prior behavior in Gen3D).
- `src/model_library_ui.rs`: visibility should be `Build` (regardless of `BuildScene`) and hidden in Play.
- Any other direct checks of `GameMode::Gen3D` should be replaced with `BuildScene::Preview` checks.

### 4) Update Automation HTTP API to reflect the new state surface

In `src/automation/mod.rs`:

- `/v1/state` should return `mode: "build"|"play"` plus a new field `build_scene: "realm"|"preview"` (when mode is build).
- `/v1/mode` should accept only `"build"` and `"play"` for the true game mode.
- For best-effort compatibility, treat `"gen3d"` / `"gen3d_workshop"` as `mode=build` + `build_scene=preview`.

### 5) Update docs to match code

Update (at least):

- `README.md`: data directory section should document the depot path (`~/.gravimera/depot/models/...`) and note Gen3D saves generated models to the depot.
- `docs/gamedesign/02_game_modes_and_loops.md`: describe only Build and Play as modes; describe Gen3D as a Build sub-scene (Preview).
- `docs/gamedesign/specs.md`: add a new spec link for a depot model storage format.
- Add `docs/gamedesign/<new>_model_depot_v1.md` documenting the depot directory layout and file naming rules (UUID folder per model, `prefabs/` holding prefab JSON + `.desc.json` descriptor).
- `docs/gamedesign/34_realm_prefabs_v1.md`: remove or clarify any statements that Gen3D output is stored inside realm prefabs by default.

Keep `README.md` succinct; place detailed depot format notes in the new `docs/gamedesign/*` spec doc.

## Concrete Steps

Run these from the repo root:

1. `rg -n "GameMode::Gen3D" src` and update each usage to the new state model.
2. `cargo test`
3. `cargo run -- --headless --headless-seconds 1`

## Validation and Acceptance

Acceptance is manual + automated:

- Automated: `cargo test` passes; headless start does not crash.
- Manual (rendered): In Build mode you can switch between Realm and Preview with the new switch button; Gen3D UI only appears in Preview; Models panel is visible in both; world input (selection/build placement/unit move/drag) remains disabled while in Preview.
- Manual (persistence): Saving a Gen3D draft writes prefab JSON under `~/.gravimera/depot/models/<uuid>/prefabs/` and the model appears in the Models list after save.

## Idempotence and Recovery

- Switching modes/scenes should be safe to repeat; entering/leaving Preview must despawn Gen3D UI roots/cameras/lights cleanly (no duplicates).
- If the Automation API is used to force modes, entering Play should always return BuildScene to Realm.

## Artifacts and Notes

- If behavior differs from expectations, check Bevy logs and `~/.gravimera/cache/` artifacts; Gen3D save artifacts include `save_*.json` in the active run directory.
