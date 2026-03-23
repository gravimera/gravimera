# Gen3D: Stable edit pipeline (no agent-step fallback) + DraftOps contract hardening

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Seeded Edit/Fork Gen3D runs (“edit sessions”) should be predictable and bounded: they should either (a) complete in pipeline mode without silently switching orchestrators, or (b) stop with a specific, actionable error that tells the user what failed and how to retry.

Today, pipeline edit runs can fall back to the model-driven `agent_step` loop when an LLM-backed tool emits malformed output (even after schema-repair attempts). That fallback is expensive and can lead to long runs with many `pass_N/` directories and repeated motion authoring calls, eventually hitting the 30‑minute time budget.

After this change:

- Pipeline mode never switches to `agent_step`. The pipeline either finishes or stops with an explicit failure reason.
- Every LLM-backed “operation producer” tool (especially `llm_generate_draft_ops_v1`) has a hardened, contract-first interface:
  - If tool output is malformed, the engine returns a precise error and automatically performs exactly one schema-repair retry (two total attempts: first + repair).
  - If the repair attempt still fails, the pipeline stops with an actionable error (no fallback).
- Edit sessions can request *any* supported DraftOps modification (including animation slot edits like `upsert_animation_slot`) without common schema mismatch traps (e.g. mistakenly placing `clip` at the DraftOp top level).
- DraftOps animation-slot edits are no longer blocked by over-strict “ambiguity” checks:
  - Root component animation slots are supported (the root has an implicit “root edge” from the draft root to the root component).
  - When multiple slots exist for a channel (common for `attack_primary` variants), DraftOps operations apply deterministically instead of rejecting as ambiguous.

How to see it working (after implementation):

1. Start a seeded edit run on an existing Gen3D prefab with the prompt “Add a new motion: dancing.” and ensure `gen3d_orchestrator=pipeline`.
2. Observe that the run:
   - does not emit “Pipeline fallback → agent-step …” in Info Store events, and
   - finishes normally, or stops with a clear error after at most one schema-repair retry for any malformed tool output.
3. In the failure case, the UI status and run artifacts clearly identify the invalid field(s) and show a minimal example of the correct shape.


## Progress

- [x] (2026-03-23) Investigated a real run cache that fell back from pipeline → agent-step due to malformed DraftOps (`run_id=694d0671-16c8-44d3-8d25-30f950f8bbdf`).
- [x] Draft/validate a “contract-first” checklist for edit-session tools and close the motivating gaps (DraftOps nested slot/clip guidance, normalization/repair hints, pipeline stop semantics).
- [x] Remove pipeline → agent-step fallback and replace with explicit pipeline stop behavior.
- [x] Harden `llm_generate_draft_ops_v1` for animation-slot edits (`upsert_animation_slot`) with clearer schema guidance + more actionable repair prompts.
- [x] Add deterministic DraftOps normalization for legacy/alias shapes (e.g. `component`→`child_component`, `duration_secs`→`duration_units`, top-level `clip`/`driver` moved into `slot`).
- [x] Support editing animation slots on the root component (store root slots in planned state; sync into the draft root’s object-ref part).
- [x] Add bounded iteration budgets for pipeline edit runs (two total schema attempts: initial + 1 repair; two cycles max for `rejected_ops` re-suggest) and stop deterministically when exhausted.
- [x] Validate with `cargo test`.
- [ ] Run the rendered smoke test and verify on a real edit run.


## Surprises & Discoveries

- Observation: Pipeline mode can (and did) switch into agent-step mode automatically.
  Evidence: In the motivating run, Info Store contains `Pipeline fallback → agent-step (reason: tool_failed:llm_generate_draft_ops_v1:...)` at `~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/info_store_v1/events.jsonl`.

- Observation: `llm_generate_draft_ops_v1` was invoked for a motion edit (“Add a new motion: dancing.”) and returned malformed `DraftOp` objects for `upsert_animation_slot` using top-level keys like `clip` / `clip_kind`.
  Evidence: `~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/attempt_0/pass_1/gen3d_run.log` includes:
    - `DraftOp kind="upsert_animation_slot" includes unknown key "clip"`
    - `DraftOp kind="upsert_animation_slot" includes unknown key "clip_kind"`
  Additional evidence: `~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/attempt_0/pass_1/draft_ops_raw.txt` shows a legacy-ish shape with top-level `driver`, `clip.duration_secs`, `clip.keyframe_times`, and a separate `keyframes` array.

- Observation: The schema-repair prompt is generic and does not include tool-specific “allowed keys” or a tiny correct example for the failing sub-shape.
  Evidence: `src/gen3d/ai/agent_tool_poll.rs::schedule_llm_tool_schema_repair` appends a generic REPAIR REQUEST with the error string only.

- Observation: Root component motion cannot be authored today because “motion authoring” only targets attachment edges (child components with `attach_to`).
  Evidence: `src/gen3d/ai/agent_tool_poll.rs` rejects motion-authoring edges that reference the root (`attach_to=None`).

- Observation: `AGENTS.md` references `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md`, but these files are not present in this working tree.
  Evidence: `docs/execplans/gen3d_deterministic_pipeline.md` already notes this; `docs/agent_skills/` only contains `SKILL_agent.md`.


## Decision Log

- Decision: Remove pipeline → agent-step fallback entirely; pipeline must finish or stop.
  Rationale: Fallback is the main source of runaway pass counts and “can’t stop” user experience. Stability here means the orchestrator never changes silently.
  Date/Author: 2026-03-23 / user + assistant

- Decision: Standardize on “two chances” for malformed LLM tool outputs: one initial attempt + exactly one schema-repair retry; after that, stop with an actionable error.
  Rationale: This is the smallest bounded loop that still recovers from common formatting mistakes without risking long oscillations.
  Date/Author: 2026-03-23 / user + assistant

- Decision: Keep contracts strict, but add deterministic normalization only for clearly unambiguous alias/misplacement patterns, and always report repairs as a diff-like summary in tool results.
  Rationale: Strictness prevents silent misinterpretation; deterministic normalization improves robustness without heuristics. “Repaired=true + repair_diff” makes the behavior observable and debuggable.
  Date/Author: 2026-03-23 / user + assistant

- Decision: DraftOps animation-slot edits must be able to target the root component and must not reject “multiple slots per channel” as ambiguous.
  Rationale: Root is a valid component in any object; users must be able to animate it (and common rigs have multiple `attack_primary` variants). Ambiguity rejection blocks valid edits; deterministic bulk semantics keep the tool generic and usable.
  Date/Author: 2026-03-23 / user + assistant

## Outcomes & Retrospective

(To be written after implementation and real-run verification.)


## Context and Orientation

Gen3D has two orchestration modes:

- **Agent mode** (`Gen3dAiMode::Agent`): the LLM returns “next step” tool calls (`agent_step`), which can iterate many times.
  - Primary code: `src/gen3d/ai/agent_loop/`, `src/gen3d/ai/agent_step.rs`, `src/gen3d/ai/agent_tool_dispatch.rs`, `src/gen3d/ai/agent_tool_poll.rs`.
- **Pipeline mode** (`Gen3dAiMode::Pipeline`): a deterministic state machine calls tools in a fixed order, using bounded retries and explicit budgets.
  - Primary code: `src/gen3d/ai/pipeline_orchestrator.rs`.

In edit sessions (seeded Edit/Fork), the pipeline currently does:

1. Preserve-mode `llm_generate_plan_ops_v1` (diff-first replanning).
2. Capture per-component edit interfaces via `query_component_parts_v1`.
3. Request edit suggestions via `llm_generate_draft_ops_v1`.
4. Apply them deterministically via `apply_draft_ops_v1`.
5. Run `qa_v1`, then remediate via deterministic fixits, motion authoring, and/or review-delta, then finish.

Key concepts:

- **DraftOps**: a list of deterministic operations applied by `apply_draft_ops_v1` that can update primitives, attachments, joints, anchors, and animation slots.
- **Schema repair**: a second LLM attempt automatically scheduled when the first tool output cannot be parsed or applied. The maximum is controlled by `GEN3D_LLM_TOOL_SCHEMA_REPAIR_MAX_ATTEMPTS` (currently `2`).
- **Pass artifacts**: per-run artifacts are written under `~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_N/`.

The motivating failure path:

- Pipeline stage `EditSuggestDraftOps` calls `llm_generate_draft_ops_v1`.
- The tool output fails DraftOps validation due to unexpected keys (`clip`, `clip_kind`).
- The engine schedules schema repair (2 attempts total).
- After repair still fails, pipeline falls back to agent-step (undesired).


## Plan of Work

### 1) Audit edit-session “supported modifications” and identify hard failures

Make an explicit inventory of what an edit session is supposed to be able to change, and which tool/stage owns it. Do not rely on prompt heuristics (“dance” keyword); use only tool contracts and deterministic stage selection.

At minimum, document these buckets and their deterministic mechanisms:

- Plan/root edits (components, mobility, collider, attack profile, aim): `get_plan_template_v1` → `llm_generate_plan_ops_v1` → `apply_plan_ops_v1`.
- Draft geometry/appearance edits (primitives): `query_component_parts_v1` → `llm_generate_draft_ops_v1` → `apply_draft_ops_v1`.
- Attachment/joint/anchor edits: `llm_generate_draft_ops_v1` → `apply_draft_ops_v1` (ops like `set_attachment_joint`, `set_anchor_transform`, `set_attachment_offset`).
- Motion edits:
  - “Slot-level” edits: `upsert_animation_slot` / `remove_animation_slot` in DraftOps.
  - “Rig-level clip authoring”: `llm_generate_motions_v1` (batch) / `llm_generate_motion_v1` (one channel) (LLM+mutates).

Important clarification for motion edits:

- DraftOps `*_animation_slot` ops edit animation slots on the component reference part:
  - For non-root components, this is the attachment edge stored under `attach_to` (parent→child).
  - For the root component, this is the implicit “root edge” stored on the draft root’s object-ref part (root→root_component).

Then, for each bucket, list the current “failure surface”:

- What malformed output patterns do we see in real caches?
- Does the tool error include the exact fix path (what to change and what to retry)?
- Does the pipeline stop deterministically when retries are exhausted?

This inventory becomes the acceptance checklist for “support all modifications” (meaning: any modification the engine already supports via tools should be reachable in edit sessions without fallback and with actionable errors when the LLM misformats).

## Decision Log

- (2026-03-23) Pipeline mode never falls back to `agent_step`; persistent tool failures stop the run via `finish_job_best_effort(...)` with an explicit stage/tool/call_id reason.
- (2026-03-23) Treat root motion as an “implicit root edge”: root animations live on the draft root’s object-ref part and are edited via DraftOps when `child_component` is the root.
- (2026-03-23) DraftOps animation-slot ops apply deterministically when multiple slots exist for a channel (edit/remove/scale all matches instead of rejecting as ambiguous).
- (2026-03-23) Schema repair is capped to exactly one retry (two total attempts: initial + repair) to prevent long runs from repeated re-authoring loops.

## Outcomes & Retrospective (So Far)

- The motivating failure mode (“unknown key clip/clip_kind” → schema repair exhaustion → pipeline fallback → long run) is addressed by a clearer DraftOps contract, normalization for legacy-ish shapes, and removing pipeline fallback.
- Root/torso animation edits are supported without rewires by adding `root_animations` to planned state and syncing it through `sync_attachment_tree_to_defs(...)`.
- Remaining validation: run the rendered smoke test and confirm behavior on a real seeded edit session.

### 2) Harden DraftOps for animation slots (the motivating `clip`/`clip_kind` mismatch)

This work has two layers: tool contract clarity (prompt/schema) and deterministic engine-side behavior (validation + repair messaging + normalization).

Contract clarity improvements:

- Update the DraftOps system instructions (`src/gen3d/ai/prompts.rs::build_gen3d_draft_ops_system_instructions`) to include a tiny, copy/pasteable example of `upsert_animation_slot` showing that:
  - `clip` is nested under `slot.clip`,
  - `kind` is nested under `clip.kind` (not `clip_kind`),
  - top-level DraftOp keys are only `kind`, `child_component`, `channel`, `slot`.

Actionable schema-repair improvements:

- Extend the schema-repair prompt path (`src/gen3d/ai/agent_tool_poll.rs::schedule_llm_tool_schema_repair`) so that when the error matches known DraftOps failures (unknown keys, missing `slot`, etc.), the repair request embeds:
  - the allowed key list for that DraftOp kind, and
  - the smallest valid JSON example for the failing sub-shape (only a few lines).

Deterministic normalization (only when unambiguous):

- Add a DraftOps “normalization” step before validation that can rewrite certain malformed-but-unambiguous patterns into the canonical schema and report `repaired=true` + `repair_diff` in the tool result. For example:
  - `upsert_animation_slot` with top-level `clip` but missing `slot` can be rewritten into `slot={driver:"always",speed_scale:1,time_offset_units:0,clip:<clip>}`.
  - `clip_kind` can be rewritten into `clip.kind` when `clip` object exists.

Important: if the normalization cannot be done unambiguously, do not guess. Fail with a precise error and trigger schema repair.

### 2.5) Support root animation slots (generic torso/root fix)

Implement root animation support in a way that does not require plan rewires:

- Extend the planned component state with a `root_animations` list (only meaningful when `attach_to=None`).
- Update `sync_attachment_tree_to_defs(...)` to copy `root_animations` into the draft root’s object-ref part that references the root component.
- Update DraftOps animation-slot ops so that when `child_component` is the root:
  - `upsert_animation_slot` edits `root_animations`.
  - `remove_animation_slot` removes from `root_animations`.
  - `scale_animation_slot_rotation` scales in `root_animations`.

This makes “torso is root → cannot animate torso” go away without relying on heuristics or unsafe rewires.

### 3) Remove pipeline → agent-step fallback and replace with deterministic pipeline stops

Eliminate all uses of `fallback_to_agent_step(...)` in `src/gen3d/ai/pipeline_orchestrator.rs`.

Replace each fallback trigger with one of:

- A deterministic pipeline retry (only when the next action is deterministic and bounded, e.g. one more DraftOps suggestion cycle after `rejected_ops`), or
- A deterministic stop (best-effort finish or hard fail) with an actionable error that includes:
  - the stage name,
  - the exhausted budget counters,
  - the last failing tool id + call id (when relevant),
  - the exact next steps (e.g. “retry the run”, “simplify the edit prompt”, or “switch to agent orchestrator if you explicitly want unbounded exploration”).

This is the core “pipeline mode is stable” requirement: pipeline never silently changes orchestrator.

### 4) Standardize per-stage iteration budgets for pipeline edit runs

Make budgets consistent and explainable (“two chances”):

- For LLM formatting/schema failures: rely on the existing per-tool schema repair cap (two total attempts: initial + repair).
- For pipeline-level re-suggest cycles (e.g. `rejected_ops` from `apply_draft_ops_v1`): allow at most 2 cycles:
  - Cycle 1: suggest → apply
  - Cycle 2: suggest with `rejected_ops` context → apply
  - If still rejected: stop with an actionable error that includes the `rejected_ops` summary.

Remove large open-ended counters in pipeline mode (e.g. 12 QA loops) unless there is a clear, deterministic reason they are needed. Prefer small bounded remediation loops:

- QA run
- apply deterministic fixits (bounded count)
- QA re-run
- then either succeed or stop

### 5) Update regression coverage (offline tests + real-run verification)

Update and/or add tests under `src/gen3d/ai/*tests.rs` to cover:

- Pipeline never switches to agent mode (`job.mode` remains `Pipeline`) under tool failure.
- DraftOps malformed `upsert_animation_slot` output triggers schema repair and then stops deterministically if still invalid.
- DraftOps normalization (when enabled) produces a repaired canonical payload and applies successfully via `apply_draft_ops_v1`.

Then verify on a real edit run (non-mock AI) using the same prompt that previously triggered failure.


## Concrete Steps

From repo root:

1. Implement changes described in “Plan of Work”.
2. Run unit tests:

    cargo test

3. Run the required rendered smoke test (non-headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4. Run a real seeded edit run (manual):

   - Start Gravimera normally.
   - Enter Gen3D edit session for an existing prefab.
   - Prompt: “Add a new motion: dancing.”
   - Ensure pipeline mode is enabled.
   - Observe: no fallback, bounded retries, clear stop or finish.


## Validation and Acceptance

Acceptance criteria:

- Pipeline mode never emits a “Pipeline fallback → agent-step …” Info Store event for any edit run.
- When `llm_generate_draft_ops_v1` output is malformed:
  - the engine schedules exactly one schema repair retry (two total attempts),
  - after the second failure, the run stops with a clear error that includes the invalid key(s) and a minimal correct example.
- For the motivating edit prompt (“Add a new motion: dancing.”), a real run completes without hitting the 30‑minute time budget and without creating many `pass_N/` directories due to agent-step looping.


## Idempotence and Recovery

- All pipeline stop reasons must be safe to retry: rerunning the same edit prompt should not corrupt the prefab/draft (especially in overwrite-save flows).
- If a stage fails due to malformed tool output, the failure should be reproducible from the run cache artifacts (`tool_calls.jsonl`, `tool_results.jsonl`, `draft_ops_raw.txt`, `gen3d_run.log`), and reruns should append new artifacts rather than overwriting prior ones.


## Artifacts and Notes

Motivating run cache:

    ~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/

Key evidence:

- Pipeline tool activity and DraftOps schema failures:

    ~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/attempt_0/pass_1/gen3d_run.log

- Pipeline fallback event:

    ~/.gravimera/cache/gen3d/694d0671-16c8-44d3-8d25-30f950f8bbdf/info_store_v1/events.jsonl
