# Gen3D Status Summary And Parallel Logs

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with [PLANS.md](/Users/flow/workspace/github/gravimera/PLANS.md).

## Purpose / Big Picture

After this change, the Gen3D Status panel should make pipeline progress and parallel work legible without opening run artifacts. The summary block should show the current pipeline step number out of the total steps for the current run shape, and it should expose how many reuse operations were regular copies versus mirrored copies. The scrolling log should show live batch task counts so a user can see how many component or motion jobs are running, queued, and in the batch total.

The behavior is visible directly in the Gen3D UI. During a build or edit run, the summary line should read like `Pipeline: 3/6 | Components`, and the active log line for batch component or motion work should include `tasks: running X | queued Y | total Z`.

## Progress

- [x] (2026-03-29 11:37Z) Read `PLANS.md`, the current Gen3D status/log UI, pipeline stage state, batch pollers, and copy/reuse metrics plumbing.
- [x] (2026-03-29 11:49Z) Implemented route-aware pipeline progress helpers, live batch task-count helpers, and copy-vs-mirror tracking in `src/gen3d/ai/job.rs`.
- [x] (2026-03-29 11:51Z) Updated Gen3D tool results and UI rendering so the summary shows pipeline/reuse data and the log shows batch task counts.
- [x] (2026-03-29 11:56Z) Updated `docs/gen3d/README.md`, ran focused Gen3D job-state tests, and passed the required rendered smoke test.
- [ ] Commit the result.

## Surprises & Discoveries

- Observation: Gen3D already tracks copy activity in `src/gen3d/ai/job.rs` via `Gen3dCopyMetrics`, but the current counters do not distinguish mirrored reuse from normal copy reuse.
  Evidence: `Gen3dCopyMetrics::note_tool_result(...)` increments copy totals from `apply_reuse_groups_v1` and manual copy tools, but the stored outcome JSON only contains `source`, `target`, and `mode`.

- Observation: The active Status log line is rendered in `src/gen3d/ui.rs`, not in the batch pollers, which means live running/queued/total data can be appended at render time without mutating the persisted log entry model.
  Evidence: `workshop.status_log.active` only stores `seq`, `step`, `why`, and `started_at`; `ui.rs` formats the active line separately.

- Observation: `apply_reuse_groups_v1` intentionally truncates its stored `outcomes[]` preview, so mirrored-count tracking would be wrong for large reuse batches unless the tool result also emits aggregate copy/mirror totals.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` caps stored reuse outcomes at `MAX_OUTCOMES = 24`.

## Decision Log

- Decision: Use dynamic pipeline progress based on the run’s current route instead of a fixed global enum ordinal.
  Rationale: Create runs, preserve-mode edits, and optional review stages do not all traverse the same stage list. The UI should report progress against the route this run is actually following.
  Date/Author: 2026-03-29 / Codex

- Decision: Record mirrored counts from tool-result payloads instead of inferring them from component names or reuse-group shape.
  Rationale: The repository rule for Gen3D algorithms is “no heuristics.” Adding explicit alignment data to copy/reuse tool results keeps the status counters generic and observable.
  Date/Author: 2026-03-29 / Codex

## Outcomes & Retrospective

The Status summary now shows route-aware pipeline progress and explicit reuse totals, and the active log line for component/motion batches shows running, queued, and total task counts. Finished batch summaries also preserve the total task count so the log stays useful after the batch completes.

The implementation stayed generic: copy-vs-mirror counts come from explicit tool-result alignment data, not from component names or symmetry heuristics. Focused `job.rs` tests passed, and the required rendered smoke test (`cargo run -- --rendered-seconds 2` with a temporary `GRAVIMERA_HOME`) exited successfully.

## Context and Orientation

The user-facing Gen3D Status panel is rendered in `src/gen3d/ui.rs`. That file builds two strings: a compact summary block and a multiline log block. The summary currently shows state, prefab save state, draft counts, run attempt/step/time, token totals, and the last active or finished step. The log currently shows finished entries from `Gen3dStatusLog.entries` plus one active entry from `Gen3dStatusLog.active`.

The deterministic pipeline state machine lives in `src/gen3d/ai/pipeline_orchestrator.rs`, and its stage enum lives in `src/gen3d/ai/job.rs` as `Gen3dPipelineStage`. “Pipeline stage” means the high-level phase such as planning, generating components, QA, or review. The job also owns pending component and motion batch state, plus in-flight and queued task vectors.

Batch tools are polled by `src/gen3d/ai/agent_component_batch.rs` and `src/gen3d/ai/agent_motion_batch.rs`. Those files already know the counts for completed, in-flight, queued, and total tasks. They also build the final tool-result JSON for the batch, which means they are the right place to ensure finished log summaries have stable task totals.

Copy and mirror reuse is tracked in `src/gen3d/ai/job.rs` by `Gen3dCopyMetrics`, which is updated whenever a tool result is recorded. Automatic reuse comes from `apply_reuse_groups_v1` in `src/gen3d/ai/agent_tool_dispatch.rs`. Manual copy and mirror tools also return per-target outcomes from the same file.

## Plan of Work

First, update `src/gen3d/ai/job.rs` so the job can answer three UI questions directly: what the current pipeline progress is, what the live batch task counts are, and how many successful reuse operations were regular copies versus mirrored copies. The pipeline progress helper will build the ordered stage list for the current run shape. The copy metrics logic will start reading explicit `alignment` data from tool results.

Second, update the tool-result JSON that feeds those metrics. `apply_reuse_groups_v1` and the manual copy/mirror tools in `src/gen3d/ai/agent_tool_dispatch.rs` should emit the selected alignment per outcome. The batch tool-result summaries in `src/gen3d/ai/status_steps.rs` should include task counts so finished log entries preserve the total batch size.

Third, update `src/gen3d/ui.rs` so the summary block shows `Pipeline: current/total | label` and `Reuse: copied X | mirrored Y`. The active step rendering should append the live task counters for component and motion batches. The scrolling log should keep its existing structure but include the appended task counters on the active line and the richer finished summaries from `status_steps.rs`.

Finally, update the Gen3D docs in `docs/gen3d/README.md` so the Status panel behavior is documented, then run the required rendered smoke test and commit the work.

## Concrete Steps

From `/Users/flow/workspace/github/gravimera`:

1. Edit:
   - `src/gen3d/ai/job.rs`
   - `src/gen3d/ai/agent_tool_dispatch.rs`
   - `src/gen3d/ai/status_steps.rs`
   - `src/gen3d/ui.rs`
   - `docs/gen3d/README.md`

2. Run a fast compile-oriented check for the touched code if needed.

3. Run the required smoke test:

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4. Inspect `git diff --stat` and commit with a clear message.

## Validation and Acceptance

Acceptance is met when all of the following are true:

- During a Gen3D run, the Status summary shows a pipeline line with a current step number, total step count, and stage label.
- The same summary shows copy and mirrored counts derived from real tool outcomes, not guessed from names.
- When a component or motion batch is active, the log’s active line includes `running`, `queued`, and `total` task counts.
- When a component or motion batch finishes, the finished log summary still exposes the total batch size.
- The rendered smoke test starts and exits without a crash.

## Idempotence and Recovery

The code and doc edits are additive and safe to re-run. If the smoke test fails because of an unrelated local environment issue, rerun the same command with a fresh temporary `GRAVIMERA_HOME` to confirm whether the failure reproduces. If any UI wording needs adjustment after running, update the same helper methods rather than reintroducing duplicated string formatting.

## Artifacts and Notes

Expected UI examples after implementation:

    Pipeline: 3/6 | Components
    Reuse: copied 2 | mirrored 1

    [012] Generate components — Generate missing components from the plan. | tasks: running 2 | queued 1 | total 5 → running… (8.4s)

## Interfaces and Dependencies

The following helper surfaces should exist by the end of the work:

- In `src/gen3d/ai/job.rs`, a job-level helper that returns the current pipeline progress as `(current_step, total_steps, label)` or an equivalent small struct.
- In `src/gen3d/ai/job.rs`, a job-level helper that returns live batch task counts for the active component or motion batch.
- In `src/gen3d/ai/job.rs`, copy metrics accessors that return successful regular-copy and mirrored-copy totals.
- In `src/gen3d/ai/agent_tool_dispatch.rs`, copy/reuse tool-result payloads that include per-outcome alignment data so metrics can distinguish mirrored work from regular copy work.

Change note: Updated after implementation to record the explicit aggregate mirror-count fix, the completed validation steps, and the remaining commit step.
