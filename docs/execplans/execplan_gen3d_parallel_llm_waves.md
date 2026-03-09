# Gen3D: Parallel LLM Waves (Batch Component Generation + Fewer Reviews)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D Build runs are currently dominated by LLM latency because the agent calls `llm_generate_component_v1` and `llm_review_delta_v1` serially (one at a time). Even when the engine can render/validate quickly, the wall time adds up to many minutes.

After this change, Gen3D should be faster while keeping generation quality by:

1) adding a new tool that generates multiple components concurrently (a single tool call that internally runs many LLM requests in parallel, bounded by a config knob), and
2) nudging the agent to work in “waves” (generate a batch of core components, then render+review once) instead of reviewing after every small change.

Users should be able to observe the speedup by inspecting `gen3d_cache/<run_id>/agent_trace.jsonl` and the per-pass `gen3d_run.log`:

- fewer sequential component tool calls,
- multiple component requests in flight at once,
- fewer review calls per run (typically 1 review after the main build, then optional regen+review).

## Progress

- [x] (2026-02-05) Write this ExecPlan (self-contained).
- [x] (2026-02-05) Implement `llm_generate_components_v1` tool (batch component generation with parallelism).
- [x] (2026-02-05) Update Gen3D agent system prompt to prefer batch waves (generate batch -> render -> review).
- [x] (2026-02-05) Update mock agent test backend to use the batch tool (regression coverage without network).
- [x] (2026-02-05) Run `cargo test` and a headless smoke run (`cargo run -- --headless --headless-seconds 2`).
- [x] (2026-02-05) Commit.

## Surprises & Discoveries

- Observation: The current agent tool execution is intentionally sequential: a tool call that starts async work blocks the remainder of the step until completion.
  Evidence: `src/gen3d/ai/agent_step.rs` returns `ToolCallOutcome::StartedAsync` and the agent step resumes only after `src/gen3d/ai/agent_tool_poll.rs` completes the pending tool.

## Decision Log

- Decision: Add a new batch tool (`llm_generate_components_v1`) rather than trying to execute multiple async tools concurrently in one agent step.
  Rationale: The current agent protocol is sequential by design; a batch tool preserves a simple strict protocol while enabling internal parallelism safely.
  Date/Author: 2026-02-05 / Codex

- Decision: Keep the draft state updates deterministic by applying completed component results sequentially in a stable order, even if LLM requests finish out-of-order.
  Rationale: Determinism improves debuggability and reduces “heisenbugs” in assembly transforms and copy/linked behavior.
  Date/Author: 2026-02-05 / Codex

## Outcomes & Retrospective

Completed (2026-02-05):

- Added a new Gen3D agent tool `llm_generate_components_v1` that generates multiple components concurrently, bounded by `gen3d.max_parallel_components`, and applies results to the shared draft.
- Updated the agent system prompt to prefer “waves” (batch generate -> render -> review) to reduce sequential LLM wall time while keeping visual QA.
- Extended the mock OpenAI backend used by unit tests so the end-to-end mock agent run exercises the batch tool path.

## Context and Orientation

Gen3D AI code lives in `src/gen3d/ai/`.

Key files:

- `src/gen3d/ai/agent_step.rs` + `src/gen3d/ai/agent_tool_dispatch.rs` + `src/gen3d/ai/agent_tool_poll.rs`: Executes agent tool calls; `llm_generate_component_v1` is 1 component at a time and blocks until done.
- `src/gen3d/agent/tools.rs`: Tool registry and tool descriptions shown to the agent.
- `src/config.rs`: Contains `gen3d_max_parallel_components` (existing user-facing config knob).
- `src/gen3d/ai/mod.rs`: Owns `spawn_gen3d_ai_text_thread` and shared LLM call plumbing; also contains an older “parallel components” helper used outside the agent tool path.

Definitions:

- **Wave**: a phase where the agent performs multiple generation actions (plan or multiple components) before doing a review render and `llm_review_delta_v1`.
- **Batch component generation**: generating multiple component drafts concurrently and applying them to the shared draft deterministically.

## Plan of Work

### 1) Add a batch component generation tool

Add a new tool id: `llm_generate_components_v1`.

In `src/gen3d/agent/tools.rs`:

- Add `TOOL_ID_LLM_GENERATE_COMPONENTS` constant.
- Add it to `Gen3dToolRegistryV1::list()` and `describe()` with a clear schema:

  - args:
    - `component_indices`: array of 0-based indices (optional)
    - `component_names`: array of strings (optional)
    - `missing_only`: bool (optional; default true when no explicit indices are provided)
    - `force`: bool (optional; if true, regen even if already generated)

  - result:
    - `ok`: bool
    - `requested`: number
    - `succeeded`: number
    - `failed`: array of `{ index, name, error }`

In `src/gen3d/ai/agent_tool_dispatch.rs` + `src/gen3d/ai/agent_tool_poll.rs`:

- Extend `execute_tool_call` (in `src/gen3d/ai/agent_tool_dispatch.rs`) to start the batch tool:
  - Resolve the requested indices.
  - Store pending batch metadata in `job.agent` (requested indices, per-index completion tracking).
  - Initialize `job.component_queue` with the indices.
  - Clear `job.component_in_flight`.
  - Set `job.agent.pending_llm_tool` to a new `Gen3dAgentLlmToolKind::GenerateComponentsBatch`.
  - Set `job.phase = Gen3dAiPhase::AgentWaitingTool`.

- Extend `poll_agent_tool` (in `src/gen3d/ai/agent_tool_poll.rs`) to support the batch tool:
  - While batch is pending:
    - poll and apply completed component requests,
    - start new requests up to `job.max_parallel_components` (bounded by `config.gen3d_max_parallel_components`),
    - update `workshop.status` with batch progress.
  - When all requested indices are done:
    - emit a single `Gen3dToolResultJsonV1` for the batch tool,
    - clear pending batch state,
    - resume `Gen3dAiPhase::AgentExecutingActions`.

Important: the batch tool must not switch the overall Gen3D run phase to “auto review” or “finish”; it is just a tool called by the agent.

### 2) Improve agent behavior: prefer waves

In `src/gen3d/ai/agent_prompt.rs` within `build_agent_system_instructions()`:

- Add explicit guidance:
  - Prefer `llm_generate_components_v1` to generate multiple missing components at once.
  - Prefer a “core build wave” (plan → batch generate core components → render → review) over reviewing after every individual component.
  - Still use top/bottom views for vehicles in review renders, but avoid repeated review calls when nothing changed.

This preserves quality (review exists) while reducing sequential LLM calls.

### 3) Tests and validation

Add/adjust tests (offline) to catch regressions:

- Ensure `Gen3dToolRegistryV1::list()` includes the new tool id.
- Ensure `get_tools_detail_v1` returns `args_schema` and `args_example` for requested tool ids.
- Ensure `summarize_tool_result()` prints a compact summary for the batch tool.

Then run:

  - `cargo test`
  - `cargo run -- --headless --headless-seconds 2`

### 4) Docs + config

If user-facing behavior changed, update:

- `config.example.toml` (already contains `max_parallel_components`; update the comment to mention it affects `llm_generate_components_v1` tool too).
- `README.md` (short note: “Gen3D uses `max_parallel_components` to batch component generation.”)

Commit changes.

## Concrete Steps

From repo root (`/Users/flow/workspace/github/gravimera`):

1) Implement tool additions and agent prompt update:
    - `cargo test`

2) Run smoke test:
    - `cargo run -- --headless --headless-seconds 2`

3) Commit:
    - `git status`
    - `git commit -am "Gen3D: batch component generation tool + parallel waves"`
