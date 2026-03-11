# `get_plan_template_v1` (Gen3D tool)

`get_plan_template_v1` is a **read-only** Gen3D tool that writes an engine-generated **plan JSON template** into the run cache and returns an `artifact_ref`.

The intent is to give the agent a deterministic “starting JSON” for preserve-mode replans so the model doesn’t forget required existing component names, accidentally change the root, or invent parent names.

## When to use it

Use it in preserve-existing-components edit sessions when:

- `llm_generate_plan_v1` fails due to missing existing component names/root mismatch, or
- you want a safe “copy and edit” starting point for a preserve-mode patch.

## What it writes

The artifact is a **plan JSON (version 8)** derived from current engine state:

- includes **all current component names** and the **current root component**,
- includes each component’s current `attach_to` interface,
- includes the current anchor frames (pos/forward/up) for existing anchors,
- includes preserved plan-level fields when available (e.g. `collider`, `rig.move_cycle_m`),
- includes attack/aim details when they can be deterministically reconstructed from the current draft.

The template is intended to be **copied and minimally edited** by `llm_generate_plan_v1` (see below).

## Tool output (v1)

The tool returns:

- `artifact_ref`: relative path under the run directory (use as an input to other tools),
- `bytes`: size of the written JSON,
- `components_total`: number of components in the template.

## Using it with `llm_generate_plan_v1`

`llm_generate_plan_v1` accepts:

- `plan_template_artifact_ref` (string)

Recommended flow in preserve mode:

1) Call `get_plan_template_v1`.
2) Call `llm_generate_plan_v1` with:

   - `constraints.preserve_existing_components=true`
   - your chosen `constraints.preserve_edit_policy`
   - `plan_template_artifact_ref` set to the returned `artifact_ref`

The engine injects the template JSON into the plan prompt as “copy+edit” context. The engine does **not** apply any edits silently; the plan is still produced by the model and validated normally.

