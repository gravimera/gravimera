# Gen3D Units: AI-Driven Attacks + Event-Driven Attack Animation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, a player can use Gen3D to generate a *movable* model (a “unit”) that is also *attack-capable* without writing code. The AI decides, from the prompt and/or photos, whether the generated object should be:

- Not attack-capable (e.g., furniture, props, most buildings).
- A melee attacker (e.g., goblin/orc with an axe/spear).
- A ranged attacker (e.g., a robot with a cannon).

When the player selects the unit and holds `A` (attack), the unit will repeatedly attack using the AI-chosen attack type and effect. Attack animations will be efficient and consistent: one attack animation cycle per attack event (instead of running continuously while `A` is held).

You can see this working by:

1. Generating a unit via Gen3D with a prompt like “a goblin with a spear”.
2. Clicking **Save**.
3. Exiting Gen3D, selecting the saved unit, holding `A`, and left-clicking an enemy to target it.
4. Observing: the unit faces the target, plays an attack animation per attack, and damages enemies (kills increase score).

## Progress

- [x] (2026-01-31 22:00Z) Write/maintain this ExecPlan and keep it self-contained.
- [x] (2026-01-31 22:00Z) Add an event-driven `attack_time` animation driver and an `AttackClock` that activates `attack_primary` only for the duration of one attack.
- [x] (2026-01-31 22:00Z) Extend Gen3D plan schema to include an optional `attack` decision (none/melee/ranged) plus ranged projectile spec.
- [x] (2026-01-31 22:00Z) Persist the attack decision into saved Gen3D prefab defs and map ids correctly during Save.
- [x] (2026-01-31 22:00Z) Implement runtime unit attacks:
  - Ranged: spawn projectile(s) using AI-provided projectile spec (damage/speed/obstacle rule).
  - Melee: apply damage in front of the attacker within AI-provided range.
- [x] (2026-01-31 22:00Z) Update RTS fire control to work for any selected unit (not only the hero) and compute aim direction per unit.
- [x] (2026-01-31 22:00Z) Validation: `cargo test` and `cargo run -- --headless --headless-seconds 1`.
- [x] (2026-01-31 22:10Z) Update `README.md` if behavior changes and commit.

## Surprises & Discoveries

- Observation: (to fill)
  Evidence: (to fill)

## Decision Log

- Decision: Use `AttackClock { started_at_secs, duration_secs }` and derive `attack_time` as `wall_time - started_at_secs`.
  Rationale: Avoids per-frame clock integration, keeps the driver cheap and deterministic, and matches “one animation per attack event”.
  Date/Author: 2026-01-31 / Codex

## Outcomes & Retrospective

(To fill at completion.)

## Context and Orientation

Key concepts in this repo:

- **Object prefab**: `ObjectDef` (in `src/object/registry.rs`) describes a composed object built from parts (primitive meshes, models, or nested object refs). Runtime entities store `ObjectPrefabId(u128)` pointing to a prefab in `ObjectLibrary`.
- **Gen3D**: `src/gen3d/ai.rs` requests an AI plan (component assembly with anchors/attachments) and component drafts (primitives). `src/gen3d/save.rs` converts the draft defs into saved defs with fresh UUID object ids and spawns the saved model next to the hero.
- **RTS controls**: `src/rts.rs` handles selection, click-to-move, and fire control (`A` + LMB to set attack target). Today, `FireControl` is effectively hero-only for aiming/shooting.
- **Animations**: `src/object/visuals.rs` plays attachment animations (`attach_to.animations`) using channel priority (`attack_primary > move > idle > ambient`). Drivers exist for `always`, `move_phase`, and `move_distance`. There is no event-driven attack clock yet.
- **Combat**: `src/combat.rs` handles hero bullets/laser and bullet-vs-enemy collisions. `src/enemies.rs` handles enemy projectiles vs player/objects.

Terms used here:

- **Attack event**: A single discrete attack action (one melee swing or one projectile shot) that can repeat due to a cooldown while holding `A`.
- **Attack animation window**: The short duration (typically one clip cycle) during which `attack_primary` is considered active on a unit so it plays once per attack event.

## Plan of Work

### 1) Add event-driven attack animation driver

Update `src/object/registry.rs`:

- Extend `PartAnimationDriver` to include `AttackTime`.

Update `src/types.rs`:

- Add a new component `AttackClock { started_at_secs: f32, duration_secs: f32 }`.

Update `src/locomotion.rs`:

- Update `update_animation_channels_active()` so `channels.attacking_primary` is derived from `AttackClock` activity rather than `FireControl.active`.

Update `src/object/visuals.rs`:

- Add a `Query<&AttackClock>` and implement `PartAnimationDriver::AttackTime` by reading `AttackClock` on `player.root_entity`.

### 2) Extend Gen3D plan schema to include attacks

Update `src/gen3d/ai.rs`:

- Extend the plan system prompt text to include a new “Combat decision” section and describe an `attack` schema at the plan top-level.
- Extend `AiPlanJsonV1` to parse the new optional `attack` field.
- Add types for:
  - `AiAttackJson` (none/melee/ranged_projectile)
  - `AiProjectileSpecJson` (shape/color/radius/length/speed/ttl/damage/obstacle_rule/spawn_energy_impact)
  - A reference to where the attack originates (component+anchor).
- Convert the parsed plan `attack` to a runtime prefab field (see next section).

### 3) Store attack profile in prefab defs and map ids on Save

Update `src/object/registry.rs`:

- Add `attack: Option<UnitAttackProfile>` to `ObjectDef`.
- Add `ObjectLibrary::attack()` accessor.

Update `src/scene_store.rs` serialization:

- Keep compiling by extending the scene schema for the new optional `attack` field. (Even though only build objects are persisted today, the type must remain serializable.)

Update `src/gen3d/save.rs`:

- During id remap, also remap `attack.projectile_prefab` if present.

### 4) Runtime attacks for player-controlled units

Update `src/rts.rs`:

- Remove hero-only restriction in fire control.
- Keep `FireControl` as global target selection, but compute aim direction per selected unit based on the target.

Add new combat systems (likely in `src/combat.rs` for reuse of damage/score logic):

- `unit_attack_tick_cooldowns`: per-unit attack cooldown ticking.
- `unit_attack_execute`: when `A` is held and a target exists, trigger attack events for selected units with `attack` profiles.
  - For ranged: spawn a `Bullet` entity with the projectile spec from the prefab.
  - For melee: overlap enemies in front of the unit and apply damage; spawn existing blood effects.
  - For both: set/update `AttackClock` so `attack_primary` runs once per attack.

Update `src/combat.rs` bullet collision behavior:

- Make bullet-vs-object collisions respect projectile `obstacle_rule`.
- If a projectile has `spawn_energy_impact`, spawn the energy impact particles on collision.

### 5) Validation + docs + commit

- Run `cargo test`.
- Run `cargo run -- --headless --headless-seconds 1` as the smoke start.
- Update `README.md` if controls/behavior changed.
- Commit with a descriptive message.

## Concrete Steps

From repo root:

    cargo test
    cargo run -- --headless --headless-seconds 1

Then run the rendered game and manually verify:

- Gen3D: generate “orc with an axe”, Save, exit Gen3D.
- Select the orc, hold `A`, left-click an enemy.
- Observe repeated attacks with one-cycle attack animation and enemy health decreasing.

## Validation and Acceptance

Acceptance is met when:

- Gen3D can produce a movable unit with an `attack` profile.
- Selected units can attack while holding `A` (melee or ranged as decided by AI).
- `attack_primary` animations are event-driven: they play once per attack event, not continuously during the full `A`-hold period.
- The game starts without crashing (headless smoke test passes).

## Idempotence and Recovery

- Gen3D cache artifacts are written per run; repeated builds are safe.
- If parsing fails due to AI schema mismatch, increase prompt strictness or add parse-time error messages and retry (existing retry logic in Gen3D should be reused).

## Artifacts and Notes

(To fill with any useful logs or screenshots captured during development.)

## Interfaces and Dependencies

New/changed interfaces expected:

- In `crate::types`:

    #[derive(Component, Clone, Copy, Debug, Default)]
    pub(crate) struct AttackClock { pub(crate) started_at_secs: f32, pub(crate) duration_secs: f32 }

- In `crate::object::registry`:

    #[derive(Clone, Copy, Debug)]
    pub(crate) enum UnitAttackKind { Melee, RangedProjectile }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct AnchorRef { pub(crate) object_id: u128, pub(crate) anchor: Cow<'static, str> }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct MeleeAttackProfile { pub(crate) range: f32, pub(crate) radius: f32, pub(crate) arc_degrees: f32 }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct RangedAttackProfile { pub(crate) projectile_prefab: u128, pub(crate) muzzle: AnchorRef }

    #[derive(Clone, Copy, Debug)]
    pub(crate) struct UnitAttackProfile { pub(crate) kind: UnitAttackKind, pub(crate) cooldown_secs: f32, pub(crate) damage: i32, pub(crate) anim_window_secs: f32, pub(crate) melee: Option<MeleeAttackProfile>, pub(crate) ranged: Option<RangedAttackProfile> }

  And `ObjectDef { attack: Option<UnitAttackProfile>, ... }`.
