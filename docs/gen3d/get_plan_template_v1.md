# `get_plan_template_v1` (Gen3D tool)

`get_plan_template_v1` is a **read-only** Gen3D tool that writes an engine-generated **plan JSON template** into the **Info Store (KV)** and returns a `plan_template_kv` reference.

The intent is to give the agent a deterministic “starting JSON” for preserve-mode replans so the model doesn’t forget required existing component names, accidentally change the root, or invent parent names.

## When to use it

Use it in preserve-existing-components edit sessions when:

- you are about to do a preserve-mode replan (the engine requires `plan_template_kv` when an existing plan is present), or
- `llm_generate_plan_v1` fails due to missing existing component names/root mismatch and you want a safe “copy and edit” starting point.

## What it writes

The artifact is a **plan JSON (version 8)** derived from current engine state:

- includes **all current component names** and the **current root component**,
- includes each component’s current `attach_to` interface,
- includes the current anchor frames (pos/forward/up) for existing anchors,
- includes preserved plan-level fields when available (e.g. `collider`, `rig.move_cycle_m`),
- includes attack/aim details when they can be deterministically reconstructed from the current draft.

The template is intended to be **copied and minimally edited** by `llm_generate_plan_v1` (see below).

## Tool output (v2)

The tool returns:

- `plan_template_kv`: KV reference (namespace + key + selector) that can be inspected via `info_kv_get_v1` and passed into `llm_generate_plan_v1`,
- `mode`: `"auto"|"full"|"lean"` (what was requested),
- `max_bytes`: the enforced byte budget (clamped),
- `bytes`: size of the stored JSON value written to the Info Store (compact JSON bytes),
- `bytes_full`: size before any trimming,
- `truncated`: whether the tool produced a trimmed/lean template to fit `max_bytes`,
- `omitted_fields`: which fields were stripped (bounded; example values: `"assembly_notes"`, `"components[].modeling_notes"`, `"components[].contacts"`),
- `components_total`: number of components in the template.

## Using it with `llm_generate_plan_v1`

`llm_generate_plan_v1` accepts:

- `plan_template_kv` (KV reference)

Recommended flow in preserve mode:

1) Call `get_plan_template_v1`.
2) Call `llm_generate_plan_v1` with:

   - `constraints.preserve_existing_components=true`
   - your chosen `constraints.preserve_edit_policy`
   - `plan_template_kv` set to the returned `plan_template_kv`

The engine injects the template JSON into the plan prompt as “copy+edit” context. The engine does **not** apply any edits silently; the plan is still produced by the model and validated normally.

## Args (v2)

- `mode`:
  - `"auto"` (default): return a full template when it fits; otherwise return a lean template under budget.
  - `"full"`: refuse if the full template exceeds `max_bytes`.
  - `"lean"`: prefer a smaller template (may omit text-heavy fields even if a full template would fit).
- `max_bytes`: optional override (clamped to the engine’s maximum accepted template size).
