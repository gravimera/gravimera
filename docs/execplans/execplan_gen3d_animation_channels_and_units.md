# Gen3D: AI-authored animation channels + mobility-driven Save (unit vs building)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D currently generates static models (prefabs composed of primitives) and can optionally attach a single loop animation to an attachment, but it cannot express “this part animates while moving” vs “this part animates while attacking” in a robust way, and it always saves as a Build object.

After this change:

1. Gen3D can generate **data-driven component animations** via **animation channels** on attachments:
   - `attack_primary`, `move`, `idle`, `ambient`, with priority `attack_primary > move > idle > ambient` per part.
   - AI decides which components animate and how (legs swing, wheels rotate, wings flap, recoil, etc.) by splitting the model into components and assigning animation slots.
   - The engine does not hardcode “human legs swing”; it only plays what the prefab data describes, driven by generic gameplay signals (moving/attacking).

2. Gen3D can ask AI to decide **mobility** (movable or not):
   - If AI marks the generated model as movable, **Save** spawns a controllable **unit** (Commandable) instead of a Build object.
   - If AI marks it non-movable, Save spawns a Build object as before.

3. Gen3D becomes more robust for big outputs:
   - Stage 1 plan asks for mobility + animations. If animations/mobility are missing/invalid, the game runs a second AI call to “fill” them without regenerating geometry.

You can see it working by generating:

- A “goblin with spear”: legs swing while moving; arms/spear animate while attacking (when holding `A`).
- A “warcar”: wheels spin while moving.
- A building (e.g., “stone tower”): Save produces a Build object (non-movable).

## Progress

- [x] (2026-01-30 10:50Z) Write this ExecPlan and keep it current.
- [x] (2026-01-30 10:50Z) Implement multi-channel animations on `ObjectPartDef` (replace single `animation` field).
- [x] (2026-01-30 10:50Z) Implement runtime channel selection with priority and generic channel activation (moving/attacking).
- [x] (2026-01-30 10:50Z) Add mobility-aware Save: movable => unit, non-movable => Build object.
- [x] (2026-01-30 10:50Z) Extend Gen3D plan schema to include mobility + channel animations; add “fill animations/mobility” fallback request.
- [x] (2026-01-30 10:50Z) Persist multi-channel animations + new drivers in `scene.dat` (bump version; no backward compatibility).
- [x] (2026-01-30 10:50Z) Add tests: scene.dat roundtrip for multi-channel animations and drivers.
- [x] (2026-01-30 10:50Z) Update `README.md` to document Gen3D mobility/unit Save and animation channels.
- [x] (2026-01-30 10:50Z) Run `cargo test` + smoke test `cargo run -- --headless --headless-seconds 1`.
- [x] (2026-01-30 10:52Z) Commit the changes.

## Surprises & Discoveries

- Observation: continuous 360° spinning is fragile with loop keyframes because the start and end rotation are the same orientation.
  Evidence: quaternion interpolation tends to pick the shortest arc; naive “0°→360°” loops can appear static or snap at loop wrap.
  Resolution: introduce a procedural `Spin` clip (axis + radians-per-unit) for best correctness and performance.

## Decision Log

- Decision: Use animation channels `ambient`, `idle`, `move`, `attack_primary` with per-part priority `attack_primary > move > idle > ambient`.
  Rationale: Keeps the engine generic (no object-type logic) while allowing the AI to animate different parts differently (legs move while arms attack).
  Date/Author: 2026-01-30 / Codex + user

- Decision: Add a procedural `Spin` animation clip for wheels/fans/propellers.
  Rationale: More correct and cheaper than keyframing continuous rotation; avoids quaternion wrap issues.
  Date/Author: 2026-01-30 / Codex + user

- Decision: Prefer `MoveDistance` as the driver for wheel spin (distance traveled), and use `MovePhase` for gait swings (phase driven by movement distance).
  Rationale: Wheel spin looks physically consistent; gait loops can be authored as cycles-per-meter without coupling to `mobility.max_speed`.
  Date/Author: 2026-01-30 / Codex + user

- Decision: Use a hybrid AI plan strategy: ask for mobility + animations in Stage 1, but if missing/invalid, run a follow-up AI request that outputs only mobility + animations (no geometry).
  Rationale: Improves reliability when the model produces incomplete large JSON; limits retries to the smallest step.
  Date/Author: 2026-01-30 / Codex + user

## Outcomes & Retrospective

- Gen3D plans can now describe per-component attachment animations using channels, and the engine plays them based on generic gameplay signals (moving / attacking).
- Gen3D can now produce “unit” prefabs by having the AI set `mobility`, and Save spawns them as controllable entities instead of build objects.
- `scene.dat` v6 persists multi-channel animation slots and mobility on embedded prefab defs (build instances only).

## Context and Orientation

Current relevant code locations:

- `src/object/registry.rs`: prefab data model (parts, anchors, attachments, animation spec, mobility).
- `src/object/visuals.rs`: spawns visuals from prefabs and updates animated parts each frame.
- `src/locomotion.rs`: maintains a per-entity locomotion clock used for move-driven animations.
- `src/scene_store.rs`: protobuf persistence for embedded prefab defs + build instances (`scene.dat`).
- `src/gen3d/ai.rs`: Gen3D prompts, OpenAI calls, JSON parsing, prefab assembly.
- `src/gen3d/save.rs`: Save button logic (spawns either a unit or a Build object, based on mobility).
- `src/rts.rs`: selection + RMB move orders + hold-A fire targeting (currently hero-only targeting semantics).

Key definitions:

- Channel: a named “when to play” bucket (`ambient`, `idle`, `move`, `attack_primary`).
- Slot: an animation spec attached to a part for a given channel.
- Driver: what the animation time is derived from (wall time, locomotion phase, distance traveled, etc.).
- Clip: how a transform delta is generated over time (loop keyframes; procedural spin).
- Unit: a runtime entity that is `Commandable` and has mobility; it can be selected and moved via RTS controls.

## Plan of Work

### Milestone A — Data model changes (animations become multi-channel)

1. In `src/object/registry.rs`:
   - Replace `ObjectPartDef.animation: Option<PartAnimationSpec>` with `ObjectPartDef.animations: Vec<PartAnimationSlot>`.
   - Define `PartAnimationSlot { channel: Cow<'static, str>, spec: PartAnimationSpec }`.
   - Extend `PartAnimationDriver`:
     - Rename existing `Move` to `MovePhase` (to clarify it is a generic locomotion phase driver).
     - Add `MoveDistance` (driven by cumulative XZ distance traveled).
     - Keep `Always`.
   - Extend `PartAnimationDef`:
     - Keep `Loop`.
     - Add `Spin { axis: Vec3, radians_per_unit: f32 }`.

2. Update `src/object/visuals.rs`:
   - Update `PartAnimationPlayer` to store:
     - base transform,
     - attachment info,
     - all animation slots (vector),
     - minimal per-part runtime state (e.g., last active channel for transitions if needed).
   - In `update_part_animations`, choose the active slot by:
     - Reading generic channel activity for the owning root entity.
     - Priority order `attack_primary > move > idle > ambient`.
     - Applying delta to base transform; if attached, resolve attachment transform using the animated base.

3. Add generic channel activity:
   - Introduce a component `AnimationChannelsActive` (per root entity) with at least:
     - `moving: bool`
     - `attacking_primary: bool`
   - Set `moving` based on locomotion metrics.
   - Set `attacking_primary` based on fire input for selected units in Play mode (see milestone C).

### Milestone B — Locomotion distance metrics

In `src/types.rs` and `src/locomotion.rs`:

- Extend `LocomotionClock` to also track:
  - `distance_m: f32` (cumulative horizontal distance traveled; XZ only).
  - Keep existing `t` (phase time scaled by speed/max_speed).
- Update `update_locomotion_clocks` to accumulate `distance_m` and keep it finite.

### Milestone C — Mobility-driven Save (unit vs building) + selection targeting generalization

1. Extend Gen3D plan schema to allow AI to decide mobility and set it on the root prefab:
   - Example: `"mobility": { "mode": "ground", "max_speed": 3.0 }` or omit for static.

2. Update `src/gen3d/save.rs`:
   - When saving the root prefab, do not force “building interaction” unconditionally.
   - If the saved root prefab has mobility:
     - Spawn as a **unit** entity with `Commandable`, `Collider` (circle), `ObjectPrefabId`, `Transform`.
     - Do not mark it `BuildObject`; do not include AABB collider.
     - Do not auto-save `scene.dat` (scene persistence is build objects only for now).
   - Else:
     - Spawn as Build object as today and request a scene save.

3. Update selection input to allow selecting units (not only hero):
   - In `src/rts.rs`, selection candidates should include `Commandable` entities (and Build objects only in Build mode).
   - Update selection ring gizmo drawing to use `Commandable` entities (not `Player` only).

4. Generalize attack targeting enough to drive `attack_primary` animations:
   - Make fire targeting (`FireControl`) not depend on hero selection, only on:
     - `A` pressed,
     - Play mode,
     - selection not empty.
   - Keep actual shooting (spawning bullets/laser) hero-only for now, but:
     - rotate all selected units toward the target while `A` is held (already implemented in `apply_fire_facing`, but must use per-entity direction).

### Milestone D — Gen3D AI schemas and hybrid “fill” step

1. Bump plan JSON schema version (e.g. `version: 5`) and update prompts:
   - Add top-level `mobility` (optional).
   - Replace `attach_to.animation` with `attach_to.animations` (map of channel -> spec).

2. Implement a fallback request:
   - If plan missing mobility/animations or they fail schema validation, call AI again with:
     - The already-accepted component plan (names/anchors/attach_to).
     - Ask for STRICT JSON returning only `mobility` + per-component `animations`.
   - Merge results into the parsed plan before generating component geometry.

3. Convert parsed channel animations into `ObjectPartDef.animations` on the parent component’s `ObjectRef` part.

### Milestone E — Persistence

In `src/scene_store.rs`:

- Bump scene version (no backward compatibility).
- Replace `SceneDatPartDef.animation` with `repeated SceneDatPartAnimationSlot`.
- Persist:
  - channel string
  - driver enum
  - speed_scale
  - clip kind (`loop` or `spin`)
- Add/adjust tests to roundtrip a prefab with multiple channels and drivers.

### Milestone F — Docs + validation

- Update `README.md` Gen3D section:
  - Explain that Save produces either a unit or a build object depending on AI mobility.
  - Explain channels and how to trigger them (move vs hold-A).
- Run:
  - `cargo test`
  - `cargo run -- --headless --headless-seconds 1`
- Commit.

## Concrete Steps

All commands run from repo root (`/Users/flow/workspace/github/gravimera`).

1. Tests:

    cargo test

2. Smoke:

    cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

Acceptance behaviors:

1. Gen3D “warcar”:
   - AI marks it movable (mobility present).
   - Save spawns it as a controllable unit (selectable + RMB move).
   - Wheels spin when moving (MoveDistance + Spin).

2. Gen3D “stone tower”:
   - AI marks it non-movable (mobility absent).
   - Save spawns it as a Build object and persists to `scene.dat`.

3. Gen3D “goblin with spear”:
   - AI can create a move animation (legs swing) and an attack_primary animation (arms/spear motion).
   - While holding `A` with the unit selected, the attack_primary animation plays (even if no damage system exists yet).

4. `scene.dat` load/save continues working for build objects with the new schema version (old versions ignored).

## Idempotence and Recovery

- If `scene.dat` is from an older version, it will be ignored. Deleting `scene.dat` is a safe reset.
- Gen3D cache folders are per-run and can be deleted.

## Artifacts and Notes

(fill in key transcripts and any “surprises” encountered)

## Interfaces and Dependencies

At the end of this plan, the following types must exist:

In `src/object/registry.rs`:

    pub(crate) struct PartAnimationSlot { channel: Cow<'static, str>, spec: PartAnimationSpec }
    pub(crate) enum PartAnimationDriver { Always, MovePhase, MoveDistance }
    pub(crate) enum PartAnimationDef { Loop{...}, Spin{ axis: Vec3, radians_per_unit: f32 } }
    pub(crate) struct ObjectPartDef { ..., animations: Vec<PartAnimationSlot>, ... }

In `src/types.rs`:

    pub(crate) struct AnimationChannelsActive { moving: bool, attacking_primary: bool }
    pub(crate) struct LocomotionClock { t: f32, distance_m: f32, last_translation: Vec3 }

In `src/scene_store.rs`:

    SceneDatPartDef.animations: repeated SceneDatPartAnimationSlot

In `src/gen3d/ai.rs`:

- Plan schema includes optional `mobility` and per-attachment `animations` per channel.

---

Plan revision notes:

- (none yet)
