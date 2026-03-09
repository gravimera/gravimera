# Gen3D: `review_delta_v1` (engine semantics)

This document describes the *engine* interpretation of the structured output produced by
`llm_review_delta_v1`.

Source of truth:
- `src/gen3d/ai/schema.rs` (JSON types)
- `src/gen3d/ai/structured_outputs.rs` (LLM JSON schema)
- `src/gen3d/ai/convert.rs` (application logic)
- `src/gen3d/ai/prompts.rs` (prompt/instructions)

## Coordinate frames (quick refresher)

- **World axes**: +X right, +Y up, +Z forward.
- **Join frame**: the coordinate frame at `attach_to.parent_anchor`.
  - `attach_to.offset.pos` is expressed in this join frame.
  - `attach_to.offset.rotation` is also applied in this join frame.
- **Anchor frames**:
  - `parent_anchor_frame` is the parent anchor frame in **parent-local** coordinates.
  - `child_anchor_frame` is the child anchor frame in **child-local** coordinates.

The compact scene graph summary printed for review-delta includes:
- `offset.rot_quat_xyzw(join_frame)`
- `parent_anchor_frame.forward/up(local)`
- `child_anchor_frame.forward/up(local)`

## Action: `tweak_component_resolved_rot_world`

Use this action when the intent is to set a component’s **resolved WORLD rotation** (for example:
“make the shin upright in world”), without having the model manipulate join-frame rotations
directly.

### JSON shape

```json
{
  "kind": "tweak_component_resolved_rot_world",
  "component_id": "<uuid>",
  "rot": { "forward": [0,0,1], "up": [0,1,0] },
  "reason": "..."
}
```

`rot` may also be a quaternion:

```json
{ "rot": { "quat_xyzw": [0,0,0,1] } }
```

### Semantics (deterministic; no heuristics)

This sets the component’s resolved world rotation to the requested `rot` by *solving* the
attachment offset rotation using the known parent/child/anchor frames.

Let:
- `R_parent_world` = resolved world rotation of the parent component
- `R_parent_anchor_local` = parent anchor rotation in parent-local space
- `R_child_anchor_local` = child anchor rotation in child-local space
- `R_join_world = R_parent_world * R_parent_anchor_local`
- `R_target_world` = requested world rotation

Then the engine sets:

`R_offset = inverse(R_join_world) * R_target_world * R_child_anchor_local`

This writes `attach_to.offset.rotation = R_offset` (translation/scale are unchanged).

