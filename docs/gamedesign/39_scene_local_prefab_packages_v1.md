# Scene-Local Prefab Packages (v1)

This spec defines the **scene-local prefab package** on-disk layout: how non-built-in prefabs (including Gen3D-saved prefabs) are stored under a scene directory for **editing, copying, and tooling**.

This is intentionally separate from runtime persistence: **runtime scene load must not require these packages**. A scene must be able to load and render from `scene.dat` alone (plus built-in prefab defs compiled into the game).

## Goals

- **Scene reliability:** `scene.dat` is the single required runtime file for loading a scene.
- **Locality:** user-created prefabs are stored under the scene they were created in (no global depot, no realm-wide shared packs).
- **Restart-safe Gen3D edit:** a Gen3D-saved prefab includes the minimum persisted metadata required to restart the game and continue editing (without relying on `~/.gravimera/cache/`).
- **Prefab-scoped assets:** any non-built-in assets (textures/meshes/etc) associated with a prefab are stored under that prefab’s own folder so assets from different prefabs do not intermix.

## Non-Goals

- This spec does not define a “shared library” or cross-scene asset store. Copying between scenes (export/import) is an explicit tool operation.
- This spec does not replace `scene.dat`. Packages exist to support authoring workflows, not as a runtime dependency.

## Terms

- **Scene directory:** `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/`.
- **Prefab package:** one directory containing a root prefab and any related files (prefab JSON, descriptor, Gen3D edit metadata, assets).
- **Root prefab id:** the prefab UUID that identifies a saved object “as a thing you can spawn/edit”. This is the directory name of the prefab package.
- **Prefab defs:** one or more `PrefabFileV1` JSON documents (see `docs/gamedesign/34_realm_prefabs_v1.md`) representing the root prefab plus any internal component prefabs referenced by `object_ref`.
- **Prefab descriptor:** an optional `PrefabDescriptorFileV1` JSON document stored next to a prefab JSON (see `docs/gamedesign/35_prefab_descriptors_v1.md`).
- **Gen3D source bundle:** a directory containing the editable Gen3D draft defs (used to restart editing with high fidelity).
- **Gen3D edit bundle:** a single JSON file containing “session hydration” state (planned components, plan hash, etc.) required to continue editing after restarting.

## Directory Layout

All scene-local prefab packages live under:

- `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/prefabs/`

Each prefab package directory is named by the root prefab UUID:

- `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/prefabs/<root_prefab_uuid>/`

Within a prefab package:

- `prefabs/`
  - `<prefab_uuid>.json` (required; `PrefabFileV1`)
  - `<prefab_uuid>.desc.json` (optional; `PrefabDescriptorFileV1`)
- `gen3d_source_v1/` (optional; only for Gen3D-saved prefabs)
  - `*.json` (prefab defs saved in the same JSON schema as `PrefabFileV1`)
- `gen3d_edit_bundle_v1.json` (optional; only for Gen3D-saved prefabs; required for restart-safe edit)
- `materials/` (reserved; may be empty today)
  - any prefab-scoped external assets (textures/meshes/etc)

Notes:

- The `prefabs/` directory contains the **published** runtime prefab defs for this package. Tools may load these defs into memory on demand to spawn/copy/edit, but runtime scene load must not depend on them.
- `gen3d_source_v1/` contains **draft** defs (including the Gen3D draft root prefab id used by the Gen3D engine). Tools must not treat these draft ids as stable world prefab ids.
- `materials/` is always scoped per prefab package. Future `ObjectPartKind::Model { scene }` references that point to external files must resolve under this folder (or another folder inside this package) rather than a global depot.

## Enumeration Rules

To list scene-local prefab packages for a scene:

1. List direct children of `.../scenes/<scene_id>/prefabs/`.
2. Keep only directory entries whose names parse as UUIDs.
3. Treat each UUID as a root prefab id.

Non-UUID directories are ignored.

## Runtime Load Rules (Critical)

- **A scene must load and render using only:**
  - built-in prefab defs (compiled into the game), and
  - the `defs` embedded in `scene.dat`.
- Runtime scene load must not scan, parse, or depend on `.../prefabs/` packages.
- Deleting or renaming `.../scenes/<scene_id>/prefabs/` must not prevent scene load.

If tooling wants to spawn/copy/edit prefabs that are not already referenced by the current `scene.dat`, it must load prefab defs from the relevant package **explicitly** (edit-time only).

## Gen3D: Persisted Edit Metadata

Gen3D-saved prefab packages must include:

- `gen3d_source_v1/` (draft defs), and
- `gen3d_edit_bundle_v1.json` (session hydration state).

If `gen3d_edit_bundle_v1.json` is missing, the prefab is considered **not editable** via Gen3D Edit/Fork after restart (tools must return a clear error).

### `gen3d_edit_bundle_v1.json` Schema (v1)

`gen3d_edit_bundle_v1.json` is a JSON object with the following fields:

- `version`: integer. Must be `1`.
- `root_prefab_id_uuid`: UUID string. The root prefab id of the saved prefab package.
- `created_at_ms`: integer. Unix epoch milliseconds.
- `plan_hash`: string. The plan hash that the Gen3D agent uses to correlate state.
- `assembly_rev`: integer. The current assembly revision (used by `apply_draft_ops_v1` optimistic checks).
- `assembly_notes`: string. Optional but recommended (may be empty).
- `plan_collider`: optional collider JSON (same shape as Gen3D AI schema `AiColliderJson`).
- `planned_components`: array. Deterministic component tree + attachment metadata required to:
  - rebuild the attachment tree in defs, and
  - allow `apply_draft_ops_v1` to operate on anchors/attachments/parts after restart.
- `rig_move_cycle_m`: optional float (meters). If present, informs motion authoring.
- `motion_authoring`: optional JSON (same shape as `AiMotionAuthoringJsonV1`).
- `reuse_group_warnings`: optional array of strings.

Tools may add new fields in the future; readers should ignore unknown fields.

### `planned_components` item shape (v1)

Each `planned_components[]` entry is a JSON object:

- `display_name`: string. Human-friendly component name (may include spacing/case).
- `name`: string. Stable component key used by tools (snake_case recommended).
- `purpose`: string. Optional semantic intent (may be empty).
- `modeling_notes`: string. Optional notes (may be empty).
- `pos`: `[x,y,z]` floats. Current resolved translation in the assembled root frame.
- `rot_quat_xyzw`: `[x,y,z,w]` floats. Current resolved rotation (quaternion) in the assembled root frame.
- `planned_size`: `[x,y,z]` floats. Planned size (meters).
- `actual_size`: optional `[x,y,z]` floats. Populated once a component has generated geometry.
- `anchors`: array of anchors (may be empty). Each anchor:
  - `name`: string (non-empty)
  - `transform`: `{ translation, rotation_quat_xyzw, scale }`
- `contacts`: optional array of Gen3D contacts (same shape as `AiContactJson` in `src/gen3d/ai/schema.rs`).
- `attach_to`: optional attachment object. If omitted, the component is the root of the component tree.

### `attach_to` shape (v1)

`attach_to` is a JSON object:

- `parent`: string. Parent component `name`.
- `parent_anchor`: string. Parent anchor name (or `"origin"`).
- `child_anchor`: string. Child anchor name (or `"origin"`).
- `offset`: `{ translation, rotation_quat_xyzw, scale }`. Attachment offset transform.
- `joint`: optional joint JSON (same shape as `AiJointJson` in `src/gen3d/ai/schema.rs`).
- `animations`: optional array of animation slots applied to the attachment edge. Each slot:
  - `channel`: string
  - `spec`: `{ driver, speed_scale, time_offset_units, clip }`

### `TransformBundleV1` shape (v1)

Transforms in the edit bundle use a single JSON shape:

- `translation`: `[x,y,z]` floats
- `rotation_quat_xyzw`: `[x,y,z,w]` floats
- `scale`: `[x,y,z]` floats

## Idempotence and Overwrite Rules

- Saving a Gen3D edit in “overwrite” mode overwrites the existing prefab package directory for that root prefab id.
- Saving a Gen3D fork writes a new prefab package directory named by the new root prefab id.
- Tools should treat repeated writes as idempotent: delete/rewrite `gen3d_source_v1/` on each save to avoid stale files; delete stale prefab JSON under `prefabs/` that no longer correspond to the saved defs.
