# Gen3D: Step Folders (no pass), Motion-only Retry, and Explicit Reuse Tool

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `PLANS.md` (repository root).

## Purpose / Big Picture

Before this change, Gen3D pipeline runs dumped almost all artifacts into a single `attempt_0/pass_0` folder, and pipeline call ids included a meaningless `p0` segment. This made debugging regressions hard (files overwrote each other, and there was no clear “what happened when” timeline).

After this change, each pipeline tool call writes its artifacts into a dedicated, ordered **step folder** (append-only), and there is no `pass_*` directory. QA-driven “second chance” retries happen **only for motion authoring**. Component reuse (copy/mirror) becomes explicit and debuggable by adding a deterministic tool that applies the plan’s `reuse_groups` and by having the pipeline call it (instead of hiding reuse inside component generation).

You can see it working by running a Gen3D build and then inspecting the run cache directory:

1. Artifacts live under `<run_id>/attempt_0/steps/step_####/` per tool call.
2. Each step dir contains `tool_calls.jsonl` / `tool_results.jsonl` for that tool call (and `step.json` for ordering).
3. Reuse application is visible as an explicit tool call result (and copies are no longer “invisible” side-effects of component batching).

## Progress

- [x] (2026-03-26) Add step-folder artifact layout (no `pass_*` dirs).
- [x] (2026-03-26) Remove plan retry; keep motion-only retry with QA feedback.
- [x] (2026-03-26) Add deterministic `apply_reuse_groups_v1` tool and pipeline stage.
- [x] (2026-03-26) Improve plan prompt for reuse + chain anchor orientation.
- [x] (2026-03-26) Fix motion-authoring schema regression (`version` must be numeric) and include prior motion failures in retry feedback.
- [x] (2026-03-26) Ensure cached `qa_v1` calls still write `qa.json` / `validate.json` / `smoke_results.json` into the step dir.
- [x] (2026-03-26) Update docs and add/adjust tests for the new layout + reuse tool.
- [x] (2026-03-26) Run `cargo test`, run rendered smoke start, and commit.

## Surprises & Discoveries

- Observation (before fix): Pipeline runs produced `attempt_0/pass_0` and overwrote key files like `qa.json` / `smoke_results.json` on each QA run, making intermediate states hard to inspect.
  Status: Resolved by per-tool-call step dirs under `attempt_N/steps/step_####/`.

- Observation: “Weird arms” in the provided run correlate with `motion_validation` warnings like `chain_axis_mismatch`, which are driven by plan anchor orientation (not motion authoring).
  Evidence: `qa.json` in the run shows `chain_axis_mismatch` warnings on `left_upper_arm` / `right_upper_arm`.

- Observation: Motion authoring can fail structured parsing when the model sets `"version":"gen3d_motion_authoring_v1"` (string) instead of `1` (number), resulting in missing motion channels even after retries.
  Status: Resolved by tightening the motion-authoring system prompt to explicitly require `version=1` (numeric) and by propagating prior motion tool failures into the retry `qa_feedback`.

- Observation: Cached `qa_v1` calls returned a cached JSON payload but did not emit `qa.json` / `validate.json` / `smoke_results.json` into the current step dir, making “cached QA” steps look empty in the run timeline.
  Status: Resolved by writing the same artifacts on cache hits before returning the cached result.

## Decision Log

- Decision: Implement step folders by making each pipeline tool call select a fresh artifact directory under `attempt_N/steps/` (append-only `step_####` ordering), while keeping cached inputs and “latest plan” extracts available at the attempt root.
  Rationale: Keeps history append-only and makes “what happened when” obvious (via step order and `agent_trace.jsonl`) without overwriting artifacts.
  Date/Author: 2026-03-26 / Codex

- Decision: Move reuse application out of `llm_generate_components_v1` internals into an explicit deterministic tool `apply_reuse_groups_v1`.
  Rationale: Makes reuse observable/debuggable and prevents the perception that the “reuse tool was removed”.
  Date/Author: 2026-03-26 / Codex

## Outcomes & Retrospective

- Artifacts are now append-only per tool call under `attempt_N/steps/step_####/` (no more `pass_0` overwrites).
- Pipeline spends its QA “second chance” only on motion authoring (no plan retry loop).
- Reuse is explicit and debuggable via `apply_reuse_groups_v1` (no hidden component-batch side effects).
- Prompts tightened for reuse + chain-axis anchor orientation; motion authoring prompt now embeds a schema-derived key contract and explicitly requires `version=1` (numeric) to avoid schema mistakes.
- Cached `qa_v1` calls now still write step artifacts (`qa.json`, `validate.json`, `smoke_results.json`) so the run timeline stays debuggable even when QA is cached.

## Context and Orientation

Key code:

- Pipeline orchestration: `src/gen3d/ai/pipeline_orchestrator.rs`
- Tool runtime / dispatch: `src/gen3d/ai/agent_tool_dispatch.rs`, `src/gen3d/ai/agent_tool_poll.rs`
- Artifact helpers: `src/gen3d/ai/artifacts.rs`
- Current run dir / step dir management: `src/gen3d/ai/orchestration.rs` (`gen3d_set_current_attempt_step`, `gen3d_advance_step`)
- Reuse groups parsing + application: `src/gen3d/ai/reuse_groups.rs`, `src/gen3d/ai/copy_component.rs`

Current artifact layout:

- `<run_id>/attempt_N/steps/step_####/*` for per-tool-call artifacts (LLM prompt/response files, QA JSON, motion JSON, etc).
- `<run_id>/attempt_N/inputs/*` for cached user prompt + images.
- Some “latest accepted plan” artifacts are also mirrored at the attempt root for convenience (example: `attempt_N/plan_extracted.json`).

## Plan of Work

First, replace pass directories with step directories:

1. Create `attempt_N` as the stable attempt root.
2. Create `attempt_N/steps/step_####/` per tool call.
3. Ensure tool runtime writes per-call artifacts into the current step directory.
4. Ensure step dirs contain `step.json` and per-call `tool_calls.jsonl` / `tool_results.jsonl` for traceability.

Second, adjust QA retry policy:

1. Remove pipeline “plan retry on complaints”.
2. Keep motion-only retry with `qa_feedback` passed to motion authoring tools.

Third, make reuse explicit:

1. Implement deterministic tool `apply_reuse_groups_v1` that applies `job.reuse_groups` to the current draft and planned components.
2. Add a pipeline stage that runs after component generation and before QA to apply reuse.

Finally, improve plan quality:

1. Tighten plan system instructions around:
   - declaring `reuse_groups` for symmetric/repeated parts, and
   - aligning chain anchors so limb motion is well-defined (`chain_axis_mismatch` avoidance).

Update docs and tests, then validate via `cargo test` and the required rendered smoke run.

## Concrete Steps

All commands run from repo root:

    cargo test

Smoke start (rendered; do NOT use headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Validation and Acceptance

- Running a Gen3D build produces `attempt_0/steps/` with monotonically increasing `step_####` folders; there is no `pass_0` folder.
- `apply_reuse_groups_v1` appears as an explicit tool call in `agent_trace.jsonl` (and in the corresponding step dir’s `tool_calls.jsonl`) when `reuse_groups` is present.
- Pipeline does not issue a second `llm_generate_plan_v1` call just because QA returned complaints; it may issue a second `llm_generate_motions_v1` when motion QA complaints exist.

## Idempotence and Recovery

- Re-running builds is safe; each build gets a new run id folder.
- Resume continues to create new step folders append-only within the attempt.

## Artifacts and Notes

(Keep short transcripts here as they arise.)

## Interfaces and Dependencies

- New deterministic tool id: `apply_reuse_groups_v1` implemented in `src/gen3d/ai/agent_tool_dispatch.rs` and wired into the pipeline in `src/gen3d/ai/pipeline_orchestrator.rs`.
