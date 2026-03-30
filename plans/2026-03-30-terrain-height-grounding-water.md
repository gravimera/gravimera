# Terrain Height Grounding And Water Blocking

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

After this change, the world terrain is no longer treated as a flat y=0 plane. Model placement, dragging, movement, pathing, and ground unit grounding all use the terrain's relief height at each point, so units no longer sink into the ground. Terrain relief heights below y=0 are treated as water: ground units and ground placement are blocked unless the object is supported by a `supports_standing` surface; air units are exempt. Grounding uses the maximum relief height under an instance footprint and applies a 0.02m sink for natural contact, except when the sampled height is exactly 0 (the default flat terrain should not be sunk).

## Progress

- [x] (2026-03-30 10:30 CST) Added terrain sampling helpers, footprint sampling, and grounding sink constant; re-exported interfaces via `src/genfloor/mod.rs`.
- [x] (2026-03-30 11:40 CST) Updated cursor pick, drag, placement, build preview, and automation placement logic to use terrain heights and block water for ground/static objects.
- [x] (2026-03-30 13:05 CST) Updated physics grounding, enemy spawns, and player setup to use terrain sampling and to block water without support.
- [x] (2026-03-30 14:15 CST) Updated navigation and callers (RTS orders, automation, host plugin) with walkability closures that block water.
- [x] (2026-03-30 15:05 CST) Updated docs to describe terrain grounding, sink behavior, and water blocking rules.
- [x] (2026-03-30 16:25 CST) Ran `cargo test` and the rendered smoke test.
- [x] (2026-03-30 16:35 CST) Committed changes.

## Surprises & Discoveries

Terrain-aware grounding required small test setup adjustments. The floor library button test now needs `FloorLibraryUiState` seeded, and the commandable grounding override test now derives its expected height from terrain sampling to avoid assuming a flat y=0 plane.

## Decision Log

- Decision: Grounding uses the maximum relief height under the instance footprint and applies a 0.02m sink, but skips the sink when the sampled height is exactly 0.
  Rationale: Prevents models from sinking into flat default terrain while still grounding on relief.
  Date/Author: 2026-03-30 / Codex
- Decision: Terrain heights below y=0 are treated as water that blocks ground placement and ground movement unless a `supports_standing` object provides a platform; air units are exempt.
  Rationale: Matches the requested water semantics while preserving existing support surfaces.
  Date/Author: 2026-03-30 / Codex
- Decision: Terrain sampling uses relief only and ignores animated waves.
  Rationale: Avoids oscillating model heights driven by wave animation.
  Date/Author: 2026-03-30 / Codex

## Outcomes & Retrospective

Terrain grounding now uses relief height sampling for placement, dragging, physics, and navigation, with water blocking for ground units unless supported by `supports_standing` objects. The 0.02m sink applies only when the sampled height is above 0, keeping default flat terrain unsunk. Tests and the rendered smoke test completed successfully.

## Context and Orientation

The active terrain is provided by `src/genfloor/runtime.rs` via the `ActiveWorldFloor` resource, which drives the world terrain mesh and provides relief height sampling. Existing placement, drag, navigation, and physics logic previously assumed ground was y=0. These behaviors now need to sample terrain height and avoid water (relief < 0), while still allowing ground units to stand on `supports_standing` build objects.

Key touchpoints include placement and drag in `src/model_library_ui.rs`, `src/world_drag.rs`, and `src/build.rs`; grounding and collision in `src/physics.rs`; navigation in `src/navigation.rs` plus callers in `src/rts.rs`, `src/automation/mod.rs`, and `src/intelligence/host_plugin.rs`; and spawn/setup in `src/enemies.rs` and `src/setup.rs`.

## Plan of Work

First, add terrain sampling helpers in `src/genfloor/runtime.rs` that return height and water flags for a point or footprint. Export these helpers from `src/genfloor/mod.rs`. Define a grounding sink constant in `src/constants.rs` and implement `apply_floor_sink` so it subtracts 0.02m for heights above zero while leaving height 0 unchanged.

Next, update cursor picking, drag-and-drop placement, and build previews to use terrain sampling instead of a fixed y=0 ground plane. When the picked surface is the terrain, use the footprint's maximum relief height plus sink to place ground/static objects. If the footprint crosses water and there is no supporting object, block placement. Continue allowing placement on the tops of `supports_standing` objects, even when the terrain underneath is water.

Then update physics and spawning so ground units follow terrain height and cannot move into water unless supported. Enemy spawning and player setup should sample terrain height, and enemy spawn candidates should avoid water. Ground units should remain at their previous position if their movement step would enter water without support.

Finally, update navigation to accept a walkability closure and ensure all callers supply terrain-aware water blocking logic. Pathfinding and path smoothing should reject water nodes unless support exists. Update docs to describe terrain grounding, sinking, and water behavior.

## Concrete Steps

Working directory: `/Users/lxl/projects/aiprojects/gravimera`

1) Implement code and doc changes as described above. Re-run `rg` to confirm no stale call sites remain.

2) Run tests:

    cargo test

3) Run the rendered smoke test (not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4) Commit with a clear message, for example 'Ground models to terrain height and block water placement'.

## Validation and Acceptance

- Dragging or placing models on terrain with relief should align the model to the terrain, with a subtle 0.02m sink on non-zero heights and no sink when the terrain height is exactly 0.
- Terrain relief below y=0 should be treated as water: ground/static placement and ground unit movement should be blocked there unless a `supports_standing` object provides a platform; air units should remain unrestricted.
- Ground units can stand on the top surfaces of `supports_standing` objects even when the terrain beneath is water.
- The rendered smoke test should start and exit after 2 seconds without a crash.

## Idempotence and Recovery

These changes are deterministic and safe to re-apply. If a step fails, revert to the previous commit and re-run the plan. The smoke test uses a temporary `GRAVIMERA_HOME` so it does not mutate persistent data.

## Artifacts and Notes

If additional test artifacts are needed, place them under `./test/run_1` to keep the workspace clean. Do not add details to `README.md`; keep detailed notes in `docs/`.

## Interfaces and Dependencies

In `src/constants.rs`, define:

    pub(crate) const FLOOR_GROUND_SINK_M: f32 = 0.02;

In `src/genfloor/runtime.rs`, define (names may vary but must be stable and reusable):

    pub(crate) enum FloorFootprint { Circle { radius: f32 }, Aabb { half: Vec2 } }
    pub(crate) struct FloorSample { pub(crate) height: f32, pub(crate) is_water: bool }
    pub(crate) struct FloorFootprintSample { pub(crate) max_height: f32, pub(crate) min_height: f32, pub(crate) is_water: bool }
    pub(crate) fn sample_floor_point(active: &ActiveWorldFloor, x: f32, z: f32) -> FloorSample
    pub(crate) fn sample_floor_footprint(active: &ActiveWorldFloor, center: Vec2, footprint: FloorFootprint) -> FloorFootprintSample
    pub(crate) fn apply_floor_sink(height: f32) -> f32

`apply_floor_sink` must subtract `FLOOR_GROUND_SINK_M` only when `height > 0`, leaving `height == 0` untouched.

In `src/genfloor/mod.rs`, re-export the above types and helpers.

In `src/cursor_pick.rs`, extend `SurfacePick` with:

    pub(crate) struct SurfacePick { pub(crate) hit: Vec3, pub(crate) surface_y: f32, pub(crate) block_top: Option<(Vec2, Vec2)>, pub(crate) floor_is_water: bool }

and update `cursor_surface_pick` to accept `ActiveWorldFloor` and return terrain sample heights instead of y=0.

In `src/navigation.rs`, update `find_path_height_aware` and `smooth_path_height_aware` to accept a walkability closure and block water when the closure returns false. Update callers in `src/rts.rs`, `src/automation/mod.rs`, and `src/intelligence/host_plugin.rs` to pass a closure that uses terrain sampling and `supports_standing` object checks.

Plan change note (2026-03-30): Created the plan from the user-provided ExecPlan, translated it into English, updated Progress to reflect implementation status, and added the explicit 'no sink at height 0' decision requested by the user.
Plan change note (2026-03-30): Updated Progress after running tests and the smoke test, and captured the terrain-aware test setup adjustments in Surprises & Discoveries.
Plan change note (2026-03-30): Marked the work complete after tests and smoke validation and recorded outcomes.
