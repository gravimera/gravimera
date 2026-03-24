# Gen3D: Seeded-edit “Edit Strategy” router (pipeline-only)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Seeded Gen3D Edit/Fork sessions (“edit sessions”) are currently pipeline-only, but the seeded-edit pipeline is still somewhat “one size fits all”: it always runs preserve-mode PlanOps and captures part snapshots for every component before asking the model for DraftOps. For large plans this wastes tokens, and because the DraftOps prompt truncates to 16 snapshots, it can also hide relevant parts from the model.

After this change, seeded edit sessions start with a small, schema-constrained LLM step that selects an **edit strategy** and a **snapshot scope**:

- The deterministic pipeline remains the only orchestrator (no agent-step loop).
- The model never chooses tool sequences; it only returns a small JSON decision (`strategy`, `snapshot_components`, `reason`).
- The pipeline uses that decision to:
  - optionally skip PlanOps when the edit can be satisfied via DraftOps alone, and
  - capture component part snapshots only for the components needed by DraftOps, then call `llm_generate_draft_ops_v1` with `scope_components` to match.

How to see it working:

1. Run a seeded edit on a mock warcar plan with prompt “Make the cannon longer.”.
2. Observe in `tool_calls.jsonl` that the pipeline calls `llm_select_edit_strategy_v1`, captures parts only for `cannon`, and calls `llm_generate_draft_ops_v1` with `scope_components=["cannon"]`.
3. Validate that the edit run still applies DraftOps and increments `assembly_rev`, and does not regenerate components by default.


## Progress

- [x] (2026-03-25) Write the ExecPlan and identify the touched modules.
- [x] Add the `llm_select_edit_strategy_v1` tool id + structured-output schema (`gen3d_edit_strategy_v1`).
- [x] Implement prompts for the new tool (short, contract-first, no heuristics).
- [x] Implement tool dispatch + tool polling parse/validation + mock backend response.
- [x] Integrate the new pipeline stage and snapshot scoping (seeded edit path only).
- [x] Update Gen3D docs to include the new stage and the meaning of `scope_components`.
- [x] (2026-03-25) Validate with `cargo test`, rendered smoke test, and Automation HTTP API runs (mock and real when credentials exist).


## Surprises & Discoveries

- Observation: DraftOps prompt truncates component snapshots to 16 entries.
  Evidence: `src/gen3d/ai/prompts.rs::build_gen3d_draft_ops_user_text` iterates `component_parts_snapshots.iter().take(16)` and emits a truncation note when more exist.
- Note: The mock `llm_select_edit_strategy_v1` parser initially failed to detect the component-name list due to a leading newline after the marker; fixed by `trim_start()` before scanning.


## Decision Log

- Decision: Keep orchestration pipeline-only; add a bounded “router” tool that outputs only a strategy + scope JSON.
  Rationale: This preserves debuggability and token discipline while allowing seeded-edit flows to branch without an agent-step loop.
  Date/Author: 2026-03-25 / user + assistant

- Decision: Keep the initial strategy set small (`draft_ops_only`, `plan_ops_then_draft_ops`, `plan_ops_only`, `rebuild`) and treat everything else as `unknown`.
  Rationale: Smaller schema reduces model error rate and makes pipeline branching predictable.
  Date/Author: 2026-03-25 / assistant


## Outcomes & Retrospective

- Seeded edit sessions now begin with `llm_select_edit_strategy_v1` and can scope part snapshots + DraftOps to only the needed components.
- Deterministic pipeline remains the only orchestrator; the router tool only returns a small JSON decision.
- Validation (2026-03-25):
  - Unit tests: `cargo test -q`
  - Rendered smoke test: `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
  - Automation HTTP API (rendered, mock://gen3d):
    - `python3 test/run_1/gen3d_tasks_queue_api/run.py`
    - `python3 test/run_1/gen3d_tasks_queue_seeded_api/run.py`
    - `python3 test/run_1/gen3d_edit_rerun_api/run.py`


## Context and Orientation

Key modules:

- Tool ids: `src/gen3d/agent/tools.rs`
- Pipeline stage machine: `src/gen3d/ai/pipeline_orchestrator.rs`
- Pipeline state enum/struct: `src/gen3d/ai/job.rs` (`Gen3dPipelineStage`, `Gen3dPipelineState`)
- LLM tool dispatch (build prompts, spawn async requests): `src/gen3d/ai/agent_tool_dispatch.rs`
- LLM tool polling (parse JSON and apply to draft/job): `src/gen3d/ai/agent_tool_poll.rs`
- Structured outputs JSON Schema definitions: `src/gen3d/ai/structured_outputs.rs`
- Rust deserialization structs for tool outputs: `src/gen3d/ai/schema.rs`
- Mock offline backend (tests): `src/gen3d/ai/openai.rs` (`mock_generate_text_via_openai`)
- UI status log labels: `src/gen3d/ai/status_steps.rs`
- Gen3D docs: `docs/gen3d/README.md`, `docs/gen3d/pipeline_walkthrough.md`

Definitions:

- “Seeded edit session”: a Gen3D run started from an existing Gen3D-saved prefab (Edit/Fork). In-code this is typically `job.edit_base_prefab_id.is_some()`.
- “Preserve mode”: `job.preserve_existing_components_mode=true`, meaning already-generated components are not regenerated by default.
- “DraftOps”: a list of deterministic primitive/attachment/animation edits applied atomically by `apply_draft_ops_v1`.
- “Parts snapshot”: the deterministic per-component edit interface produced by `query_component_parts_v1` and stored in Info Store under a workspace-scoped key. `llm_generate_draft_ops_v1` requires these snapshots.


## Plan of Work

1. Add a new LLM-backed tool:
   - Tool id: `llm_select_edit_strategy_v1`.
   - Output JSON schema: `gen3d_edit_strategy_v1` with:
     - `version: 1`
     - `strategy: "draft_ops_only" | "plan_ops_then_draft_ops" | "plan_ops_only" | "rebuild"`
     - `snapshot_components: [ "<component_name>", ... ]`
     - `reason: "<short sentence>"`

2. Add prompt builders (short and rule-focused) for the router tool in `src/gen3d/ai/prompts.rs`.

3. Implement tool dispatch in `src/gen3d/ai/agent_tool_dispatch.rs` to spawn the request with structured outputs enabled.

4. Implement tool polling parsing in `src/gen3d/ai/agent_tool_poll.rs`:
   - Deserialize to `AiEditStrategyJsonV1`.
   - Validate that `snapshot_components` only references known component names (or return an actionable error + schedule one schema-repair retry).

5. Integrate pipeline:
   - Add `Gen3dPipelineStage::EditSelectStrategy`.
   - On seeded edit runs in preserve mode, start at `EditSelectStrategy` instead of `EditPlanTemplate`.
   - Store the decision in `Gen3dPipelineState` (strategy + snapshot scope).
   - Update `EditQueryComponentParts` and `EditSuggestDraftOps` to use the stored scope:
     - Query snapshots only for those components.
     - Call `llm_generate_draft_ops_v1` with `scope_components` matching the snapshot list.
   - Branch:
     - `draft_ops_only`: skip PlanOps and go straight to EnsureComponents → DraftOps flow.
     - `plan_ops_then_draft_ops`: run PlanOps then DraftOps.
     - `plan_ops_only`: run PlanOps then QA (skip DraftOps).
     - `rebuild`: disable preserve mode and route to `CreatePlan` (full rebuild).

6. Update docs in `docs/gen3d/pipeline_walkthrough.md` to list the new stage and explain snapshot scoping.

7. Validation:
   - Unit tests: update/extend `src/gen3d/ai/pipeline_orchestrator_tests.rs` to assert:
     - new tool is called in seeded edit runs, and
     - draft ops is scoped (args include `scope_components`).
   - Run `cargo test -q`.
   - Run rendered smoke test (per `AGENTS.md`):
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
   - Run Automation HTTP API scripts (mock, plus real provider if local config has credentials).


## Concrete Steps

All commands run from the repo root.

1. Unit tests:

   - `cargo test -q`

2. Rendered smoke test:

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

3. Automation API (mock) sample:

   - `python3 test/run_1/gen3d_tasks_queue_api/run.py`


## Validation and Acceptance

Acceptance criteria:

- Seeded edit sessions in preserve mode call `llm_select_edit_strategy_v1` once near the start of the run.
- For edits that only need DraftOps, the pipeline:
  - does not run PlanOps, and
  - captures part snapshots only for the scoped components, and
  - calls `llm_generate_draft_ops_v1` with matching `scope_components`.
- `cargo test -q` passes.
- Rendered smoke test starts and exits cleanly.


## Idempotence and Recovery

- The new router tool is read-only with respect to the draft; it only returns a decision. If it fails, the pipeline should stop with an actionable error; rerunning the build should be safe.
- If `snapshot_components` is invalid, the tool should emit an error that tells the user to retry (and the engine performs at most one repair retry).


## Artifacts and Notes

- New tool call artifacts are written under the pass directory as:
  - `tool_edit_strategy_<call_id>_system_text.txt`
  - `tool_edit_strategy_<call_id>_user_text.txt`
  - `tool_edit_strategy_<call_id>_mock.txt` in mock mode


## Interfaces and Dependencies

- No new external dependencies.
- New tool id constant added to `crate::gen3d::agent::tools`.
- New structured output schema kind added to `crate::gen3d::ai::structured_outputs`.
- New deserialization structs added to `crate::gen3d::ai::schema`.
