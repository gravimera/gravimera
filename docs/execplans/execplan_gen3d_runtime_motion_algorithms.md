# Gen3D: Runtime motion algorithms (rig contracts + realtime switching)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Today, Gen3D often asks the AI to generate *both* geometry and per-model animations (per attachment edge, per channel). This yields model-specific motion that is hard to reuse, hard to tune globally, and difficult to swap in realtime.

After this change, Gen3D’s “default strategy” becomes:

1) The AI generates a **static** model (components + anchors + attachments) plus a small amount of **necessary motion metadata** (“rig contract”), but does **not** author detailed per-edge animation clips by default.

2) The engine provides a library of **generic, deterministic motion algorithms** (example: `biped_walk_v1`) that can be applied to any model that explicitly declares a compatible rig contract.

3) A player (or agent) can **switch motion algorithms at runtime** per object instance (for example, toggle `move` motion between `none` and `biped_walk_v1`) without regenerating the model and without changing the animation channel semantics (`idle`/`move`/`attack_primary`).

You can see it working by:

- Saving a Gen3D unit that declares a `biped_v1` rig contract (two legs).
- Selecting that unit and pressing a hotkey to switch its `move` motion algorithm between `none` and `biped_walk_v1`.
- Observing that the unit’s visual motion changes immediately while the game’s channel activation logic stays the same (movement still drives the `move` channel; attacks still drive `attack_primary`).

## Progress

- [x] (2026-02-23) Write this ExecPlan and keep it current.
- [ ] Define and document the `MotionRigV1` contract format (stored in prefab descriptors).
- [ ] Add a runtime “motion algorithm” framework (registry + per-instance controller component).
- [ ] Implement `biped_walk_v1` as a first generic algorithm (no heuristics; explicit rig only).
- [ ] Add realtime switching UX (hotkey) and a minimal on-screen/console indicator.
- [ ] Add validation/tests and run the required rendered smoke test.
- [ ] Update docs (`gen_3d.md`, `docs/gamedesign/35_prefab_descriptors_v1.md`) to reflect the new contract and workflow.
- [ ] Commit implementation.

## Surprises & Discoveries

- None yet.

## Decision Log

- Decision: Do not use heuristic detection to decide “this model has two legs”.
  Rationale: Repo rule (AGENTS.md): Gen3D algorithms must be generic; a user may ask for any object, so we must not guess structure from names/geometry. Motion algorithms are applied only when a model explicitly declares the required rig contract.
  Date/Author: 2026-02-23 / Codex + user

- Decision: Preserve the existing channel system (`idle`/`move`/`attack_primary`/`ambient`) and implement algorithm switching by changing the per-part animation slots for those channels.
  Rationale: Channels already integrate with gameplay signals and UI (preview dropdown, forced channel hotkeys). Algorithm switching should not require changing core animation playback logic.
  Date/Author: 2026-02-23 / Codex + user

- Decision: Store rig contracts in prefab descriptors (`*.desc.json`) under `interfaces.extra` rather than changing the prefab definition JSON format.
  Rationale: Prefab descriptors explicitly allow arbitrary extra JSON, are already loaded at runtime, and are the right place for semantic contracts that complement structural prefab data.
  Date/Author: 2026-02-23 / Codex + user

## Outcomes & Retrospective

- Not started. Update after implementation milestones land.

## Context and Orientation

### Current animation model (what exists today)

Gravimera’s animations are data-driven and per-part:

- Prefabs are `ObjectDef` graphs stored in `ObjectLibrary` (`src/object/registry.rs`).
- Each `ObjectPartDef` can hold multiple `PartAnimationSlot`s, each labeled by a channel string (commonly `idle`, `move`, `attack_primary`, `ambient`).
- A slot contains a `PartAnimationSpec`:
  - `driver`: `Always`, `MovePhase`, `MoveDistance`, `AttackTime`
  - `clip`: `Loop`/`Once`/`PingPong` (keyframes of delta transforms) or `Spin` (procedural)
- At runtime, `update_part_animations` (`src/object/visuals.rs`) picks one slot per part based on:
  - the owning root entity’s activity (`AnimationChannelsActive` from `src/locomotion.rs`),
  - an optional override (`ForcedAnimationChannel`),
  - priority: `attack_primary > move > idle > ambient`,
  and applies `animated = base * delta(t)` before composing attachment alignment transforms.

Gen3D currently often asks the AI to author `attach_to.animations` per component attachment edge and the engine converts them into `PartAnimationSlot`s (`src/gen3d/ai/schema.rs`, `src/gen3d/ai/convert.rs`).

### What “switching a motion algorithm” means in this repo

Switching an algorithm must not change the channel selection logic. It should only change the actual slot content for a given channel (most importantly `move`, optionally `idle`), and it must be possible to do this per instance at runtime.

Because part animation evaluation reads `PartAnimationPlayer.animations` (a per-visual-entity clone of prefab slots), runtime switching cannot rely only on mutating prefab definitions in `ObjectLibrary`. It must update the spawned visual entities (or provide an indirection layer).

### Non-negotiable constraint: no heuristics

Motion algorithms must be generic and deterministic. They may:

- use explicit rig contracts (declared by Gen3D AI output or human-authored descriptors),
- use geometry/anchor transforms already present in the prefab (deterministic),
- use well-defined math algorithms (for example, chain IK solvers),

but they must not:

- guess limb roles from component names,
- infer “legs” by searching for anchors that “look like feet”,
- special-case “humanoid” vs “vehicle” based on shape.

If the rig contract is missing or incompatible, the algorithm must refuse to apply (and surface a clear error/status).

## Plan of Work

### Milestone A — Define the rig contract format (descriptor-only)

Add a small, versioned “motion rig” JSON blob that can live inside a prefab descriptor.

Implementation target:

- Rig data lives under:

    PrefabDescriptorFileV1.interfaces.extra.motion_rig_v1

The contract must be sufficient for motion algorithms to identify *exactly* which attachment edges they may animate, without any detection logic.

Define `MotionRigV1` as:

- `version`: integer (start at 1)
- `kind`: string enum (start with `biped_v1`)
- `components`: map of semantic names to prefab UUID strings (component-prefab ids inside the saved model graph)
- `edges`: array of explicit attachment edges (each edge identifies one `ObjectRef` attachment edge in the prefab graph):
  - `parent_component`: semantic name (key into `components`)
  - `child_component`: semantic name (key into `components`)
  - `parent_anchor`: string
  - `child_anchor`: string
- `biped`: object present only when `kind = biped_v1`:
  - `left_leg_edges`: array of edge indices into `edges` (ordered proximal→distal)
  - `right_leg_edges`: array of edge indices into `edges` (ordered proximal→distal)
  - `move_cycle_m`: float (meters per gait cycle; required for `move_phase`)

This format deliberately avoids implicit meaning. If a model has 3 leg segments, the chain will have 3 edges; if it has 1 segment, the chain has 1 edge. The algorithm is responsible for handling variable chain length.

Documentation updates in this milestone:

- Update `docs/gamedesign/35_prefab_descriptors_v1.md` to recommend `interfaces.extra.motion_rig_v1` as a conventional key and document `MotionRigV1` at a high level.
- Update `gen_3d.md` to explain that Gen3D may produce rig contracts and that animations can be engine-injected rather than AI-authored.

### Milestone B — Runtime motion algorithm framework (no Gen3D prompt changes yet)

Implement a generic “motion algorithm” system that can be driven by descriptor rig contracts, without changing Gen3D prompts yet. This allows manual testing by editing a `.desc.json` file for a saved model.

Core design:

- Introduce a per-instance component on root entities (units/build objects) that selects algorithms, for example:
  - `MoveMotionAlgorithm`: `none` | `biped_walk_v1` | (future)
- Add an engine-side algorithm registry (a simple match/enum is fine initially; avoid dynamic plugin complexity).
- Add a system that, when the selected algorithm changes, updates the spawned visual entities so that the `move` channel slots are replaced by the algorithm’s generated slots on the declared edges.

Important implementation detail: enable per-instance injection without per-frame overhead when disabled.

To do that, add a lightweight, spawn-time “attachment edge reference” component to the visual tree for every `ObjectRef` part with an attachment. This component must store enough information to later insert/update/remove a `PartAnimationPlayer` deterministically, without needing to look back up the original part index.

Concretely:

- In `src/object/visuals.rs`, when spawning a child entity for an `ObjectRef` part, insert a new component (example name: `AttachmentEdgeBinding`) containing:
  - `root_entity: Entity`
  - `parent_object_id: u128`
  - `child_object_id: u128`
  - `parent_anchor: String`
  - `child_anchor: String`
  - `base_offset: Transform` (the part’s `transform` field; what `PartAnimationPlayer.base_transform` uses)
  - `base_slots: Vec<PartAnimationSlot>` (the prefab-authored slots for this edge)

Then implement injection by:

- Finding all `AttachmentEdgeBinding` for a given root entity.
- Matching bindings against rig edges (`parent_component_id`, `child_component_id`, anchors).
- For matched edges, computing an override `Vec<PartAnimationSlot>` for the `move` channel.
- Writing the effective slots into the edge’s `PartAnimationPlayer`:
  - If the edge has no `PartAnimationPlayer` yet, insert one.
  - If the algorithm is `none`, restore `base_slots`, and remove `PartAnimationPlayer` if it becomes empty (to preserve current performance behavior).

This makes algorithm switching immediate and per-instance, while keeping the existing animation evaluation system unchanged.

### Milestone C — Implement `biped_walk_v1`

Implement a first generic, deterministic two-leg walk algorithm.

Non-heuristic input:

- The algorithm requires `MotionRigV1.kind = biped_v1` and explicit `left_leg_edges` / `right_leg_edges`.
- The algorithm must not attempt to discover legs; it only animates declared edges.

Output contract:

- The algorithm produces `move` channel slots for the declared leg edges.
- Driver should be `move_phase` (meters-driven) using `move_cycle_m` as the loop length.
- Use `time_offset_units` to phase-shift right vs left legs (typically half a cycle).

Implementation approach (generic, deterministic):

- Generate keyframed `Loop` clips with a small number of keyframes (for example, 5 including the wrap to 1.0).
- Author rotations as delta transforms in the attachment join frame using small swing angles (keep within typical hinge/ball limits).
- Optionally add a subtle vertical bob on a “body” edge if the rig declares one explicitly (do not guess).

Acceptance for this milestone is purely visual: the legs swing while moving when `biped_walk_v1` is selected.

### Milestone D — Realtime switching UX

Add a simple in-game control for switching the move algorithm on selected objects.

Prescriptive UX (keep minimal):

- Add a hotkey in `src/rts.rs`:
  - `F6`: cycle `move` algorithm for selected entities: `none -> biped_walk_v1 -> none`
  - `Shift+F6`: cycle in reverse (optional)
- When the hotkey applies, print an `info!` log line showing the chosen algorithm and how many entities were updated.
- Add a minimal on-screen indicator (optional but recommended): a small HUD line that shows the selected unit’s current move algorithm.

### Milestone E — Gen3D strategy shift (optional, but the intended default)

Once runtime injection works, update Gen3D so that the AI stops authoring per-edge animations by default and instead emits rig contracts when appropriate.

Plan:

- Extend the Gen3D plan schema to allow an optional rig contract declaration (for example, `rig.kind` plus a biped section that references component names).
- During `Save`, translate the plan-level rig declaration (component names) into saved-prefab ids and write `interfaces.extra.motion_rig_v1` into the saved model’s descriptor.
- Keep a compatibility flag in config:
  - `gen3d.ai_authored_animations = true|false` (default can remain `true` until the new workflow is proven).

The key requirement is that Gen3D remains functional even when the AI does not output any `attach_to.animations`.

## Concrete Steps

All commands run from repo root (`/Users/flow/workspace/github/gravimera`).

During implementation, after any code change:

1) Run unit tests:

    cargo test

2) Run the required rendered smoke test (AGENTS.md; do NOT use `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance behaviors after implementation:

1) Manual rig test:
   - Generate and Save a simple Gen3D biped-like model (or use an existing saved model).
   - Edit its descriptor file (`*.desc.json`) to include a valid `motion_rig_v1` biped contract.
   - Start the game, select the unit, press `F6`.
   - Observe the unit’s motion changes immediately while moving (no crash).

2) Switching does not affect gameplay state:
   - Movement still toggles `move` channel; attack still toggles `attack_primary`.
   - Only the animation content changes.

3) Non-rigged models are safe:
   - Selecting a model without `motion_rig_v1` and pressing `F6` does not crash; it prints a clear message that the rig is missing/incompatible.

## Idempotence and Recovery

- Descriptor edits are reversible by removing `interfaces.extra.motion_rig_v1`.
- If a rig contract is invalid, the engine should treat it as absent and log a warning rather than panic.
- If motion injection goes wrong visually, switching back to `none` should restore base animation slots exactly.

## Artifacts and Notes

- Record (in this ExecPlan) any discovered edge cases, especially around:
  - mirrored components / negative scales,
  - attachment bases with non-identity rotations,
  - multi-use of a single component prefab id in multiple edges,
  - performance impact if too many edges become animated.

## Interfaces and Dependencies

The implementation should introduce at minimum:

- A JSON shape `MotionRigV1` stored under `PrefabDescriptorInterfacesV1.extra["motion_rig_v1"]`.
- A per-instance component that selects the move algorithm (exact type name is up to implementation, but it must be persisted in the ECS and be easy to edit via hotkey).
- A spawn-time visual binding component for attachment edges that enables deterministic injection without heuristics and without requiring part-index lookups.

