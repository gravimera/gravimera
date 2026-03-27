# Gen3D pipeline walkthrough (tools + prompts)

Gen3D is **pipeline-only** now: a deterministic state machine drives the build/edit flow and calls a fixed set of tools (some deterministic, some LLM-backed). There is **no “agent-step” orchestrator** that decides which tool to call next.

If you want the most concrete view of “what happened in a run”, look at the run artifacts:

- `tool_calls.jsonl` / `tool_results.jsonl` (exact tool ids + args + results)
- `*_system_text.txt` / `*_user_text.txt` (exact LLM prompts per tool call, when the backend is OpenAI-compatible)

This doc explains the whole picture and shows where to find each prompt.

## Mental model

**Entry point (game loop)**

- The Bevy system `gen3d_poll_ai_job` runs each frame and advances the current Gen3D run.
- It enforces hard budgets (time + tokens) and then delegates orchestration to the pipeline:
  - `src/gen3d/ai/orchestration.rs` (`gen3d_poll_ai_job(...)`)
  - `src/gen3d/ai/pipeline_orchestrator.rs` (`poll_gen3d_pipeline(...)`)

**Two layers of “state”**

1. **Pipeline stage** (deterministic “what should happen next”):
   - `src/gen3d/ai/job.rs` (`Gen3dPipelineStage`, `Gen3dPipelineState`)
2. **Async phase** (what we’re currently waiting for):
   - `src/gen3d/ai/job.rs` (`Gen3dAiPhase`)
   - Examples: waiting for an LLM response, waiting for a render capture to finish, etc.

**Tools**

The pipeline issues “tool calls” by creating a `Gen3dToolCallJsonV1` and executing it via:

- `src/gen3d/ai/pipeline_orchestrator.rs` (`start_pipeline_tool_call(...)`)
- `src/gen3d/ai/agent_tool_dispatch.rs` (`execute_tool_call(...)`)
- `src/gen3d/ai/agent_tool_poll.rs` (`poll_agent_tool(...)`) — finishes async tool work and records the final `Gen3dToolResultJsonV1`.

Even though these live under `agent_*` modules, they are simply the **tool runtime** now (shared plumbing for LLM calls, artifacts, structured-output enforcement, etc.).

## What happens in a run (whole-picture flow)

At a high level, there are two common flows:

### A) Create session (“Build”: new object)

Pipeline stages (simplified):

1. `CreatePlan`
   - Tool: `llm_generate_plan_v1`
2. `EnsureComponents`
   - Tool: `llm_generate_components_v1` (missing-only) until every planned component has `actual_size`
   - Tool: `apply_reuse_groups_v1` (deterministic) to copy/mirror component geometry per the plan’s `reuse_groups` once the reuse source exists
   - Important: `reuse_groups` does NOT create components. Every `reuse_groups.source` and `reuse_groups.targets[]` entry must also exist as a component `name` in the plan’s `components[]` (the engine rejects plans that reference missing components).
   - Note: For `reuse_groups`, prefer omitting `anchors` (default is `preserve_interfaces`). Avoid `anchors=preserve_target` for `copy_component_subtree` reuse groups; it can keep internal join anchors unchanged and drift descendant attachments.
3. `Qa`
   - Tool: `qa_v1`
   - If QA provides deterministic “fixits”, pipeline applies them using `apply_draft_ops_v1` and re-runs QA.
   - If QA returns non-fatal `complaints[]` (quality hints), pipeline spends a *second chance* on improvement when possible:
     - Motion complaints → re-run `llm_generate_motions_v1` once with `qa_feedback` attached, then re-run QA.
     - Plan complaints are surfaced as hints but do not currently trigger an automatic plan retry.
   - If motion channels are missing for a movable unit, pipeline calls `llm_generate_motions_v1` (with a schema reminder + prior failures as `qa_feedback`) and re-runs QA.
4. Optional appearance review (if enabled)
   - Tools: `render_preview_v1` → `llm_review_delta_v1`
   - Then loop back to `EnsureComponents` (review-delta can request regen/replan).
5. `Finish`

The actual stage machine is in `src/gen3d/ai/pipeline_orchestrator.rs` (match on `job.pipeline.stage`).

### B) Seeded Edit/Fork session (“Edit”: preserve-mode edits)

Pipeline stages (simplified):

1. `EditSelectStrategy`
   - Tool: `llm_select_edit_strategy_v1` (choose strategy + DraftOps snapshot scope)
2. Optional plan patch
   - Stages: `EditPlanTemplate` → `EditPlanOps`
   - Tools: `get_plan_template_v1` → `llm_generate_plan_ops_v1`
   - Skipped when strategy is `draft_ops_only`
3. `EnsureComponents`
   - Tool: `llm_generate_components_v1` (missing-only)
4. DraftOps-first in-place edits:
   - `EditQueryComponentParts` (tool: `query_component_parts_v1` for each scoped component)
   - `EditSuggestDraftOps` (tool: `llm_generate_draft_ops_v1` with `scope_components`)
   - `EditApplyDraftOps` (tool: `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev=<current>`)
5. `Qa` → optional render/review-delta → `Finish`

## Which tools are called (and the args the pipeline uses)

This list matches the deterministic calls in `src/gen3d/ai/pipeline_orchestrator.rs`:

- Planning
  - `llm_generate_plan_v1`
    - Args (create): `{ "prompt": "<user prompt>", "qa_feedback": "<optional QA complaints text>" }`
    - Args (preserve replan): `{ "prompt": "...", "plan_template_kv": {...}, "constraints": { "preserve_existing_components": true, "preserve_edit_policy": "allow_offsets" } }`
  - `llm_select_edit_strategy_v1`
    - Args: `{ "prompt": "<edit prompt>" }`
  - `get_plan_template_v1`
    - Args: `{ "version": 2, "mode": "auto" }`
  - `llm_generate_plan_ops_v1`
    - Args: `{ "prompt": "...", "plan_template_kv": {...}, "constraints": { "preserve_existing_components": true, "preserve_edit_policy": "allow_offsets" }, "max_ops": 32 }`

- Component drafting (primitives generation)
  - `llm_generate_components_v1`
    - Args (missing-only): `{ "missing_only": true }`
    - Args (forced regen): `{ "component_indices": [0,2,3], "force": true }`
  - `apply_reuse_groups_v1`
    - Args (pipeline): `{ "version": 1 }`

- Seeded edit: part snapshots + DraftOps
  - `query_component_parts_v1`
    - Args: `{ "component": "<component name>", "max_parts": 128 }`
  - `llm_generate_draft_ops_v1`
    - Args: `{ "prompt": "<edit prompt>", "scope_components": ["..."], "max_ops": 24, "strategy": "conservative" }`
      - If `scope_components=[]` (or omitted), the tool defaults to “all components”, which is more expensive and may truncate snapshots.
  - `apply_draft_ops_v1`
    - Args (pipeline): `{ "version": 1, "atomic": true, "if_assembly_rev": <u32>, "ops": [...] }`

- QA / remediation
  - `qa_v1`
    - Args: `{}`
  - `llm_generate_motions_v1`
    - Args: `{ "channels": ["move","action"], "qa_feedback": "<optional QA complaints text>" }` (and `attack` when needed)

- Appearance review
  - `render_preview_v1`
    - Args: `{ "views": ["front","left_back","right_back","top","bottom"], "image_size": 768, "prefix": "pipeline_review", "include_motion_sheets": false }`
  - `llm_review_delta_v1`
    - Args: `{}` or `{ "preview_blob_ids": ["..."] }` (when `render_preview_v1` returned blob ids)

## Where the LLM prompts come from

Each LLM-backed tool has:

- **System instructions**: a long-lived “contract + rules”
- **User text**: run-specific context (prompt + summaries + plan excerpts + constraints)

Prompt builders live in `src/gen3d/ai/prompts.rs`.

The tool implementations build prompts here:

- Plan: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_PLAN`)
  - System: `build_gen3d_plan_system_instructions()`
  - User: `build_gen3d_plan_user_text_with_hints(...)` or `build_gen3d_plan_user_text_preserve_existing_components(...)`
- Plan ops: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_PLAN_OPS`)
  - System: `build_gen3d_plan_ops_system_instructions()`
  - User: `build_gen3d_plan_ops_user_text(...)`
- Components (single): `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_COMPONENT`)
  - System: `build_gen3d_component_system_instructions()`
  - User: `build_gen3d_component_user_text(...)`
- Components (batch): `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_COMPONENTS`)
  - Internally spawns per-component calls using the same component prompt builders.
- DraftOps: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_DRAFT_OPS`)
  - System: `build_gen3d_draft_ops_system_instructions()`
  - User: `build_gen3d_draft_ops_user_text(...)`
- Edit strategy: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_SELECT_EDIT_STRATEGY`)
  - System: `build_gen3d_edit_strategy_system_instructions()`
  - User: `build_gen3d_edit_strategy_user_text(...)`
- Motions: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_MOTIONS`)
  - System: `build_gen3d_motion_authoring_system_instructions()`
  - User: `build_gen3d_motion_authoring_user_text(...)`
- Review delta: `src/gen3d/ai/agent_review_delta.rs` (`start_agent_llm_review_delta_call(...)`)
  - System: `build_gen3d_review_delta_system_instructions(...)`
  - User: `build_gen3d_review_delta_user_text(...)`

Structured outputs:

- Expected JSON schemas are defined via `src/gen3d/ai/structured_outputs.rs` (`Gen3dAiJsonSchemaKind`)
- The engine enforces structured outputs (and retries on truncated streams) in `src/gen3d/ai/ai_service.rs`.

## Where to find the exact prompts used in a run (artifacts)

For OpenAI-compatible backends (OpenAI/MiMo), the engine writes the final system + user prompt texts under the tool call’s **step artifact dir**:

- `<run_id>/attempt_N/steps/step_####/<artifact_prefix>_system_text.txt`
- `<run_id>/attempt_N/steps/step_####/<artifact_prefix>_user_text.txt`

The `artifact_prefix` is derived from the tool call id. Example prefixes:

- Plan: `tool_plan_<call_id>`
- Component i: `tool_component<i>_<call_id>`
- Motion: `tool_motion_<channel>_<call_id>`
- Review delta: `tool_review_<call_id>`
- DraftOps: `tool_draft_ops_<call_id>`
- Edit strategy: `tool_edit_strategy_<call_id>`

Implementation reference:

- Prompt persistence: `src/gen3d/ai/openai.rs` (writes `*_system_text.txt` + `*_user_text.txt`)
- Tool call args persistence: `src/gen3d/ai/pipeline_orchestrator.rs` (writes `tool_calls.jsonl` / `tool_results.jsonl` into each step dir)

## Concrete example you can run locally (mock backend)

There are end-to-end tests that run the entire pipeline using the deterministic `mock://gen3d` backend:

- Create session: `gen3d_mock_pipeline_builds_warcar_prompt_end_to_end`
- Seeded edit: `gen3d_mock_pipeline_seeded_edit_prefers_draft_ops_and_does_not_regen`

Run one test:

```bash
cargo test gen3d_mock_pipeline_builds_warcar_prompt_end_to_end -q
```

It writes a temporary run dir under your OS temp folder (see `src/gen3d/ai/pipeline_orchestrator_tests.rs::make_temp_gen3d_run_dir`).

Inside that run dir, open:

- `agent_trace.jsonl` to see the tool sequence (call id + tool id + args/results)
- `attempt_0/steps/step_*/tool_calls.jsonl` to see per-step tool calls (one entry per step dir)
- For the plan prompt, find the step dir whose `tool_calls.jsonl` contains `"tool_id":"llm_generate_plan_v1"`, then open `tool_plan_*_system_text.txt` / `tool_plan_*_user_text.txt`
- For per-component prompts, open the step dirs whose `tool_calls.jsonl` contain `"tool_id":"llm_generate_component_v1"` / `"tool_id":"llm_generate_components_v1"` and inspect `tool_component*_system_text.txt` / `tool_component*_user_text.txt`
