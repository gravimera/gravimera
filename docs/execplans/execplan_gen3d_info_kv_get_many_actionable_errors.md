# Gen3D: Make `info_kv_get_many_v1` misuse errors actionable (and align scene graph summary fields)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D agents frequently “inspect” repeatedly instead of acting when an inspection tool call fails in a non-actionable way. A concrete example is miscalling `info_kv_get_many_v1` by placing `selector` inside each `items[]` entry (because `info_kv_get_v1` accepts `selector` and the prompt’s tool list can be truncated). Today this produces a terse serde parse error that does not tell the agent how to correct the call.

At the same time, `get_scene_graph_summary_v1` tool-result summaries shown to the agent mention `attachment_edges=[…]`, but the stored `ws.<id>.scene_graph_summary` KV value does not include a top-level `attachment_edges` field. Agents then try `info_kv_get_v1(json_pointer:"/attachment_edges")` and fail again, triggering more inspection loops.

After this change:

1) `info_kv_get_many_v1` returns compiler-style diagnostics for the most common misuse (misplaced `selector`) including an explicit corrected payload the agent can copy/paste.
2) `build_gen3d_scene_graph_summary()` includes top-level `components_total`, `attachments_total`, and `attachment_edges` fields so the prompt’s “attachment_edges” concept matches real stored data and JSON pointers work.
3) The tool registry descriptions/examples are tightened so the agent is less likely to make the mistake in the first place.

How to see it working:

- Run `cargo test -q gen3d_info_kv_get_many_selector_misplaced_error_is_actionable` and verify it passes.
- Run a Gen3D run and confirm `get_scene_graph_summary_v1` output includes `components_total`, `attachments_total`, and `attachment_edges`, and that `info_kv_get_v1` can fetch `/attachment_edges`.
- (Repo requirement) Run the rendered smoke test: `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`.

## Progress

- [x] (2026-03-14 07:55Z) Draft this ExecPlan and keep it updated while implementing.
- [x] (2026-03-14 07:59Z) Implement actionable `info_kv_get_many_v1` parse error hint for misplaced `selector`.
- [x] (2026-03-14 08:01Z) Add `components_total`, `attachments_total`, and `attachment_edges` to `build_gen3d_scene_graph_summary()`.
- [x] (2026-03-14 08:03Z) Update tool registry summaries/examples for `info_kv_get_many_v1` (and related KV tools) to reduce misuse.
- [x] (2026-03-14 08:05Z) Add regression tests covering the new error message and new scene graph summary fields.
- [x] (2026-03-14 08:15Z) Run unit tests and the required rendered smoke test.
- [x] (2026-03-14 08:16Z) Commit with a clear message.

## Surprises & Discoveries

- Observation: `get_scene_graph_summary_v1` tool-result summaries surface `attachment_edges`, but the serialized scene graph summary KV does not currently include `attachment_edges`, and also lacks `components_total` / `attachments_total`.
  Evidence: The tool-result compact summary includes `attachment_edges=[…]` (derived from `components[].attach_to`) while `build_gen3d_scene_graph_summary()` returns only `{…, components:[…]}` without the top-level fields.

## Decision Log

- Decision: Keep `info_kv_get_many_v1` schema unchanged and improve diagnostics instead of silently “fixing up” malformed calls.
  Rationale: Tools should be deterministic and not apply implicit mutations to the request. The correct recovery path is to teach the agent how to form a valid call.
  Date/Author: 2026-03-14 / GPT-5.2

- Decision: Align scene graph summary JSON shape with the prompt’s compact summary terminology by adding explicit `attachment_edges`.
  Rationale: If the prompt teaches a noun (“attachment_edges”), it should be queryable via KV tools; otherwise agents will keep trying to fetch it via JSON pointers and get stuck in inspection loops.
  Date/Author: 2026-03-14 / GPT-5.2

## Outcomes & Retrospective

- Outcome: `info_kv_get_many_v1` now emits a fix-it style error when `selector` is mistakenly nested inside `items[]`, explaining that selector is shared and top-level and including a corrected example payload.
- Outcome: `build_gen3d_scene_graph_summary()` now includes `components_total`, `attachments_total`, and a derived `attachment_edges` list, aligning stored KV shape with the prompt’s tool-result summaries and making `/attachment_edges` JSON-pointer projections succeed.
- Outcome: The tool registry entry for `info_kv_get_many_v1` now calls out selector placement explicitly and provides a concise example.
- Outcome: Added regression tests and validated with `cargo test` and the required rendered smoke run.

## Context and Orientation

Relevant modules and what they do:

- `src/gen3d/ai/agent_tool_dispatch.rs`: Implements the dispatcher for agent tool calls. This is where tool args are parsed and errors are returned as `Gen3dToolResultJsonV1::err(...)`.
- `src/gen3d/agent/tools.rs`: Tool registry. The content here is rendered into the agent prompt as “Available tools (args signature + example shown)”. Truncated examples can mislead the agent, so examples must be short and unambiguously correct.
- `src/gen3d/ai/orchestration.rs`: Contains `build_gen3d_scene_graph_summary(...)`, the canonical JSON shape returned by `get_scene_graph_summary_v1` and stored under `ws.<workspace_id>.scene_graph_summary` in the Info Store.
- `src/gen3d/ai/agent_prompt.rs`: Builds “Recent tool results (compact)” lines. For `get_scene_graph_summary_v1`, it derives a bounded `attachment_edges=[…]` list from `components[].attach_to`.

Key terms:

- “Info Store”: the run-scoped JSONL store (`info_store_v1/*.jsonl`) that holds KV values, events, and blobs. KV values are fetched via `info_kv_get_v1` / `info_kv_get_many_v1`.
- “JSON pointer”: a string like `/components/0/name` used by KV tools to project a subset of a stored JSON value.

## Plan of Work

1) Add a helper that formats `info_kv_get_many_v1` arg parse errors. When serde reports an unknown `selector` field inside `items[]`, detect it by inspecting the original `call.args` JSON and return an error message that includes:

   - What was wrong: `selector` is not allowed inside `items[]`.
   - What is expected: `selector` is a single, shared top-level field.
   - What to do next: a corrected example payload.

2) Update `build_gen3d_scene_graph_summary()` to include the following top-level fields:

   - `components_total`: integer, `components.len()`
   - `attachments_total`: integer, count of components where `attach_to` is present (excluding root/unattached)
   - `attachment_edges`: an array where each element is a compact, structured representation of an attachment edge. Each edge should at minimum include:
     - `child`, `parent`, `parent_anchor`, `child_anchor`
     - `offset_pos` (join-frame translation; the same value the compact tool-result summary prints)
     - `joint_kind` (string or null)

   The `components[]` array remains the source of truth; `attachment_edges` is a derived convenience view that must be deterministic and stable.

3) Update tool registry entries in `src/gen3d/agent/tools.rs`:

   - For `info_kv_get_many_v1`, explicitly state in `one_line_summary` that `selector` is top-level and shared; do not place `selector` inside `items[]`.
   - Keep `args_example` short enough to avoid truncation and include the shared selector in the example.
   - Keep the examples consistent with the actual scene graph summary fields (now that `components_total` / `attachments_total` exist).

4) Add regression tests:

   - A unit test in `src/gen3d/ai/agent_tool_dispatch.rs` that calls the new error formatter with a malformed args payload (selector inside items) and asserts the returned message includes the actionable “move selector to top-level” guidance and a corrected example.
   - Extend `src/gen3d/ai/regression_tests.rs` `gen3d_scene_graph_summary_includes_joint_kind` to also assert the new top-level fields exist and are correct for a tiny 2-component graph.

5) Run validation:

   - `cargo test`
   - Required rendered smoke test:
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

6) Commit the change with a clear message describing the tool diagnostics + scene graph summary alignment.

## Concrete Steps

All commands are run from the repo root (`/Users/flow/workspace/github/gravimera`).

1) Edit code (paths are from repo root):

   - `src/gen3d/ai/agent_tool_dispatch.rs`:
     - Add `format_info_kv_get_many_args_error(...)` (or similarly named helper).
     - Use it in the `info_kv_get_many_v1` args parse error path.
     - Add a unit test `gen3d_info_kv_get_many_selector_misplaced_error_is_actionable`.

   - `src/gen3d/ai/orchestration.rs`:
     - Add `components_total`, `attachments_total`, and `attachment_edges` to the returned JSON from `build_gen3d_scene_graph_summary()`.

   - `src/gen3d/agent/tools.rs`:
     - Update the `info_kv_get_many_v1` descriptor summary and example.

   - `src/gen3d/ai/regression_tests.rs`:
     - Extend `gen3d_scene_graph_summary_includes_joint_kind` to assert new fields.

2) Run tests:

   - `cargo test -q gen3d_info_kv_get_many_selector_misplaced_error_is_actionable`
   - `cargo test`

3) Run smoke test:

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

4) Commit:

   - `git status`
   - `git commit -am "gen3d: make info_kv_get_many errors actionable"`
     - If new files are added (this ExecPlan), include them in the commit.

## Validation and Acceptance

Acceptance criteria:

- `info_kv_get_many_v1` called with `selector` inside `items[]` returns `ok=false` with an error message that explicitly instructs the caller to move `selector` to the top-level and provides a corrected example payload.
- `get_scene_graph_summary_v1` output includes `components_total`, `attachments_total`, and `attachment_edges`, and `info_kv_get_v1(json_pointer:"/attachment_edges")` succeeds for the stored `ws.<id>.scene_graph_summary` KV value.
- All unit tests pass and the rendered smoke test starts the game without crashing.

## Idempotence and Recovery

These changes are safe to rerun:

- The unit tests do not depend on network access and can be re-executed repeatedly.
- The smoke test uses an isolated temporary `GRAVIMERA_HOME`, so it does not pollute the user’s real local state.

If the smoke test fails after this change, revert the last commit and rerun to confirm the regression was introduced here.

## Artifacts and Notes

When adding error messages, keep them short and machine-actionable. Prefer “Fix: …” phrasing and include a compact JSON example that can be copied as the next tool call args.
