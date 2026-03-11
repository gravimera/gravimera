# `suggest_motion_repairs_v1`

Read-only Gen3D tool that proposes **deterministic repair patches** for `motion_validation` errors.

This tool does **not** mutate the draft. To apply a repair, the agent must explicitly call a mutation tool (typically `apply_draft_ops_v1`) with one chosen patch.

## Why this exists

Motion validation failures like `hinge_limit_exceeded` are often caused by small, mechanical mismatches (for example: a hinge limit is ±30°, but the authored motion reaches 31°). Re-authoring the entire motion with an LLM can be slow and unstable for such tiny issues.

This tool provides:

- deterministic, generic (non-heuristic) fixes,
- as concrete, reviewable diffs (draft ops),
- with explicit “apply” control (no silent auto-fix).

## Args (v1)

Schema:

```json
{ "version": 1, "max_suggestions": 8, "safety_margin_degrees": 0.2 }
```

- `max_suggestions` (optional): clamps to `[1, 32]`.
- `safety_margin_degrees` (optional): clamps to `[0.0, 5.0]`.
  - The margin is applied so suggested fixes leave a small buffer under the limit (helps avoid “just barely” failing due to numeric tolerances).

## Output (v1)

Top-level keys:

- `rig_summary`: copied from motion validation (cycle length, joints count, etc.).
- `motion_validation`: the current motion validation result (`ok`, `issues[]`).
- `suggestions[]`: ordered list of candidate repairs (bounded by `max_suggestions`).
- `truncated`: whether suggestion generation stopped due to `max_suggestions`.

Each `suggestions[]` item includes:

- `id`: stable-ish identifier string for logging / UI.
- `kind`: repair kind (see below).
- `issue_kind`, `component_name`, `channel`: what the repair targets.
- `message`: short explanation.
- `impact`: numeric metadata (degrees, scale factor).
- `apply_draft_ops_args`: ready-to-use args object for `apply_draft_ops_v1`.

## Implemented repair kinds

### `relax_joint_limits`

When `hinge_limit_exceeded` occurs on a hinge joint with declared limits, this suggests a patch that widens the exceeded bound just enough to include the observed angle (plus `safety_margin_degrees`).

Patch shape (simplified):

```json
{
  "version": 1,
  "atomic": true,
  "if_assembly_rev": 12,
  "ops": [
    { "kind": "set_attachment_joint", "child_component": "tongue", "set_joint": { "...": "..." } }
  ]
}
```

### `scale_animation_slot_rotation`

When `hinge_limit_exceeded` occurs and there is exactly one animation slot for the `(component_name, channel)`, this suggests a patch that scales **delta rotation amplitudes** for that slot down by a factor in `(0, 1]` so the worst observed hinge angle fits within limits (with margin).

Patch shape (simplified):

```json
{
  "version": 1,
  "atomic": true,
  "if_assembly_rev": 12,
  "ops": [
    { "kind": "scale_animation_slot_rotation", "child_component": "tongue", "channel": "move", "scale": 0.982 }
  ]
}
```

Notes:

- This op scales only rotation deltas. It does not change translation or scale keyframes.
- The scale suggestion is offered only when the computed factor is a pure shrink `(0, 1]`.

## How to apply a suggestion (explicit)

1. Call `suggest_motion_repairs_v1`.
2. Pick exactly one entry from `suggestions[]`.
3. Call `apply_draft_ops_v1` with the corresponding `apply_draft_ops_args`.
4. Re-run `qa_v1` (or `smoke_check_v1`) to confirm `motion_validation.ok=true`.

If `apply_draft_ops_v1` rejects due to `if_assembly_rev` mismatch, re-run `get_state_summary_v1` to observe the current assembly rev and either:

- re-run `suggest_motion_repairs_v1` to get a fresh patch, or
- re-issue the same patch with `if_assembly_rev` removed/updated (at the agent’s discretion).

## Limitations

- Current implementation focuses on `hinge_limit_exceeded`. Other motion issues may return zero suggestions.
- The tool does not “choose” a best repair; it returns options so the agent can decide and explicitly apply.

