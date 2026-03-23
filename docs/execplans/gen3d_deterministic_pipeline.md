# Gen3D deterministic pipeline orchestrator (DraftOps-first) with agent-step fallback

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D “Build” is currently orchestrated by a Codex-style `agent_step` loop: the model decides which tool to call next (plan, component generation, QA, review, etc.). That makes sequencing slow (extra LLM turns), fragile (wrong next step / malformed tool args), and hard to test.

After this change, Gen3D will have a deterministic **pipeline orchestrator** that manages the fixed workflow as a state machine. The model will still be used for the inherently open-ended parts (plan generation, component drafts, plan ops, review delta, and DraftOps suggestions), but the *process management* (what happens next, retries, gating, budgets, stopping conditions) will be deterministic and testable.

In this plan, “deterministic” means: given the same in-memory job state + the same tool results, the engine makes the same decision about what to do next. It does **not** mean the LLM always emits identical text. LLM-backed tools are treated as suggestion producers that must conform to strict schemas, and their effects are applied by deterministic code paths (`plan_ops`, `draft_ops`, `convert::apply_ai_review_delta_actions`, validation/QA). Pipeline decisions must be based on explicit state/tool outputs (counters, hashes, `qa_v1` fields), not object-type heuristics (“chairs vs snakes”).

We will keep the existing `agent_step` loop as a **fallback**. When the pipeline cannot make progress (tool schema failures beyond repair budget, repeated atomic DraftOps rejections, etc.), it will switch to agent-step and continue the run using the current implementation.

The most important product requirement is **DraftOps-first editing**: in seeded Edit/Fork sessions, user requests like “make the cannon longer” should prefer in-place primitive edits (`apply_draft_ops_v1`) rather than regenerating whole components. This requires a new LLM-backed tool that *suggests* DraftOps deterministically (strict schema), then the engine applies them via `apply_draft_ops_v1` (atomic + revision-gated).

How to see it working (after implementation):

1. Start the game and enter Gen3D Workshop (Build Preview scene).
2. Build a new object from a text prompt. Observe status text like “Pipeline: planning → generating components → QA → review”.
3. Seed an Edit/Fork session from a Gen3D-saved prefab, enter an edit prompt like “make the barrel longer and darken it”, click Build, and observe that the run performs DraftOps edits (with diffs in artifacts) instead of component regeneration.
4. Intentionally trigger a failure (e.g., mock backend returning invalid JSON) and observe that the run switches to agent-step fallback (status includes a clear reason).


## Progress

- [x] (2026-03-18 22:10 CST) Drafted this ExecPlan.
- [x] (2026-03-18 23:07 CST) Revised this ExecPlan to call out pipeline reachability (all entrypoints), add a deterministic QA/motion remediation loop, tighten tool contracts/gates, and note mock-backend gaps.
- [x] (2026-03-18 23:40 CST) Implemented pipeline orchestrator skeleton + config toggle (pipeline currently falls back to agent-step; deterministic pipeline logic comes next).
- [x] (2026-03-19 01:35 CST) Added new LLM tool: `llm_generate_draft_ops_v1` (suggestions only; no mutation) + strict structured-output schema + engine-side validation.
- [x] (2026-03-19 01:35 CST) Implemented create-session pipeline flow (new Build): plan → components → QA → (render/review) loops → finish.
- [x] (2026-03-19 01:35 CST) Implemented edit-session pipeline flow (seeded Edit/Fork): plan-ops → DraftOps suggest+apply (atomic) → QA → (render/review) loops → finish.
- [x] (2026-03-19 01:35 CST) Implemented deterministic fallback to agent-step (bounded retries; explicit status + Info Store event).
- [x] (2026-03-19 01:35 CST) Added mock backend responses + offline tests so the pipeline is regression-tested without network.
- [x] (2026-03-19 01:35 CST) Updated docs (`docs/gen3d/README.md`) to describe pipeline mode + fallback and the new DraftOps tool.
- [x] (2026-03-19 01:36 CST) Ran `cargo test`.
- [x] (2026-03-19 01:48 CST) Ran the rendered smoke test (`cargo run -- --rendered-seconds 2`).


## Surprises & Discoveries

- Observation: The repo already has a “legacy” non-agent pipeline code path in `src/gen3d/ai/orchestration.rs` (`WaitingPlan`, `WaitingComponent`, `CapturingReview`, `WaitingReview`), but `gen3d_start_build_from_api` currently always sets `job.mode = Gen3dAiMode::Agent`, so that path is not used for normal runs.
  Evidence: `src/gen3d/ai/orchestration.rs::gen3d_start_build_from_api` sets `job.mode = Gen3dAiMode::Agent` and `gen3d_poll_ai_job` dispatches to `agent_loop::poll_gen3d_agent` when in Agent mode.

- Observation: `llm_review_delta_v1` cannot directly edit primitives; it can tweak attachments/anchors/transforms/mobility/attack and can request regeneration or replan, but not “edit part X color/scale”.
  Evidence: `src/gen3d/ai/schema.rs::AiReviewDeltaActionJsonV1` and `src/gen3d/ai/convert.rs::apply_ai_review_delta_actions`.

- Observation: The deterministic primitive-edit mechanism already exists as `apply_draft_ops_v1`, and it returns actionable diffs (`diff_summary`, `applied_ops`, `rejected_ops`) and supports atomic application + `if_assembly_rev` gating.
  Evidence: `src/gen3d/ai/draft_ops.rs::apply_draft_ops_v1`.

- Observation: `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md` are referenced in `AGENTS.md` but are not present in this working tree (directory exists, files missing). This plan therefore embeds the needed “contract-first” tool guidance instead of referencing those docs.

- Observation: Pipeline mode is currently not reachable because all session entrypoints force agent mode, and `Gen3dAiMode` currently only contains `Agent`.
  Evidence: `src/gen3d/ai/orchestration.rs::gen3d_start_build_from_api`, `src/gen3d/ai/orchestration.rs::gen3d_resume_build_from_api`, and `src/gen3d/ai/orchestration.rs::gen3d_start_seeded_session_from_prefab_id_from_api` set `job.mode = Gen3dAiMode::Agent`. Also `src/gen3d/ai/job.rs::Gen3dAiMode` currently has only the `Agent` variant.
  Update: Resolved (2026-03-18) by introducing `[gen3d].orchestrator = "pipeline"` and dispatching `gen3d_poll_ai_job` to the pipeline orchestrator when enabled.

- Observation: `mock://gen3d` currently has no mock responses for `tool_plan_ops_*` and `tool_review_*` artifact prefixes, and will error if tests/pipeline try to call those tools without extending the mock backend.
  Evidence: `src/gen3d/ai/openai.rs::mock_gen3d_response_text` handles `tool_plan_`, `tool_component`, and `tool_motion_...`, but returns `mock://gen3d has no response for artifact_prefix ...` for other prefixes.
  Update: Resolved (2026-03-19) by adding mock responses for `tool_plan_ops_*`, `tool_review_*`, and `tool_draft_ops_*`.


## Decision Log

- Decision: The default orchestrator will be a deterministic pipeline state machine; `agent_step` remains available as a fallback path.
  Rationale: Deterministic sequencing reduces LLM turns and prevents many classes of agent mistakes, while fallback preserves maximum capability for edge cases.
  Date/Author: 2026-03-18 / assistant + user

- Decision: Editing will be DraftOps-first and will require adding `llm_generate_draft_ops_v1` (suggestions-only) rather than expanding `llm_review_delta_v1`.
  Rationale: Review-delta’s schema is intentionally high-level and does not cover primitive part edits. DraftOps are already implemented as a deterministic patch language (`apply_draft_ops_v1`) with good diffs; we need an LLM tool that outputs this patch language under a strict schema.
  Date/Author: 2026-03-18 / assistant + user

- Decision: No “AskUserClarification” state will exist in the pipeline.
  Rationale: Explicit user requirement. Ambiguity will be handled by bounded best-effort attempts, LLMRepair, and then agent-step fallback or best-effort stop.
  Date/Author: 2026-03-18 / assistant + user

- Decision: Avoid object-type heuristics in pipeline logic; transitions must be based on explicit tool outputs/state flags, not special-casing “chairs vs snakes”.
  Rationale: Gen3D must be generic (“generate any object”); process management must not encode per-object heuristics.
  Date/Author: 2026-03-18 / assistant

- Decision: Make QA failure handling explicit and deterministic in the pipeline: apply deterministic QA fixits first (when provided), then attempt motion authoring if motion is the blocker, then use review-delta for replan/regen, and only then fall back to agent-step.
  Rationale: Without a defined QA remediation loop, the pipeline will either stall or fall back immediately, defeating the purpose. Using `qa_v1` outputs (including capability-gaps fixits) keeps the logic generic and testable.
  Date/Author: 2026-03-18 / assistant

- Decision: When appearance review is enabled, have the pipeline call `render_preview_v1` explicitly and pass the resulting blob ids into `llm_review_delta_v1`, instead of relying on the tool’s internal “prerender if missing” behavior.
  Rationale: It keeps pipeline sequencing observable (“render” is a first-class step), makes artifacts predictable in tests, and avoids hidden branching inside `llm_review_delta_v1` dispatch.
  Date/Author: 2026-03-18 / assistant


## Outcomes & Retrospective

- Outcome: Gen3D can now run in a deterministic pipeline mode (engine-driven state machine) with bounded retries and explicit fallback to agent-step.
- Outcome: Seeded Edit/Fork runs are DraftOps-first via a new schema-constrained tool `llm_generate_draft_ops_v1` (suggestions-only) + deterministic application via `apply_draft_ops_v1` (atomic + `if_assembly_rev`).
- Outcome: Offline regression coverage exists for create + seeded edit flows, plus a forced-failure case that triggers pipeline fallback to agent-step on persistent DraftOps schema failures (mock backend marker).
- Outcome: Rendered smoke run starts and exits cleanly (`--rendered-seconds 2`).


## Context and Orientation

Gen3D lives under `src/gen3d/*`. The relevant parts for orchestration are:

- `src/gen3d/ai/orchestration.rs`: top-level Gen3D “Build” start/resume, budgets, and the legacy (non-agent) pipeline phases.
- `src/gen3d/ai/agent_loop/mod.rs`: current tool-driven `agent_step` polling loop.
- `src/gen3d/ai/agent_step.rs`: parses `gen3d_agent_step_v1`, executes actions, and auto-requests the next agent step.
- `src/gen3d/ai/agent_tool_dispatch.rs`: executes one tool call (deterministic tools + LLM-backed tools). This is where `llm_generate_plan_v1`, `llm_generate_components_v1`, etc. are spawned.
- `src/gen3d/ai/agent_tool_poll.rs`: polls in-flight tool calls, parses structured outputs, runs LLMRepair on schema errors, and applies mutations for LLM-backed tools (plan/component/review/motion).
- `src/gen3d/ai/draft_ops.rs`: deterministic “patch language” for primitive edits (`apply_draft_ops_v1`) and component inspection (`query_component_parts_v1`).
- `src/gen3d/ai/plan_ops.rs`: deterministic “patch language” for plan edits (`apply_plan_ops_v1`) and its schema.
- `src/gen3d/agent/tools.rs`: tool registry and tool ids shown to the agent.
- `docs/gen3d/README.md`: Gen3D workflow + tool contracts doc.

Definitions used in this plan:

- “Agent-step”: the LLM call that returns a `gen3d_agent_step_v1` JSON object deciding which tools to call next (`src/gen3d/agent/protocol.rs`).
- “Pipeline orchestrator”: an engine-driven state machine that decides the next tool call deterministically.
- “LLM-backed tool”: a tool whose execution spawns an LLM request with a strict JSON schema (examples: `llm_generate_plan_v1`, `llm_generate_components_v1`, `llm_generate_plan_ops_v1`, `llm_review_delta_v1`).
- “DraftOps”: the deterministic patch format consumed by `apply_draft_ops_v1` (primitive edits, anchor/attachment edits, animation-slot edits).
- “PlanOps”: the deterministic patch format consumed by `apply_plan_ops_v1` (add/remove components, anchors, attachments, reuse groups, etc.).


## Plan of Work

### 1) Add a pipeline orchestrator mode without deleting the agent path

Introduce a new run mode (for example `Gen3dAiMode::Pipeline`) and a small pipeline state struct stored on `Gen3dAiJob`. The pipeline’s job is to decide “what tool to run next” based on current job state and the most recent tool result; it must never call `spawn_agent_step_request`.

The agent path must remain unchanged and reachable:

- Config toggle: add a config knob (e.g. `[gen3d].orchestrator = "pipeline"|"agent"`) so developers can force agent mode for comparison/debugging.
- Fallback: pipeline can switch `job.mode` to agent mode mid-run and continue by entering `Gen3dAiPhase::AgentWaitingStep` and calling `spawn_agent_step_request`.

Pipeline must be reachable from every entrypoint, not only from polling. Today, all of the following overwrite `job.mode` to `Agent`, which would silently disable pipeline mode unless they are updated to respect the orchestrator config:

- `src/gen3d/ai/orchestration.rs::gen3d_start_build_from_api` (new Build)
- `src/gen3d/ai/orchestration.rs::gen3d_resume_build_from_api` (Continue)
- `src/gen3d/ai/orchestration.rs::gen3d_start_seeded_session_from_prefab_id_from_api` (Edit/Fork seed/reset)

Keep the tool execution machinery shared. The pipeline should reuse `agent_tool_dispatch::execute_tool_call` and `agent_tool_poll::poll_agent_tool` rather than re-implementing LLM spawning, structured-output repair, artifact logging, or regen/QA gates.

Important integration detail (fixes a real conflict with the current agent implementation): `poll_agent_tool` and the agent execution path both append tool results into `job.agent.step_tool_results`, which is intended to be “recent tool results for the next agent_step prompt”. In pipeline mode we must avoid unbounded accumulation and we must keep agent fallback clean. Implement the pipeline so it treats each pipeline action as “one tool call at a time” and does the following bookkeeping:

- Before starting a pipeline tool call, clear `job.agent.step_tool_results` (so there is at most one result after the tool finishes).
- Start the tool call using the same tracing as the agent (trace event, `tool_calls.jsonl`, `job.metrics.note_tool_call_started`, and Info Store `ToolCallStart`). Prefer extracting a helper from `src/gen3d/ai/agent_step.rs` so both orchestrators share the exact same instrumentation.
- When the tool finishes (immediate or async), read the single tool result from `job.agent.step_tool_results.last()`, copy it into pipeline state (for “most recent tool result”), then clear `job.agent.step_tool_results` again.
- Only when switching to agent fallback should `job.agent.step_tool_results` be populated (as part of a normal agent step). The pipeline must switch with `step_tool_results` empty.

Concrete file targets for this milestone (so a novice can start from this plan):

- In `src/gen3d/ai/job.rs`, extend `Gen3dAiMode` with a new variant (name TBD, e.g. `Pipeline`) and add a pipeline state struct on `Gen3dAiJob` (for example: `pipeline: Gen3dPipelineState` containing `phase`, `last_tool_result`, retry counters, and a simple `call_seq` for generating unique `call_id`s).
- In `src/gen3d/ai/orchestration.rs::gen3d_poll_ai_job`, dispatch to a new `pipeline::poll_gen3d_pipeline(...)` function when `job.mode` is pipeline mode.
- Add a new module (suggested path: `src/gen3d/ai/pipeline.rs`) that owns all deterministic “what tool next” logic and never constructs `gen3d_agent_step_v1`.
- In `src/config.rs` and `config.example.toml`, add the config knob used to select the orchestrator (default should remain the current behavior until you are ready to flip it).

Contract-first rule (embedded here): Any new tool must return actionable results and actionable errors, and must enforce its own gatekeeping (validation, budgets, forbidden states) inside the tool implementation. Do not add “agent prompt rules” as the primary enforcement mechanism.

### 2) Implement deterministic “next tool” selection (create flow)

For a fresh Build (non-seeded), the pipeline should repeatedly select the next tool using only explicit state + prior tool outputs. A novice should implement this as a small loop (“pick next action; execute tool; incorporate result; repeat”) with bounded retries and a no-progress guard based on the existing state-hash logic.

The create-flow next-step rules:

- If user images exist and `job.user_image_object_summary` is missing, run the existing image-summary request (the same implementation used by agent mode), then continue. (Do not use agent-step for this.)
- Ensure a plan exists. If no accepted plan exists yet (empty `job.plan_hash` or `job.planned_components` empty), run `llm_generate_plan_v1`.
- Ensure geometry exists. If any planned component has `actual_size.is_none()`, run `llm_generate_components_v1` in missing-only mode until all planned components have sizes.
- Run QA by calling `qa_v1`. This produces explicit `ok/errors/warnings` plus `capability_gaps` (blockers only: severity=`error`, sometimes with deterministic “fixits” already expressed as tool calls). Warn-only motion validation findings are informational and appear only under `warnings` (not as capability gaps).

If QA fails (`qa_v1.ok=false`), the pipeline must not “give up” immediately. Handle QA failure deterministically, in this order:

1. Apply deterministic QA fixits when present. If `qa_v1.capability_gaps[*].fixits[*]` includes entries with `tool_id="apply_draft_ops_v1"`, execute up to a small bounded number of them (for example: max 3 fixits per QA pass, max 2 QA-fix passes per run) and re-run `qa_v1`. This is generic and does not rely on heuristics: it simply applies engine-provided fixes.
2. If motion is still a blocker, run motion authoring. Concretely: if the latest QA payload indicates motion failure (for example `qa_v1.smoke.motion_validation.ok=false`, or a `capability_gaps` entry with kind like `missing_motion_channel` / `motion_validation_error` (severity=`error`)), call `llm_generate_motions_v1` (e.g. `{"channels":["move","action"]}`), then re-run `qa_v1`.
3. If QA is still failing after bounded attempts, call `llm_review_delta_v1` to ask the model for a replan/regen/tweak delta using the current plan + smoke results. Because QA failed, regen will be allowed by existing rules (see `agent_review_delta.rs::regen_allowed`). Apply the returned delta deterministically (using the existing conversion/application code), then loop back to QA.
4. If the pipeline exceeds its explicit retry budgets or hits the no-progress guard, fall back to agent-step with an explicit reason.

If QA succeeds (`qa_v1.ok=true`), optionally run appearance review loops:

- If `review_appearance=true`, call `render_preview_v1` (choose a stable prefix and stable view set), then call `llm_review_delta_v1` with `preview_blob_ids` set to the blob ids created by `render_preview_v1`.
- Apply the resulting review delta deterministically. If it triggers replan or regen, do so (respecting existing regen budgets and QA gates), then loop back to QA.

Finish deterministically when “complete enough” (reuse or extract logic equivalent to `run_complete_enough_for_auto_finish` in `src/gen3d/ai/agent_step.rs`). “Complete enough” must be evaluated based on explicit state: primitive-part count, all components generated, QA ok, motion ok (when applicable), and (when enabled) appearance review completed.

### 3) Implement deterministic “next tool” selection (edit flow, DraftOps-first)

For seeded Edit/Fork sessions (where `job.edit_base_prefab_id.is_some()` and preserve mode defaults to true), the pipeline must preserve as much of the existing geometry as possible and prefer deterministic patch tools (`plan_ops`, then `draft_ops`) over regeneration. Regeneration is still possible, but must respect the existing QA gating and “preserve mode” defaults.

- Always run a preserve-mode plan ops pass first: `get_plan_template_v1` (mode="auto") → `llm_generate_plan_ops_v1`.
  - Note: `llm_generate_plan_ops_v1` already *applies* the generated ops internally (see `plan_ops::apply_llm_generate_plan_ops_v1`). Do not call `apply_plan_ops_v1` separately unless you are intentionally applying a deterministic, non-LLM patch you authored in code.
  - This handles add/remove/rewire/anchor-interface edits deterministically (and yields actionable errors when not possible).
  - If plan ops fails semantically, retry with `inspect_plan_v1` information in the prompt (bounded retries); then fall back to full `llm_generate_plan_v1` with `constraints.preserve_existing_components=true` if needed.
- Generate any missing components created by plan ops.
- DraftOps-first geometry edits:
  - Deterministically capture “editable part interfaces” by calling `query_component_parts_v1` for each component (bounded `max_parts` and no non-primitives by default).
  - Call the new tool `llm_generate_draft_ops_v1` to output a strict DraftOps list (suggestions only).
  - Apply the suggested ops using `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev=job.assembly_rev`.
  - If application rejects ops (non-empty `rejected_ops`), call `llm_generate_draft_ops_v1` again with the rejection payload and the current `query_component_parts_v1` snapshot (LLMRepair-style loop), bounded by a small retry budget.
  - Only when DraftOps cannot satisfy the request should regeneration be used. In preserve mode, regeneration is QA-gated; the pipeline must respect that gate.
- Run QA and remediate failures using the same deterministic QA loop as the create flow (deterministic fixits → motion authoring → review delta → fallback). In edit sessions, ensure that preserve mode remains enabled unless/until QA failure explicitly allows regen (again, matching `regen_allowed` rules).
- If enabled, run render+review-delta loops as in the create flow, passing `preview_blob_ids` explicitly.

### 4) Add the `llm_generate_draft_ops_v1` tool (suggestions-only)

Add a new tool id and contract:

- Tool id: `llm_generate_draft_ops_v1`
- Purpose: Produce a bounded list of DraftOps (`ops`) that can be applied by `apply_draft_ops_v1` to satisfy the user’s edit prompt, using only component/part information supplied by the engine (especially `query_component_parts_v1` output with `part_id_uuid` and `recipes`).
- Must be strict structured output (JSON schema, `additionalProperties=false`).
- Must never mutate state by itself.
- Must be safe by default: limited op count; prefer non-destructive edits (transforms/recolors) and only remove parts when required to satisfy the user request.

Suggested tool args (engine-validated; actionable errors):

- `prompt: string` (the user’s edit request; required)
- `scope_components?: string[]` (optional; if omitted, the tool may consider all components described in the supplied “component parts snapshot” text)
- `max_ops?: number` (default 24, clamp 1..64)
- `strategy?: "conservative"|"balanced"` (optional; influences whether it prefers recolor/scale vs adding/removing primitives)

Suggested tool output schema:

- `{ version: 1, ops: DraftOp[] }`
  - `DraftOp` is the exact op set supported by `apply_draft_ops_v1` (see `src/gen3d/ai/draft_ops.rs::DraftOpJsonV1`), but with fields narrowed to only what the engine supports today (no unknown keys).

Prompting requirements (system + user prompt builders):

- System must demand “JSON only” and must instruct the model to use only part ids present in the provided snapshots.
- User text must include:
  - The effective user prompt.
  - A compact scene graph summary (component list, attachment structure).
  - For each component in scope: the `query_component_parts_v1` snapshot, including part ids and copy/pasteable recipes (bounded).
  - Any relevant guards (atomic apply, `if_assembly_rev` usage, op limits).

Engine-side validation requirements (contract-first): regardless of prompting, the tool handler must enforce safety deterministically. At minimum: clamp `max_ops`, reject unknown keys, reject ops referencing unknown components/part ids, and require `if_assembly_rev` + `atomic=true` on application.

Concrete integration checklist for this tool (to avoid prompt/tool mismatches and “silent” behavior changes):

- Register the tool in `src/gen3d/agent/tools.rs` (`TOOL_ID_LLM_GENERATE_DRAFT_OPS`, descriptor entry with `args_schema` + `args_example`) so agent-step fallback can use it.
- Add a new `Gen3dAgentLlmToolKind` variant (e.g. `GenerateDraftOps`) in `src/gen3d/ai/job.rs`.
- Extend `src/gen3d/ai/structured_outputs.rs` with a new schema kind (e.g. `DraftOpsV1`) that matches the output `{version:1, ops:[...]}` with `additionalProperties=false` everywhere, and wire it through the LLM request as `expected_schema`.
- Implement tool dispatch and polling alongside other LLM-backed tools:
  - `src/gen3d/ai/agent_tool_dispatch.rs`: spawn the LLM request for this tool (artifact prefix like `tool_draft_ops_<call_id>`).
  - `src/gen3d/ai/agent_tool_poll.rs`: parse the structured output and return it as a tool result (do not apply it here; application is via `apply_draft_ops_v1`).
- Extend the mock backend (`mock://gen3d` in `src/gen3d/ai/openai.rs`) to return a small, valid DraftOps payload for `tool_draft_ops_*` prefixes so offline tests can cover the pipeline.

### 5) Deterministic fallback to agent-step

Define clear fallback triggers (no heuristics, only explicit counters / tool outcomes), for example:

- Tool schema repair exceeded for an LLM-backed tool (existing repair budget, similar to `GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS`).
- `llm_generate_draft_ops_v1` suggestions repeatedly fail atomic apply (N attempts) without changing state hash.
- Pipeline loops exceed “no progress” guard (reuse the existing state-hash guard logic from agent mode).

When falling back:

- Record an Info Store event (`InfoEventKindV1::EngineLog` or a new kind) with the reason and the relevant counters.
- Update UI status: “Pipeline fallback → agent-step (reason: …)”.
- Switch `job.mode` and enter agent mode cleanly (clear pipeline-specific pending state; ensure no in-flight tool call is lost).

### 6) Tests and mock backend

Add offline regression coverage so CI doesn’t need network access:

- Extend the mock backend (`mock://gen3d` in `src/gen3d/ai/openai.rs`, and equivalents for Gemini/Claude if required) to return deterministic outputs for:
  - `tool_plan_ops_*` (even if empty ops)
  - `tool_review_*` (a simple “accept” delta)
  - `tool_draft_ops_*` (suggest a small, valid DraftOps list for a known mock object)
- Note: the current `mock://gen3d` backend explicitly rejects image inputs. Keep pipeline regression tests prompt-only (no reference images) unless/until mock image support is added.
- Add unit tests for pipeline “next step selection” given synthetic job states (no Bevy world needed).
- Add at least one end-to-end offline test that runs the pipeline on mock backend from “start build” through finish, asserting that:
  - No `agent_step` is called in pipeline mode unless fallback is triggered.
  - DraftOps are applied (assembly_rev increments; `apply_draft_ops_last.json` present).

### 7) Documentation

Update `docs/gen3d/README.md` to document:

- The new deterministic pipeline mode.
- The fallback behavior and what the player sees.
- The new DraftOps tool and why it exists (review-delta cannot edit primitives).
- Any new config knobs.


## Concrete Steps

All commands below are run from the repo root (`/Users/flow/workspace/github/gravimera`).

During implementation, use frequent small commits. After each set of code changes, run at least:

    cargo test

And run the rendered smoke test (UI, not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

If the pipeline introduces new test assets (configs, scene files), place them under the existing `test/` directory as required by `AGENTS.md`.


## Validation and Acceptance

The change is accepted when all of the following are true:

1. With pipeline mode enabled, a normal Gen3D Build (prompt-only) completes without crashing, produces a draft with primitives, and ends with a deterministic “finished” status.
2. In a seeded Edit/Fork session, a prompt-only edit request that can be satisfied by primitive edits results in `apply_draft_ops_v1` being invoked (artifacts show `apply_draft_ops_last.json` and assembly rev increments) and does not regenerate components by default.
3. When the pipeline hits a forced failure mode (mock backend returning invalid JSON once), it performs bounded retries (LLMRepair) and, if still failing, switches to agent-step fallback with an explicit reason.
4. `cargo test` passes.
5. The rendered smoke test command starts the game and exits cleanly (no crash).


## Idempotence and Recovery

The pipeline must be safe to retry:

- `apply_draft_ops_v1` should always be called with `if_assembly_rev` so stale suggestions cannot apply to a changed assembly.
- DraftOps application should default to `atomic=true` so partial edits do not accumulate when the suggestion is invalid.
- If a run is stopped (Stop button), the job should remain resumable (Continue button), regardless of whether it was in pipeline mode or agent fallback mode.


## Interfaces and Dependencies

At the end of implementation, the following interfaces must exist and be exercised by tests:

- A new “pipeline orchestrator” mode that can run Gen3D without calling `agent_step`.
- A new tool contract `llm_generate_draft_ops_v1`:
  - Returns strict JSON under a declared schema (structured outputs).
  - Produces only DraftOps suggestions (no mutation).
  - Has actionable errors when inputs are missing (no plan, no component parts snapshot, invalid args).
- Deterministic application via existing `apply_draft_ops_v1` with diffs visible in tool results and artifacts.
- `llm_generate_draft_ops_v1` must be present in the tool registry shown to agent-step so fallback mode can continue to be DraftOps-first.


## Note on future revisions

- (2026-03-18) Clarified what “deterministic” means, listed all entrypoints that must respect orchestrator selection, specified a deterministic QA/motion remediation loop, tightened the DraftOps tool contract with enforceable gates, and documented current `mock://gen3d` prefix gaps so offline tests are implementable.
