# `info_kv_get_v1` (Gen3D tool)

`info_kv_get_v1` is a **read-only** Info Store KV inspection tool for fetching a single KV value
by key + selector (latest / specific revision / as-of).

This tool is designed to be:

- **deterministic** (selector resolves to an immutable KV revision),
- **bounded** (selected value is capped by `max_bytes`),
- **actionable** (oversize reads return a deterministic `shape_preview` + mechanical `fixits`).

## Args (v1)

- `namespace: string`
- `key: string`
- `selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number }`
  - default: `{ "kind": "latest" }`
- `json_pointer?: string`
  - If provided, it must resolve within the selected KV value.
  - If omitted, the full KV value is selected.
- `max_bytes?: number`
  - default `65536` (64 KiB)
  - clamped to `[1024, 524288]` (1 KiB .. 512 KiB)

## Result (v1)

On success:

- `ok: true`
- `record`: KV record metadata
  - `kv_rev`, `written_at_ms`, `attempt`, `pass`, `assembly_rev`, `workspace_id`
  - `key: { namespace, key }`
  - `summary`, `bytes`, optional `written_by: { tool_id, call_id }`
- `value`: the selected JSON value (bounded by `max_bytes`)
- `truncated: false` (reserved for future; this tool errors on oversize)
- `json_pointer?: string` (echoed if provided)
- `cached: bool` / `no_new_information: bool`
  - Repeating an identical call within the same pass (same resolved `record.kv_rev` + `json_pointer` + caps) returns the same payload with:
    - `cached=true`
    - `no_new_information=true`

## Oversize behavior (actionable error)

If the selected value exceeds `max_bytes`, the tool returns `ok:false` with:

- a concise `error` string, and
- a structured diagnostic payload in `result` (still bounded), including:
  - `kind: "kv_value_too_large"`
  - `record` (metadata; includes `kv_rev`)
  - `max_bytes`, `selected_bytes_capped`
  - `shape_preview` (deterministic; no semantic heuristics):
    - scalar: the scalar itself
    - string: `{ "kind":"string", "len_bytes": <n> }`
    - array: `{ "kind":"array", "len": <n> }`
    - object: `{ "kind":"object", "keys_sample": [sorted first 16 keys], "keys_total": <n> }`
  - `fixits[]`: purely mechanical next calls (ex: retry with `json_pointer`, or use `info_kv_get_paged_v1` for arrays)

This avoids follow-up model work just to discover navigable pointers/keys.

