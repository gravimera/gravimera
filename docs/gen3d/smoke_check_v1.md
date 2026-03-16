# `smoke_check_v1` (Gen3D tool)

`smoke_check_v1` is a behavior/motion “sanity check” tool for the current Gen3D draft and planned components.

It returns:

- a bounded `issues[]` list (errors/warnings),
- a bounded `motion_validation` report (including per-joint/channel problems),
- a bounded `capability_gaps[]` list with optional deterministic `fixits[]` payloads (never applied silently),
- and an `info_kv` pointer to the stored result under `ws.<workspace_id>.smoke`.

## Args (v1)

`{}` (no args)

## Result (v1)

Key fields (existing + important):

- `ok: bool`
- `issues: object[]`
- `motion_validation: { ok: bool, issues: object[] }`
- `rig_summary: object`
- `attack_required_by_prompt: bool`
- `mobility_present: bool`
- `attack_present: bool`

Capability gaps (bounded, deterministic):

- `capability_gaps: CapabilityGap[]` (max 16)
  - Each entry has:
    - `kind: string` (e.g. `missing_motion_channel`, `missing_root_field`, `motion_validation_error`)
    - `severity: "error"|"warn"`
    - `message: string` (1 sentence)
    - `evidence: object`
    - `fixits?: { tool_id: string, args: object, note?: string }[]` (max 3 per gap; never applied silently)
    - `blocked?: bool` + `blocked_reason?: string` when no deterministic fix exists with current tools

KV pointer:

- `info_kv: { namespace, key, selector:{kind:"kv_rev", kv_rev:number} }`

## Notes on `attack_required_by_prompt`

`attack_required_by_prompt` is derived from prompt keywords and is considered **legacy heuristic** logic. Prefer treating it as a hint for QA messaging rather than a source-of-truth requirement system.

## Example

```json
{}
```
