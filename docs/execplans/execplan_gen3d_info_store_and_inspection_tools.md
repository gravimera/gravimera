# Gen3D: Replace file-based run artifacts with an AI-friendly Info Store (KV + Events + Blobs)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D currently exposes run inspection via file-oriented tools (`list_run_artifacts_v1`, `read_artifact_v1`, `search_artifacts_v1`). These tools leak implementation details (paths, pass folder layout, lexicographic ordering) into the agent prompt. In real runs the agent frequently “inspects” instead of acting, because it cannot reliably fetch “the latest thing” when listing truncates and substring search only searches file contents (not filenames). This causes no-progress and time-budget burns.

After this change, the agent can fetch all run info it needs through a small set of **run-scoped, bounded, paged** “Info Store” tools:

1) **KV**: structured JSON values under stable keys (with history and “as-of” selectors).
2) **Events**: an append-only, queryable event stream for logs + tool calls/results (with paging and search).
3) **Blobs**: opaque identifiers for binary outputs (images, motion sheets), so no tool contract needs filesystem paths.

User-visible outcome: Gen3D edit runs stop spending steps “locating artifacts” and instead make forward progress. When inspection is needed, it is explicit, deterministic, and returns the precise next-step payload (latest scene graph summary, QA output, recent tool errors, last render images) without filesystem navigation.

This system is generic (no object-specific heuristics) and follows the tool contract rules in `docs/agent_skills/tool_authoring_rules.md`: bounded by design, versioned tool ids, actionable results and errors, and no silent mutation.

## Progress

- [x] (2026-03-13 10:39Z) Write this ExecPlan and commit it.
- [x] (2026-03-13 11:05Z) Review and refine the Info Store spec (bounds/defaults, blob label filters, and migration off file-based artifact refs).
- [x] (2026-03-13) Implement `Gen3dInfoStore` core types (KV, events, blobs) and persistence in the run cache directory.
- [x] (2026-03-13) Add Info Store read-only tools (`info_kv_*`, `info_events_*`, `info_blobs_*`) to the tool registry and dispatcher.
- [x] (2026-03-13) Migrate existing tools to write to the Info Store (and return KV/blob refs instead of paths where applicable).
- [x] (2026-03-13) Remove file-based artifact tools from the agent tool list (`list_run_artifacts_v1`, `read_artifact_v1`, `search_artifacts_v1`) and update the agent prompt/docs accordingly.
- [x] (2026-03-13) Add unit tests + a small regression harness under `test/` proving paging/sorting and “latest” retrieval work without truncation bugs.
- [x] (2026-03-13) Run tests and the required rendered smoke test.
- [x] (2026-03-13) Commit the implementation with a clear message.
- [x] (2026-03-14) Follow-ups: harden blob path resolution (reject absolute/traversal) and include KV provenance (`written_by`) in Info KV tool outputs.
- [x] (2026-03-14) Follow-ups: align `llm_review_delta_v1` prompt + tool contract (avoid empty-args mismatch); remove unsupported `include_original_images` arg.

## Surprises & Discoveries

- Observation: `list_run_artifacts_v1` returns entries in lexicographic filename order and truncates at `max_items`, so it is not a reliable way to find “latest pass output” once a run has many files.
  Evidence: `src/gen3d/ai/artifacts.rs` `read_dir_sorted()` sorts by filename and `list_files_recursive_sorted()` stops at `max_items`.

- Observation: `search_artifacts_v1` searches file contents line-by-line (`line.contains(query)`), not artifact paths, so searching for a filename like `scene_graph_summary.json` mostly matches tool/help text rather than identifying the latest summary artifact.
  Evidence: `src/gen3d/ai/artifacts.rs` `search_artifacts_v1()` loops over `head_text.lines()` and checks `line.contains(query)`.

- Observation: A real run can burn most of its 30-minute budget repeating “inspect/list/search” steps before the agent applies a deterministic fix, then later hits the time-budget stop during a long review call.
  Evidence: Run cache `~/.gravimera/cache/gen3d/99b577ae-e3a5-480e-ab2d-97b014a4ca5d` shows repeated inspection passes and stops with `Time budget exhausted` in `attempt_0/pass_51/gen3d_run.log`.

- Observation: Even when `list_run_artifacts_v1` succeeds, prompt summarization often collapses the result down to `items=<N> truncated=<bool>` without including any `artifact_ref`s, so the agent cannot take the next step (read/search a specific file) unless it already knows the exact ref.
  Evidence: In the run above, pass_43 user text shows `- list_run_artifacts_v1 (call_1): ok items=200 truncated=true` with no listed refs.

## Decision Log

- Decision: Replace file-path based inspection tools with a run-scoped Info Store (KV + events + blobs), and remove filesystem paths from tool contracts (especially image paths).
  Rationale: The agent’s job is to reason about the model and apply deterministic edits, not to navigate the run cache filesystem. Stable keys and opaque blob ids are more robust than filenames and avoid truncation/order pitfalls.
  Date/Author: 2026-03-13 / GPT-5.2

- Decision: Prefer typed, domain-level navigation (components/attachments/parts) over generic “JSON tree browsing”.
  Rationale: The Gen3D model is not purely a tree; it is a graph (workspaces, linked components, attachment edges). Navigation should be explicit and bounded, with stable ids, rather than path-like JSON pointers as the primary mechanism.
  Date/Author: 2026-03-13 / GPT-5.2

- Decision: Enforce conservative default bounds and add multi-label blob filtering (`labels_any`/`labels_all`) so agents can deterministically fetch “the latest front render” without path inspection.
  Rationale: In practice, “list everything and eyeball paths” is the main cause of inspection loops. Explicit bounds and label filters keep retrieval deterministic and cheap while preserving debuggability.
  Date/Author: 2026-03-13 / GPT-5.2

- Decision: Treat “artifact refs” as a path leak and migrate LLM tools that accept `*_artifact_ref` (notably plan templates) onto KV references.
  Rationale: If a tool contract still requires a file ref, the agent will still be pushed into filesystem-shaped reasoning and prompts will drift. KV refs keep contracts stable and inspectable without exposing paths.
  Date/Author: 2026-03-13 / GPT-5.2

## Outcomes & Retrospective

- Outcome: The agent no longer calls file-based artifact tools to locate “latest” outputs; it uses Info Store KV (`info_kv_get_v1`) and Info Store blobs (`info_blobs_list_v1`) instead.
- Outcome: Deterministic tools write required KV keys and return stable KV refs (`info_kv` / `plan_template_kv`), and rendering returns opaque `blob_id`s (no paths).
- Outcome: Tool call start/result and best-effort stops are recorded as Info Store events and can be queried via `info_events_*`.
- Outcome: The run cache directory remains for human debugging, but the agent tool surface is no longer file-oriented and avoids leaking run-cache paths.
- Outcome: Blob storage paths are validated as relative/no-traversal as a defense-in-depth guard; resolving a `blob_id` cannot escape the run dir.
- Outcome: KV inspection tools include `written_by` provenance (tool_id + call_id) when available, making it easy to trace “who wrote this KV”.

## Context and Orientation

### What existed before

Gen3D’s agent-facing “artifact inspection” tools were implemented in:

- `src/gen3d/ai/artifacts.rs`:
  - `list_run_artifacts_v1()` returns run-cache files under an optional prefix.
  - `read_artifact_v1()` reads head/tail bytes of a file and optionally parses JSON.
  - `search_artifacts_v1()` does bounded substring search over file contents.

These tools were exposed to the agent via:

- `src/gen3d/agent/tools.rs` (tool registry + args_schema + args_example).
- `src/gen3d/ai/agent_tool_dispatch.rs` (tool dispatcher match arms).
- `src/gen3d/ai/agent_prompt.rs` (system instructions told the agent to inspect artifacts/logs via these tools).

### What exists now

Gen3D’s agent-facing inspection uses the Info Store tools:

- KV: `info_kv_list_keys_v1` / `info_kv_list_history_v1` / `info_kv_get_v1` / `info_kv_get_many_v1`
- Events: `info_events_list_v1` / `info_events_search_v1` / `info_events_get_v1`
- Blobs: `info_blobs_list_v1` / `info_blobs_get_v1`

The file-oriented artifact helpers remain for human debugging, but are no longer exposed as agent tools.

Gen3D “passes” write many files into the run cache directory under:

  `~/.gravimera/cache/gen3d/<run_id>/attempt_<n>/pass_<m>/...`

The agent frequently needs only a small subset of this information:

- latest `scene_graph_summary` (attachments, anchors, resolved transforms)
- latest QA output (`qa.json`, smoke/validate issues)
- last deterministic draft mutation summary (`apply_draft_ops_last.json`, review delta)
- last preview render images (front/top/etc)
- recent tool-call errors/warnings and budget stops

### Key terms used in this plan

- “Info Store”: a run-scoped, queryable store for Gen3D inspection information, independent of filesystem paths. It is an internal data structure that can be persisted to the run cache directory for resume/debugging.
- “KV record”: a JSON value written under a stable key, with metadata (`ts_ms`, `attempt`, `pass`, `assembly_rev`, `workspace_id`) and a monotonically increasing `kv_rev`.
- “Event”: an append-only log entry (tool_call_start, tool_call_result, engine_warning, budget_stop, etc) with queryable fields and a stable `event_id`.
- “Blob”: a binary artifact addressed by an opaque `blob_id` (images, sheets). The engine may still store the bytes as files in the run cache, but the tool contract uses `blob_id` instead of paths.
- “As-of selector”: a way to ask for the “latest record at or before” some state boundary (for example `assembly_rev <= 10`), so the agent cannot accidentally mix data from different assembly revisions.

## Plan of Work

Implement a new internal module `src/gen3d/ai/info_store.rs` that owns three collections: KV, events, and blobs. Store entries must be bounded and queryable with paging and sorting; persistence must be explicit and deterministic.

Add a new set of read-only agent tools that query the Info Store:

- KV: list keys, list history for a key, fetch a value (with “as-of” selector and bounded projection).
- Events: list recent events with filters, search events for a substring (bounded).
- Blobs: list blobs with metadata and stable ids (no raw paths).

Integrate the Info Store into the Gen3D run loop:

- Any existing tool or engine step that currently writes a JSON artifact for inspection must also write a KV record under a stable key and return that KV ref in the tool result.
- Any tool or engine step that produces images must register them as blobs and return blob ids. Tools that consume images (notably `llm_review_delta_v1`) must accept blob ids rather than file paths.
- The dispatcher must write events for tool call start/result (and budget stops), so the agent can debug without reading log files.

Once the Info Store tools are working and the prompt is updated, remove the file-based artifact tools from the agent-facing registry and update docs. We do not need to preserve backwards compatibility.

## Interfaces and Dependencies

### New internal types (Rust)

In `src/gen3d/ai/info_store.rs`, define:

1) KV:

   - `struct InfoKvKey { namespace: String, key: String }`
   - `struct InfoProvenance { tool_id: String, call_id: String }`
   - `struct InfoKvRecord { kv_rev: u64, written_at_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, workspace_id: String, key: InfoKvKey, value: serde_json::Value, summary: String, bytes: u64, written_by: Option<InfoProvenance> }`

2) Events:

   - `enum InfoEventKind { ToolCallStart, ToolCallResult, EngineLog, BudgetStop, Warning, Error }` (exact set may evolve; keep it versioned internally)
   - `struct InfoEvent { event_id: u64, ts_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, kind: InfoEventKind, tool_id: Option<String>, call_id: Option<String>, message: String, data: serde_json::Value }`

3) Blobs:

   - `struct InfoBlob { blob_id: String, created_at_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, content_type: String, bytes: u64, labels: Vec<String>, storage: InfoBlobStorage }`
   - `enum InfoBlobStorage { RunCacheFile { relative_path: String } }` (internal only; agent never sees paths)

4) Query helpers:

   - `struct InfoPage { limit: u32, cursor: Option<String> }`
   - `struct InfoPageOut<T> { items: Vec<T>, next_cursor: Option<String>, truncated: bool }`
   - Cursors must be opaque strings returned by the tool. The implementation may encode offsets or `(ts,id)` tuples, but callers must treat it as opaque.

Persistence decision (selected): store must be recoverable from the run cache directory. Use **append-only JSONL files** under the run dir:

- `info_store_v1/kv.jsonl`
- `info_store_v1/events.jsonl`
- `info_store_v1/blobs.jsonl`

Rationale: JSONL keeps dependencies minimal, is deterministic, and matches existing run-cache debugging patterns. The system can keep an in-memory index for fast lookups during a run and rebuild the index by replaying JSONL on resume.

### Bounds and defaults (must be enforced in tools)

These are intentionally conservative. If a future change requires bigger bounds, bump tool versions or add explicit opt-in flags; do not silently increase worst-case output size.

- KV:
  - `info_kv_*_v1.page.limit`: default 50, max 200.
  - `info_kv_get_v1.max_bytes`: default 64 KiB, max 512 KiB.
  - `info_kv_get_many_v1.max_items`: default 20, max 50.
- Events:
  - `info_events_*_v1.page.limit`: default 100, max 500 (events are small; list calls still return bounded `data_preview`).
  - `info_events_search_v1.query`: max length 256 bytes.
  - `info_events_get_v1.max_bytes`: default 64 KiB, max 512 KiB.
- Blobs:
  - `info_blobs_list_v1.page.limit`: default 50, max 200.
  - `info_blobs_list_v1.filters.labels_*`: arrays capped at 8 labels; each label max 64 bytes.

All list tools must include a deterministic total order for paging (and must encode the sort + filter parameters into the opaque cursor, rejecting mismatched reuse).

### KV key naming + required keys (agent-facing stability)

The Info Store is only useful if keys are stable and predictable. Treat KV keys like API endpoints: once referenced in prompts/docs/tests, changing them requires a deliberate migration and likely a tool version bump.

Key conventions:

- `namespace` should be a small, stable set. Start with a single namespace: `gen3d`.
- `key` must be ASCII, lowercase, and separator-based (recommend `.`). No whitespace. Max 128 bytes.
- Keys must be workspace-scoped without requiring extra tool args. Encode workspace id in the key:
  - Prefix every key with `ws.<workspace_id>.`
  - Example: `ws.main.scene_graph_summary`

Minimum required keys (written by deterministic tools during normal runs):

- `ws.<id>.state_summary` (written by `get_state_summary_v1`)
- `ws.<id>.scene_graph_summary` (written by `get_scene_graph_summary_v1`)
- `ws.<id>.qa` (written by `qa_v1`)
- `ws.<id>.validate` (written by `validate_v1`)
- `ws.<id>.smoke` (written by `smoke_check_v1`)
- `ws.<id>.apply_draft_ops_last` (written by `apply_draft_ops_v1`)
- `ws.<id>.component_parts.<component_name>` (written by `query_component_parts_v1`, bounded by `max_parts`)
- `ws.<id>.plan_template.preserve_mode.v1` (written by `get_plan_template_v1` once migrated off `*_artifact_ref`)

KV `summary` must be short and stable (recommend <= 160 bytes) and should mention what changed or what the record contains (for example: “scene graph summary (components=12 attachments=11)”, “qa ok=false errors=2 warnings=1”).

### New agent tools (contracts)

Add these tool ids (all read-only; all bounded; all return `truncated` + `next_cursor` when applicable):

1) `info_kv_list_keys_v1`

   Summary: “Read-only: list Info Store KV keys (stable identifiers) with latest metadata; supports paging and sorting.”

   Args schema:
     `{ namespace?: string, key_prefix?: string, sort?: "key_asc"|"last_written_desc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ namespace, key, latest: { kv_rev, written_at_ms, attempt, pass, assembly_rev, workspace_id, summary, bytes } }], next_cursor?: string, truncated: bool }`

2) `info_kv_list_history_v1`

   Summary: “Read-only: list historical revisions for one KV key; supports paging and sorting.”

   Args schema:
     `{ namespace: string, key: string, sort?: "rev_desc"|"rev_asc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ kv_rev, written_at_ms, attempt, pass, assembly_rev, workspace_id, summary, bytes }], next_cursor?: string, truncated: bool }`

3) `info_kv_get_v1`

   Summary: “Read-only: fetch a KV value by key and selector (latest / specific kv_rev / as-of assembly_rev/pass); optional JSON pointer projection.”

   Args schema:
     `{ namespace: string, key: string, selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number }, json_pointer?: string, max_bytes?: number }`

   Result shape:
     `{ ok: true, record: { kv_rev, written_at_ms, attempt, pass, assembly_rev, workspace_id, key: {namespace,key}, summary, bytes }, value: any, truncated: bool, json_pointer?: string }`

   Notes:
   - `kv_rev` must be unique and monotonically increasing within a run. `latest` selects by greatest `kv_rev`.
   - If `max_bytes` is exceeded, return `ok:false` with an actionable error telling the agent to use `json_pointer` (or request a smaller projection). Do not return partial JSON.
   - Selector tie-breaks must be deterministic:
     - `latest`: choose the record with the greatest `kv_rev`.
     - `as_of_assembly_rev`: choose the record with the greatest `assembly_rev <= requested`, then greatest `kv_rev`.
     - `as_of_pass`: choose the record with the greatest `pass <= requested`, then greatest `kv_rev`.

4) `info_kv_get_many_v1`

   Summary: “Read-only: fetch multiple KV values by key list, using a shared selector; returns per-key records or per-key errors, bounded.”

   Args schema:
     `{ items: [{ namespace: string, key: string, json_pointer?: string, max_bytes?: number }], selector?: { kind: "latest"|"kv_rev"|"as_of_assembly_rev"|"as_of_pass", kv_rev?: number, assembly_rev?: number, pass?: number }, max_items?: number }`

   Result shape:
     `{ ok: true, items: [{ namespace, key, ok: bool, record?: { kv_rev, written_at_ms, attempt, pass, assembly_rev, workspace_id, summary, bytes }, value?: any, truncated?: bool, error?: string }], truncated: bool }`

5) `info_events_list_v1`

   Summary: “Read-only: list recent Info Store events with filters; supports paging and sorting.”

   Args schema:
     `{ filters?: { kind?: string, tool_id?: string, call_id?: string, min_ts_ms?: number, max_ts_ms?: number, attempt?: number, pass?: number }, sort?: "ts_desc"|"ts_asc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ event_id, ts_ms, attempt, pass, assembly_rev, kind, tool_id?: string, call_id?: string, message, data_preview }], next_cursor?: string, truncated: bool }`

   Notes:
   - `data_preview` must be bounded (for example: selected fields + truncated JSON string). Do not return arbitrarily large blobs of tool results in list calls.

6) `info_events_get_v1`

   Summary: “Read-only: fetch one event by id; optional JSON pointer projection for the event `data`.”

   Args schema:
     `{ event_id: number, json_pointer?: string, max_bytes?: number }`

   Result shape:
     `{ ok: true, event: { event_id, ts_ms, attempt, pass, assembly_rev, kind, tool_id?: string, call_id?: string, message, data }, truncated: bool, json_pointer?: string }`

7) `info_events_search_v1`

   Summary: “Read-only: substring search over event messages (and optionally selected data fields), bounded and paged.”

   Args schema:
     `{ query: string, filters?: { kind?: string, attempt?: number, pass?: number }, page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, matches: [{ event_id, ts_ms, kind, message_excerpt }], next_cursor?: string, truncated: bool }`

8) `info_blobs_list_v1`

   Summary: “Read-only: list blobs (opaque ids for images/sheets) with metadata; supports paging and sorting.”

   Args schema:
     `{ filters?: { label_prefix?: string, labels_any?: string[], labels_all?: string[], content_type_prefix?: string, attempt?: number, pass?: number }, sort?: "created_desc"|"created_asc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ blob_id, created_at_ms, attempt, pass, assembly_rev, content_type, bytes, labels }], next_cursor?: string, truncated: bool }`

9) `info_blobs_get_v1`

   Summary: “Read-only: fetch one blob’s metadata by id (no bytes).”

   Args schema:
     `{ blob_id: string }`

   Result shape:
     `{ ok: true, blob: { blob_id, created_at_ms, attempt, pass, assembly_rev, content_type, bytes, labels } }`

### Tool migrations (contract updates)

To fully “remove the file-based solution” from tool contracts, update these existing tools:

- `render_preview_v1`:
  - Replace returned `images: string[]` (paths) with `blob_ids: string[]`.
  - Keep writing image files to the run cache directory for humans, but do not return paths to the agent.
  - Standardize blob `labels` so the agent can pick views deterministically without reading paths:
    - `kind:render_preview`
    - `workspace:<workspace_id>`
    - `view:front|left_back|right_back|top|bottom`
    - `kind:motion_sheet` + `motion:move|attack` when applicable

- `llm_review_delta_v1`:
  - Replace `preview_images?: string[]` with `preview_blob_ids?: string[]` (array of blob ids).
  - Default behavior (“use last render”) should use blob ids, not paths.

- `get_state_summary_v1`:
  - Replace `last_render_images: string[]` with `last_render_blob_ids: string[]`.
  - Keep an internal/human debug escape hatch only if required, but do not show raw paths in the agent prompt.

To fully remove file-based “artifact refs” from tool contracts (not just the *listing/search* tools), also migrate:

- `get_plan_template_v1` + `llm_generate_plan_v1`:
  - Replace `plan_template_artifact_ref?: string` with `plan_template_kv?: { namespace: string, key: string, selector?: ... }` (or equivalent stable KV reference).
  - Store the template JSON in KV (example key: `ws.<workspace_id>.plan_template.preserve_mode.v1`) so the agent can fetch/inspect it via `info_kv_get_v1` and the LLM plan tool can load it without touching paths.
  - If preserving a “copy+edit template” workflow is important, return a new KV key or `kv_rev` that the agent can modify via the existing deterministic patch workflows (no ad-hoc filesystem editing).

Update `src/gen3d/ai/agent_prompt.rs` to remove instructions that mention `list_run_artifacts_v1` / `read_artifact_v1` / `search_artifacts_v1`, and instead teach the agent:

- Use KV for “latest scene graph / QA / draft ops”.
- Use events for “what went wrong in the last step”.
- Use blobs for “latest render images”.

This is a tool-contract enforcement change; do not add prompt “micromanagement” rules beyond the new tool summaries and actionable error messages.

## Concrete Steps

All commands are run from the repository root.

1) Spec-first: implement the new tool contracts in the registry and write unit tests for args parsing and bounded outputs before wiring into the agent prompt. Use `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md` as checklists.

2) Implement `Gen3dInfoStore` and ensure it is run-scoped (tied to `run_id`) and can be persisted under the run cache directory. Add a small “toy” integration that writes a KV record for `scene_graph_summary` whenever `get_scene_graph_summary_v1` is called.

3) Add the read-only tools to query the store. Ensure paging + sorting are deterministic, and ensure every tool returns actionable errors (for example, “unknown key”, “selector not found”, “value too large; use json_pointer”).

4) Integrate events: append tool call start/result and budget stops into the events stream so the agent can debug without reading `gen3d_run.log`.

5) Migrate image handling to blob ids. Ensure review tools can still send images to the LLM (blob id resolves to a run-cache file internally), but the tool contract never exposes paths.

6) Remove artifact tools from the registry and dispatcher, and delete/retire prompt text and docs that instruct using them.

7) Validation:

   - Run unit tests:
     - `cargo test -q`
   - Run the required rendered smoke test (do NOT use `--headless`):
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`

## Validation and Acceptance

Acceptance is user-visible behavior:

1) A Gen3D run that previously burned steps on “inspecting” can fetch:
   - latest scene graph summary via `info_kv_get_v1`
   - latest QA output via `info_kv_get_v1`
   - recent tool failures via `info_events_list_v1` / `info_events_search_v1`
   - latest render images via `info_blobs_list_v1` (blob ids)

2) The agent prompt and tool contracts do not contain run-cache filesystem paths. Image selection is done via blob ids.

3) Paging/sorting is deterministic and bounded:
   - listing more than the default page size never silently hides “latest” entries due to filename ordering or truncation without a cursor.

4) The required rendered smoke test runs without crashing.

## Idempotence and Recovery

- If the Info Store persistence format changes in the future (for example JSONL → SQLite), bump the on-disk store directory name (for example `info_store_v2/`) and provide a deterministic rebuild/conversion path (“replay old JSONL into the new store”) that is safe to run multiple times.
- Keep a temporary compatibility shim (optional) only while migrating callers; remove it before completing the plan since backwards compatibility is not required.

## Artifacts and Notes

Example transcripts (indented, no code fences):

- `info_kv_list_keys_v1` returning keys with latest metadata and `next_cursor`:

    tool_call info_kv_list_keys_v1 args={"namespace":"gen3d","key_prefix":"ws.main.","sort":"last_written_desc","page":{"limit":2}}
    result ok items=2 truncated=true next_cursor="<opaque>"
    result.items[0].key="ws.main.qa" latest.kv_rev=123
    result.items[1].key="ws.main.scene_graph_summary" latest.kv_rev=122

- `info_kv_get_v1` retrieving `scene_graph_summary` with `selector.kind="latest"`:

    tool_call info_kv_get_v1 args={"namespace":"gen3d","key":"ws.main.scene_graph_summary","selector":{"kind":"latest"},"json_pointer":"/components_total"}
    result ok record.kv_rev=122 value=12 truncated=false

- `info_events_search_v1` finding a recent tool error:

    tool_call info_events_search_v1 args={"query":"Unknown args key","filters":{"kind":"tool_call_result"},"page":{"limit":5}}
    result ok matches=1 truncated=false
    result.matches[0].event_id=77 kind="tool_call_result"

- `render_preview_v1` returning blob ids, and `llm_review_delta_v1` consuming them:

    tool_call render_preview_v1 args={"views":["front","left_back","right_back","top","bottom"],"resolution":960,"include_motion_sheets":true}
    result ok blob_ids=7 static_blob_ids=5 motion_sheet_blob_ids.move="<blob_id>" motion_sheet_blob_ids.attack=null
    tool_call llm_review_delta_v1 args={"preview_blob_ids":["<blob_id_front>","<blob_id_left_back>","<blob_id_right_back>","<blob_id_top>","<blob_id_bottom>"]}
    result ok (applied tweak ops, or returned pending regen indices)

- `llm_review_delta_v1` reviewing the latest render cache (no explicit ids):

    tool_call llm_review_delta_v1 args={"preview_blob_ids":[]}
    result ok (uses latest render cache if review_appearance=true)

## Follow-ups (optional)

These are defense-in-depth and “contract drift” hardening steps. They are optional in the sense that the core Info Store works without them, but they reduce regressions in real agent runs.

- [x] Harden blob path resolution: reject absolute paths and `..` traversal even if a blob record is malformed.
- [x] Include KV provenance (`written_by`) in Info KV tool outputs so the agent can trace which tool/call produced a value.
- [x] Keep prompt ↔ tool contracts aligned for review images:
  - Do not allow unsupported args (example: remove `include_original_images` if it always errors).
  - Provide a non-empty default call pattern that satisfies the “no empty `{}` args” prompt rule (example: `preview_blob_ids: []` means “use the latest render cache”).

## Code Review Guidance (Prompt ↔ Tool Contract Mismatch Prevention)

This change touches the agent prompt, the tool registry, tool argument parsing, and tool result shapes. Most regressions in Gen3D “agent loops” come from mismatches between these layers. During review, treat tool contracts like compiler interfaces: contract-first, versioned, and test-backed.

Reviewers should verify (in the same PR) that:

1) Tool registry, parsing, and examples match.

   - `src/gen3d/agent/tools.rs` includes every new/changed tool id with `args_schema` and `args_example` that only uses accepted keys (watch for `#[serde(deny_unknown_fields)]`).
   - `src/gen3d/ai/agent_tool_dispatch.rs` (and helper arg parsers) accept exactly those keys and produce actionable errors for missing/unknown keys.

2) Prompt text matches the tool contract (no drift).

   - `src/gen3d/ai/agent_prompt.rs` and any tool-specific prompts mention the correct tool ids and argument names.
   - If an arg key changes (example: `preview_images` → `preview_blob_ids`), update prompt text in the same patch and add a unit test that asserts the prompt contains the new signature and does not mention removed tools.

3) Results remain actionable and bounded.

   - List tools return `next_cursor` + `truncated` and do not dump large payloads (use `data_preview` and `*_get_v1` tools for full retrieval).
   - `*_get_v1` tools enforce `max_bytes` and fail with an actionable message when exceeded (suggest `json_pointer`), rather than silently truncating JSON.
   - All returned records include the identifiers needed for follow-up actions (`kv_rev`, `event_id`, `blob_id`) plus provenance (`attempt`, `pass`, `assembly_rev`, `workspace_id`, `tool_id`/`call_id` where applicable).

4) No path leaks and no silent mutation.

   - Tool results and the agent prompt must not contain absolute run-cache paths. Add a test that asserts prompt text does not contain `/.gravimera/cache/gen3d/`.
   - Tool results and the agent prompt must not contain file-based refs (`artifact_ref`, `plan_template_artifact_ref`, `preview_images`) once migrations are complete. Add tests that assert prompt text does not mention `list_run_artifacts_v1` / `read_artifact_v1` / `search_artifacts_v1` and does not contain `artifact_ref`.
   - Read-only tools must not mutate state. Mutation tools must return diffs and record an event describing what changed.

5) Smoke test is run for safety.

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`

This guidance is derived from `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md`, but is embedded here so reviewers can apply it without context switching.
