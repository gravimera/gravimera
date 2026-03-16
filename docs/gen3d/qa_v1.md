# `qa_v1` (Gen3D tool)

`qa_v1` is a composed QA tool that runs **`validate_v1` + `smoke_check_v1`** and returns a combined, bounded summary.

It also implements a **tool-level no-progress gate**: repeated calls on an unchanged assembled state return a cached result with an explicit “mutate before retrying” hint. This prevents “repeating QA” inspection loops.

KV records written under the current workspace (namespace `gen3d`):

- `ws.<workspace_id>.validate`
- `ws.<workspace_id>.smoke`
- `ws.<workspace_id>.qa` (the combined result; returned as `info_kv`)

## Args (v1)

- `force?: bool` (default `false`)
  - When `false`, `qa_v1` may return a cached result if called repeatedly on unchanged state.
  - When `true`, bypass caching and re-run validate/smoke.

## Result (v1)

Always returned as a successful tool call (`Gen3dToolResultJsonV1.ok=true`) unless args/runtime fail.

Key fields:

- `ok: bool` (combined verdict: `validate.ok && smoke.ok`)
- `validate: object` (full `validate_v1` results)
- `smoke: object` (full `smoke_check_v1` results; includes `capability_gaps`)
- `errors: object[]` (merged issues with `source` = `validate|smoke|motion_validation`)
- `warnings: object[]` (merged issues with `source`)
- `info_kv: { namespace, key, selector:{kind:"kv_rev", kv_rev:number} }` (pointer to the stored combined QA record)

No-progress/cache fields:

- `cached: bool`
- `no_new_information: bool`
- `no_new_information_message?: string` (present when cached)
- `basis: { workspace_id, state_hash, plan_hash, assembly_rev }`
  - `state_hash` is computed from the assembled state (including motion value digests) and is used to detect repetition.

Capability gaps (bounded, deterministic):

- `capability_gaps: CapabilityGap[]` (max 16)
  - Each entry has:
    - `kind: string` (e.g. `missing_motion_channel`, `missing_root_field`, `motion_validation_error`)
    - `severity: "error"|"warn"`
    - `message: string` (1 sentence)
    - `evidence: object` (small structured fields)
    - `fixits?: { tool_id: string, args: object, note?: string }[]` (max 3 per gap; never applied silently)
    - `blocked?: bool` + `blocked_reason?: string` when no deterministic fix exists with current tools

## Caching / no-progress semantics

`qa_v1` caches by `(workspace_id, state_hash)`:

- If you call `qa_v1` again with the same basis and `force!=true`, the tool returns the previously computed result with:
  - `cached=true`
  - `no_new_information=true`
  - the original `info_kv` pointer preserved
  - an actionable hint to mutate before retrying
- On the cached path, `qa_v1` **does not write new KV records**.

Use `{"force": true}` only as an explicit escape hatch.

## Example

Run QA normally:

```json
{}
```

Force a re-run (bypass caching):

```json
{ "force": true }
```
