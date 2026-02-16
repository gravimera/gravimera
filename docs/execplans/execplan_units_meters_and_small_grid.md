# Project-wide Units: meters + small grid + cm persistence

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Today the project uses Bevy “world units” as an implicit length system, but several subsystems derive their resolution from gameplay constants (for example the Build grid is derived from hero height). This creates hidden coupling: changing a character’s size can unintentionally change build snapping, pathfinding discretization, and persistence quantization. In an AI-authored world, that kind of coupling makes it harder to align objects predictably and can cause “incompatibility” between content sources (built-ins, Gen3D outputs, scene sources, and saved scenes).

After this change, the whole project uses a single, explicit size system:

- **World space is meters** (`1.0 == 1 meter`) for all `Transform.translation`, `ObjectDef.size`, collider sizes, and speeds.
- **Build placement snaps to a small, fixed grid** (chosen here as **5 cm**).
- **Scene persistence quantizes positions in centimeters** (so saves are stable and independent of gameplay constants).

You can see it working by:

1) Placing objects in Build mode and observing the snap step is 5 cm (much finer than before).
2) Confirming RTS/pathfinding still behaves similarly (it must not become “micro-steppy”).
3) Saving a scene and inspecting that `scene.dat` loads without crashing; older `scene.dat` versions are ignored rather than crashing.

## Progress

- [x] (2026-02-16 05:41Z) Wrote ExecPlan for normalizing units to meters + small build grid + cm persistence.
- [x] (2026-02-16 05:41Z) Decoupled Build grid from hero size (explicit constants in `src/constants.rs`).
- [x] (2026-02-16 05:41Z) Decoupled navigation discretization from Build grid (added `NAV_HEIGHT_QUANT_SIZE`; updated `src/navigation.rs`).
- [x] (2026-02-16 05:41Z) Updated fence/build preview math to stop assuming grid size == build module size.
- [x] (2026-02-16 05:41Z) Changed `scene.dat` quantization to centimeters; bumped `SCENE_DAT_VERSION`; kept “ignore unsupported version” behavior.
- [x] (2026-02-16 05:41Z) Replaced “missing size” fallbacks that used `BUILD_UNIT_SIZE` with a stable meter-based default.
- [x] (2026-02-16 05:41Z) Updated docs (`README.md`) to state the unit contract and grid step.
- [x] (2026-02-16 05:41Z) Ran `cargo test` and the headless smoke test.
- [x] (2026-02-16 05:41Z) Committed changes (`14b0730`).

## Surprises & Discoveries

- Observation: The Build grid size is derived from hero height (`BUILD_UNIT_SIZE = HERO_HEIGHT_WORLD / 7.0`) and reused outside Build mode.
  Evidence: `src/constants.rs` defines `BUILD_UNIT_SIZE` from `HERO_HEIGHT_WORLD` and sets `BUILD_GRID_SIZE = BUILD_UNIT_SIZE`, and other systems derive values from `BUILD_GRID_SIZE`.

- Observation: Some prefab geometry math assumes `BUILD_GRID_SIZE` is roughly the “build module” size, not the snap step.
  Evidence: Fence stake placement uses `stake_offset = BUILD_FENCE_LENGTH * 0.5 - BUILD_GRID_SIZE * 0.5`. If `BUILD_GRID_SIZE` becomes small, the stakes extend outside `ObjectDef.size` bounds.

- Observation: The project already ignores unsupported `scene.dat` versions at load time (good).
  Evidence: `src/scene_store.rs` checks `scene.version != SCENE_DAT_VERSION`, warns, and returns `Ok(0)`.

## Decision Log

- Decision: Standardize “world units are meters” and document it as an explicit contract.
  Rationale: It removes ambiguity across subsystems and makes AI prompts and authored assets easier to reason about.
  Date/Author: 2026-02-16 / assistant

- Decision: Use a 5 cm Build snap grid (`0.05 m`) and keep navigation/pathfinding grid size separate (coarser).
  Rationale: Build placement benefits from fine snapping, but pathfinding must remain computationally manageable and visually stable.
  Date/Author: 2026-02-16 / assistant

- Decision: Quantize `scene.dat` positions in centimeters (units-per-meter) and bump the file version; unsupported versions are ignored.
  Rationale: Persistence should not depend on gameplay constants like hero height or build grid size. The product requirement is “no compatibility needed; don’t crash on old/unknown formats”.
  Date/Author: 2026-02-16 / assistant

## Outcomes & Retrospective

Implemented the new unit contract across the codebase:

- World distances are treated as meters and the Build snap grid is now 5 cm.
- Navigation uses its own constants (`NAV_GRID_SIZE` and `NAV_HEIGHT_QUANT_SIZE`) so it does not become “micro-steppy” when Build snapping is small.
- `scene.dat` now stores position units as centimeters (units-per-meter) and the version was bumped; unsupported versions continue to be ignored rather than crashing.
- Documentation (`README.md`) now states the units contract.

What remains: ship the commit and then (optionally) update additional design docs to explicitly state that distances are meters.

## Context and Orientation

Key definitions:

- “World unit”: the numeric unit used by Bevy `Transform` values.
- “Meter contract”: a project-wide rule that `1.0` world unit is treated as **1 meter**.
- “Build grid”: the snapping step when placing/editing objects in Build mode.
- “Navigation grid”: the XZ discretization size used by A* pathfinding; it should not be tied to Build snapping.
- “Scene persistence”: `scene.dat` save/load (`src/scene_store.rs`) for build objects and units; it already ignores unsupported versions.

Key files:

- `src/constants.rs`: current derived grid constants; will be updated to explicit meter-based values.
- `src/build.rs`: Build snapping and preview gizmos; contains fence preview math that assumes old grid behavior.
- `src/object/types/buildings/fence_x.rs`, `src/object/types/buildings/fence_z.rs`: fence prefab geometry math that currently uses `BUILD_GRID_SIZE`.
- `src/navigation.rs`: height-aware A*; currently quantizes ground height using `BUILD_GRID_SIZE` (will be split).
- `src/rts.rs`: uses `NAV_GRID_SIZE` for pathfinding and uses `BUILD_GRID_SIZE` as a safety clamp in goal selection.
- `src/gen3d/save.rs`: uses `BUILD_GRID_SIZE` as a spacing/padding unit for saved model spawn positions (will be adjusted).
- `src/scene_store.rs`: `scene.dat` encode/decode and quantization; will be changed to centimeters and version bumped.

## Plan of Work

1) Make the unit contract explicit in `src/constants.rs` and decouple grid constants:

   - Keep “world units are meters” as comments/documentation.
   - Change `BUILD_UNIT_SIZE` from “derived from hero” to a fixed meter value used only as a prefab sizing convenience (target: 0.25 m).
   - Change `BUILD_GRID_SIZE` to a fixed small snap step (target: 0.05 m).
   - Change `NAV_GRID_SIZE` to a fixed coarser cell size (target: 0.50 m).
   - Add `NAV_HEIGHT_QUANT_SIZE` (target: 0.25 m) and stop using `BUILD_GRID_SIZE` for ground-height quantization.
   - Decouple `CLICK_MOVE_WAYPOINT_EPS` from `BUILD_GRID_SIZE` (keep approximately the old behavior in meters).

2) Update navigation to use the new height quantization constant (`NAV_HEIGHT_QUANT_SIZE`).

3) Update any gameplay logic that used `BUILD_GRID_SIZE` as a generic padding unit:

   - `src/gen3d/save.rs`: use `BUILD_UNIT_SIZE` (or explicit meter constants) for spawn spacing/padding and air height, but keep `BUILD_GRID_SIZE` for snapping.
   - `src/rts.rs`: replace `min_half = BUILD_GRID_SIZE * 0.5` with a more stable meter-based clamp (likely `BUILD_UNIT_SIZE * 0.5`).

4) Fix prefab geometry math that assumed build grid == build module:

   - Fences: compute stake offsets based on stake thickness (or fence width), not the snap grid step.
   - Keep the visual results similar and ensure `ObjectDef.size` still bounds the part geometry.

5) Update `scene.dat` quantization:

   - Bump `SCENE_DAT_VERSION`.
   - Change the quantization field to “units per meter” (centimeters: 100).
   - Remove `BUILD_UNIT_SIZE` from the quantization math so persistence stays stable even if build prefabs change.
   - Preserve the existing behavior: if the file version is unsupported, warn and ignore (do not crash).

6) Replace “missing size” fallbacks:

   - In screenshot framing and scene imports, use a stable meter-based default size such as `Vec3::splat(1.0)` rather than `Vec3::splat(BUILD_UNIT_SIZE)`.

7) Update docs:

   - `README.md`: add a short “Units” section: world units are meters, build grid is 5 cm, persistence quantizes to centimeters.
   - If any design docs describe transforms without units, add a sentence “All distances are meters”.

8) Validate and commit:

   - Run `cargo test`.
   - Run smoke test (AGENTS.md): `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --headless --headless-seconds 1`.
   - Commit with a message like: `units: meters world + 5cm build grid + cm scene.dat`.

## Concrete Steps

All commands below run from the repo root: `/Users/flow/workspace/github/gravimera`.

1) Code edits (in order):

   - Edit `src/constants.rs` as described in Plan of Work (decouple units and grids).
   - Edit `src/navigation.rs` to use `NAV_HEIGHT_QUANT_SIZE` (no `BUILD_GRID_SIZE` usage).
   - Edit `src/rts.rs` and `src/gen3d/save.rs` to remove unintended dependence on `BUILD_GRID_SIZE`.
   - Edit fence definitions in:
     - `src/object/types/buildings/fence_x.rs`
     - `src/object/types/buildings/fence_z.rs`
     - and matching preview math in `src/build.rs`.
   - Edit `src/scene_store.rs` to bump version and use centimeters.
   - Replace missing-size fallbacks in:
     - `src/scene_sources_runtime.rs`
     - `src/scene_build_ai.rs`
     - `src/scene_store.rs` spawn helpers.

2) Tests:

    cargo test

3) Smoke test (AGENTS.md):

    tmpdir=$(mktemp -d)
    GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --headless --headless-seconds 1

4) Commit:

    git status
    git commit -am "units: meters world + small build grid + cm scene.dat"

## Validation and Acceptance

This work is accepted when:

- Build snapping uses the new small grid step and does not depend on hero size.
- Pathfinding still works and does not become micro-steppy (nav grid remains coarser than build snap).
- `scene.dat` loads without crashing; unsupported versions are ignored with a warning.
- `cargo test` passes and the headless smoke test runs and exits cleanly.
- `README.md` states the unit contract and grid step.

## Idempotence and Recovery

- If the build/grid changes produce surprising gameplay, revert the commit and adjust constants only (do not mix refactors).
- If `scene.dat` changes cause load issues, the system must continue to ignore unsupported versions and start with an empty world rather than crashing.

## Artifacts and Notes

Key target numbers (meters):

- Build snap step: 0.05 m (5 cm).
- Build prefab “module” constant: 0.25 m (25 cm).
- Nav grid cell size: 0.50 m (50 cm).
- Scene persistence quantization: 100 units per meter (1 cm).
