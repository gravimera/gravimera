# Prefab Definitions (`PrefabFileV1`, v1)

This spec defines the **prefab definition JSON** format (`PrefabFileV1`): how an object definition (a “prefab”) is stored as a stable, text-based JSON document.

Prefab definition JSON is used in multiple places, but the primary on-disk home for user-created prefabs is the **scene-local prefab package** (see `docs/gamedesign/39_scene_local_prefab_packages_v1.md`).

## Goals

- **Git/process friendly:** textual, stable, diffable.
- **Versioned:** forward migrations are possible without corrupting the source.
- **Generic:** no game-specific heuristics; supports “any object”.

## Directory Layout

Prefab definition JSON documents are stored under a prefab package’s `prefabs/` directory:

- `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/prefabs/<root_prefab_uuid>/prefabs/<prefab_uuid>.json`

Notes:

- `<prefab_uuid>.json` filename **should match** the document’s `prefab_id`.
- A prefab package may contain multiple prefab JSON files: the root prefab plus any internal component prefabs referenced by `object_ref`.

## Prefab Document: `PrefabFileV1`

Each prefab is stored as one JSON document.

Top-level fields:

- `format_version`: integer. **Must be `1`** for this spec.
- `prefab_id`: UUID string. Stable prefab id.
- `role`: `"root"` | `"internal"`. Informational hint for tooling (root prefab vs internal component).
- `label`: string. Human-friendly name.
- `size`: `{x,y,z}` floats. Authoring/physics extents used by the engine.
- `ground_origin_y`: optional float. Distance from the prefab’s local origin to the ground plane when the object is resting on the ground. When absent, the engine assumes the prefab is vertically centered and uses `size.y / 2`. When an instance is scaled, grounding uses `ground_origin_y * abs(scale.y)`.
- `collider`: collider profile (see below).
- `interaction`: interaction profile (see below).
- `aim`: optional aim profile (units/weapons).
- `mobility`: optional mobility definition (units/vehicles).
- `anchors`: array of anchors (attachment points).
- `parts`: array of parts composing the prefab.
- `minimap_color_rgba`: optional `{r,g,b,a}` floats in `[0,1]`.
- `health_bar_offset_y`: optional float.
- `projectile`: optional projectile profile.
- `attack`: optional unit attack profile.

### `size`

- `size.x`, `size.y`, `size.z`: floats.

### `ground_origin_y` (optional)

- Float. Must be finite and `>= 0`.
- Interpreted as a local-space distance from the prefab origin to the ground plane when resting.
- If omitted, the engine falls back to `size.y / 2`.

### `collider`

Tagged union (`kind`):

- `{ "kind": "none" }`
- `{ "kind": "circle_xz", "radius": <f32> }`
- `{ "kind": "aabb_xz", "half_extents": { "x": <f32>, "y": <f32> } }`

### `interaction`

- `blocks_bullets`: bool
- `blocks_laser`: bool
- `movement_block`: optional:
  - `{ "kind": "always" }`
  - `{ "kind": "upper_body_fraction", "fraction": <f32> }`
- `supports_standing`: bool

### `aim` (optional)

- `max_yaw_delta_degrees`: optional float
- `components`: array of UUID strings (prefab ids)

### `mobility` (optional)

- `mode`: `"ground"` | `"air"`
- `max_speed`: float

### `anchors`

Each anchor:

- `name`: non-empty string
- `transform`: `{ translation, rotation, scale }`

### `transform`

- `translation`: `{x,y,z}` floats
- `rotation`: quaternion `{x,y,z,w}` floats
- `scale`: `{x,y,z}` floats

### Coordinate System

All vectors/transforms use a single right-handed coordinate system:

- `+X` is right
- `+Y` is up
- `+Z` is forward

This applies to anchor transforms and part transforms alike.

### `parts`

Each part:

- `part_id`: optional UUID string (stable identity for part-level edits).
- `render_priority`: optional integer. Hint for resolving near-coplanar overlaps: higher values are biased slightly closer to the camera during depth testing. Keep values small (suggested: -3..3). When omitted, the renderer applies a small automatic bias for `object_ref` parts to reduce z-fighting at component seams.
- `kind`: tagged union (`kind`):
  - `{ "kind": "object_ref", "object_id": <uuid> }` (references another prefab def by id)
  - `{ "kind": "primitive", "primitive": <primitive_visual_def> }`
  - `{ "kind": "model", "scene": <string> }` (engine asset reference)
- `attachment`: optional attachment definition:
  - `parent_anchor`: non-empty string
  - `child_anchor`: non-empty string
- `animations`: array of animation slots (may be empty)
- `transform`: `{ translation, rotation, scale }`

### `primitive_visual_def`

Tagged union (`kind`):

- Mesh-backed:
  - `{ "kind": "mesh", "mesh": <mesh_key>, "material": <material_key> }`
- Procedural primitive:
  - `{ "kind": "primitive", "mesh": <mesh_key>, "params": <primitive_params?>, "color_rgba": {r,g,b,a}, "unlit": <bool> }`

### `mesh_key`

String enum (snake_case):

- `unit_cube`
- `unit_cylinder`
- `unit_cone`
- `unit_sphere`
- `unit_plane`
- `unit_capsule`
- `unit_conical_frustum`
- `unit_torus`
- `unit_triangle`
- `unit_tetrahedron`
- `tree_trunk`
- `tree_cone`

### `material_key`

Tagged union (`kind`):

- `{ "kind": "build_block", "index": <usize> }`
- `{ "kind": "fence_stake" }`
- `{ "kind": "fence_stick" }`
- `{ "kind": "tree_trunk", "variant": <usize> }`
- `{ "kind": "tree_main", "variant": <usize> }`
- `{ "kind": "tree_crown", "variant": <usize> }`

### `primitive_params` (optional)

Tagged union (`kind`):

- Capsule:
  - `{ "kind": "capsule", "radius": <f32>, "half_length": <f32> }`
- Conical frustum:
  - `{ "kind": "conical_frustum", "radius_top": <f32>, "radius_bottom": <f32>, "height": <f32> }`
- Torus:
  - `{ "kind": "torus", "minor_radius": <f32>, "major_radius": <f32> }`

### `animations`

An animation slot:

- `channel`: non-empty string
- `spec`:
  - `driver`: `"always"` | `"move_phase"` | `"move_distance"` | `"attack_time"`
  - `speed_scale`: float
  - `time_offset_units`: float (deterministic phase offset)
  - `clip`: tagged union (`kind`):
    - Loop:
      - `duration_secs`: float
      - `keyframes`: array of `{ time_secs, delta }`, where `delta` is a `transform`
    - Spin:
      - `axis`: `{x,y,z}` floats
      - `radians_per_unit`: float

### `projectile` (optional)

- `obstacle_rule`: `"bullets_blockers"` | `"laser_blockers"`
- `speed`: float
- `ttl_secs`: float
- `damage`: int
- `spawn_energy_impact`: bool

### `attack` (optional)

- `kind`: `"melee"` | `"ranged_projectile"`
- `cooldown_secs`: float
- `damage`: int
- `anim_window_secs`: float
- `melee`: optional `{ range, radius, arc_degrees }`
- `ranged`: optional:
  - `projectile_prefab`: UUID string (prefab id)
  - `muzzle`: `{ object_id: <uuid>, anchor: <string> }`

## Validation Rules (Non-Exhaustive)

- Any UUID field **must** be a valid UUID string.
- `format_version` **must** equal `1` for this spec.
- `anchors[].name`, `parts[].animations[].channel`, `attachment.*_anchor`, and `AnchorRef.anchor` **must be non-empty**.
- Prefab ids referenced by `object_ref`, `aim.components`, ranged `projectile_prefab`, etc. should exist in the realm’s prefab library at load time.

## Canonical JSON

For stable diffs, writers **should**:

- sort JSON object keys recursively,
- pretty-print with a trailing newline.
