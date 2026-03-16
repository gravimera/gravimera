# `llm_generate_plan_ops_v1` (Gen3D tool)

`llm_generate_plan_ops_v1` is an **LLM + deterministic mutation** tool for *diff-first* replanning in Gen3D.

Instead of re-emitting a full plan JSON like `llm_generate_plan_v1`, the model outputs a bounded list of explicit **PlanOps** (a patch). The engine then:

1) reconstructs a full-fidelity snapshot of the current accepted plan from engine state,
2) applies the ops deterministically,
3) re-validates preserve-mode policy constraints (when enabled),
4) accepts the patched plan (or captures it as a pending attempt on failure).

Artifacts written under the current Gen3D `pass/` dir:

- `plan_ops_generated.json` (raw generated ops payload)
- `plan_ops_generated_normalized.json` (only when the engine applies deterministic micro-repairs)
- `plan_ops_apply_last.json` (apply summary; accepted/failed + bounded diagnostics)

## When to use it

Use `llm_generate_plan_ops_v1` in seeded edit sessions (existing plan) when the desired change is small and local, for example:

- add a small component (hat, backpack, antenna) and attach it,
- add/adjust one attachment edge within the preserve-mode edit policy,
- upsert a missing anchor used by an attachment.

If you intend a wide redesign or topology overhaul, prefer `llm_generate_plan_v1`.

If you want an explicit deterministic patch you authored yourself (no LLM patch generation), use `apply_plan_ops_v1`.

## Args (v1)

- `prompt?: string` (defaults to the current user prompt)
- `constraints?: { preserve_existing_components?: bool, preserve_edit_policy?: "additive"|"allow_offsets"|"allow_rewire", rewire_components?: string[] }`
  - Note: `llm_generate_plan_ops_v1` is intended for preserve-mode replanning and requires `constraints.preserve_existing_components=true`.
- `plan_template_kv?: { namespace: string, key: string, selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number } }`
  - Required when preserve mode is enabled **and** an existing plan is present (same gate as `llm_generate_plan_v1`).
- `scope_components?: string[]`
  - Optional allow-list for scope enforcement: the engine rejects ops that touch **existing** components outside the scope.
  - Newly-added component names are allowed.
- `max_ops?: number`
  - Default 32, max 64. The engine rejects outputs with more than `max_ops` ops.

## Result (v1)

Key fields are designed to be actionable and bounded:

- `accepted`: whether the patched plan passed semantic validation and was accepted.
- `ops_total`: how many ops were generated.
- `repaired`: whether the engine applied deterministic micro-repairs to the generated JSON before parsing.
- `repair_diff`: bounded list of applied micro-repairs (empty when `repaired=false`).
- `diff_summary`: compact counts + `touched_components` (bounded).
- `new_plan_summary`: bounded summary of the patched plan.
- `new_errors`: `null` when accepted; otherwise bounded diagnostics (includes preserve-policy errors and `inspect_plan_v1`-style structural errors/fixits).

On semantic failure, the engine also captures a `pending_plan_attempt` so you can follow up with `inspect_plan_v1` and/or `apply_plan_ops_v1`.

## Deterministic micro-repair (known aliases)

To reduce expensive schema-repair roundtrips for common low-entropy mismatches, the engine applies a small set of deterministic micro-repairs before parsing the generated JSON.

Currently supported:

- For `{ "kind": "add_component", ... }` ops:
  - If `name` is missing and `component` is a string, the engine maps `component -> name`.
  - If both `name` and `component` exist and differ (after trimming), the tool errors (refuses to guess).

When a micro-repair is applied:

- the tool result returns `repaired=true` and a bounded `repair_diff[]`, and
- `plan_ops_generated_normalized.json` is written under the current `pass/` dir for auditability.

## Example

Diff-first preserve-mode edit (add a hat), with a scoped template for smaller prompts:

1) Get a preserve-mode template (optional `scope_components` keeps full anchors for just the head):

```json
{ "version": 2, "mode": "auto", "scope_components": ["head"] }
```

2) Call `llm_generate_plan_ops_v1`:

```json
{
  "constraints": {
    "preserve_existing_components": true,
    "preserve_edit_policy": "additive"
  },
  "plan_template_kv": {
    "namespace": "gen3d",
    "key": "ws.main.plan_template.preserve_mode.v1",
    "selector": { "kind": "latest" }
  },
  "scope_components": ["head"],
  "max_ops": 16
}
```

If the patch is rejected:

- Call `inspect_plan_v1` to see computed constraints and fixits.
- Or patch explicitly with `apply_plan_ops_v1` (base_plan="pending" to patch the captured pending attempt).
