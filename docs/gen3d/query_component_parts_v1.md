# `query_component_parts_v1` (Gen3D tool)

`query_component_parts_v1` is a **read-only** Gen3D tool that lists the parts of one component in the current draft (bounded), including stable identifiers (`part_id_uuid`) needed for deterministic edits via `apply_draft_ops_v1`.

It also returns bounded, copy/pasteable `recipes` (example `apply_draft_ops_v1` payloads) so the agent can perform common non-regeneration edits (recolor / transform tweaks) without extra “inspection loops”.

## When to use it

Use it when you need to deterministically edit already-generated geometry without regeneration, for example:

- recolor a component’s primitive parts,
- move/scale/rotate a specific primitive part,
- audit which parts are primitives vs object refs vs models.

## Tool args (v1)

Provide **either**:

- `component`: component name, or
- `component_index`: 0-based index into the current planned component list.

Optional args:

- `include_non_primitives`: when `true`, includes `object_ref` and `model` parts in the `parts` list.
- `max_parts`: bound on returned parts (hard-capped by the engine); sets `truncated=true` when exceeded.

## Tool output (v1)

Key fields:

- `component`, `component_index`, `component_id_uuid`: identify the component.
- `assembly_rev`: current assembly revision (use with `apply_draft_ops_v1.if_assembly_rev` for safe edits).
- `parts[]`: bounded list of parts.
  - Each part includes:
    - `part_id_uuid` (nullable): stable id for `apply_draft_ops_v1` primitive edits.
    - `kind`: `primitive` | `object_ref` | `model`.
    - `transform`: `{ pos, rot_quat_xyzw, scale }`.
  - Primitive parts include `primitive.*`:
    - `primitive.mesh`: debug label of the mesh variant.
    - `primitive.mesh_apply`: canonical mesh string intended for `apply_draft_ops_v1` (`cube`, `cylinder`, `capsule`, ...). May be `null` when the primitive is not patch-editable via `apply_draft_ops_v1`.
    - `primitive.params`: either `null` or a tagged params object (e.g. `{ "kind":"capsule", ... }`).
    - `primitive.color_rgba`: current color.
- `recipes`: bounded “next-step payloads”:
  - `recolor_sample`: example `apply_draft_ops_v1` args to recolor a small sample of primitives.
  - `update_transform_sample`: example `apply_draft_ops_v1` args to set a part transform (absolute set).
- `hints`: short reminders about edit semantics and constraints.
- `info_kv`: KV reference written by the dispatcher (keyed by workspace + component) for later inspection via Info Store tools.

Notes:

- `update_primitive_part.set_primitive` requires `mesh` (and `params` for parameterized meshes). Prefer using `primitive.mesh_apply` + `primitive.params` from this tool when building patch ops.
- If `part_id_uuid` is `null`, that part cannot be directly targeted by `apply_draft_ops_v1.update_primitive_part`.

