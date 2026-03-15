# `inspect_plan_v1` (Gen3D tool)

`inspect_plan_v1` is a **read-only** Gen3D tool that returns **deterministic, computed hints** when planning fails — especially in preserve-existing-components edit sessions.

It is designed to reduce thrash where the agent repeatedly calls `get_scene_graph_summary_v1` and `llm_generate_plan_v1` without converging.

## When to use it

Call `inspect_plan_v1` immediately after `llm_generate_plan_v1` returns an error that looks like a **semantic** plan failure, for example:

- unknown/missing `attach_to.parent` component names,
- referenced-but-undefined component names (reported as `missing_component_reference`), including references coming from `reuse_groups`, `aim.components`, and `attack.muzzle.component`,
- missing `attach_to.parent_anchor` / `attach_to.child_anchor` anchors (anchors referenced by attachments but not present in the plan’s `anchors[]` for that component),
- preserve-mode requirements (missing existing component names, root changed),
- root ambiguity (multiple components with no `attach_to`, or invalid `root_component`).

In preserve mode, prefer `inspect_plan_v1` over `get_scene_graph_summary_v1` because it returns the *exact constraints* the engine is enforcing (names/root/policy), not a full scene dump.

## What it returns (v1)

The tool returns a JSON object with these top-level fields:

- `has_pending_plan`: whether the engine captured a recently-rejected `llm_generate_plan_v1` output for inspection.
- `constraints`: current preserve-mode constraints (existing component names + root, and whether preserve mode is enabled).
- `pending`: details about the last rejected plan attempt (call id, error string, preserve policy inputs) if available.
- `plan_summary`: a bounded summary of the rejected plan (component names, root).
- `analysis`:
  - `ok`: `true` if no semantic issues were detected by this inspector.
  - `errors[]`: structured error kinds (e.g. `unknown_parent`, `missing_component_reference`, `preserve_missing_existing_component_names`, `preserve_root_changed`).
  - `fixits[]`: optional **suggested PlanOps** (bounded). These are suggestions only and are produced only when the repair is logically forced by the plan’s explicit references (no heuristics).
  - `hints[]`: short, actionable next-step guidance.

Notes:

- `inspect_plan_v1` does **not** mutate the draft or apply any repair.
- It does **not** “auto-map” names. For `unknown_parent`, it may include a small `suggestions[]` list derived from existing component names (token/substring match), but you must still edit/replan explicitly.
- If `analysis.fixits[]` is present, you can apply one of the suggested ops via `apply_plan_ops_v1` (explicit mutation + revalidation).

## Recommended follow-ups

- Preserve-mode replanning with an existing plan requires `plan_template_kv`: call `get_plan_template_v1`, then re-run `llm_generate_plan_v1` with `plan_template_kv`.
- If the error is “unknown parent”: fix the plan to use an existing component name exactly (case-sensitive); consider using the template as a starting point.
- If the error is local and you can express an explicit patch (rename a parent, add a missing component definition, add missing anchors): consider `apply_plan_ops_v1` instead of a full replan.
- If the error is a preserve edit-policy violation: either broaden `constraints.preserve_edit_policy`, use `apply_draft_ops_v1`, or disable preserve mode for a full rebuild.
