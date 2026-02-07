# Speed Up Gen3D Agent Runs (Prompt + Reasoning Compaction)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D Build runs are currently dominated by slow `/responses` calls. Users experience long waits (tens of minutes) even when the engine-side work is fast. After this change, Gen3D runs should be meaningfully faster by reducing the amount of text we send to the model for each step (especially `agent_step` and `review_delta`), and by lowering reasoning effort for high-frequency calls that do not need “high” reasoning.

The user-visible behavior stays the same (Build/Stop/Save workflow). The improvement should be observable by looking at `gen3d_cache/<run_id>/agent_trace.jsonl`: fewer tokens per `agent_step` and faster median call durations, while keeping correctness via tool-driven validation/review.

## Progress

- [x] (2026-02-03) Add per-request reasoning-effort override support in the Gen3D OpenAI request plumbing and apply conservative caps per request kind (agent_step/component/review/plan).
- [x] (2026-02-03) Compact `agent_step` prompt: keep state summary, but replace large “recent tool results” JSON blobs with a short, lossless summary (success/failure + key fields + artifact paths).
- [x] (2026-02-03) Compact component-generation prompt: remove full “Component plan summary” dump; replace with only root + parent/child context needed for scale and anchor correctness.
- [x] (2026-02-03) Compact review prompt: avoid embedding full `scene_graph_summary.json` pretty JSON; provide a compact, rounded summary containing only the fields the review-delta schema needs (component id/name/parent/anchors/transform) plus smoke results.
- [x] (2026-02-03) Add/adjust tests that ensure prompts are compact (size/contains key info) and that the agent loop still functions.
- [x] (2026-02-03) Run `cargo test` and a smoke run (`cargo run -- --headless --headless-seconds 1`) to ensure the game starts without crashing.
- [x] (2026-02-03) Commit changes (README updated for reasoning-effort caps).

## Surprises & Discoveries

- Observation: In the warcar example run, most wall time was spent waiting on `/responses` (33 calls). `agent_step_user_text.txt` grew to ~90KB in later passes because it embedded pretty-printed “recent tool results”, including large `describe_tool_v1` descriptions.
  Evidence: `target/debug/gen3d_cache/983620c4-.../attempt_0/pass_6/agent_step_user_text.txt` is ~90KB; `agent_trace.jsonl` shows ~59 minutes in LLM waits.
- Observation: The review prompt embedded `scene_graph_summary.json` pretty-printed (~71KB) which inflates review token usage and latency.
  Evidence: `target/debug/gen3d_cache/983620c4-.../attempt_0/pass_6/scene_graph_summary.json` is ~71KB; `tool_review_call_12_user_text.txt` is ~72KB.

## Decision Log

- Decision: Keep the tool protocol unchanged (tools and schemas stay the same), but reduce what we inline into the agent prompts. If the agent needs more detail, it can call read tools again.
  Rationale: This minimizes risk and keeps the system maintainable while reducing tokens.
  Date/Author: 2026-02-03 / Codex
- Decision: Cap reasoning effort per request kind using a “min(config_effort, cap)” rule rather than hard-coding a single value.
  Rationale: Users who set lower reasoning effort should not be upgraded; users who set “high” still get high where it matters (planning), but frequent steps remain faster.
  Date/Author: 2026-02-03 / Codex

## Outcomes & Retrospective

(Fill in after completion.)

Current outcome (2026-02-03):

- Implemented per-request reasoning-effort caps for high-frequency Gen3D agent calls, while keeping plan generation at the configured effort.
- Compacted the largest prompt payloads (`agent_step`, component generation, review delta) by removing large inline JSON dumps and replacing them with concise summaries.
- Added a unit test that ensures `agent_step` prompts do not balloon when a tool returns a huge payload.
- Verified with `cargo test` and `cargo run -- --headless --headless-seconds 1`.

## Context and Orientation

Gen3D has a “Codex-style agent” mode where the model outputs a strict JSON step (`gen3d_agent_step_v1`) containing tool calls. The engine executes those tools and then prompts the model again.

Key files:

- `src/gen3d/ai/agent_loop.rs`: Builds `agent_step` prompts (`build_agent_user_text`) and executes tool calls including `llm_generate_plan_v1`, `llm_generate_component_v1`, and `llm_review_delta_v1`.
- `src/gen3d/ai/prompts.rs`: Builds the system/user prompts for plan generation, component generation, and review delta (currently includes large dumps of plan summaries and scene graph summaries).
- `src/gen3d/ai/mod.rs`: Owns `spawn_gen3d_ai_text_thread`, the shared thread-based OpenAI call helper used by agent_step and LLM tools.
- `src/gen3d/ai/openai.rs`: Builds and sends `/responses` requests, accepts a `reasoning_effort` string.

“Reasoning effort” is a Responses API parameter that affects latency and cost. In this repository it is configured via `config.toml` (OpenAI settings) and currently applied uniformly to all calls.

## Plan of Work

First, make the OpenAI request helper accept an explicit `reasoning_effort` override, and thread it through all agent-related call sites. Then define a small function that computes the effective effort per request kind by taking the minimum of the configured effort and a conservative cap (for example: agent_step capped at low, components capped at medium, reviews capped at medium, plan uses configured).

Second, shrink the agent_step user prompt by removing the pretty-printed JSON blob of recent tool results. Replace it with a compact text summary that includes:

1) each tool call id and tool id,
2) success vs failure,
3) if failure: the error string (truncated),
4) if success: a few key fields for known heavy tools (render paths count, component name/index, plan hash), and
5) paths to on-disk artifacts when applicable (the pass directory already contains full artifacts).

Third, shrink the component-generation prompt by removing the full component plan dump (all components + all anchors). Replace it with:

- root component name + size (for scale),
- the parent component (if any): name + size,
- the target component’s own metadata + required anchors (unchanged),
- direct children (names + which anchors they use),
- assembly notes (unchanged, but consider truncation if extremely long).

Fourth, shrink review-delta prompt by replacing the full pretty JSON `scene_graph_summary` with a compact summary string that still contains all identifiers needed by the schema:

- run_id/attempt/plan_hash/assembly_rev (already required),
- for each component: name, component_id_uuid, parent component name/id (if any), attachment anchor names, resolved_transform (pos/forward/up rounded), and anchor name list.

Keep `smoke_results.json` in full (it is tiny).

Finally, validate by running tests and a headless smoke run. Update `README.md` only if the user-visible config semantics changed. Commit all changes.

## Concrete Steps

Run these commands from the repository root (`/Users/flow/workspace/github/gravimera`):

  - `cargo test`
    Expected: all tests pass.

  - `cargo run -- --headless --headless-seconds 1`
    Expected: the game starts and exits without panic.

To spot improvements in a real run:

  - Start Gen3D Build and inspect the new `gen3d_cache/<run_id>/agent_trace.jsonl`.
  - Compare `agent_step_user_text.txt` sizes across passes; they should stay small (no ~90KB growth).
  - Observe `request_start ... reasoning_effort=...` lines in `attempt_*/pass_*/gen3d_run.log` showing lower effort for agent_step/component calls.

## Validation and Acceptance

Acceptance criteria:

1) Unit tests and headless smoke run pass.
2) `agent_step_user_text.txt` no longer includes full pretty-printed tool result JSON.
3) `build_gen3d_component_user_text` no longer dumps all components’ anchors; it includes only root/parent/child context + required anchors.
4) Review prompt no longer includes a full pretty JSON `scene_graph_summary`.
5) `gen3d_run.log` shows capped reasoning effort for agent_step and component generation when config is “high”.

## Idempotence and Recovery

These changes are safe to re-apply. If the agent becomes “confused” due to less inline information, it can call read tools (list/describe/get_state/get_scene_graph) again; the engine keeps writing full artifacts to disk, so issues remain debuggable.

If a change causes regressions in JSON parsing, revert only the prompt compaction for the failing area and re-run tests; do not revert unrelated changes.

## Artifacts and Notes

Before-change evidence (example):

  - `target/debug/gen3d_cache/983620c4-.../attempt_0/pass_6/agent_step_user_text.txt` (~90KB)
  - `target/debug/gen3d_cache/983620c4-.../attempt_0/pass_6/scene_graph_summary.json` (~71KB)
  - `agent_trace.jsonl` shows ~60 minutes wall, ~59 minutes LLM waits.

After-change evidence should show significantly smaller prompt artifacts and lower reasoning effort for frequent calls.

## Interfaces and Dependencies

No new dependencies are required. Use existing modules:

- `crate::gen3d::ai::openai::generate_text_via_openai` already accepts a `reasoning_effort: &str`; thread this override through `spawn_gen3d_ai_text_thread`.
- Keep public/serialized tool protocol structs unchanged (`src/gen3d/agent/protocol.rs`).
