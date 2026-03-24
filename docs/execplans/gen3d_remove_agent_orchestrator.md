# Gen3D: Remove agent orchestrator (pipeline-only)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D currently supports two orchestration modes:

- an LLM-driven “agent step” loop (the model decides which tools to call next), and
- a deterministic pipeline state machine (the engine decides which tool to call next).

The agent-step loop adds a large prompt surface (tool list + orchestration rules), increases variance (wrong next-step decisions), and requires ongoing prompt↔tool contract maintenance.

After this change, Gen3D is **pipeline-only**:

- There is no user-facing “agent orchestrator” option in config.
- The game never issues an `agent_step` LLM request.
- The large agent-step prompt is deleted, reducing tokens and failure modes.

How to see it working:

1. Run `cargo test` and confirm all tests pass.
2. Run the rendered smoke test:

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

   and confirm the game starts and exits without crashing.
3. (Optional) Run a real Gen3D build via UI or Automation API and confirm the Status text shows “Pipeline: …” stages and never mentions “agent-step”.

## Progress

- [x] (2026-03-24) Create ExecPlan for removing agent orchestrator.
- [ ] Remove the config surface for agent orchestrator (pipeline is the only supported orchestrator).
- [ ] Remove the agent-step orchestrator loop (prompt + parsing + polling), while keeping the pipeline and tool execution runtime.
- [ ] Remove `AgentStepV1` structured-output schema and any mock-backend branches that emit agent-step payloads.
- [ ] Update docs to describe pipeline-only Gen3D (`docs/gen3d/README.md`, `config.example.toml`).
- [ ] Run `cargo test`.
- [ ] Run the rendered smoke test (`--rendered-seconds 2`, not headless).
- [ ] Commit with a clear message.

## Surprises & Discoveries

- Observation: The deterministic pipeline orchestrator already reuses several “agent_*” modules (tool dispatch/polling, render capture, finish-run sequence) even when `job.mode=Pipeline`.
  Evidence: `src/gen3d/ai/pipeline_orchestrator.rs` imports `agent_tool_dispatch::execute_tool_call`, `agent_tool_poll::poll_agent_tool`, and finish helpers from `agent_step.rs`.

## Decision Log

- Decision: Fully remove the Gen3D agent-step orchestrator as a supported runtime path (not merely hide it behind config).
  Rationale: The user wants simpler code and a shorter/more focused prompt surface to reduce token cost and errors.
  Date/Author: 2026-03-24 / assistant

- Decision: Keep the deterministic tool execution runtime (tool dispatch + async polling + render capture + finish-run snapshotting) even if filenames remain `agent_*` for now, but delete the orchestration loop that prompts for `gen3d_agent_step_v1`.
  Rationale: The pipeline depends on the same deterministic tooling runtime; renaming/refactoring can be a follow-up once behavior is stable.
  Date/Author: 2026-03-24 / assistant

## Outcomes & Retrospective

(To be written after implementation.)

## Context and Orientation

Key files/modules (paths are repo-relative):

- `src/gen3d/ai/orchestration.rs`: entrypoints for starting/resuming/polling Gen3D jobs.
- `src/gen3d/ai/pipeline_orchestrator.rs`: deterministic pipeline state machine; starts one tool call at a time.
- `src/gen3d/ai/agent_loop/` + `src/gen3d/ai/agent_prompt.rs` + `src/gen3d/ai/agent_step.rs`: current agent-step orchestration loop (this plan removes it).
- `src/gen3d/ai/agent_tool_dispatch.rs`: executes a tool call by tool id and args (shared by pipeline).
- `src/gen3d/ai/agent_tool_poll.rs`: polls in-flight async tool calls (shared by pipeline).
- `src/config.rs` + `config.example.toml`: Gen3D orchestrator config surface (currently supports `agent` and `pipeline`).
- `docs/gen3d/README.md`: Gen3D workflow doc (currently describes both orchestrators).

Terminology:

- “Tool call”: an internal engine operation identified by `tool_id` with JSON `args` that returns a JSON result (`Gen3dToolResultJsonV1`). Some tools are LLM-backed (plan/component generation), others are deterministic (QA, apply_draft_ops, rendering).
- “Agent-step”: an LLM call whose *output* is a `gen3d_agent_step_v1` JSON object containing an ordered list of tool calls to execute.
- “Pipeline”: an engine-driven state machine that selects the next tool call based on explicit job state and prior tool results.

## Plan of Work

1. Config surface cleanup (`src/config.rs`, `config.example.toml`)

   Make pipeline the only supported orchestrator:

   - Remove `Gen3dOrchestrator` and `AppConfig.gen3d_orchestrator`.
   - Delete `parse_gen3d_orchestrator*` and associated tests.
   - Update `config.example.toml` to no longer advertise an `agent` option.

2. Remove runtime agent-step orchestration (keep tool runtime)

   - Delete the `Gen3dAiMode::Agent` mode. The job should always run as pipeline mode.
   - Remove the “agent-step request” code paths from `src/gen3d/ai/orchestration.rs` and `src/gen3d/ai/mod.rs` module list.
   - Remove `agent_prompt.rs`, `agent_parsing.rs`, and the `poll_agent_step(...)` loop from `agent_step.rs`.
   - Keep the shared helpers used by pipeline:
     - finish-run sequence
     - tool execution wrappers/outcome types
     - render capture/pass snapshot/descriptor meta polling

   If necessary, split `agent_step.rs` into a smaller “tool runtime” module and delete only the agent-step orchestrator portions.

3. Structured outputs + mock backend cleanup

   - Remove `Gen3dAiJsonSchemaKind::AgentStepV1` and its schema definition from `structured_outputs.rs`.
   - Remove the OpenAI mock backend branch that generates `agent_step` payloads (artifact prefix `"agent_step"`).

4. Docs updates

   - Update `docs/gen3d/README.md` to describe a single orchestrator (pipeline) and remove “agent-step” sections and agent-step-only helper tools.
   - Keep `README.md` clean; put details under `docs/`.

5. Validation

   From repo root:

   - `cargo test`
   - Rendered smoke test:
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

6. Commit

   Commit all changes with a message like:

   - `gen3d: remove agent orchestrator; pipeline-only`

## Concrete Steps

As implementation proceeds, capture the exact command transcripts here (short) so a novice can compare expected output.

## Validation and Acceptance

Acceptance criteria:

- The codebase compiles.
- `cargo test` passes.
- The rendered smoke test starts and exits without crash.
- Gen3D can be started from the UI (Build Preview scene) and progresses via pipeline stages.
- No code path can issue an `agent_step` LLM request (no prompt building/parsing for `gen3d_agent_step_v1`).

## Idempotence and Recovery

- The test commands are safe to re-run.
- If the rendered smoke test fails after changes, revert to the last passing commit and re-apply changes in smaller steps.

## Artifacts and Notes

(Populate with key diffs/transcripts as work proceeds.)

