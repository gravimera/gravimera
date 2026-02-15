# Scene Layer Kinds (v1) — Deterministic Procedural Primitives

_(Spec document; see `docs/gamedesign/22_scene_creation.md` for product goals.)_

This spec defines additional **procedural layer kinds** usable in scene sources under
`scenes/<scene_id>/src/layers/<layer_id>.json`.

These layer kinds are intentionally **generic**. They do not encode domain rules (no “town
heuristics”). They are purely parameterized placement primitives that can be composed by humans or
AI agents to author any scene.

## Shared Rules (Normative)

1) **Deterministic compilation**  
   Given identical sources, compilation must produce identical outputs (instance ids and transforms).

2) **Stable instance ids**  
   All layer-owned instances derive `instance_id` using the v1 rule defined in:
   `docs/gamedesign/30_scene_sources_and_build_artifacts.md` (UUID v5 over
   `scene_id + layer_id + local_id`).

3) **Ownership and regeneration**  
   Regeneration follows “layer owns outputs unless pinned” (see `docs/gamedesign/22_scene_creation.md`).

4) **Unknown fields**  
   Tools should preserve unknown fields when round-tripping where possible.

## Layer Kind: `explicit_instances` (Reference)

The minimal layer kind is defined in:

- `docs/gamedesign/30_scene_sources_and_build_artifacts.md` (`kind = "explicit_instances"`)

This document defines additional kinds below.

## Layer Kind: `grid_instances` (v1)

A `grid_instances` layer places a single prefab on a 2D grid on the XZ plane.

### Schema (v1, minimal / normative)

Required fields:

- `format_version`: integer (`1`)
- `layer_id`: string
- `kind`: `"grid_instances"`
- `prefab_id`: UUID string
- `origin`: vec3 `{ "x": <f32>, "y": <f32>, "z": <f32> }`
- `count`: object `{ "x": <u32>, "z": <u32> }` (number of cells)
- `step`: object `{ "x": <f32>, "z": <f32> }` (cell spacing)

Optional fields:

- `rotation`: quat `{ "x": <f32>, "y": <f32>, "z": <f32>, "w": <f32> }` (default identity)
- `scale`: vec3 `{ "x": <f32>, "y": <f32>, "z": <f32> }` (default `{1,1,1}`)
- `tint_rgba`: color `{ "r": <f32>, "g": <f32>, "b": <f32>, "a": <f32> }`

### Semantics (v1, normative)

For each integer grid coordinate:

- `ix` in `[0, count.x)`
- `iz` in `[0, count.z)`

Generate one instance with:

- `local_id = "x{ix}_z{iz}"`
- `translation = origin + (ix * step.x, 0, iz * step.z)`
- `rotation`, `scale`, and optional `tint_rgba` from the layer fields
- `instance_id` derived from `(scene_id, layer_id, local_id)` using the shared v1 id rule.

Placement notes (non-normative):

- `origin.y` is the center height of the placed instances. To rest objects on the ground plane (y=0),
  choose `origin.y = (prefab_size.y * abs(scale.y)) / 2`.

Constraints:

- `count.x` and `count.z` may be zero (generates zero instances).
- `step.x` and `step.z` must be finite and non-zero.
- All generated numeric values must be finite.

## Layer Kind: `polyline_instances` (v1)

A `polyline_instances` layer places a single prefab along a polyline path at a fixed spacing.

### Schema (v1, minimal / normative)

Required fields:

- `format_version`: integer (`1`)
- `layer_id`: string
- `kind`: `"polyline_instances"`
- `prefab_id`: UUID string
- `points`: array of vec3; must contain at least 2 points
- `spacing`: `<f32>`; must be finite and `> 0`

Optional fields:

- `start_offset`: `<f32>` distance along the polyline to place the first instance (default `0`)
- `rotation`: quat (default identity)
- `scale`: vec3 (default `{1,1,1}`)
- `tint_rgba`: color

### Semantics (v1, normative)

Let `segments` be the consecutive point pairs `points[i] → points[i+1]`.

- A segment with zero length is invalid (reject the layer).
- `total_length` is the sum of segment lengths.

If `start_offset > total_length`, generate zero instances.

Otherwise compute:

- `count = floor((total_length - start_offset) / spacing) + 1`

For each `k` in `[0, count)`:

- `d = start_offset + k * spacing`
- Find the segment containing distance `d` (using cumulative segment lengths).
- Interpolate linearly on that segment to compute `translation`.
- Set `local_id = "i{k}"`
- Use the layer’s `rotation`, `scale`, and optional `tint_rgba`.
- Derive `instance_id` from `(scene_id, layer_id, local_id)` using the shared v1 id rule.

Placement notes (non-normative):

- The `points[].y` values are the center heights of the placed instances. To rest objects on the ground plane (y=0),
  author points with `y = (prefab_size.y * abs(scale.y)) / 2` (or set `scale` appropriately).

Notes:

- This layer does not impose orientation alignment to the path tangent in v1; authors can use
  multiple layers or explicit instances if they need per-instance rotations.

