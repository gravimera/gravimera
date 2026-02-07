# Gen3D: Codex-Style Tool-Driven Agent (Initiative + Observability)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gen3D behaves like a Codex-style agent: instead of following a fixed “plan → generate → review” pipeline, the AI is in charge of deciding what to do next during a Build run. The AI can plan, generate, validate, render, review, and patch the current draft by calling a strict in-engine tool protocol. The engine stays safe and maintainable because:

1. The AI can only perform actions via versioned, validated tools (no free-form engine mutations).
2. Every AI decision and every tool input/output is recorded in a rich per-run cache so we can debug and improve the toolset.

From the player’s perspective, Gen3D stays simple: **Build / Stop / Save**.

You can see this working by:

1. Running the game and entering Gen3D.
2. Dropping 0–6 images and/or typing a prompt, then clicking **Build**.
3. Watching the status panel show high-level summaries (not logs).
4. Inspecting `gen3d_cache/<run_id>/agent_trace.jsonl` plus per-step artifacts and renders.
5. Clicking **Save** multiple times during a run to capture multiple versions, and clicking **Stop** to halt at any point.

## Progress

- [x] (2026-02-01 02:30Z) Write this ExecPlan.
- [x] (2026-02-01 02:55Z) Implement the tool protocol schema and tool registry (introspectable `list_tools` / `describe_tool`).
- [x] (2026-02-01 07:30Z) Add per-run `agent_trace.jsonl` and persist tool I/O + AI messages to cache (`agent_trace.jsonl`, per-pass `tool_calls.jsonl` / `tool_results.jsonl`, per-pass request artifacts).
- [x] (2026-02-01 07:30Z) Refactor Gen3D orchestration into a tool-driven agent loop (AI chooses next tool calls via `gen3d_agent_step_v1`).
- [x] (2026-02-01 07:30Z) Add preview tools with AI-chosen camera angles/resolution (implemented `render_preview_v1` with view list + width/height + `axes_grid` overlay preset).
- [x] (2026-02-01 07:30Z) Add multi-workspace preview support (implemented `create_workspace_v1` / `delete_workspace_v1` / `set_active_workspace_v1` with optional component subset previews).
- [x] (2026-02-01 07:30Z) Make **Save** available after the first usable draft and allow multiple saves while building (snapshots draft; writes `save_*.json`).
- [x] (2026-02-01 07:30Z) Add a no-progress guard to avoid infinite loops (`[gen3d].no_progress_max_steps`).
- [x] (2026-02-01 07:30Z) Keep budgets “very large” by default (time/tokens) and remove pass-count limitations (passes iterate until Stop/budgets/no-progress).
- [x] (2026-02-01 07:30Z) Validation: `cargo test` and `cargo run -- --headless --headless-seconds 1`.
- [x] (2026-02-01 07:42Z) Update `README.md` / `gen_3d.md` (tool-driven agent + artifacts + config) and commit.

## Surprises & Discoveries

- Observation: Some OpenAI-compatible providers support `/responses` but not `previous_response_id` continuation.
  Evidence: Existing code already tracks `responses_continuation_supported` and probes with a single request.

- Observation: Gen3D already has valuable observability primitives (attempt/pass folders + per-pass engine logs + stored AI requests).
  Evidence: `src/gen3d/ai/mod.rs` writes `attempt_N/pass_M/` artifacts and starts per-pass `gravimera.log` capture.

## Decision Log

- Decision: Keep the player UX fixed at **Build / Stop / Save**.
  Rationale: The agent loop is an internal implementation detail; user should not be forced to manage phases.
  Date/Author: 2026-02-01 / Codex

- Decision: Use a strict, versioned “tool protocol” rather than accepting arbitrary JSON mutations.
  Rationale: Tools provide a stable surface area, type validation, easier maintenance, and safe evolution.
  Date/Author: 2026-02-01 / Codex

- Decision: Keep the existing `attempt_N/pass_M/` cache layout but reinterpret “pass” as an “agent step iteration”.
  Rationale: This preserves debuggability and avoids inventing a second artifact layout while still allowing “no pass limit”.
  Date/Author: 2026-02-01 / Codex

- Decision: Allow the AI to change semantics (mobility/combat) unless the user prompt forbids it.
  Rationale: Many user prompts are ambiguous; letting the agent adjust semantics improves results without fighting the harness.
  Date/Author: 2026-02-01 / Codex

## Outcomes & Retrospective

(To fill as milestones complete.)

## Context and Orientation

This repository is a Bevy game. Gen3D is an in-game workshop mode that generates object prefabs from primitives and persists them into `scene.dat`.

Key modules and current behavior:

- `src/gen3d/mod.rs`: Gen3D module glue, constants, and public re-exports.
- `src/gen3d/state.rs`: Gen3D state resources (workshop UI state, draft state, preview camera state).
- `src/gen3d/ui.rs`, `src/gen3d/status.rs`, `src/gen3d/images.rs`, `src/gen3d/tool_feedback_ui.rs`: UI systems.
- `src/gen3d/preview.rs`: Preview “studio” scene and orbit controls.
- `src/gen3d/save.rs`: Saves the current `Gen3dDraft` into `ObjectLibrary` and spawns it into the world; triggers scene save.
- `src/gen3d/ai/mod.rs`: The current AI orchestration state machine (“plan → optional plan-fill → generate components → capture fixed review renders → request review delta → apply delta → regen/replan”).
- `src/gen3d/ai/schema.rs`: JSON schemas for AI plan/component/review delta.
- `src/gen3d/ai/openai.rs`: `/responses` first, fallback to `/chat/completions`.

Terms used in this plan:

- “Run”: one click of **Build** in Gen3D. A run creates `gen3d_cache/<run_id>/`.
- “Attempt”: a full re-plan from scratch within a run (attempt_0, attempt_1, …).
- “Pass”: in current code it is an auto-review iteration. In this plan it becomes a general “agent step iteration” so it can be unlimited.
- “Tool protocol”: a strict JSON format where the AI can only act by calling versioned tools. The engine executes tools and returns tool results.
- “Workspace”: an isolated preview context the agent can render from, optionally containing only a subset of components or a variant of the draft.

Important constraints:

- The engine must not hard-code domain-specific placement heuristics (no “tree branches must be above trunk” rules). It can validate generic correctness (finite transforms, anchors exist, attachments resolve) and report issues, but “what looks logical” is an AI responsibility.
- The game must remain stable: Stop must always halt quickly, and budgets must prevent infinite loops.

## Plan of Work

This change is large, so implementation must be staged. Each milestone must be independently verifiable, and we must keep `cargo test` and the headless smoke run passing at every step.

### Milestone 1: Define the tool protocol and tool registry

Add a new module `src/gen3d/agent/` containing:

- `src/gen3d/agent/protocol.rs`: serde JSON structs for the agent step format and tool calls/results.
- `src/gen3d/agent/tools.rs`: a registry of tools (id + description + argument validation + executor).
- `src/gen3d/agent/trace.rs`: a small JSONL writer to append structured events to `gen3d_cache/<run_id>/agent_trace.jsonl`.

The protocol must be string-dispatched and introspectable:

- `list_tools_v1()` returns a list of `{ tool_id, title, one_line_summary }`.
- `describe_tool_v1(tool_id)` returns a detailed schema (description, args fields, result fields, error modes).

The protocol must be additive and versioned:

- Tool ids must include a version suffix (`*_v1`) so future changes add `*_v2` instead of breaking old behavior.

Tool execution rules:

- Every tool call must have a `call_id` (string) so results can be matched.
- Tool args must be validated by the engine. Invalid args result in a tool error result; they must not crash the game.
- Every tool call and tool result must be persisted to `agent_trace.jsonl` and to per-pass artifacts (for forensic debugging).

### Milestone 2: Convert Gen3D orchestration into a tool-driven agent loop

Introduce a new resource `Gen3dAgentJob` (in `src/gen3d/agent/mod.rs`) that owns:

- Run/attempt/pass ids and directories.
- A conversation/session state for the “agent brain”.
- The current plan + current draft state handles (reusing `Gen3dDraft` and the existing plan representation if possible).
- Budgets (time/tokens) and the no-progress guard state.
- The “Stop requested” flag.

Replace the current fixed flow in `src/gen3d/ai/mod.rs` with:

1. Call the agent brain LLM with system instructions describing the tool protocol and the overall goal (“create a good draft matching the prompt/images”).
2. Parse a strict `gen3d_agent_step_v1` JSON that contains:
   - A short `status_summary` string (for the UI).
   - An ordered list of actions, where actions are mostly `tool_call` entries.
   - An optional `done` action.
3. Execute tool calls in order (with a per-step cap to prevent infinite “tool spam” in one step).
4. Feed tool results back to the agent brain and request the next step.

The key difference from today: the AI decides the order. The engine only provides tools and guardrails.

To keep implementation manageable, treat existing operations as tools first:

- `llm_generate_plan_v1`
- `llm_generate_component_v1`
- `llm_review_delta_v1`

These tools should internally reuse the existing prompt builders/parsers/convertors:

- `src/gen3d/ai/prompts.rs`
- `src/gen3d/ai/parse.rs`
- `src/gen3d/ai/convert.rs`
- `src/gen3d/ai/openai.rs`

This lets the agent brain orchestrate “plan/generate/review” without losing existing robustness, while still giving it freedom to repeat, skip, or re-order steps as needed.

### Milestone 3: Add preview and validation tools with AI-chosen parameters

Expose engine abilities as tools that do not require more LLM calls:

- `get_user_inputs_v1`: return prompt text and cached input image filenames/paths for this run.
- `get_scene_graph_summary_v1`: return the same info as today’s `scene_graph_summary.json`.
- `validate_v1`: run generic validations and return structured issues with severity.
- `smoke_check_v1`: run the lightweight behavioral smoke checks used today (move/attack sheet capture applicability).

Add render tools:

- `render_preview_v1`:
  - Args: `workspace_id`, `views` (each view defines yaw/pitch or a named view), `resolution`, and `overlay_preset`.
  - Returns: file paths to PNGs.
- `render_motion_sheet_v1`:
  - Args: `workspace_id`, `kind` (`move`/`attack_primary`), `resolution`, `overlay_preset`.
  - Returns: file path to sheet PNG.

Overlay presets must be explicit and stable:

- Provide a small enum-like set of preset ids (e.g. `none`, `axes_grid`, `anchors`, `colliders`, `bounds`).
- Implement preset toggling by enabling/disabling existing overlay entities and collision overlays (do not spawn/despawn every time unless necessary).

### Milestone 4: Support multiple preview workspaces

Implement “workspace” as an isolated draft view the agent can use to test ideas without overwriting the main preview:

- `create_workspace_v1(name, include_components=...)` clones the current draft defs and optionally filters components.
- `delete_workspace_v1(workspace_id)`
- `set_active_workspace_v1(workspace_id)` controls what the player sees in the preview panel.

The agent can create multiple workspaces to:

- Render only one component to inspect it.
- Render an alternate assembly tweak while keeping the main assembly intact.

All workspace operations must be tool calls and must be trace-logged.

### Milestone 5: Save behavior and no-progress guard

Change **Save** behavior:

- Enable Save once there is a “usable draft”, defined as:
  - Root draft object exists, and
  - At least one generated component exists, and
  - Draft is structurally valid (anchors/attachments resolve), even if incomplete.
- Allow Save while building by snapshotting the current `Gen3dDraft` (clone defs) and saving that snapshot, so concurrent build updates cannot corrupt the saved result.
- Each Save produces a new saved object id/instance id and stores a `save_*.json` artifact in the run folder so we can correlate “what the user saved” with the agent loop state.

Add a no-progress guard:

- Track a stable hash of the current assembled state (plan hash + assembly_rev + a hash of draft defs).
- If the agent performs N consecutive steps without changing the state hash and without producing new renders/validations, stop the run as “best effort” with a clear summary.
- Make N configurable in `config.toml` under `[gen3d]` (default: 12).

### Milestone 6: Budgets and docs

Budgets must be “very large” by default:

- Time: 3600 seconds (1 hour).
- Tokens: 10,000,000.
- No pass-count limit (passes/steps run until Stop/budgets/no-progress guard).

Ensure budget exhaustion produces a clear status summary and always preserves artifacts.

Update `README.md` and `gen_3d.md`:

- Explain the tool-driven agent loop at a high level.
- Explain what gets written under `gen3d_cache/<run_id>/` and where to find `agent_trace.jsonl`.
- Document the key `[gen3d]` config keys for budgets and guards.

## Concrete Steps

All commands below run from the repo root: `/Users/flow/workspace/github/gravimera`.

1. Create the new agent modules and wire them into Gen3D:

   - Edit files under `src/gen3d/agent/` as described above.
   - Wire the new Bevy systems into the app in `src/app.rs` (only active in `GameMode::Gen3D`).

2. Keep the build green at each milestone:

   - Run `cargo test`
   - Run `cargo run -- --headless --headless-seconds 1`

3. Manual verification scenario (rendered mode):

   - Run `cargo run`
   - Enter Gen3D.
   - Provide either images or a prompt, click Build.
   - Verify the cache folder includes `agent_trace.jsonl` and that the agent is calling tools.
   - Click Save multiple times and confirm multiple saved instances appear next to the hero.
   - Click Stop while building and confirm the run stops quickly and preserves artifacts.

## Validation and Acceptance

This change is accepted when:

- The user-facing UI remains **Build / Stop / Save** and works in Gen3D.
- During a Build run, `gen3d_cache/<run_id>/agent_trace.jsonl` is created and grows with tool calls/results.
- The AI can call `list_tools_v1` and `describe_tool_v1` successfully and those calls are logged.
- The AI can request renders with different angles/resolutions via tools, and those PNGs are stored in the cache folder and referenced by tool results.
- Stop halts the run quickly without crashing, and the status panel shows a short summary.
- Save is available after the first usable draft and can be clicked multiple times during a run to create multiple saved objects.
- `cargo test` passes and `cargo run -- --headless --headless-seconds 1` exits cleanly.

## Idempotence and Recovery

- Runs are idempotent: every Build uses a new `<run_id>` folder; prior runs are never overwritten.
- If an AI step JSON is invalid, the engine must:
  - write the invalid text into the current pass folder,
  - record a schema error in `agent_trace.jsonl`,
  - request a repaired step (bounded retries),
  - and if still failing, stop as “best effort” instead of crashing.
- If a tool call fails, it returns a structured error result to the agent; the engine does not panic.

## Artifacts and Notes

Artifacts that must exist per run:

- `gen3d_cache/<run_id>/agent_trace.jsonl`
- `gen3d_cache/<run_id>/attempt_0/inputs/` (prompt + images + manifest)
- `gen3d_cache/<run_id>/attempt_N/pass_M/` with:
  - any renders requested in that step
  - `gen3d_run.log`
  - `gravimera.log`
  - JSON summaries for validate/smoke tools

## Interfaces and Dependencies

No new external services are introduced beyond the existing OpenAI-compatible endpoints already used by Gen3D.

New internal interfaces to implement:

- `crate::gen3d::agent::protocol` must define:
  - `Gen3dAgentStepJsonV1`
  - `Gen3dToolCallJsonV1`
  - `Gen3dToolResultJsonV1`
- `crate::gen3d::agent::tools` must define:
  - a registry that can answer `list_tools_v1` and `describe_tool_v1`
  - a single executor function `execute_tool_call(call, state) -> tool_result`
- `crate::gen3d::agent::trace` must define:
  - `append_trace_event(run_dir, event_json)` writing JSONL safely (best-effort).

When revising this plan during implementation, add a note at the end of this file describing what changed and why.
