# Object Forms + Mechanical Transform (Tab/C) + Persistence

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository’s ExecPlan requirements live in `PLANS.md` at the repo root. Maintain this document accordingly.

## Purpose / Big Picture

After this change, any eligible object instance can have multiple “forms” (each form is a prefab id). In both Build and Play, the player can press `Tab` to switch all currently selected objects to their next form. In Build and Play, the player can press `C` to enter “copy current form” mode: the current selection becomes the destination set, and the next clicked object becomes the source; the source object’s *current* form is appended (deduped) to each destination’s form list and each destination immediately switches to that new form.

Switching forms is visualized with an automatic “mechanical transform” animation between the old prefab’s primitive parts and the new prefab’s primitive parts (no authoring panel). Any object with more than one form shows a circular badge with `i/n` (e.g. `2/3`) above its top-left in screen space.

Forms persist across saves:

- `scene.dat` (protobuf) includes the form list + active index for each saved instance (no backwards compatibility required).
- Scene sources pinned instances JSON include the optional form list + active index (backwards compatible by optional fields).

Hard rule: units and buildings cannot transform between each other. A “unit” is an instance with `Commandable` and a prefab whose `mobility` is present; a “building” is an instance with `BuildObject` and a prefab whose `mobility` is absent. Appending/switching to a form of the opposite category is blocked/skipped.

## Progress

- [x] (2026-02-19) Write spec docs for ObjectForms, inputs, persistence, and badge UI.
- [x] (2026-02-19) Add Build/Play UI toggle button and remove `Tab` binding from mode toggle.
- [x] (2026-02-19) Add `ObjectForms` component (forms + active index) and systems for `Tab` switching (all selected).
- [x] (2026-02-19) Add copy-flow state machine: `C` then click source; append+dedupe+switch.
- [x] (2026-02-19) Add `scene.dat` persistence for forms (bump version; no compat) and scene-sources pinned instance persistence (optional JSON fields).
- [x] (2026-02-19) Add world-anchored “i/n” badge UI for all multi-form objects.
- [x] (2026-02-19) Add mechanical transform animation rig for primitive parts when switching forms.
- [x] (2026-02-19) Run smoke test (`cargo run -- --headless --headless-seconds 1`).
- [x] (2026-02-19) Commit.

## Surprises & Discoveries

- Observation: Bevy UI in this repo uses absolute-position `Node` entities in world overlay space (examples: health popups, minimap markers), so form badges should follow that pattern.
  Evidence: `src/ui.rs` uses `camera.world_to_viewport` and spawns `Node { position_type: Absolute, left/top: Px(..) }`.

- Observation: `scene.dat` load is guarded by a strict version check; bumping `SCENE_DAT_VERSION` is the simplest “no backward compatibility” approach.
  Evidence: `src/scene_store.rs` rejects any `scene.version != SCENE_DAT_VERSION`.

## Decision Log

- Decision: Copy operation appends only the source’s current form (not all forms), dedupes, and immediately switches destinations to the appended form.
  Rationale: Matches the requested UX and keeps operations predictable.
  Date/Author: 2026-02-19 / GPT-5.2

- Decision: `Tab` switches forms for all selected objects; switching is skipped for entities with only one form.
  Rationale: Matches request and preserves existing selection semantics.
  Date/Author: 2026-02-19 / GPT-5.2

- Decision: Units/buildings are disjoint categories; cross-category transforms are blocked.
  Rationale: Required constraint; avoids complex component graph mutations (BuildObject ↔ Commandable).
  Date/Author: 2026-02-19 / GPT-5.2

- Decision: `scene.dat` version bump to encode forms (no backward compatibility).
  Rationale: Explicitly allowed; simplest safe path.
  Date/Author: 2026-02-19 / GPT-5.2

## Outcomes & Retrospective

- Delivered instance-level multi-form objects with `Tab` cycling and `C`-copy flow, automatic mechanical transform animation, always-visible `i/n` badge, and persistence in `scene.dat` + pinned instances JSON.
- Kept the change generic (no authoring UI); cross unit/building transforms are blocked.

## Context and Orientation

Key files and systems involved:

- `src/build.rs`: Build-mode behaviors and key bindings; Build/Play is toggled via `F1` and a top-left UI button.
- `src/rts.rs`: Selection and RTS input (mouse selection logic lives here).
- `src/object/registry.rs`: Prefab (`ObjectDef`) definitions, parts, anchors/attachments, and the `ObjectLibrary`.
- `src/object/visuals.rs`: Spawns prefab visuals from `ObjectDef.parts` and manages per-part animations.
- `src/scene_store.rs`: `scene.dat` save/load. Strict version check; protobuf structs defined in this file.
- `src/scene_sources_runtime.rs`: Import/export of “scene sources” pinned instances JSON.
- `src/ui.rs`: UI overlay patterns (health popups, minimap markers) with `camera.world_to_viewport`.
- `src/setup.rs`: UI setup for top-left buttons and generated UI images (minimap triangle mask).

Terminology:

- “Prefab id”: `u128` UUID identifying an `ObjectDef` in `ObjectLibrary` and stored in `ObjectPrefabId` component.
- “Form”: a prefab id in an instance’s per-object `forms[]` list.
- “Active form”: `forms[active]` (0-based index) which must equal the instance’s `ObjectPrefabId`.
- “Unit”: entity with `Commandable` that uses a prefab with `mobility: Some(_)`.
- “Building”: entity with `BuildObject` that uses a prefab with `mobility: None`.
- “Mechanical transform rig”: temporary child entities (primitives) spawned during a form switch that animate from old-part transforms/colors to new-part transforms/colors.

## Plan of Work

Implement the feature in these layers:

1) Inputs and mode toggle:
   - Add a UI button for Build/Play toggle, and remove the `Tab` binding from `toggle_game_mode`.
   - Reserve `Tab` for form switching.

2) Data model:
   - Add `ObjectForms { forms: Vec<u128>, active: usize }` component.
   - Define invariant: `forms.len() >= 1`, `active < forms.len()`, and `ObjectPrefabId == forms[active]`.

3) Selection-driven operations:
   - `Tab`: switch all selected entities that have `ObjectForms` (or default to `[ObjectPrefabId]` if missing).
   - `C` then click: store destination set on `C`, then on next click compute source entity and apply append+dedupe+switch to destinations.

4) Persistence:
   - `scene.dat`: bump version; extend instance schema with forms list + active index. Ensure defs include all referenced forms.
   - Scene sources pinned instances JSON: add optional fields `forms` and `active_form` and round-trip them.

5) UI badge:
   - For every entity with `ObjectForms.forms.len() > 1`, spawn a screen-space circular badge anchored above the object, showing `active+1/forms.len()`.

6) Mechanical transform animation:
   - On form switch, spawn a temporary rig under the object that:
     - flattens both prefabs into leaf primitives (resolve `ObjectRef` + attachments),
     - builds a deterministic mapping (prefer same primitive type),
     - animates transforms and colors over a fixed duration,
     - then despawns rig and spawns normal visuals for the target prefab.

7) Validation:
   - Add unit tests for dedupe+switch and save/load round-trips.
   - Run a smoke test to ensure the game starts and doesn’t crash.

## Concrete Steps

All commands run from repo root:

    cargo test
    cargo run -- --headless --headless-seconds 1

Manual acceptance run:

    cargo run

Then:

- Place/build a couple objects (or spawn from scene sources).
- Use `C` then click to copy current form as a new form and observe:
  - destination switches immediately (badge updates),
  - mechanical transform animation plays,
  - badge appears for multi-form objects.
- Select multiple objects and press `Tab` and observe all selected cycle forms with animation.
- Save/load (toggle Build→Play triggers save) and confirm forms persist.

## Validation and Acceptance

Acceptance criteria:

- Build/Play mode is toggled via a UI button; `Tab` no longer toggles modes.
- `Tab` cycles forms for all selected objects; cross-category forms are not allowed.
- `C` then click source appends source current form (deduped) to all destination objects and switches immediately.
- A circular `i/n` badge appears for all multi-form objects and tracks them on screen.
- Switching forms triggers an automatic mechanical transform animation (primitive parts move/scale/color; cross-shape uses shrink/fade + grow/fade).
- Forms persist in `scene.dat` and in scene sources pinned instance JSON.
- `cargo test` passes and a headless smoke run starts without crashing.

## Idempotence and Recovery

- Scene sources JSON writes are deterministic and can be re-run safely.
- `scene.dat` version bump means old saves may be ignored; delete existing `scene.dat` if it causes confusion.

## Artifacts and Notes

- Keep README clean; place any feature notes under `docs/` (update `docs/gamedesign/specs.md` to index the new spec doc).

## Interfaces and Dependencies

Add/ensure the following internal interfaces exist:

- `crate::types::ObjectForms` component.
- `crate::types::SelectionClickEvent` (or similar) message emitted on click-selection to drive the copy flow.
- `crate::forms` (or similar module) systems:
  - `forms_handle_tab_switch`
  - `forms_handle_copy_flow`
  - `forms_apply_prefab_change_and_update_colliders`
  - `forms_spawn_and_update_badges`
  - `forms_spawn_and_update_transform_rig`
