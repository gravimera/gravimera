# Gen3D: Prevent QA-gated force-regeneration deadlocks (long-running “inspect/regenerate” loops)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

In Gen3D seeded edit sessions, the engine defaults to “preserve existing components” mode to avoid regenerating already-generated geometry unless the agent explicitly requests it.

Today there is a contract mismatch that can cause a long-running loop:

1) `llm_review_delta_v1` can request regeneration for style/shape changes (via `regen_component` actions).
2) The agent prompt tells the agent to “apply pending regen next” by calling `llm_generate_components_v1` with `force=true`.
3) The engine refuses `force=true` regeneration unless the latest QA indicates errors (`last_validate_ok=false` or `last_smoke_ok=false`).

When QA is clean, the agent repeatedly tries the same blocked `force=true` regeneration, the engine repeatedly refuses it, and the run consumes its time budget without making progress.

After this change:

- “Pending regen” will never contain force-regeneration work that is currently impossible under the QA gate.
- The agent will be told (in structured state) when regeneration was requested but is blocked by the QA gate, along with the deterministic next steps (use `apply_draft_ops_v1` / `llm_review_delta_v1` tweaks, or disable preserve mode for a full rebuild).
- Even if the agent still tries a blocked force-regeneration call, the engine will clear the pending regen queue so it cannot deadlock.

User-visible outcome: Gen3D edit runs stop burning minutes repeatedly “inspecting and regenerating” when regeneration is QA-blocked, and instead either make forward progress via deterministic tweaks or finish best-effort quickly.

## Progress

- [x] (2026-03-11 01:32Z) Write this ExecPlan and capture the current deadlock contract.
- [x] (2026-03-11 01:32Z) Implement new agent-state bucket for “regen requested but QA-gated”.
- [x] (2026-03-11 01:32Z) Update review-delta apply path to bucket regen requests into actionable vs. QA-gated vs. budget-gated.
- [x] (2026-03-11 01:32Z) Update generation tool dispatch to clear pending regen when rejecting `force=true` (safety net).
- [x] (2026-03-11 01:32Z) Update agent system prompt to remove contradictory instructions and document the escape hatch (disable preserve mode).
- [x] (2026-03-11 01:32Z) Update documentation (`docs/gen3d/edit_preserve_existing_components.md`) to match the new semantics.
- [x] (2026-03-11 01:32Z) Add deterministic unit tests (no LLM/network) that fail before and pass after.
- [x] (2026-03-11 01:32Z) Run `cargo test` and the required rendered smoke test.
- [ ] (2026-03-11 01:32Z) Commit.

## Surprises & Discoveries

- Observation: The agent prompt currently instructs “If `pending_regen_component_indices` is non-empty, call `llm_generate_components_v1` with `force=true`”, but the engine refuses `force=true` regeneration when QA is clean.
  Evidence: `src/gen3d/ai/agent_prompt.rs` (pending regen rule) vs `src/gen3d/ai/agent_tool_dispatch.rs` (QA-gated force-regeneration).

- Observation: When `llm_generate_components_v1` returns the “Refusing force:true regeneration…” error, the current batch tool implementation returns early before removing the requested indices from `pending_regen_component_indices`, so the agent sees the same pending queue forever.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` batch generation early-return path runs before the `request_set` → `pending_regen_component_indices.retain(...)` clearing.

## Decision Log

- Decision: Treat “pending regen” as “pending generation work that is currently executable”, and move non-executable regen requests into a separate, explicit state bucket.
  Rationale: This is deterministic, machine-observable, and avoids relying on heuristic “agent will figure it out” behavior.
  Date/Author: 2026-03-11 / flow + agent

- Decision: Keep the QA gate on `force=true` regeneration (preserve-mode escape hatch remains: disable preserve mode for full rebuild/regeneration).
  Rationale: Preserve mode is intended to prevent endless subjective regeneration loops; large style changes should be expressed explicitly via a non-preserve replan/rebuild.
  Date/Author: 2026-03-11 / flow + agent

## Outcomes & Retrospective

- (2026-03-11 01:32Z) Implemented a deterministic bucketing of review-delta regen requests into actionable vs QA-gated, exposed via `get_state_summary_v1`, and added a batch-tool safety net so a refused force-regen cannot deadlock the pending queue.
  Evidence: `cargo test` passes; rendered smoke test runs via `GRAVIMERA_HOME="$(mktemp -d)/.gravimera" cargo run -- --rendered-seconds 2`.

## Context and Orientation

Key concepts (plain language):

- “Seeded edit session”: a Gen3D run started from an existing prefab (“Edit”/“Fork”). In these runs the engine sets `preserve_existing_components_mode=true` by default so already-generated components are not regenerated accidentally.
- “Force regeneration”: passing `force=true` to `llm_generate_components_v1` / `llm_generate_component_v1` to explicitly regenerate already-generated components in preserve mode.
- “QA gate”: the engine refuses `force=true` regeneration unless the latest QA indicates errors (`last_validate_ok=false` OR `last_smoke_ok=false`).
- “Pending regen”: the list `state_summary.pending_regen_component_indices` surfaced to the agent, intended to tell the agent what to generate/regenerate next.

Relevant code (paths from repo root):

- `src/gen3d/ai/agent_prompt.rs`: the Gen3D agent system prompt; includes the “apply pending regen next” instruction and emits `state_summary.*` JSON.
- `src/gen3d/ai/agent_tool_poll.rs`: applies `llm_review_delta_v1` results and sets `job.agent.pending_regen_component_indices` based on requested regen actions.
- `src/gen3d/ai/agent_tool_dispatch.rs`: implements tool semantics, including the QA gate for `force=true` regeneration and the batch generation tool’s pending-queue clearing.
- `docs/gen3d/edit_preserve_existing_components.md`: documents preserve mode and the QA-gated `force=true` rule.

## Plan of Work

### Milestone 1 — Add an explicit “QA-gated regen requests” state bucket

In `src/gen3d/ai/job.rs`, extend `Gen3dAgentState` with:

    pending_regen_component_indices_blocked_due_to_qa_gate: Vec<usize>

This bucket holds component indices that were requested for regeneration (typically via review-delta) but cannot be executed in preserve mode because the QA gate is not open.

In `src/gen3d/ai/agent_prompt.rs`, include this field in `state_summary` so the agent can respond deterministically without “guessing”.

### Milestone 2 — Bucket review-delta regen requests into actionable vs blocked

In `src/gen3d/ai/agent_tool_poll.rs`, in the `llm_review_delta_v1` apply path:

- Take `apply.regen_indices` and deduplicate.
- For each requested index:
  - If it refers to a missing component (`actual_size=None`), keep it actionable (pending generation).
  - If it refers to an already-generated component (`actual_size=Some`) AND `preserve_existing_components_mode=true` AND the QA gate is closed (`last_validate_ok!=Some(false)` AND `last_smoke_ok!=Some(false)`), move it into `pending_regen_component_indices_blocked_due_to_qa_gate`.
  - Otherwise, it is actionable; keep it in `pending_regen_component_indices` (subject to regen budget gating for already-generated components).
- Keep the existing `pending_regen_component_indices_skipped_due_to_budget` behavior, but only apply budget gating to already-generated components.

### Milestone 3 — Safety net: clear pending regen on QA-gated refusal

In `src/gen3d/ai/agent_tool_dispatch.rs`, in the `llm_generate_components_v1` batch tool handler:

- When rejecting a request due to the QA gate (“Refusing force:true regeneration…”), clear the requested regen indices out of `job.agent.pending_regen_component_indices` before returning.
- Record those indices into `pending_regen_component_indices_blocked_due_to_qa_gate` (dedup + sort) so state_summary remains explanatory.

This prevents a single bad step from permanently deadlocking the run.

### Milestone 4 — Update agent prompt rules to remove contradictions

In `src/gen3d/ai/agent_prompt.rs`, update the “pending regen” rules so they match the engine contract:

- Do not instruct the agent to always use `force=true` just because pending regen is non-empty.
- In preserve mode, only use `force=true` when QA indicates errors (`last_validate_ok=false` OR `last_smoke_ok=false`).
- If `pending_regen_component_indices_blocked_due_to_qa_gate` is non-empty, do not retry `force=true` regeneration. Either:
  - use `apply_draft_ops_v1` / `llm_review_delta_v1` for placement/attachment fixes, or
  - if the user requested a true rebuild/style change, disable preserve mode by calling `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`, then regenerate without `force`.

### Milestone 5 — Documentation

Update `docs/gen3d/edit_preserve_existing_components.md` to describe:

- the new “blocked due to QA gate” bucket in `get_state_summary_v1`,
- how to interpret it,
- and the deterministic recovery options (tweak tools vs turning off preserve mode).

### Milestone 6 — Tests (no LLM/network dependency)

Add unit tests that validate the bucketing logic. Suggested approach:

- Extract the “bucket regen indices” logic into a small helper function (for example in `src/gen3d/ai/agent_tool_poll.rs` or a new small module), then unit test it directly.
- Test cases:
  - Preserve mode + QA clean + regen requested for generated components ⇒ indices appear in `blocked_due_to_qa_gate`, not in `pending_regen_component_indices`.
  - Preserve mode + validate_ok=false ⇒ regen requested for generated components ⇒ indices appear in `pending_regen_component_indices`.
  - Missing component indices are never budget-gated and remain actionable.

### Milestone 7 — Validation and shipping

From repo root:

1) Run tests:

    cargo test

2) Run the required rendered smoke test (UI; do NOT use `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3) Commit with a clear message (example):

    gen3d: prevent QA-gated force-regen deadlocks

## Concrete Steps

Work should proceed in small commits that keep the project building:

1) Add the new agent-state field and surface it in `get_state_summary_v1`.
2) Implement regen-request bucketing in the review-delta apply path.
3) Add the batch-tool safety net to clear pending regen on QA-gated refusal.
4) Update the agent prompt rules.
5) Add and run unit tests.
6) Update docs.
7) Run `cargo test` and the rendered smoke test.
8) Commit.

## Validation and Acceptance

Acceptance is human-verifiable behavior:

- In a preserve-mode edit session with clean QA, a review-delta regen request no longer causes the agent loop to repeatedly attempt `force=true` regeneration for the same components until the time budget is exhausted.
- `get_state_summary_v1` explicitly shows regen requests that are blocked by the QA gate.
- `cargo test` passes.
- The rendered smoke test starts and renders without crashing.

## Idempotence and Recovery

- All new bucketing logic is deterministic and safe to repeat.
- If a regen request is blocked, the run remains valid and resumable; the recovery path is explicit:
  - tweak placement/attachments with deterministic ops, or
  - disable preserve mode and rebuild/regenerate explicitly.

## Artifacts and Notes

When debugging a stuck run, the following should make the situation obvious from artifacts/state:

- `state_summary.pending_regen_component_indices` (actionable work)
- `state_summary.pending_regen_component_indices_blocked_due_to_qa_gate` (non-actionable under current policy)
- tool results for `llm_generate_components_v1` showing `Refusing force:true regeneration…` (if the agent tries anyway)

## Interfaces and Dependencies

Do not add new external dependencies. Keep all changes within the existing Gen3D modules.

At the end of implementation:

- `get_state_summary_v1` includes a new field: `pending_regen_component_indices_blocked_due_to_qa_gate`.
- The agent prompt rules match the engine’s QA-gated `force=true` regeneration contract.
- Batch component regeneration cannot deadlock the run by leaving an un-executable pending regen queue intact.

---

Plan revision notes:

- 2026-03-11 01:32Z: Updated `Progress` and `Outcomes & Retrospective` to reflect completed implementation work and validation evidence.
