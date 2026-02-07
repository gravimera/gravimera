# Gen3D anchors, attachments, and component-level animation foundation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D currently assembles multi-component models using per-component absolute placement (`pos` + `forward`/`up`). This sometimes produces incorrect placement/rotation for parts like arms/legs/doors because the assembly relies on ambiguous direction vectors and the AI’s ability to “guess” exact transforms.

After this change, Gen3D assemblies are constrained by **named anchors** and **tree-style attachments**:

- The plan defines which component attaches to which parent component (tree), and which anchors are used.
- Each component draft defines its anchors precisely on its generated geometry.
- The engine assembles the final object by **aligning child anchors to parent anchors** (no Euler angles), yielding more deterministic placement.

Additionally, we introduce the **data model foundation** for component-level animation (components are “bones”; animation acts by changing the attachment offset transform over time). This plan focuses on getting anchors/attachments correct, persisted, and debuggable; actual animation playback behavior can be layered on without redesigning the data model.

User-visible outcome: in Gen3D, generated models with multiple components (chair legs, arms, wall+door, glasses+face) should assemble with fewer “90° rotated / mirrored / misplaced” errors. The system should also persist through `scene.dat` and reload cleanly.

## Progress

- [x] (2026-01-28 22:00Z) Create anchor/attachment ExecPlan (this file) and audit current Gen3D + object/persistence codepaths.
- [x] (2026-01-28 22:15Z) Extend object definitions to include anchors/attachments; resolve attachment-based transforms at spawn time.
- [x] (2026-01-28 22:25Z) Redesign `scene.dat` schema (bump version; no backward compatibility) to persist anchors/attachments and per-part animation.
- [x] (2026-01-28 22:45Z) Update Gen3D JSON schemas/prompts/parsing to output anchors + tree attachments; assemble via anchors and stop using Euler angles in AI schemas.
- [x] (2026-01-28 23:05Z) Update docs (`gen_3d.md`, `object_system.md`, `README.md`) to match the new anchor/animation design.
- [x] (2026-01-28 23:20Z) Run `cargo test` + smoke test; record transcripts in this ExecPlan.

## Surprises & Discoveries

- Observation: component regeneration overwrote child `ObjectRef` attachment parts.
  Evidence: previously, regenerating a component prefab replaced its `parts` list entirely, dropping already-attached child components and making the assembled draft incomplete.
  Fix: preserve and re-merge existing `ObjectRef` parts when updating a generated component definition.

- Observation: recentering generated component geometry must also recenter anchors.
  Evidence: anchors are defined in component-local space; recentering parts without shifting anchors breaks attachments.
  Fix: canonicalization shifts both primitive part translations and anchor translations by the same center offset.

## Decision Log

- Decision: Anchors are defined in both the plan JSON and each component draft JSON.
  Rationale: The plan needs stable anchor names and expected frames; the component draft needs the precise frames that match the generated geometry. Using both enables validation and deterministic assembly.
  Date/Author: 2026-01-28 / Codex

- Decision: Attachments are tree-style only (each component has at most one parent attachment).
  Rationale: Simpler and sufficient for current Gen3D; avoids constraint solving and cycle handling complexity.
  Date/Author: 2026-01-28 / Codex

- Decision: No backward compatibility for `scene.dat` for this change.
  Rationale: Avoid complex migration code while the format is still evolving rapidly.
  Date/Author: 2026-01-28 / Codex

- Decision: Stop using Euler in AI-visible schemas for rotations.
  Rationale: Euler is ambiguous and correlated with mis-assemblies; use direction vectors and normalize to quaternions internally.
  Date/Author: 2026-01-28 / Codex

- Decision: Represent component-level animation as an optional per-part loop of keyframed delta transforms.
  Rationale: A component is an `ObjectRef` part in the root draft; animating that part’s attachment offset enables “bone-like” animation without introducing a separate skeleton system.
  Date/Author: 2026-01-28 / Codex

## Outcomes & Retrospective

- Implemented anchor-based, tree-style attachments end-to-end (object defs, Gen3D plan/drafts, spawn-time resolution, persistence).
- Implemented per-part animation foundation + runtime playback (delta(t) * base) and persisted it in `scene.dat` (version bump, no backward compat).
- Updated user-facing and design docs to reflect anchors/attachments and animation.
- Verified: `cargo test` passes and the headless smoke run starts and exits cleanly.

## Context and Orientation

Key files/modules:

- `src/object/registry.rs`: Defines `ObjectDef` (prefab) and `ObjectPartDef` (composition parts).
- `src/object/visuals.rs`: Spawns visuals for prefabs; this is where part transforms are applied when spawning nested `ObjectRef`s.
- `src/scene_store.rs`: Protobuf schema (prost) for `scene.dat`, save/load, and conversions between `ObjectDef` / `ObjectPartDef` and the persisted representation.
- `src/gen3d/ai.rs`: Gen3D pipeline; calls OpenAI, parses JSON plan/draft, creates component prefabs and the root combined prefab.
- `gen_3d.md`: Documentation of the Gen3D feature and its AI schemas.
- `object_system.md`: Design doc for the object system.

Definitions:

- Anchor: A named coordinate frame on an object, expressed as a transform in that object’s local space.
- Attachment: A relationship that places a child object by aligning one of the child’s anchors to one of the parent’s anchors, optionally with an additional offset transform.
- Tree-style attachments: Attachments form a tree; each child has at most one parent (no cycles).
- Component: In Gen3D, a logical sub-part of the generated object (e.g., chair seat, chair back, one leg). Each component becomes its own `ObjectDef` prefab.

## Plan of Work

We will implement anchors/attachments end-to-end across:

1) Object system:
   - Add `anchors: Vec<AnchorDef>` to `ObjectDef`.
   - Add `attachment: Option<AttachmentDef>` to `ObjectPartDef`.
   - Add helpers to fetch anchors by name, with implicit `origin` anchor.

2) Visual spawning:
   - When a part has an attachment, compute its local transform by anchor alignment:

       child_root_in_parent =
         parent_anchor_in_parent * offset_in_parent_anchor * inverse(child_anchor_in_child)

     Then spawn the child using this computed transform.

3) Persistence (`scene.dat`):
   - Bump `SCENE_DAT_VERSION`.
   - Add `anchors` to `SceneDatObjectDef`.
   - Add `attachment` to `SceneDatPartDef`.
   - Update encode/decode conversions and add tests that round-trip anchors/attachments.

4) Gen3D:
   - Update plan schema to include:
     - `root_component` (or infer root by missing attachment).
     - `components[].anchors[]`
     - `components[].attach_to` (parent + anchor names) for non-root components.
   - Update component draft schema to include:
     - `anchors[]` (precise anchor frames)
     - `parts[]` (primitives)
   - Remove Euler-based fields (`rot_degrees`, `yaw_degrees`) from AI schemas and prompts; use direction vectors and normalize.
   - Assemble the root combined prefab with `ObjectRef` parts that include `attachment`.

5) Component-level animation foundation:
   - Extend the data model to allow an optional per-part animation description.
   - Play it at runtime by applying `delta(t) * base` every frame; if the part is attached, recompute the resolved attachment transform using the animated offset.

6) Validation:
   - `cargo test`
   - Smoke test (AGENTS.md): `cargo run -- --headless --headless-seconds 1`

## Concrete Steps

All commands run from repo root (`/Users/flow/workspace/github/gravimera`):

1) Run unit tests:

    cargo test

2) Run smoke test:

    cargo run -- --headless --headless-seconds 1

Expected result: process exits without panic/crash.

## Validation and Acceptance

Acceptance criteria:

1) `scene.dat`:
   - Saving and loading a scene works with the new version.
   - Older versions are ignored with a clear log warning (no crash).

2) Gen3D:
   - Plan parsing accepts the new JSON schema with anchors + attachments.
   - Component parsing accepts anchors and primitive parts without Euler angles.
   - Root prefab uses attachments; components align by anchors.
   - Cache artifacts still include readable plan/draft dumps for debugging.

3) Tests:
   - New tests demonstrate the attachment transform math (anchor-to-anchor alignment).
   - New tests demonstrate protobuf round-trip for anchors/attachments/animation.

## Idempotence and Recovery

- The code changes are safe to re-run and re-test repeatedly.
- If `scene.dat` exists from older versions, the loader will ignore it and log a warning; deleting `scene.dat` is a safe manual recovery.
- Gen3D cache dirs are per-run and can be deleted to recover disk space.

## Artifacts and Notes

- Test run:

    $ cargo test
    running 8 tests
    test result: ok. 8 passed; 0 failed

- Smoke run:

    $ cargo run -- --headless --headless-seconds 1
    Headless mode: running simulation for 1.0s
    Headless simulation finished. score: 0 | health: 1000

## Interfaces and Dependencies

At the end of this change, the following Rust types should exist:

In `src/object/registry.rs`:

    pub(crate) struct AnchorDef {
        pub(crate) name: Cow<'static, str>,
        pub(crate) transform: Transform,
    }

    pub(crate) struct AttachmentDef {
        pub(crate) parent_anchor: Cow<'static, str>,
        pub(crate) child_anchor: Cow<'static, str>,
    }

    pub(crate) struct ObjectDef {
        ...
        pub(crate) anchors: Vec<AnchorDef>,
        pub(crate) parts: Vec<ObjectPartDef>,
        ...
    }

    pub(crate) struct ObjectPartDef {
        ...
        pub(crate) attachment: Option<AttachmentDef>,
        pub(crate) animation: Option<PartAnimationDef>,
        pub(crate) transform: Transform,
        ...
    }

In `src/object/visuals.rs`:

- The visual spawning path must apply attachment math when `attachment.is_some()`.
