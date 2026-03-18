# Gen3D deterministic pipeline orchestrator (DraftOps-first) with agent-step fallback

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D “Build” is currently orchestrated by a Codex-style `agent_step` loop: the model decides which tool to call next (plan, component generation, QA, review, etc.). That makes sequencing slow (extra LLM turns), fragile (wrong next step / malformed tool args), and hard to test.

After this change, Gen3D will have a deterministic **pipeline orchestrator** that manages the fixed workflow as a state machine. The model will still be used for the inherently open-ended parts (plan generation, component drafts, plan ops, review delta, and DraftOps suggestions), but the *process management* (what happens next, retries, gating, budgets, stopping conditions) will be deterministic and testable.

We will keep the existing `agent_step` loop as a **fallback**. When the pipeline cannot make progress (tool schema failures beyond repair budget, repeated atomic DraftOps rejections, etc.), it will switch to agent-step and continue the run using the current implementation.

The most important product requirement is **DraftOps-first editing**: in seeded Edit/Fork sessions, user requests like “make the cannon longer” should prefer in-place primitive edits (`apply_draft_ops_v1`) rather than regenerating whole components. This requires a new LLM-backed tool that *suggests* DraftOps deterministically (strict schema), then the engine applies them via `apply_draft_ops_v1` (atomic + revision-gated).

How to see it working (after implementation):

1. Start the game and enter Gen3D Workshop (Build Preview scene).
2. Build a new object from a text prompt. Observe status text like “Pipeline: planning → generating components → QA → review”.
3. Seed an Edit/Fork session from a Gen3D-saved prefab, enter an edit prompt like “make the barrel longer and darken it”, click Build, and observe that the run performs DraftOps edits (with diffs in artifacts) instead of component regeneration.
4. Intentionally trigger a failure (e.g., mock backend returning invalid JSON) and observe that the run switches to agent-step fallback (status includes a clear reason).


## Progress

- [x] (2026-03-18 22:10 CST) Drafted this ExecPlan.
- [ ] Implement pipeline orchestrator skeleton and config toggle; keep agent-step path unchanged and available.
- [ ] Add new LLM tool: `llm_generate_draft_ops_v1` (suggestions only; no mutation) + strict structured-output schema.
- [ ] Implement create-session pipeline flow (new Build): plan → components → QA → (render/review) loops → finish.
- [ ] Implement edit-session pipeline flow (seeded Edit/Fork): plan-ops → missing components → DraftOps suggest+apply (atomic) → QA → (render/review) loops → finish.
- [ ] Implement deterministic fallback to agent-step (bounded retries; explicit status + Info Store event).
- [ ] Add mock backend responses + offline tests so the pipeline is regression-tested without network.
- [ ] Update docs (`gen_3d.md`) to describe pipeline mode + fallback and the new DraftOps tool.
- [ ] Run `cargo test` and the rendered smoke test; fix any regressions.


## Surprises & Discoveries

- Observation: The repo already has a “legacy” non-agent pipeline code path in `src/gen3d/ai/orchestration.rs` (`WaitingPlan`, `WaitingComponent`, `CapturingReview`, `WaitingReview`), but `gen3d_start_build_from_api` currently always sets `job.mode = Gen3dAiMode::Agent`, so that path is not used for normal runs.
  Evidence: `src/gen3d/ai/orchestration.rs::gen3d_start_build_from_api` sets `job.mode = Gen3dAiMode::Agent` and `gen3d_poll_ai_job` dispatches to `agent_loop::poll_gen3d_agent` when in Agent mode.

- Observation: `llm_review_delta_v1` cannot directly edit primitives; it can tweak attachments/anchors/transforms/mobility/attack and can request regeneration or replan, but not “edit part X color/scale”.
  Evidence: `src/gen3d/ai/schema.rs::AiReviewDeltaActionJsonV1` and `src/gen3d/ai/convert.rs::apply_ai_review_delta_actions`.

- Observation: The deterministic primitive-edit mechanism already exists as `apply_draft_ops_v1`, and it returns actionable diffs (`diff_summary`, `applied_ops`, `rejected_ops`) and supports atomic application + `if_assembly_rev` gating.
  Evidence: `src/gen3d/ai/draft_ops.rs::apply_draft_ops_v1`.

- Observation: `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md` are referenced in `AGENTS.md` but are not present in this working tree (directory exists, files missing). This plan therefore embeds the needed “contract-first” tool guidance instead of referencing those docs.


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


## Outcomes & Retrospective

(To be filled during/after implementation.)


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
- `gen_3d.md`: current (agent-driven) Gen3D implementation doc.

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

Keep the tool execution machinery shared. The pipeline should reuse `agent_tool_dispatch::execute_tool_call` and `agent_tool_poll::poll_agent_tool` rather than re-implementing LLM spawning, structured-output repair, artifact logging, or regen/QA gates.

Contract-first rule (embedded here): Any new tool must return actionable results and actionable errors, and must enforce its own gatekeeping (validation, budgets, forbidden states) inside the tool implementation. Do not add “agent prompt rules” as the primary enforcement mechanism.

### 2) Implement deterministic “next tool” selection (create flow)

For a fresh Build (non-seeded):

- If user images exist and `job.user_image_object_summary` is missing: run the existing image-summary request, then continue.
- Ensure a plan exists. If no accepted plan exists yet (empty `job.plan_hash` / `job.planned_components` empty), run `llm_generate_plan_v1`.
- Generate missing components: run `llm_generate_components_v1` in missing-only mode until every planned component has `actual_size`.
- Run QA: `qa_v1` (this updates `job.agent.last_validate_ok`, `last_smoke_ok`, motion flags, and caches by state hash).
- If `review_appearance=true`: run `render_preview_v1`, then `llm_review_delta_v1`.
- Apply review delta results deterministically:
  - If it requests `replan_reason`: re-run `llm_generate_plan_v1` (or prefer plan ops if a future “plan ops for create” exists).
  - If it requests regen indices: run `llm_generate_components_v1` with those indices (respecting existing regen budgets + QA gates).
  - If it applied non-regen tweaks (anchors/attachments/etc): loop back to QA and (optionally) render/review.
- Finish deterministically when “complete enough” (reuse/extract logic equivalent to `run_complete_enough_for_auto_finish` in `src/gen3d/ai/agent_step.rs`).

### 3) Implement deterministic “next tool” selection (edit flow, DraftOps-first)

For seeded Edit/Fork sessions (where `job.edit_base_prefab_id.is_some()` and preserve mode defaults to true):

- Always run a preserve-mode plan ops pass first: `get_plan_template_v1` (mode="auto") → `llm_generate_plan_ops_v1` → `apply_plan_ops_v1`.
  - This handles add/remove/rewire/anchor-interface edits deterministically (and yields actionable errors when not possible).
  - If plan ops fails semantically, retry with `inspect_plan_v1` information in the prompt (bounded retries); then fall back to full `llm_generate_plan_v1` with `constraints.preserve_existing_components=true` if needed.
- Generate any missing components created by plan ops.
- DraftOps-first geometry edits:
  - Deterministically capture “editable part interfaces” by calling `query_component_parts_v1` for each component (bounded `max_parts` and no non-primitives by default).
  - Call the new tool `llm_generate_draft_ops_v1` to output a strict DraftOps list (suggestions only).
  - Apply the suggested ops using `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev=job.assembly_rev`.
  - If application rejects ops (non-empty `rejected_ops`), call `llm_generate_draft_ops_v1` again with the rejection payload and the current `query_component_parts_v1` snapshot (LLMRepair-style loop), bounded by a small retry budget.
  - Only when DraftOps cannot satisfy the request should regeneration be used. In preserve mode, regeneration is QA-gated; the pipeline must respect that gate.
- Run QA and, if enabled, render+review-delta loops as in the create flow.

### 4) Add the `llm_generate_draft_ops_v1` tool (suggestions-only)

Add a new tool id and contract:

- Tool id: `llm_generate_draft_ops_v1`
- Purpose: Produce a bounded list of DraftOps (`ops`) that can be applied by `apply_draft_ops_v1` to satisfy the user’s edit prompt, using only component/part information supplied by the engine (especially `query_component_parts_v1` output with `part_id_uuid` and `recipes`).
- Must be strict structured output (JSON schema, `additionalProperties=false`).
- Must never mutate state by itself.
- Must be safe by default: limited op count, avoids deleting parts unless explicitly asked, and must preserve “movable unit” requirements (do not accidentally remove move slots, etc.).

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
- Add unit tests for pipeline “next step selection” given synthetic job states (no Bevy world needed).
- Add at least one end-to-end offline test that runs the pipeline on mock backend from “start build” through finish, asserting that:
  - No `agent_step` is called in pipeline mode unless fallback is triggered.
  - DraftOps are applied (assembly_rev increments; `apply_draft_ops_last.json` present).

### 7) Documentation

Update `gen_3d.md` to document:

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


## Note on future revisions

(When this plan is revised, add a short note here describing what changed and why.)

