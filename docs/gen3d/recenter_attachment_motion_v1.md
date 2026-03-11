# Gen3D: `recenter_attachment_motion_v1` (engine semantics)

This document describes the *engine* interpretation of the deterministic tool
`recenter_attachment_motion_v1`.

Source of truth:
- `src/gen3d/ai/motion_recenter.rs` (implementation)
- `src/gen3d/agent/tools.rs` (tool registry contract)
- `src/gen3d/ai/motion_validation.rs` (the validator it targets)

## Purpose

Fix repeated motion-validation failures of kind `joint_rest_bias_large` **without changing the
actual motion**.

This typically happens when an animation channel keeps a joint far from neutral for the full
cycle (e.g. a hinge joint always at ~80°), which is often an “absolute-frame” authored delta rather
than a centered delta around the neutral pose.

## Core guarantee (no weird motion)

For each selected attachment edge (child component), the tool computes a constant rotation bias
`B` and performs an exact re-parameterization:

- `offset' = offset * B`
- For every clip sample time `t`: `delta'(t) = B^{-1} * delta(t)`

Therefore:

`offset' * delta'(t) == offset * delta(t)` for all `t`.

So the child’s animated attachment transform in world space is unchanged; only the decomposition
between `attach_to.offset` and per-keyframe `delta` changes.

## Hinge joints + limits

If the attachment joint is `kind=hinge` and it has `limits_degrees=[min,max]`, the tool shifts
those limits by the same bias (to preserve the same physical range under the new neutral):

`limits' = [min - bias_deg, max - bias_deg]`

## Channels + safety

`attach_to.offset` is shared across *all* channels on the same edge. Because of this:

- The tool computes a single bias per edge (from the requested channels, or all channels by
  default).
- It only applies the bias when it can satisfy the requested target (`warn` or `error`) **without
  introducing new `joint_rest_bias_large` issues on other slots**.

If no safe bias exists, it returns `applied=false` and the recommended next step is to re-author
motion with `llm_generate_motion_authoring_v1`.

## Spin clips

`spin` clips are only supported on hinge edges when the clip axis is aligned (or anti-aligned)
with the hinge `axis_join` and `radians_per_unit` is non-zero. Otherwise, the tool refuses to
apply, because the exact left-multiplication `B^{-1} * delta(t)` cannot be represented as a `spin`
with a single fixed axis.

