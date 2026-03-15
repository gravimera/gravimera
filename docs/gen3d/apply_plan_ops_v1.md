# `apply_plan_ops_v1` (Gen3D tool)

`apply_plan_ops_v1` is a **deterministic mutation** tool for patching a plan using explicit **PlanOps**.

By default (`base_plan="pending"`), it repairs a **pending rejected plan attempt** after `llm_generate_plan_v1` produced schema-valid JSON that failed *semantic* validation (unknown parent, missing anchors, preserve-mode diff rejection, etc).

With `base_plan="current"`, it patches the **current accepted plan** (no rejected attempt required). If the patched plan fails semantic validation, the engine captures it as `job.pending_plan_attempt` so you can follow up with `inspect_plan_v1` and/or another `apply_plan_ops_v1` call.

It applies explicit **PlanOps** to the selected base plan, re-runs semantic validation, and:

- If the plan becomes valid: **accepts** it (clears `pending_plan_attempt`, updates `planned_components`, updates `assembly_rev`).
- If still invalid: keeps the patched plan as the pending attempt and returns bounded diagnostics so the agent can patch again (or replan).

This tool does **not** invent edits. It only applies what you asked for.

Artifacts written under the current Gen3D `pass/` dir:

- `plan_ops.jsonl` (append-only audit log)
- `apply_plan_ops_last.json` (last-call summary)

## When to use it

Use `apply_plan_ops_v1` when:

- `llm_generate_plan_v1` failed semantically and the repair is **local and explicit**, such as:
  - add a missing component definition that is referenced elsewhere,
  - fix an incorrect `attach_to.parent` name,
  - add a missing anchor required by an attachment,
  - update `aim.components` or `attack.muzzle`.
- You have a local, explicit plan change you want to make **without** re-running `llm_generate_plan_v1` (use `base_plan="current"`).

For diagnosis first, prefer `inspect_plan_v1`.

## Args (v1)

- `version?: 1`
- `base_plan?: "pending"|"current"` (default `"pending"`)
- `dry_run?: bool` (default `false`)
- `constraints?: { preserve_existing_components?: bool, preserve_edit_policy?: "additive"|"allow_offsets"|"allow_rewire", rewire_components?: string[] }`
- `ops: PlanOp[]` (max 64)

Supported `PlanOp.kind` values:

- `add_component`: add a new component definition.
- `remove_component`: remove a component definition (rejected if still referenced).
- `set_attach_to`: set/replace (or remove via `null`) `attach_to` on a component.
- `set_anchor`: upsert one anchor on a component.
- `set_aim_components`: replace `aim.components`.
- `set_attack_muzzle`: set `attack.muzzle.component` + `attack.muzzle.anchor` (ranged only).
- `set_reuse_groups`: replace `reuse_groups`.

## Result (v1)

Key fields:

- `ok`: `true` iff `accepted=true` and `rejected_ops=[]`.
- `accepted`: whether the patched plan passed semantic validation and was accepted (or would be accepted in `dry_run`).
- `still_pending`: whether a pending rejected plan remains after this call.
- `applied_ops[]` / `rejected_ops[]`: per-op diffs and errors.
- `diff_summary`: compact counts + touched component names (bounded).
- `new_plan_summary`: bounded summary of the patched plan.
- `new_errors`: `null` when accepted; otherwise bounded diagnostics (includes semantic errors plus `inspect_plan_v1`-style structural errors/fixits).

## Example

Patch the current accepted plan (use `dry_run` first):

```json
{
  "version": 1,
  "base_plan": "current",
  "dry_run": true,
  "ops": [
    { "kind": "add_component", "name": "hat", "size": [0.3, 0.2, 0.3] },
    {
      "kind": "set_attach_to",
      "component": "hat",
      "set_attach_to": {
        "parent": "head",
        "parent_anchor": "origin",
        "child_anchor": "origin",
        "offset": { "pos": [0.0, 0.15, 0.0] }
      }
    }
  ]
}
```

After `inspect_plan_v1` reports a missing referenced component `arm_lower_r`, add it and attach it (use `dry_run` first):

```json
{
  "version": 1,
  "dry_run": true,
  "ops": [
    { "kind": "add_component", "name": "arm_lower_r", "size": [0.3, 0.2, 0.2] },
    {
      "kind": "set_attach_to",
      "component": "arm_lower_r",
      "set_attach_to": {
        "parent": "torso",
        "parent_anchor": "shoulder_r",
        "child_anchor": "mount",
        "offset": { "pos": [0.0, 0.0, 0.0] }
      }
    }
  ]
}
```
