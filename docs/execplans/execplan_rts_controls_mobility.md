# RTS Controls + Data-Driven Mobility + Move Animations

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gravimera uses Warcraft-like controls for commanding characters in both Build and Play modes:

- Left mouse click selects a character (dragging creates a selection box and selects multiple).
- Right mouse click issues a move command to the destination for the selected characters.
- Holding `A` makes selected characters fire continuously; `LMB` sets the fire direction/target (if `LMB` clicks an enemy, keep firing at that enemy while `A` is held).

Separately, the object system becomes able to represent “movable objects” in a data-driven way (walk/run/fly) and optionally define move-driven animations that play only while the object is moving. Gen3D can generate these mobility + animation hints as data, and `scene.dat` persists them.

You can see this working by:

- Starting the game and selecting the hero with `LMB`, then `RMB` to move.
- Drag-selecting a rectangle and issuing `RMB` to move (only movable selected entities move).
- Holding `A` to fire while clicking a point/enemy with `LMB` to set/adjust fire direction.
- Generating a movable Gen3D object (once prompts support mobility/animations) and observing that its move animations advance only while it moves.

## Progress

- [ ] (2026-01-29) Write this ExecPlan.
- [ ] (2026-01-29) Add data-driven mobility to `ObjectDef` and move-driven animation drivers.
- [ ] (2026-01-29) Replace player-only inputs with RTS selection + command inputs (Build + Play).
- [ ] (2026-01-29) Generalize click-to-move pathing to selected movable entities (ground + air).
- [ ] (2026-01-29) Switch firing to “hold A to fire; LMB sets target”.
- [ ] (2026-01-29) Extend Gen3D + `scene.dat` to encode mobility + animation drivers.
- [ ] (2026-01-29) Run `cargo test` and smoke test; update README; commit.

## Surprises & Discoveries

- (none yet)

## Decision Log

- Decision: RTS controls apply to Build + Play modes; Gen3D mode keeps its own UI interactions and does not receive RTS inputs.
  Rationale: Gen3D uses heavy UI interactions and is already gated by `in_state(GameMode::Gen3D)`; mixing RTS inputs would be confusing.
  Date/Author: 2026-01-29 / Codex

- Decision: Hold `A` to fire continuously; `LMB` sets a persistent fire target (point or enemy).
  Rationale: Matches the user’s latest control requirement; keeps the “keep firing while A pressed” behavior explicit and simple.
  Date/Author: 2026-01-29 / Codex

- Decision: `RMB` always issues move commands, even if clicking an enemy.
  Rationale: Explicitly required (“RMB on enemy: Move”).
  Date/Author: 2026-01-29 / Codex

- Decision: Camera remains orbiting the hero (not the selected unit).
  Rationale: Explicitly required; avoids camera refactors.
  Date/Author: 2026-01-29 / Codex

- Decision: Support both `Ground` and `Air` mobility modes.
  Rationale: Explicitly required; Air uses simpler “fly to XZ at fixed altitude” behavior initially.
  Date/Author: 2026-01-29 / Codex

## Outcomes & Retrospective

- (fill at completion)

## Context and Orientation

Key files/modules involved:

- `src/player.rs`: currently owns RMB click-to-move, WASD movement, cursor aim, camera orbit.
- `src/combat.rs`: currently fires using `LMB` held; laser updates use `Aim`.
- `src/build.rs`: Build placement uses LMB; Build selection/edit uses its own selection resource.
- `src/object/registry.rs`: object prefab data model (parts, anchors, attachments, animations).
- `src/object/visuals.rs`: spawns prefab visuals and plays part animations (currently always based on world time).
- `src/gen3d/*`: Gen3D plan/component JSON parsing and prefab assembly.
- `src/scene_store.rs`: `scene.dat` protobuf encode/decode for prefab defs + build instances.

Terms:

- “Mobility”: data on an object prefab describing whether/how the object can move (`Ground` vs `Air`).
- “Move-driven animation”: an animation whose time only advances when the object is actually moving; paused when stationary.
- “Selectable”: runtime marker indicating the entity can be selected via RTS controls.
- “Movable”: runtime marker derived from prefab mobility; only movable selected entities respond to move commands.

## Plan of Work

First, introduce a small, data-driven mobility description into `ObjectDef` in `src/object/registry.rs`, plus a way to mark a part animation as either always-playing or move-driven. Then, update the animation player (`src/object/visuals.rs`) to use either world time or per-entity locomotion time, so move-driven animations pause when stationary.

Second, refactor input:

- Replace player-only RMB click-to-move with a selection + command system:
  - `LMB` selects; drag creates a selection box.
  - `RMB` issues move orders to selected movable entities.
  - Holding `A` triggers firing; `LMB` sets fire target.
- Preserve Build placement behavior while placing (LMB places, RMB removes), by short-circuiting RTS inputs while Build placing is active.

Third, implement movement execution per selected entity:

- Ground: use existing height-aware pathfinding.
- Air: fly directly to XZ goal at fixed altitude (no pathfinding; ignores obstacles).

Fourth, wire firing:

- Convert `combat::player_fire` and `player::face_aim_while_shooting` to use “hold A” instead of LMB, and use the new fire target state instead of the raw cursor aim direction.

Finally, extend Gen3D and `scene.dat`:

- Gen3D plan JSON must optionally emit mobility + move-animated attachments (driver + speed scale).
- `scene.dat` protobuf must persist mobility and animation driver metadata for prefab defs.

## Concrete Steps

Run all commands from the repo root (`/Users/flow/workspace/github/gravimera`):

1) Implement data model changes:

   - Edit `src/object/registry.rs` to add `MobilityDef` and animation driver metadata.
   - Edit `src/object/visuals.rs` to advance move-driven animations based on per-entity movement.

2) Implement RTS input + selection UI:

   - Add a small UI selection rectangle overlay.
   - Add selection markers for selected entities.
   - Replace/disable `src/player.rs` keyboard movement (WASD), since `A` becomes fire.

3) Implement move commands:

   - On `RMB`, compute destination from cursor (existing ray/ground pick).
   - Apply to selected movable entities only.

4) Implement fire commands:

   - Hold `A` to keep firing.
   - `LMB` sets fire target direction (point or enemy).

5) Persist and Gen3D:

   - Extend `src/scene_store.rs` protobuf schema version and encode/decode mobility + animation driver metadata.
   - Extend `src/gen3d/ai.rs` schema parsing and prompts to emit the needed fields.

6) Validate and commit:

   - Run `cargo test`.
   - Run smoke test: `cargo run -- --headless --headless-seconds 1`.
   - Update `README.md` control descriptions if needed.
   - Commit.

## Validation and Acceptance

Acceptance criteria:

- You can select the hero with `LMB` and move with `RMB` in both Build (when not placing) and Play modes.
- Drag selection box works; issuing `RMB` moves only the movable selected entities (non-movable stay put).
- Holding `A` causes the hero to fire continuously; clicking `LMB` changes the firing direction; clicking an enemy makes the hero keep firing at that enemy while `A` is held.
- Move-driven animations stop when the entity stops moving (for objects that define them).
- Tests pass and the smoke test starts without crashing.

## Idempotence and Recovery

- All data format changes to `scene.dat` will bump a version; older files will be ignored with a warning. Deleting `scene.dat` is a safe reset.
- If the game fails to start rendered-mode on macOS (Metal unavailable), use the smoke test (`--headless`) to validate startup and core logic.

## Artifacts and Notes

- Keep commits small; after each milestone, run `cargo test` and `cargo run -- --headless --headless-seconds 1`.

## Interfaces and Dependencies

At the end of this work, these types/interfaces must exist and be used:

- `crate::object::registry::MobilityDef` and `MobilityMode` (or equivalent names).
- A per-entity locomotion clock component (e.g. `crate::types::LocomotionClock`) that drives move animations.
- A selection resource containing selected entities and drag selection state.
- A per-entity move order component that can represent “move to point” for both ground and air mobility.
