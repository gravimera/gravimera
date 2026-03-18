# Gen3D: Two-round `llm_review_delta_v1` policy (cap + focus) to prevent oscillation

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D runs can spend most of their budget in a “review-delta loop”: repeatedly calling the LLM-backed tool `llm_review_delta_v1` to apply tiny transform/anchor tweaks that do not materially improve the result. This shows up in the run cache as many `attempt_0/pass_N/` directories and ends in a time-budget stop, even when the user’s main request has already been satisfied.

After this change, **each Gen3D run will allow at most 2 calls to `llm_review_delta_v1`**, across all orchestrators (agent-step and pipeline). Those two calls become intentional “rounds”:

1. **Round 1 (broad):** find and fix all objective errors (severity=`error`) plus satisfy the user’s main request in a single comprehensive delta.
2. **Round 2 (focused):** fix any remaining objective errors and focus only on the **main issue** (the user’s request). If there is no objective error and the main issue is already satisfied (or cannot be improved deterministically from the structured summaries), the reviewer must **accept** and stop requesting further tweaks.

We explicitly do **not** enable appearance review (`gen3d.review_appearance=true`) to solve this. Appearance review is slower (renders + image payloads) and is not reliably helpful for “micro oscillation” cases where the structured scene graph already contains enough signal to determine satisfaction (e.g. a wing’s world forward vector clearly shows “not pointing forward”).

How to see it working (after implementation):

1. Trigger a Gen3D run that would previously oscillate (a seeded Edit/Fork “small edit” is a common case).
2. Observe in the run cache that there are **no more than 2** tool calls to `llm_review_delta_v1`.
3. Observe that the run finishes normally (agent outputs `done`, or pipeline reaches finish) without spending the full time budget.
4. In the second review-delta call, observe prompt text indicating round 2 focus, and observe that it does not “nibble” at minor pose variants.


## Progress

- [x] (2026-03-19) Drafted this ExecPlan.
- [x] (2026-03-19) Implemented per-run review-delta budget tracking (config + job counter + resets on new Build/Edit/Fork runs).
- [x] (2026-03-19) Enforced a tool gate for `llm_review_delta_v1` when budget is exhausted (immediate, actionable `ok=false` tool result).
- [x] (2026-03-19) Made `llm_review_delta_v1` round-aware (prompt injects round index/max and round 1 vs round 2 focus rules).
- [x] (2026-03-19) Exposed review-delta budget counters in the agent state summary (`state_summary.budgets.review_delta`).
- [x] (2026-03-19) Updated pipeline orchestrator to respect the global 2-round cap (fallback to agent-step when exhausted).
- [x] (2026-03-19) Added tests for prompt focus + tool gating + state summary budgets.
- [x] (2026-03-19) Verified with `cargo test` and the rendered smoke test (`--rendered-seconds 2`).


## Surprises & Discoveries

- Observation: A “no-progress guard” exists, but it only detects repeated identical states; it does not detect A↔B oscillation.
  Evidence: `src/gen3d/ai/agent_step.rs` computes `compute_agent_state_hash(job, draft)` and compares only to the immediately previous hash. A two-state toggle is treated as progress forever.

- Observation: In a real run cache, `llm_review_delta_v1` can be called many times and always return `accepted=false` while continuing to propose small pose changes, eventually hitting the 30-minute time budget.
  Evidence: A run under `~/.gravimera/cache/gen3d/<run_id>/attempt_0/` showed 16 `llm_review_delta_v1` tool calls and 39 passes before `Time budget exhausted`.

- Observation: When `gen3d.review_appearance=false`, the current system prompt for review-delta explicitly allows small placement/alignment tweaks even if smoke/validate report OK (edit sessions).
  Evidence: `src/gen3d/ai/prompts.rs::build_gen3d_review_delta_system_instructions` includes “You MAY propose minimal placement/alignment tweaks … even when smoke/validate report ok.”

- Observation: A review-delta tool call can be split into two phases when appearance review is enabled: a prerender capture (async) followed by the LLM review call.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` can start a review prerender capture for `llm_review_delta_v1`, and `src/gen3d/ai/agent_render_capture.rs` later calls `start_agent_llm_review_delta_call(...)` after the images are ready.


## Decision Log

- Decision: Apply the 2-round `llm_review_delta_v1` cap to **all builds**, not just edit sessions.
  Rationale: Oscillation is a process-management problem and should be controlled consistently regardless of entrypoint/orchestrator.
  Date/Author: 2026-03-19 / user + assistant

- Decision: In round 2, still fix objective severity=`error` issues even if they are not the user’s main request.
  Rationale: Runs must not “ship” a broken draft when the tooling can fix errors; the “main issue only” constraint is about avoiding endless minor improvements, not about ignoring real failures.
  Date/Author: 2026-03-19 / user + assistant

- Decision: Do not require `gen3d.review_appearance=true` for convergence.
  Rationale: Appearance review is slower and not reliably helpful for the oscillation failure mode. The two-round policy should converge using structured summaries only.
  Date/Author: 2026-03-19 / user + assistant

- Decision: Interpret `gen3d.review_delta_rounds_max=0` as “disabled” (not unlimited), and clamp values >2 down to 2.
  Rationale: The product policy is a hard cap to prevent oscillation, while still allowing developers to disable review-delta entirely for debugging.
  Date/Author: 2026-03-19 / assistant

- Decision: Increment the review-delta round budget when the LLM review call starts (not when a prerender capture starts).
  Rationale: Appearance review can prerender before the LLM call; counting at LLM start ensures the budget covers both direct calls and prerendered calls without consuming budget on render failures.
  Date/Author: 2026-03-19 / assistant


## Outcomes & Retrospective

- Outcome: `llm_review_delta_v1` is now capped to 2 calls per run and is explicitly round-aware (round 1 broad; round 2 focused on objective errors + the main issue, otherwise accept).
- Outcome: The tool gate returns an actionable error payload when exhausted, and the pipeline orchestrator consults the same budget to avoid wasting passes.
- Outcome: The agent state summary now exposes `budgets.review_delta.rounds_max/used/remaining`, so the agent can stop calling review-delta once exhausted.
- Verification: `cargo test` passed.
- Verification: rendered smoke test passed:
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- Tradeoff: Some edge cases may still benefit from >2 review-delta calls, but the product decision remains “finish early and stable” over micro-iteration. The recovery path is to start a fresh run.


## Context and Orientation

Gen3D orchestration lives under `src/gen3d/ai/*`. Two orchestrators exist:

1. **Agent-step loop**: the LLM returns `gen3d_agent_step_v1` (a JSON step describing tool calls). The engine executes those tool calls, then requests another step. Key files:
   - `src/gen3d/ai/agent_step.rs`: executes agent actions and requests the next step.
   - `src/gen3d/ai/agent_tool_dispatch.rs`: starts tool calls (including LLM-backed tools).
   - `src/gen3d/ai/agent_tool_poll.rs`: polls in-flight tool calls and applies results.

2. **Pipeline orchestrator**: a deterministic stage machine that calls tools in a fixed sequence and only falls back to agent-step when stuck.
   - `src/gen3d/ai/pipeline_orchestrator.rs`

Both orchestrators ultimately call the same tool executor:

- `src/gen3d/ai/agent_tool_dispatch.rs::execute_tool_call(...)`

`llm_review_delta_v1` is an LLM-backed tool that:

- builds a structured `scene_graph_summary` and `smoke_results`,
- calls the model with strict structured output schema (`ReviewDeltaV1` or `ReviewDeltaNoRegenV1`),
- receives JSON actions such as `tweak_component_resolved_rot_world`, `tweak_anchor`, `tweak_attachment`, etc.,
- applies those actions deterministically via `convert::apply_ai_review_delta_actions`.

The LLM request for review-delta is started here:

- `src/gen3d/ai/agent_review_delta.rs::start_agent_llm_review_delta_call`

The review-delta prompt text is built here:

- `src/gen3d/ai/prompts.rs::build_gen3d_review_delta_system_instructions`
- `src/gen3d/ai/prompts.rs::build_gen3d_review_delta_user_text`

Run artifacts are written under the Gen3D cache dir (default `~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_N/`), including:

- `tool_calls.jsonl`, `tool_results.jsonl`
- `review_delta_raw.txt` (the strict JSON emitted by the LLM tool call)
- `gen3d_run.log` (high-level run events)


## Plan of Work

### 1) Add a global per-run “review delta budget” (max 2)

Add a config knob and job state that are shared across both orchestrators:

- Config (in `config.toml` parsing + `AppConfig`):
  - Key: `[gen3d] review_delta_rounds_max = 2`
  - Default: 2
  - Values: 0..2
    - 0 disables `llm_review_delta_v1` (the tool returns an error instead of calling the model).
    - Values >2 are clamped to 2 (policy cap).

- Job state (in `src/gen3d/ai/job.rs::Gen3dAiJob`):
  - `review_delta_rounds_used: u32` (per run, counts started calls, not “successful applies”)
  - `review_delta_rounds_max: u32` (optional; can read from config each time, but caching makes logs/artifacts simpler)

Reset these fields in all run entrypoints:

- new Build
- Continue
- Seeded Edit/Fork sessions
- Any path that resets `job.run_id` / `job.run_dir`

### 2) Enforce the cap inside the `llm_review_delta_v1` tool gate

Enforcement must be in the tool executor (gate-in-tool), not only in the agent prompt. The invariant:

- If `review_delta_rounds_used >= review_delta_rounds_max` (and max > 0), `TOOL_ID_LLM_REVIEW_DELTA` must not start an LLM request.

Instead, it should return an immediate tool result that is:

- **Actionable:** includes a clear error message and “what to do next” guidance.
- **Non-destructive:** does not mutate draft/plan.
- **Observable:** writes an Info Store `ToolCallResult` event and a run log line (as existing tool results do).

Preferred error behavior:

- Return `ok=false` with `error` like: `Review-delta budget exhausted (used=2 max=2).`
- Include a structured `result` payload with fields:
  - `kind = "review_delta_budget_exhausted"`
  - `used`, `max`
  - `guidance`: short text instructing the agent/pipeline to run `qa_v1` if needed and then finish (`done`) or use deterministic tools.
  - `fixits`: suggested tool calls (only actual tools, e.g. `qa_v1`, `validate_v1`, `smoke_check_v1`) to help the agent recover.

This makes it impossible for agent-step to “silently” keep calling review-delta; repeated attempts will produce immediate tool errors, and the agent prompt can steer to `done`.

### 3) Make `llm_review_delta_v1` round-aware (round 1 vs round 2 focus)

We must make the review-delta LLM call explicitly aware of:

- `review_delta_round_index` (1-based: 1 or 2)
- `review_delta_rounds_max` (2)
- `focus_mode`:
  - Round 1: `broad`
  - Round 2: `main_issue_only`
- `main_issue` (string):
  - Always: the user prompt (for new builds, this is the full prompt; for edit sessions, it is the edit request).
  - Future extension (optional): allow the agent to override `main_issue` via a deterministic tool, but do not add extra LLM turns for this change.

Implementation approach:

- Extend prompt builders (preferred) so call sites cannot forget the round context:
  - Add parameters to `build_gen3d_review_delta_system_instructions(...)` and `build_gen3d_review_delta_user_text(...)` for round info and main-issue text.
  - Or add a small helper that constructs a round header string that is injected into both system and user text.

Round behavior rules to include in the **system** instructions:

- Round 1 (broad):
  - Fix all smoke/validate severity=`error` issues first.
  - Then satisfy the user’s request.
  - Avoid cosmetic-only changes and avoid “micro-iterations”; prefer a single, comprehensive action list.
  - If there are no meaningful actions, return ONLY `{"kind":"accept"}`.

- Round 2 (focused):
  - Fix smoke/validate severity=`error` issues first.
  - Then apply ONLY changes necessary for the main issue.
  - Do NOT propose additional “better-looking” alternatives, minor angle nudges, or exploratory tweaks.
  - If the structured summaries already indicate the main issue is satisfied (or cannot be improved deterministically without appearance review), return ONLY `{"kind":"accept"}`.

This “round contract” is what prevents the reviewer from nibbling at small pose variants in the second call.

### 4) Expose remaining review-delta budget to the agent-step prompt/state summary

Even with tool gating, agent-step should not waste turns repeatedly attempting a blocked tool. Add to the agent-step user text:

- `budgets.review_delta.rounds_max`
- `budgets.review_delta.rounds_used`
- `budgets.review_delta.rounds_remaining`

Add a short agent-step rule:

- If `rounds_remaining == 0`, do not call `llm_review_delta_v1`. Finish with `done` if QA is ok, or switch to deterministic fixes / replan / motion authoring.

This is not a “call X before Y” heuristic; it is a strict budget display that lets the agent avoid futile calls.

### 5) Pipeline orchestrator: respect the global cap

Pipeline currently uses `review_delta_attempts` as a remediation loop counter. Update it to:

- consult the shared `review_delta_rounds_used/max` budget, and
- stop calling `llm_review_delta_v1` once exhausted.

If QA is failing and review-delta is exhausted:

- Prefer deterministic fixits (DraftOps) and motion authoring if applicable.
- If still failing, fall back to agent-step with a clear reason like `review_delta_budget_exhausted_while_qa_failed`.

### 6) Tests and verification

Add tests that prove:

- A run cannot start more than 2 `llm_review_delta_v1` calls.
- Round metadata is present in prompts (system + user text).
- Round 2 prompt contains the “focused / accept if satisfied” instructions.

Where to add tests:

- Prompt tests in `src/gen3d/ai/prompts.rs` (string contains checks).
- Orchestrator/tool gate tests in `src/gen3d/ai/pipeline_orchestrator_tests.rs` and/or a new test module that calls `execute_tool_call` with a mocked job state.

Ensure `mock://gen3d` supports the artifact prefix used by review-delta tool calls (it already supports `tool_review_*` in the recent pipeline work; verify and extend if needed).


## Concrete Steps

When implementing this plan (future work), run commands from repo root:

1. Run unit tests:

   - `cargo test`

2. Run the required rendered smoke test (UI, not headless), per `AGENTS.md`:

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

3. Verify that `llm_review_delta_v1` calls are capped in a real run cache:

   - Locate the run dir: `~/.gravimera/cache/gen3d/<run_id>/attempt_0/`
   - Count tool calls:
     - `rg -n '\"tool_id\":\"llm_review_delta_v1\"' ~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_*/tool_calls.jsonl`
   - Expect: 0, 1, or 2 matches; never >2.


## Validation and Acceptance

Acceptance is behavioral and observable:

- For any run (new build or seeded edit), the engine starts **at most 2** `llm_review_delta_v1` tool calls.
- In the second review-delta call, the tool prompt includes round/focus context and explicitly instructs the reviewer to accept if the main issue is already satisfied.
- Runs that previously spammed passes due to tiny review-delta tweaks now finish significantly earlier, without hitting the time budget.
- If QA is failing and review-delta budget is exhausted, the pipeline falls back to agent-step (or finishes best-effort) with a clear, actionable status message.


## Idempotence and Recovery

- If a developer wants to experiment with different round caps, the config knob allows reducing rounds without code edits (`0..2`; values above 2 are clamped to 2).
- If the cap causes unacceptable outcomes for specific workflows, the recovery path is: start a new run (fresh budget). (Raising the cap above 2 is intentionally blocked by policy.)


## Revision Notes

- (2026-03-19) Implemented the two-round review-delta policy end-to-end (budget tracking + tool gate + prompts + pipeline + tests) and updated the living sections to reflect completion.


## Artifacts and Notes

- Add a run log line whenever review-delta is started that includes round info (e.g. `review_delta_round=1/2 focus=broad`).
- When the cap blocks a tool call, write an Info Store event that records the reason and current budget counters so it’s visible in `info_store_v1/events.jsonl`.


## Interfaces and Dependencies

No external dependencies are required. Changes are confined to:

- Config parsing (`src/config.rs`) and example config (`config.example.toml`).
- Job state (`src/gen3d/ai/job.rs`).
- Tool gating and tool error shape (`src/gen3d/ai/agent_tool_dispatch.rs` and related).
- Review-delta prompt construction (`src/gen3d/ai/prompts.rs`, plus call sites such as `src/gen3d/ai/agent_review_delta.rs`).
- Agent-step state summary prompt (`src/gen3d/ai/agent_prompt.rs`) to expose budget counters.
- Pipeline orchestrator remediation loop (`src/gen3d/ai/pipeline_orchestrator.rs`) to respect the shared cap.
