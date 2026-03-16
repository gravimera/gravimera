# `info_kv_get_paged_v1` (Gen3D tool)

`info_kv_get_paged_v1` is a **read-only** Info Store KV inspection tool for paging through a JSON **array** stored in a KV record.

Use it when `info_kv_get_v1` is not enough because the selected value is a large array and JSON Pointer cannot express array slices.

This tool is designed to be:

- **deterministic** (stateless paging with opaque cursors),
- **bounded** (per-page and per-item caps),
- **actionable** (returns stable indices + exact `next_cursor` tokens).

## When to use it

Typical use cases:

- Browse `ws.<workspace_id>.qa` arrays like `/errors` or `/warnings`.
- Browse large lists in other KV values (events summaries, component lists, etc) without dumping full blobs into the agent prompt.

## Args (v1)

- `namespace: string`
- `key: string`
- `selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number }`
- `json_pointer?: string`
  - If provided, it must resolve to a JSON **array**.
  - If omitted, the tool expects the full KV value to be a JSON **array**.
- `page?: { limit?: number, cursor?: string }`
  - `page.limit`: default `50`, max `200`
  - `page.cursor`: opaque token from `next_cursor` (treat as an exact string; do not edit)
- `max_item_bytes?: number`
  - default `4096`
  - clamped to `[256, 65536]`

## Result (v1)

- `ok: true`
- `record`: same shape as `info_kv_get_v1` (`kv_rev`, `written_at_ms`, `attempt`, `pass`, `assembly_rev`, `workspace_id`, `key:{namespace,key}`, `summary`, `bytes`, optional `written_by`)
- `array_len: number` (total length of the selected array)
- `items: [{ index: number, bytes: number, truncated: bool, value_preview: any }]`
  - `index`: absolute index in the selected array (stable across paging)
  - `bytes`: serialized JSON size, **capped** at `max_item_bytes + 1`
  - `truncated`: `true` when the item exceeds `max_item_bytes`
  - `value_preview`:
    - full item when `truncated=false`
    - deterministic “shape preview” when `truncated=true` (see below)
- `truncated: bool` (`true` when more items exist after this page)
- `next_cursor?: string` (exact token; never truncate)
- `json_pointer?: string` (echoed if provided)

## Paging semantics (regression-critical)

Paging is bound to a **single frozen KV record**:

- Even when `selector.kind="latest"`, the tool selects a concrete record first and binds the cursor to that record’s `kv_rev`.
- The cursor deterministically rejects mismatches by encoding a params signature that includes:
  - `namespace`, `key`, selected `kv_rev`, `json_pointer` (or `""`), `max_item_bytes`, and the tool id/kind.

If you reuse a cursor with different params (including a different `kv_rev`), the tool returns an error like “Cursor does not match this request”.

Practical tip: when paging, treat `(record.kv_rev, next_cursor)` as a pair. For page 2+, pin `selector.kind="kv_rev"` to the `record.kv_rev` you received; don’t keep calling with `selector.kind="latest"` if a newer KV revision might appear between pages.

## Deterministic “shape preview” (no heuristics)

When an item is too large to return inline (`truncated=true`), `value_preview` is:

- `null`/`bool`/`number`: the scalar itself
- `string`: `{ "kind":"string", "len_bytes": <n> }`
- `array`: `{ "kind":"array", "len": <n> }`
- `object`: `{ "kind":"object", "keys_sample": [sorted first 16 keys], "keys_total": <n> }`

This is intentionally not a semantic summary.

## Example

Fetch the first page of QA errors:

```json
{
  "namespace": "gen3d",
  "key": "ws.main.qa",
  "selector": { "kind": "latest" },
  "json_pointer": "/errors",
  "page": { "limit": 50 },
  "max_item_bytes": 4096
}
```

Fetch the next page using `next_cursor` from the previous result, **pinning** the `kv_rev` returned in `record.kv_rev`:

```json
{
  "namespace": "gen3d",
  "key": "ws.main.qa",
  "selector": { "kind": "kv_rev", "kv_rev": 123 },
  "json_pointer": "/errors",
  "page": { "limit": 50, "cursor": "<next_cursor>" },
  "max_item_bytes": 4096
}
```
