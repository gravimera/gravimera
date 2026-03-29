# Gen3D explicit named motion channels from prompt intent through motion authoring

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with [PLANS.md](/Users/flow/workspace/github/gravimera/PLANS.md).

## Purpose / Big Picture

After this change, a prompt that explicitly names motions such as “sing, dance, rap” will no longer lose those requests in the pipeline and collapse them into the generic `action` channel. The pipeline will still keep `action` as the default generic “handling/working” motion, but it will also preserve and author separate named motion channels when the prompt explicitly asks for them. A user can verify the change by running the Gen3D pipeline on a prompt with named motions, then inspecting the run artifacts to see `prompt_intent.json` preserve the named channels and `llm_generate_motions_v1` request them in addition to `action`.

## Progress

- [x] (2026-03-29 19:20Z) Investigated the failing cached run and confirmed the root cause: the pipeline only requested `move` and `action`, while the “do not collapse named motions into action” guidance exists only in the DraftOps prompt.
- [x] (2026-03-29 19:30Z) Wrote this ExecPlan before code changes because the fix spans prompt intent, schema, pipeline orchestration, prompts, tests, and docs.
- [x] (2026-03-29 19:55Z) Added `explicit_motion_channels` to prompt intent, updated parsing/structured outputs, and extended the mock backend to emit named motions for prompts like `唱、跳、rap`.
- [x] (2026-03-29 20:10Z) Routed named motion channels into the pipeline’s motion batch helper and updated motion-authoring prompts so `action` stays generic instead of replacing named motions.
- [x] (2026-03-29 20:25Z) Added a mock pipeline regression test that proves named motions survive into `llm_generate_motions_v1` requests, and updated Gen3D pipeline docs.
- [x] (2026-03-29 20:28Z) Ran targeted Rust tests and the required rendered smoke test successfully.
- [x] (2026-03-29 20:32Z) Committed the finished change with a clear message after tests and the rendered smoke test passed.

## Surprises & Discoveries

- Observation: the “do not collapse `sing, dance, rap` into `action`” rule already exists in `src/gen3d/ai/prompts.rs`, but only inside `build_gen3d_draft_ops_system_instructions()`, not in the motion-authoring prompt that actually handled the cached run.
  Evidence: `src/gen3d/ai/prompts.rs` contains that rule near the DraftOps prompt, while `build_gen3d_motion_authoring_system_instructions()` currently only describes generic per-channel authoring.
- Observation: the prompt-intent structured output currently stores only `requires_attack`, so explicit motion names have no durable place to survive between the raw user prompt and the later motion batch call.
  Evidence: `src/gen3d/ai/schema.rs` defines `AiPromptIntentJsonV1` with only `version` and `requires_attack`, and `src/gen3d/ai/structured_outputs.rs` matches that shape.
- Observation: the mock pipeline tests were bypassing prompt intent entirely because they seeded jobs directly in `Gen3dAiPhase::AgentExecutingActions`.
  Evidence: the first trace for the new regression test started with `llm_generate_plan_v1`, not a prompt-intent bootstrap request; changing the test to start in `Gen3dAiPhase::AgentWaitingPromptIntent` exercised the real bootstrap path.

## Decision Log

- Decision: extend prompt intent with a new `explicit_motion_channels: Vec<String>` field rather than trying to recover named motions later from the raw prompt in the pipeline.
  Rationale: prompt intent already exists specifically to cache prompt-derived gameplay intent. Adding named motion channels there makes the contract explicit, durable in artifacts, and easy for the pipeline to consume without heuristic string parsing.
  Date/Author: 2026-03-29 / Codex

- Decision: keep `action` in the motion batch request even when explicit named motions exist.
  Rationale: `action` is the repository’s generic default “handling/working” motion channel and should remain available as a reusable default. The bug is not that `action` exists; the bug is that named motions were collapsed into it.
  Date/Author: 2026-03-29 / Codex

## Outcomes & Retrospective

Gen3D prompt intent now preserves explicitly named motion channels, the deterministic pipeline carries those channels into motion batch requests, and the single-channel motion prompt now tells the model that `action` is the generic default handling/working motion rather than a replacement for named motions. The regression test around `蔡徐坤，会唱、跳、rap动作` now verifies that `llm_generate_motions_v1` receives named channels in addition to `move` and `action`, and the rendered smoke test confirms the game still boots and renders after the change.

## Context and Orientation

Gen3D prompt intent is the small structured-output step that runs before planning. It lives in `src/gen3d/ai/prompts.rs` for prompt text, `src/gen3d/ai/schema.rs` for the Rust struct, `src/gen3d/ai/structured_outputs.rs` for the JSON schema, and `src/gen3d/ai/parse.rs` for parsing. The deterministic pipeline in `src/gen3d/ai/pipeline_orchestrator.rs` stores that parsed intent into `job.prompt_intent` and writes `attempt_N/inputs/prompt_intent.json` into the run cache.

Motion authoring is done later by `llm_generate_motions_v1`, which is a batch tool that fans out into one single-channel motion-authoring prompt per requested channel. The batch tool is initiated from `src/gen3d/ai/pipeline_orchestrator.rs`, dispatched by `src/gen3d/ai/agent_tool_dispatch.rs`, and executed by `src/gen3d/ai/agent_motion_batch.rs`. The single-channel motion prompt builder is `build_gen3d_motion_authoring_system_instructions()` and `build_gen3d_motion_authoring_user_text()` in `src/gen3d/ai/prompts.rs`.

The current bug is not in the low-level motion schema. The single-channel tool already supports arbitrary channel names. The bug is that the upstream contract only preserves attack intent and the pipeline only requests `move` and `action`, so explicit user-named motions are lost before the motion authoring tool is even called.

## Plan of Work

First, update prompt intent so it extracts both attack intent and explicit named motion channels. In `src/gen3d/ai/schema.rs`, extend `AiPromptIntentJsonV1` with `explicit_motion_channels: Vec<String>`. In `src/gen3d/ai/structured_outputs.rs`, update the `gen3d_prompt_intent_v1` schema to require that array. In `src/gen3d/ai/prompts.rs`, rewrite the prompt-intent system and user text so the model is told that `action` is the generic default handling channel and that explicitly named motions must be listed separately as compact channel ids. In `src/gen3d/ai/openai.rs`, update the mock backend so pipeline tests can exercise explicit named motion channels.

Second, consume the new intent in the pipeline. In `src/gen3d/ai/pipeline_orchestrator.rs`, centralize the motion batch channel selection so every place that currently requests `move` and `action` instead requests `move`, `action`, any named prompt-intent motion channels, and `attack` when gameplay attack capability exists. Deduplicate while preserving stable order. When writing `prompt_intent.json`, include the new field in the artifact fallback path as well.

Third, update the single-channel motion prompt so the model has the correct semantics when it is asked to author `action` versus a named channel. In `src/gen3d/ai/prompts.rs`, add explicit rules to `build_gen3d_motion_authoring_system_instructions()` and `build_gen3d_motion_authoring_user_text()` stating that `action` is the generic default handling/working motion and must never substitute for user-named motions. The user text should surface the explicit named channels so each single-channel call knows the broader contract.

Fourth, add tests and docs. Extend tests in `src/gen3d/ai/pipeline_orchestrator_tests.rs` so a prompt like `蔡徐坤，会唱、跳、rap动作` causes the mock pipeline to request named channels in addition to `action`. Add parser or prompt tests where convenient in `src/gen3d/ai/prompts.rs` or `src/gen3d/ai/parse.rs`. Update `docs/gen3d/pipeline_walkthrough.md` to document that prompt intent now carries named motion channels and that motion batch requests can include user-named channels alongside `action`.

## Concrete Steps

Work from the repository root:

    cd /Users/flow/workspace/github/gravimera

Inspect current prompt-intent and motion prompt code:

    sed -n '71,114p' src/gen3d/ai/prompts.rs
    sed -n '2259,2322p' src/gen3d/ai/prompts.rs
    sed -n '605,610p' src/gen3d/ai/schema.rs
    sed -n '1062,1072p' src/gen3d/ai/structured_outputs.rs

Run targeted tests after patching:

    cargo test gen3d_mock_pipeline_builds_warcar_prompt_end_to_end -q
    cargo test gen3d_mock_pipeline_requests_named_motion_channels_from_prompt_intent -q
    cargo test gen3d::ai::prompts -q
    cargo test gen3d::ai::parse -q

Run the required rendered smoke test after all code/doc changes:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

If the smoke test succeeds, commit:

    git add src/gen3d/ai/prompts.rs src/gen3d/ai/schema.rs src/gen3d/ai/structured_outputs.rs src/gen3d/ai/parse.rs src/gen3d/ai/openai.rs src/gen3d/ai/pipeline_orchestrator.rs src/gen3d/ai/pipeline_orchestrator_tests.rs docs/gen3d/pipeline_walkthrough.md docs/execplans/gen3d_named_motion_channels_prompt_intent.md
    git commit -m "Preserve named motion channels in gen3d prompt intent"

## Validation and Acceptance

Acceptance requires both artifact-level proof and runtime proof.

Artifact-level proof:

1. Run a Gen3D pipeline test or real run with a prompt that explicitly names motions, for example `蔡徐坤，会唱、跳、rap动作`.
2. Inspect `attempt_0/inputs/prompt_intent.json` and confirm it contains `explicit_motion_channels` with separate channel ids for those named motions.
3. Inspect a later `tool_calls.jsonl` entry for `llm_generate_motions_v1` and confirm its `channels` array contains `move`, `action`, and the named motion channels rather than only `move` and `action`.

Runtime proof:

1. Run the rendered smoke test command from the repository root.
2. Confirm the game starts, renders for two seconds, and exits without a crash.

Test proof:

- The new regression test around named motions should fail before the change because the pipeline only requests `move` and `action`, and pass after the change.

## Idempotence and Recovery

These edits are safe to apply repeatedly because they are additive schema and prompt changes plus deterministic pipeline channel selection. If a test fails partway through, fix the code and rerun the same targeted tests. The rendered smoke test uses a fresh temporary `GRAVIMERA_HOME`, so rerunning it does not pollute a developer’s normal local state.

## Artifacts and Notes

Expected artifact shape after the change:

    {
      "version": 1,
      "requires_attack": false,
      "explicit_motion_channels": ["sing", "dance", "rap"]
    }

Expected motion batch tool-call shape after the change:

    {
      "tool_id": "llm_generate_motions_v1",
      "args": {
        "channels": ["move", "action", "sing", "dance", "rap"]
      }
    }

## Interfaces and Dependencies

`src/gen3d/ai/schema.rs` must define:

    pub(crate) struct AiPromptIntentJsonV1 {
        pub(crate) version: u32,
        pub(crate) requires_attack: bool,
        pub(crate) explicit_motion_channels: Vec<String>,
    }

`src/gen3d/ai/pipeline_orchestrator.rs` must contain a helper that derives the motion batch channel list from `job.prompt_intent`, the draft’s attack capability, and the default generic channels. The helper must return a stable, deduplicated ordered list suitable for `llm_generate_motions_v1`.

`src/gen3d/ai/prompts.rs` must make the following contract explicit in both prompt intent and motion authoring:

- `action` is the generic default handling/working motion.
- If the prompt explicitly names motions, those motions must be represented as separate channels.
- Named motions must not be collapsed into `action`.

Revision note (2026-03-29): updated after implementation to record the completed schema/pipeline/doc/test work and the discovery that tests must start in `AgentWaitingPromptIntent` to exercise the real prompt-intent bootstrap path.
