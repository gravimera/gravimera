# Gen3D: Real Multi-Job Concurrency (Cap 3) + Prefabs Statuses

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

Players can start up to **three** Gen3D generation jobs without waiting for the previous job to finish. Each job appears immediately in the **Prefabs** panel as a “Generating” entry. Clicking an entry opens the Gen3D panel for that specific job. When a job finishes, the Prefabs entry becomes “Completed” (and remains visible). When there are already three generating jobs, clicking the Prefabs **Gen3D** button shows a toast (“生成中任务已满”) and does not start another job. Gen3D uses **real** generation (no mock queue/delay).

The Gen3D panel itself does not show queue position or task switching controls; selection happens only via the Prefabs panel.

## Progress

- [ ] (2026-03-21) Replace single-job Gen3D resources with a `Gen3dJobManager` that can hold up to 3 jobs concurrently (in-memory).
- [ ] (2026-03-21) Make Gen3D orchestration poll **all** running jobs each frame, with deterministic, non-heuristic scheduling for any shared resources.
- [ ] (2026-03-21) Update Prefabs Gen3D button: create a new job when <3 running, otherwise toast “生成中任务已满”.
- [ ] (2026-03-21) Update Prefabs in-flight persistence and UI: statuses are `running|completed|failed` (no `queued` UI); completion marks `completed` instead of deleting.
- [ ] (2026-03-21) Update docs under `docs/` to describe the 3-job cap and statuses (keep `README.md` clean).
- [ ] (2026-03-21) Run rendered smoke test: `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`.
- [ ] (2026-03-21) Commit with clear message(s).

## Surprises & Discoveries

- (TBD)

## Decision Log

- Decision: Use a hard cap of 3 concurrent “running” jobs and **no queue** at the UI level.
  Rationale: Matches the latest product requirement (“最多保存3个生成中任务…满了就提示”) and avoids extra UI complexity.
  Date/Author: 2026-03-21 / Codex

## Outcomes & Retrospective

- (TBD)

## Context and Orientation

Gen3D currently assumes a single global run stored in ECS resources:

- `src/gen3d/state.rs`: `Gen3dWorkshop`, `Gen3dPreview`, `Gen3dDraft`.
- `src/gen3d/ai/job.rs`: `Gen3dAiJob`.
- `src/gen3d/ai/orchestration.rs`: `gen3d_start_build_from_api` starts a run and writes an in-flight entry; `gen3d_poll_ai_job` advances the run each frame.
- `src/gen3d/save.rs`: `gen3d_auto_save_when_done` auto-saves when a run completes; today it **removes** the in-flight entry.
- `src/gen3d/in_flight.rs`: realm-scoped persisted in-flight entries file (currently statuses include `queued`, used only for the previous mock queue).
- `src/model_library_ui.rs`: Prefabs panel merges real prefabs with in-flight entries and renders “Generating” rows; interactions open Gen3D mode.
- `src/app_plugins.rs`: schedules Gen3D systems (poll, apply preview, auto-save, UI updates).

The bug motivating this change is architectural: because only one `Gen3dAiJob/Workshop/Draft/Preview` set exists, the Gen3D panel can only ever show one run. Prefabs in-flight rows are metadata, not real per-run state.

## Plan of Work

Implement “real concurrency” by introducing a `Gen3dJobManager` resource that owns up to three independent Gen3D job contexts. Each context stores the full per-job runtime state (`Gen3dAiJob`, `Gen3dWorkshop`, `Gen3dDraft`, tool feedback history, preview state, and any per-job bookkeeping). The game’s update loop polls every running job each frame so jobs progress concurrently.

For UI, keep a single Gen3D panel instance that always renders the currently “active” job in the manager. The Prefabs panel is the only job selector:

- Prefabs **Gen3D** button: create a new empty job and open the Gen3D panel (unless already 3 running).
- Prefabs in-flight row click: set that run id as active and open the Gen3D panel.

For persistence, continue using `src/gen3d/in_flight.rs`, but change statuses to `running|completed|failed` and stop deleting entries on completion (mark completed instead). Persisted entries are per-realm and are used for Prefabs panel display and for restoring the list after restarts; the in-memory manager is the “source of truth” while the game is running.

All scheduling must be deterministic and generic (no heuristics). If a shared resource must be serialized, use FIFO by created time.

## Concrete Steps

1. Add new job manager types under `src/gen3d/` (new module) and initialize them from `src/app.rs`.
2. Replace system parameters throughout Gen3D systems so they operate on the manager’s active job (UI/input) or iterate over all running jobs (poll/auto-save).
3. Update Prefabs panel interactions in `src/model_library_ui.rs`:
   - Gen3D button: enforce cap 3 (toast when full).
   - In-flight row click: select the job by `run_id`.
4. Update in-flight persistence in `src/gen3d/in_flight.rs` + callers:
   - Add `Completed` status and stop using `Queued`.
   - Change completion flow (`src/gen3d/save.rs`) to mark completed.
5. Update docs under `docs/` (likely `docs/gen3d/README.md` and any control docs).
6. Run smoke test and commit.

## Validation and Acceptance

- Start the game and open Prefabs panel.
- Click Prefabs **Gen3D** 3 times and start 3 builds; all 3 appear as “Generating”.
- Click Prefabs **Gen3D** a 4th time: see toast “生成中任务已满” and no new job appears.
- While jobs run, click each “Generating” Prefabs row: Gen3D panel shows that job’s status.
- When a job finishes, its Prefabs row becomes “Completed” (not removed).
- Rendered smoke test passes:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

## Idempotence and Recovery

Edits are safe to re-run. If the job manager refactor breaks compilation, temporarily keep the old single-job systems alongside the new manager and migrate call-sites incrementally, removing the old path once all uses are updated and tests/smoke pass.

## Artifacts and Notes

- (TBD) Include smoke test transcript and any notable logs once completed.

## Interfaces and Dependencies

New public/internal interfaces to introduce:

- `crate::gen3d::jobs::Gen3dJobManager` (Bevy `Resource`) with:
  - `active_run_id: Option<Uuid>`
  - `jobs: Vec<Gen3dJobContext>` (max length 3 for `running` jobs)
  - APIs to create/select/cancel jobs and to iterate running jobs deterministically.

- `crate::gen3d::in_flight::Gen3dInFlightStatus` must include:
  - `Running`, `Completed`, `Failed`

All existing orchestration entrypoints (start/cancel/resume/save) should be adapted to accept a target `run_id` (from the active job) or to operate on a passed mutable job context.

