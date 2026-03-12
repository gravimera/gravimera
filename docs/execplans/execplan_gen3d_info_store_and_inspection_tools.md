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
- [ ] Implement `Gen3dInfoStore` core types (KV, events, blobs) and persistence in the run cache directory.
- [ ] Add Info Store read-only tools (`info_kv_*`, `info_events_*`, `info_blobs_*`) to the tool registry and dispatcher.
- [ ] Migrate existing tools to write to the Info Store (and return KV/blob refs instead of paths where applicable).
- [ ] Remove file-based artifact tools from the agent tool list (`list_run_artifacts_v1`, `read_artifact_v1`, `search_artifacts_v1`) and update the agent prompt/docs accordingly.
- [ ] Add unit tests + a small regression harness under `test/` proving paging/sorting and “latest” retrieval work without truncation bugs.
- [ ] Run tests and the required rendered smoke test.
- [ ] Commit the implementation with a clear message.

## Surprises & Discoveries

- Observation: `list_run_artifacts_v1` returns entries in lexicographic filename order and truncates at `max_items`, so it is not a reliable way to find “latest pass output” once a run has many files.
  Evidence: `src/gen3d/ai/artifacts.rs` `read_dir_sorted()` sorts by filename and `list_files_recursive_sorted()` stops at `max_items`.

- Observation: `search_artifacts_v1` searches file contents line-by-line (`line.contains(query)`), not artifact paths, so searching for a filename like `scene_graph_summary.json` mostly matches tool/help text rather than identifying the latest summary artifact.
  Evidence: `src/gen3d/ai/artifacts.rs` `search_artifacts_v1()` loops over `head_text.lines()` and checks `line.contains(query)`.

- Observation: A real run can burn most of its 30-minute budget repeating “inspect/list/search” steps before the agent applies a deterministic fix, then later hits the time-budget stop during a long review call.
  Evidence: Run cache `~/.gravimera/cache/gen3d/99b577ae-e3a5-480e-ab2d-97b014a4ca5d` shows repeated inspection passes and stops with `Time budget exhausted` in `attempt_0/pass_51/gen3d_run.log`.

## Decision Log

- Decision: Replace file-path based inspection tools with a run-scoped Info Store (KV + events + blobs), and remove filesystem paths from tool contracts (especially image paths).
  Rationale: The agent’s job is to reason about the model and apply deterministic edits, not to navigate the run cache filesystem. Stable keys and opaque blob ids are more robust than filenames and avoid truncation/order pitfalls.
  Date/Author: 2026-03-13 / GPT-5.2

- Decision: Prefer typed, domain-level navigation (components/attachments/parts) over generic “JSON tree browsing”.
  Rationale: The Gen3D model is not purely a tree; it is a graph (workspaces, linked components, attachment edges). Navigation should be explicit and bounded, with stable ids, rather than path-like JSON pointers as the primary mechanism.
  Date/Author: 2026-03-13 / GPT-5.2

## Outcomes & Retrospective

- Outcome (planned): The agent no longer calls `list_run_artifacts_v1` / `search_artifacts_v1` to find “latest scene graph summary”; it uses `info_kv_get_v1` with an explicit selector (or calls `get_scene_graph_summary_v1` directly).
- Outcome (planned): Any tool that produces “inspectable” output also registers it in the Info Store and returns stable references (KV record refs and blob ids), enabling deterministic follow-up calls.
- Outcome (planned): The run cache directory remains for human debugging, but the agent tool surface is no longer file-oriented.

## Context and Orientation

### What exists today

Gen3D’s agent-facing “artifact inspection” tools are implemented in:

- `src/gen3d/ai/artifacts.rs`:
  - `list_run_artifacts_v1()` returns run-cache files under an optional prefix.
  - `read_artifact_v1()` reads head/tail bytes of a file and optionally parses JSON.
  - `search_artifacts_v1()` does bounded substring search over file contents.

These tools are exposed to the agent via:

- `src/gen3d/agent/tools.rs` (tool registry + args_schema + args_example).
- `src/gen3d/ai/agent_tool_dispatch.rs` (tool dispatcher match arms).
- `src/gen3d/ai/agent_prompt.rs` (system instructions currently tell the agent to inspect artifacts/logs via these tools).

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
   - `struct InfoKvRecord { kv_rev: u64, written_at_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, workspace_id: String, key: InfoKvKey, value: serde_json::Value, summary: String, bytes: u64, written_by: Option<InfoProvenance> }`

2) Events:

   - `enum InfoEventKind { ToolCallStart, ToolCallResult, EngineLog, BudgetStop, Warning, Error }` (exact set may evolve; keep it versioned internally)
   - `struct InfoEvent { event_id: u64, ts_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, kind: InfoEventKind, message: String, data: serde_json::Value }`

3) Blobs:

   - `struct InfoBlob { blob_id: String, created_at_ms: u64, attempt: u32, pass: u32, assembly_rev: u32, content_type: String, bytes: u64, labels: Vec<String>, storage: InfoBlobStorage }`
   - `enum InfoBlobStorage { RunCacheFile { relative_path: String } }` (internal only; agent never sees paths)

4) Query helpers:

   - `struct InfoPage { limit: u32, cursor: Option<String> }`
   - `struct InfoPageOut<T> { items: Vec<T>, next_cursor: Option<String>, truncated: bool }`
   - Cursors must be opaque strings returned by the tool. The implementation may encode offsets or `(ts,id)` tuples, but callers must treat it as opaque.

Persistence decision (to implement): store must be recoverable from the run cache directory. Use either:

- A small number of append-only JSONL files (KV, events, blobs), or
- A single SQLite database under the run dir.

Do not introduce external services. If adding SQLite, keep it embedded and deterministic; document migration and include unit tests.

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
   - If `max_bytes` is exceeded, return `truncated=true` and include an actionable error message suggesting `json_pointer` or a smaller projection. Do not silently return partial JSON.

4) `info_events_list_v1`

   Summary: “Read-only: list recent Info Store events with filters; supports paging and sorting.”

   Args schema:
     `{ filters?: { kind?: string, tool_id?: string, min_ts_ms?: number, max_ts_ms?: number, attempt?: number, pass?: number }, sort?: "ts_desc"|"ts_asc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ event_id, ts_ms, attempt, pass, assembly_rev, kind, message, data }], next_cursor?: string, truncated: bool }`

5) `info_events_search_v1`

   Summary: “Read-only: substring search over event messages (and optionally selected data fields), bounded and paged.”

   Args schema:
     `{ query: string, filters?: { kind?: string, attempt?: number, pass?: number }, page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, matches: [{ event_id, ts_ms, kind, message_excerpt }], next_cursor?: string, truncated: bool }`

6) `info_blobs_list_v1`

   Summary: “Read-only: list blobs (opaque ids for images/sheets) with metadata; supports paging and sorting.”

   Args schema:
     `{ filters?: { label_prefix?: string, content_type_prefix?: string, attempt?: number, pass?: number }, sort?: "created_desc"|"created_asc", page?: { limit?: number, cursor?: string } }`

   Result shape:
     `{ ok: true, items: [{ blob_id, created_at_ms, attempt, pass, assembly_rev, content_type, bytes, labels }], next_cursor?: string, truncated: bool }`

### Tool migrations (contract updates)

To fully “remove the file-based solution” from tool contracts, update these existing tools:

- `render_preview_v1`:
  - Replace returned `images: string[]` (paths) with `blob_ids: string[]`.
  - Keep writing image files to the run cache directory for humans, but do not return paths to the agent.

- `llm_review_delta_v1`:
  - Replace `preview_images?: string[]` with `preview_blob_ids?: string[]`.
  - Default behavior (“use last render”) should use blob ids, not paths.

- `get_state_summary_v1`:
  - Replace `last_render_images: string[]` with `last_render_blob_ids: string[]`.
  - Keep an internal/human debug escape hatch only if required, but do not show raw paths in the agent prompt.

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

- If the Info Store persistence format changes during implementation (for example JSONL → SQLite), bump internal versions and provide a deterministic rebuild path (“re-index the run cache directory into a fresh store”) that is safe to run multiple times.
- Keep a temporary compatibility shim (optional) only while migrating callers; remove it before completing the plan since backwards compatibility is not required.

## Artifacts and Notes

When implementing, include small example transcripts in this section (indented, no code fences) showing:

- `info_kv_list_keys_v1` returning keys with latest metadata and `next_cursor`.
- `info_kv_get_v1` retrieving `scene_graph_summary` with `selector.kind="latest"`.
- `info_events_search_v1` finding a recent tool error.
- `render_preview_v1` returning blob ids, and `llm_review_delta_v1` consuming them.
