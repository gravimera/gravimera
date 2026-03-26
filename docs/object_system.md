# Object System (Design)

All 3D assets in the game are **objects**, including the hero, enemies, bullets, buildings, and decorative models.

This document describes the *target* object system architecture. It is intended to guide refactors and features such as persistence and AI-assisted generation.

## Object Definitions vs Instances

Gravimera uses a **prefab-style** object model:

- **Object definition (prefab)**: a reusable definition identified by a stable `ObjectId` (`u128`, UUID).
- **Object instance**: a runtime/spawned copy that references a base prefab by id, plus per-instance transform and overrides.

There is **no separate “object type”** (`TypeId`) layer. Composition is expressed directly as “an object has parts”.

## ObjectId

- `ObjectId` is a stable unique id (`u128`, UUID).
- Prefab ids are stable and persisted in `scene.dat`.
- Instance ids are also stable and persisted (so selections/edits can be tracked across saves).

At runtime we rely on **Bevy `Entity`** for fast references, and keep `ObjectId` mainly for persistence/debug/networking.

## ObjectDef (Prefab)

Each prefab has at least:

1. `object_id` (`u128`) and a label (debugging/tools/UI).
2. `size` (world-space; used for placement/UI, may be derived later).
3. `collider` (simple profile; see below).
4. `interaction` flags (generic, data-driven; no hard-coded logic):
   - `blocks_bullets`
   - `blocks_laser`
   - `movement_block` (e.g. “blocks upper body only” to allow bridges)
   - `supports_standing`
5. Optional `mobility`:
   - `None` means the object is static (building/prop).
   - `Some(MobilityDef { mode, max_speed })` means the object can be moved by RTS commands.
     - `mode`: `Ground` (pathfind) or `Air` (direct).
6. `anchors: Vec<AnchorDef>`: named coordinate frames in the prefab’s local space (used for deterministic attachments; `"origin"` is an implicit identity anchor).
7. `parts: Vec<PartDef>` (composition).
8. Optional generic “behavior profiles” stored as data (enemy, projectile, muzzle, etc.).

## Parts (Composition)

Each `PartDef` is an internal child of the prefab (parts are not top-level persisted objects by default):

- `part_id: u128` (stable within a prefab; future tooling/AI can target specific parts)
- `transform: Transform`
- Optional `attachment: AttachmentDef` (anchor-based placement; see below)
- Optional `animations: Vec<PartAnimationSlot>` (time-varying delta applied to `transform`; see below)
- `kind`:
  - `Primitive`: a Bevy primitive (cuboid/sphere/cylinder/cone/...) with optional primitive params, base color and `unlit`.
  - `Model`: an imported 3D model “variation”, addressed by an asset path like `foo.glb#Scene0`.
  - `ObjectRef`: reference another prefab by `object_id` (composition by referencing a prefab).

“Atom vs Combined” is purely data-driven:

- An “atom object” is a prefab with a single `Primitive` or `Model` part.
- A “combined object” is a prefab with multiple parts and/or `ObjectRef` parts.

### Anchors and Attachments (Deterministic Assembly)

An **anchor** is a named coordinate frame on a prefab:

- `name`
- `transform` (in that prefab’s local space; includes position and orientation)

An **attachment** places a child part by aligning anchors (tree-style for Gen3D):

- `parent_anchor`: anchor name on the parent prefab
- `child_anchor`: anchor name on the referenced child prefab

When a part has an attachment:

- `transform` is interpreted as an **offset in the parent anchor frame** (not an ordinary parent-local transform).
- The final part transform is resolved as:

  `parent_anchor * offset * inverse(child_anchor)`

This avoids ambiguous Euler rotations and makes assembly more deterministic (especially for AI-generated multi-component models).

### Per-Part Animation (Component-Level by Attachment)

Parts can optionally have one or more animation **slots**, each in a named **channel**.

- Channels: `ambient`, `idle`, `move`, `action`, `attack`
- Priority (highest wins): `attack > action > move > idle > ambient`

- The runtime applies `animated = delta(t) * base_transform` in the part’s local space.
- For an attached `ObjectRef`, this effectively animates the **attachment offset**, which gives component-level animation (e.g., a door hinge, an arm swing, a lever).

Animation slots are driven by generic gameplay signals and do not depend on hard-coded object ids:

- `move` is active while the owning entity is moving.
- `action` is active while the owning entity is performing an “action” window (operating/handling something important).
- `attack` is active while the owning entity is attacking/firing.
- `idle` is active when not moving, acting, or attacking.
- `ambient` is always active (fallback motions like fans/propellers).

Animation specs are data-driven:

- Drivers (what `t` means): wall time, locomotion phase, movement distance, seconds since attack start, or seconds since action start.
- Clips: loop keyframes or a procedural `Spin` (better for wheels/fans than keyframing 360°).

## Collision (Simple Now; Derivable for Combined Types)

Keep collision simple (good performance and easy reasoning). We can start with:

- `CircleXZ { radius }` for characters/projectiles.
- `AabbXZ { half_extents }` for blocks/fences.

For **combined objects**, collision can later be derived from their parts:

- Compute a single bounding shape from all colliding parts (and their local transforms).
- Start with a single shape; later we can evolve to compound collision if needed.

Implementation note: for MVP, collision is defined on the root prefab (fast and predictable).

## Persistence: `scene.dat`

Goal: efficient load/encode and future extensibility. Use protobuf.

- File: `scene.dat` next to the running binary.
- Save timing: when switching **Build → Play**.
- Load timing: on game start, try load if present.
- Errors: log an error and continue the game (do not abort).
- Persisted objects: **build objects only** for now (hero/enemies are runtime-spawned; future task may persist them).
- Stored fields per instance: `instance_id` (UUID u128), `base_object_id` (UUID u128), quantized `position`, `rotation`, `scale`, and `overrides` (currently: whole-object `tint`).
- Stored fields for embedded prefabs: `ObjectDef` records (id, parts, anchors/attachments/animation, collider, interaction, optional behavior profiles).
- `scene.dat` only embeds the **transitive closure** of prefab defs referenced by build instances (so the file stays small).

## Implementation Notes

1. Each built-in prefab lives in its own Rust file/module (for now).
2. A central library maps `object_id -> ObjectDef`.
3. Composition is expressed by data-driven parts (`Primitive` / `Model` / `ObjectRef`).
4. Copies are **live prefab instances**: an instance references `base_object_id` and can override whole-object settings like `tint` (tint cascades through referenced parts).
5. Systems operate on object **properties** (interaction flags / profiles), not by matching concrete ids.
