# Unit Combat: Move Direction vs Attention Direction (Aim Constraints)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, player-controlled units can move in one direction while attacking in another direction, without their whole body constantly snapping/oscillating between “move facing” and “attack facing”.

Each unit has two independent horizontal (yaw) directions:

- The **move/body direction**: the direction the unit’s main body and locomotion parts (wheels/legs) face. This is driven by movement.
- The **attention/aim direction**: the direction the unit’s weapon/head (or turret) faces when attacking. This is driven by the current attack target.

Different units can have different limits on how far attention can deviate from the body. If a target would require aiming beyond the unit’s allowed yaw difference, the unit will still attack but the attack direction is clamped to the closest allowed direction. Movement has priority: aiming must not rotate the whole unit away from its movement direction.

You can see this working by:

1. Running the game, selecting a unit, issuing a move order, then holding `Space` (fire) and aiming sideways/backwards.
2. Observing: the unit continues moving/facing its movement direction; its weapon/turret aims (within limits), and projectiles/melee arcs follow the clamped aim direction.
3. Running the real Gen3D prompts (warcar / soldier / horse / knight) and verifying in-scene behavior and animations.

## Progress

- [ ] (2026-02-06 00:00Z) Add `aim` profile data model to `ObjectDef`, persistence in `scene.dat`, and id-remap on Gen3D Save.
- [ ] (2026-02-06 00:00Z) Implement runtime per-unit aim yaw delta computation and clamping from `FireControl` target.
- [ ] (2026-02-06 00:00Z) Apply aim yaw delta to visual subtrees (weapon/head/turret) without rotating the root body transform.
- [ ] (2026-02-06 00:00Z) Switch unit attack execution (melee + ranged) to use aim direction (clamped) instead of body facing.
- [ ] (2026-02-06 00:00Z) Update Gen3D plan schema + prompts to allow AI to specify aim constraints and which components aim.
- [ ] (2026-02-06 00:00Z) Validation: smoke test + run `tools/gen3d_real_test.py` for 4 prompts; log issues and fix regressions.
- [ ] (2026-02-06 00:00Z) Update `README.md` and commit.

## Surprises & Discoveries

- Observation: (fill as discovered)
  Evidence: (logs / screenshots / cache folders)

## Decision Log

- Decision: Represent attention-vs-body limits as a per-prefab `AimProfile { max_yaw_delta_degrees, components }` on `ObjectDef`.
  Rationale: It is easy to persist, easy for Gen3D to emit, and keeps runtime cheap (no per-frame inference from geometry).
  Date/Author: 2026-02-06 / Codex

- Decision: Keep body rotation driven only by movement; do not rotate the root `Transform` for aiming.
  Rationale: Eliminates oscillation and satisfies “move direction has higher priority than attention direction”.
  Date/Author: 2026-02-06 / Codex

## Outcomes & Retrospective

(Fill at completion.)

## Context and Orientation

Relevant code in this repo:

- `src/rts.rs`
  - Selection, move orders, and fire targeting (`FireControl`).
  - Today, `apply_fire_facing()` rotates the whole unit `Transform` toward the fire target. This creates unwanted “snap” and prevents independent aiming.
- `src/combat.rs`
  - `unit_attack_execute()` executes melee/ranged attacks for selected units while `FireControl.active`.
  - Today it uses `direction = transform.rotation * Vec3::Z` (body facing), which implicitly depends on `apply_fire_facing()`.
- `src/object/registry.rs`
  - `ObjectDef` prefab structure. Units can have `mobility` and optional `attack: UnitAttackProfile`.
- `src/object/visuals.rs`
  - `spawn_object_visuals_*()` spawns prefab parts as a Bevy child hierarchy.
  - `update_part_animations()` updates attachment animations (wheels spin, recoil, etc.).
  - We will extend this path so specific subtrees can be yaw-rotated by “attention” without rotating the root body.
- `src/scene_store.rs`
  - Protobuf scene persistence (`scene.dat`). We must persist any new prefab fields used by saved Gen3D models.
- `src/gen3d/ai/schema.rs`, `src/gen3d/ai/prompts.rs`, `src/gen3d/ai/convert.rs`, `src/gen3d/save.rs`
  - Gen3D plan JSON schema and conversion into draft `ObjectDef`s, then Save remaps ids and persists to `scene.dat`.
- `tools/gen3d_real_test.py`
  - Real rendered integration test driver for Gen3D that builds, saves, moves, and screenshots a generated unit.
  - We will use it to validate 4 prompts at the end of this work.
- `docs/gen3d_real_test_issues.md`
  - Add newly discovered issues here while testing.

Terms:

- “Yaw”: rotation around the world up axis (Y), controlling facing direction on the ground plane (XZ).
- “Aim yaw delta”: the signed yaw difference between attention direction and body direction, clamped by the unit’s aim constraints.
- “Aim components”: which child components (in the prefab composition tree) should visually yaw with attention direction (weapon/turret/head), while the root stays body-facing.

## Plan of Work

### 1) Data model: aim constraints on prefabs

In `src/object/registry.rs`:

- Add a new struct:

  - `AimProfile { max_yaw_delta_degrees: Option<f32>, components: Vec<u128> }`

  `max_yaw_delta_degrees == None` means “no clamp” (full 360). Values >= 180 should be treated as effectively unlimited.

- Add `aim: Option<AimProfile>` to `ObjectDef`.
- Update all builtin object defs to compile (set `aim: None`).

In `src/scene_store.rs`:

- Extend the protobuf schema:
  - Add `aim: Option<SceneDatAimProfile>` to `SceneDatObjectDef` at a new tag (e.g. 15).
  - `SceneDatAimProfile` includes:
    - optional `max_yaw_delta_degrees` (use `Float32Dat` so it’s truly optional),
    - repeated `components` (list of `Uuid128Dat` object ids).
- Update `def_to_dat()` and `def_from_dat()` to serialize/deserialize the new field.

In `src/gen3d/save.rs`:

- During draft->saved id remap, also remap any `aim.components` ids using the same `id_map` used for parts and ranged muzzle/projectile ids.

### 2) Runtime: compute attention yaw delta per selected unit

In `src/types.rs`:

- Add a component to store per-entity aim delta:
  - `AimYawDelta(pub(crate) f32)` in radians. `0.0` means “aligned with body”.

In `src/rts.rs`:

- Remove or disable `apply_fire_facing()` (body rotation from attack).
- Add a new system `update_unit_aim_yaw_delta()` that:
  - Runs only when `FireControl.active` and a target exists and selection is non-empty.
  - For each selected `Commandable` with an `attack` profile:
    - Compute desired yaw to the fire target point/enemy.
    - Read the unit’s body yaw from its `Transform.rotation`.
    - Find aim constraints from `ObjectDef.aim` (and/or sensible fallback):
      - If `def.aim` exists, use its `max_yaw_delta_degrees`.
      - If missing, default:
        - ranged: unlimited (no clamp),
        - melee: clamp to something conservative like 120 degrees (so it can swing/attack sideways but not fully backwards).
    - Clamp the yaw delta and write `AimYawDelta` to the entity.
  - When fire is not active (or the unit is not attack-capable), remove `AimYawDelta` to avoid stale aiming affecting visuals.

In `src/app.rs` (schedule):

- Insert `rts::update_unit_aim_yaw_delta` after:
  - `rts::execute_move_orders` (so body yaw is updated first),
  - `rts::update_fire_control` (so target is updated),
  - and before:
  - `combat::unit_attack_execute`,
  - `crate::object::visuals::update_part_animations` (so visuals can read it).

### 3) Combat: use aim direction for melee and ranged

In `src/combat.rs` (`unit_attack_execute`):

- Replace `direction = transform.rotation * Vec3::Z` with:
  - `aim_delta = AimYawDelta` from the unit (default 0).
  - `aim_world_rotation = transform.rotation * Quat::from_rotation_y(aim_delta)`.
  - `direction = normalize_flat_direction(aim_world_rotation * Vec3::Z)`.
- Use this `direction` for:
  - melee arc orientation,
  - projectile spawn direction / velocity / bullet yaw.

### 4) Visuals: yaw-rotate aim subtrees only

In `src/object/visuals.rs`:

- Extend `PartAnimationPlayer` with a flag `apply_aim_yaw: bool`.
- In `spawn_object_visuals_with_settings`:
  - Compute the set of aim component ids for this root prefab:
    - If `def.aim` exists: use `aim.components`.
    - Else, if `def.attack` is ranged: use the ranged muzzle component id (so the weapon component is aimable by default).
  - Thread this set through `spawn_object_visuals_inner`.
- In `spawn_object_visuals_inner`, when spawning an `ObjectRef` part:
  - If the child object id is in the aim set, ensure a `PartAnimationPlayer` is inserted on that part root entity with `apply_aim_yaw = true`, even if it has no animations.
- In `update_part_animations`:
  - Add a `Query<&AimYawDelta>` for the root entity.
  - When `apply_aim_yaw` is true, pre-multiply the sampled base rotation by `Quat::from_rotation_y(delta)` where `delta` is from the root entity.
  - Apply aim even if there is no animation slot selected (treat animation delta as identity).

This produces the visible effect: wheels/legs continue to animate with movement; only the aim-marked subtrees rotate with attention.

### 5) Gen3D: allow AI to specify aim constraints and aim components

In `src/gen3d/ai/schema.rs`:

- Add `aim: Option<AiAimJson>` to `AiPlanJsonV1`.
- Define:
  - `AiAimJson { max_yaw_delta_degrees: Option<f32>, components: Vec<String> }`
    - `components` are component names from the plan (e.g. `head`, `weapon`, `turret`, `cannon`).

In `src/gen3d/ai/prompts.rs`:

- Update the plan schema documentation to include `aim`.
- Guidance:
  - For turrets/tanks: set `max_yaw_delta_degrees` to `null` (unlimited) and include the turret/cannon component(s) in `aim.components`.
  - For animals: set a limited max yaw (e.g. 45–90) and include head/weapon as appropriate.
  - For humanoids with guns: medium max yaw (e.g. 90–140) and include head + weapon.

In `src/gen3d/ai/convert.rs`:

- Convert `plan.aim` into `AimProfile` for the root draft `ObjectDef` by mapping component names to their deterministic component ids.
- If `plan.aim` is missing and the plan has ranged `attack`, set a default `AimProfile` that aims the muzzle component (unlimited yaw).

In `src/gen3d/save.rs`:

- Ensure id-remap handles `aim.components`.

### 6) Validation and iteration

Run (from repo root):

    cargo test
    cargo run -- --headless --headless-seconds 1

Then run the real rendered Gen3D prompts (resetting the scene between runs):

    python3 tools/gen3d_real_test.py --reset-scene --prompt "A warcar with a cannon as weapon"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A soldier with a gun"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A horse"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A knight on a horse"

For each run, inspect the newest `target/debug/gen3d_cache/<run_id>/external_screenshots_*` frames and any screenshots saved to the cache folder.

When issues are found, add them (with the cache folder path) to `docs/gen3d_real_test_issues.md`, then fix and re-run the relevant prompt(s).

### 7) Docs and commit

- Update `README.md` to reflect:
  - Units can move and attack in different directions.
  - Movement direction has priority; aiming rotates weapon/turret subtrees when available.
  - Fire key remains `Space` (per current code).
- Commit with a descriptive message.

## Concrete Steps

From repo root:

    cargo fmt
    cargo test
    cargo run -- --headless --headless-seconds 1

Then, real Gen3D prompt runs:

    python3 tools/gen3d_real_test.py --reset-scene --prompt "A warcar with a cannon as weapon"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A soldier with a gun"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A horse"
    python3 tools/gen3d_real_test.py --reset-scene --prompt "A knight on a horse"

## Validation and Acceptance

Acceptance is met when:

- While moving, selected units can attack toward the fire target without the root body rotation snapping toward the target.
- Weapon/turret/head visual parts (when specified by `aim.components`, or defaulted for ranged weapons) visibly yaw toward the fire target.
- When a unit has a limited `aim.max_yaw_delta_degrees`, firing at a target beyond the allowed yaw causes attacks to use the closest clamped direction.
- `tools/gen3d_real_test.py` successfully completes for the 4 prompts, saving screenshots and a saved unit in-scene, without panics/crashes.
- The game starts without crashing (headless smoke test passes).

## Idempotence and Recovery

- The protobuf `scene.dat` format changes are additive (new field tags), so older `scene.dat` files should still load with default `aim` values.
- Gen3D cache artifacts are written per run id; rerunning prompts is safe.
- If a Gen3D plan fails to parse due to missing/incorrect `aim` fields, the engine should treat `aim` as optional and fall back to ranged muzzle aiming.

## Artifacts and Notes

(Fill in with any cache folder paths and observations during development.)

