# `basis_from_up_forward_v1` (Gen3D tool)

`basis_from_up_forward_v1` is a **read-only** Gen3D math helper that returns a valid orthonormal basis `{forward, up, right}` from:

- an `up` axis (required), and
- an optional `forward_hint` (to control roll around `up`).

This is useful when authoring `forward`/`up` fields for part rotations (or anchors) without doing cross-product math by hand.

## When to use it

Use it when you need a valid basis for:

- a primitive part’s rotation (`parts[].forward` + `parts[].up`), or
- an anchor’s rotation (`anchors[].forward` + `anchors[].up`),

and you want to avoid degenerate / near-parallel vectors that the engine will reject.

Example: aiming a cylinder/cone “along direction D”.

- In Gravimera, `cylinder` and `cone` meshes have their length/height axis along the part’s local **+Y**.
- To aim them along `D`, set the part’s **`up = D`**, and choose any perpendicular `forward`.
- This tool can compute a clean `{forward, up}` pair for you.

## Tool args (v1)

Required:

- `up`: `[x,y,z]` (finite, non-zero)

Optional:

- `version`: `1` (defaults to `1` if omitted)
- `forward_hint`: `[x,y,z]` (finite, non-zero). If omitted (or parallel to `up`), the tool chooses a deterministic fallback axis.

## Tool output (v1)

Key fields:

- `forward`, `up`, `right`: unit-length, mutually-orthogonal vectors forming a right-handed basis.
- `forward_source`: `"projected_forward_hint"` or `"fallback"`.
- `fallback_axis`: `"x" | "y" | "z" | null` (only set when fallback is used).
- `notes`: bounded list of short explanations when fallbacks are used.

## Examples

Aim a cylinder/cone along world +Z (so part local +Y points +Z):

```json
{ "up": [0,0,1], "forward_hint": [1,0,0] }
```

Then use the returned `forward`/`up` directly in the part JSON.
