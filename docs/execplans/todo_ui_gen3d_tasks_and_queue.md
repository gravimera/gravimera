# Implement `docs/todo.md` (Gen3D motion + UI workflow + task queue + HTTP APIs)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gravimera currently has the core UI panels needed for prefab browsing (Prefabs panel + Preview overlay), unit inspection (Meta panel), and Gen3D generation (Gen3D workshop). However, several workflow gaps make Gen3D feel “single-threaded” and awkward to drive from UI and from external automation:

- Gen3D pipeline orchestrator can finish without authoring motion channels/clips for movable units.
- Meta panel still exposes Gen3D Copy/Edit/Fork actions that don’t match the intended “Prefabs-first” workflow.
- Double-clicking an instance only opens Meta (units), instead of also surfacing the underlying prefab in Prefabs + Preview.
- Prefabs/Preview panels don’t offer “Modify” (Gen3D edit) and “Duplicate” (new prefab id) affordances.
- Gen3D panel UX has redundant/fragmented “clear” behavior and confusing Build vs Continue semantics.
- Automation HTTP API can drive Gen3D, but today it is coupled to the “Build Preview” scene and lacks a clear “task list/status” abstraction for queued Gen3D requests.

After this change, a player (and an external test rig) can:

1. Generate/edit prefabs with a consistent Prefabs → Preview → Gen3D flow:
   - Double-click an instance: Meta opens (if unit) and Prefabs opens with that prefab selected; Preview overlay pops.
   - Preview overlay offers `Modify` (Gen3D edit) and `Duplicate` (copy new prefab id) actions.
2. See Gen3D activity directly in Prefabs:
   - A new-build “placeholder” entry appears immediately after starting a build.
   - Prefabs being edited show a working/waiting indicator on their thumbnails.
3. Use Gen3D pipeline mode reliably for motion:
   - Pipeline mode deterministically authors motion when required, instead of finishing with no move channel coverage.
4. Drive Gen3D via HTTP without opening the Gen3D UI panel:
   - Queue Gen3D tasks, poll a task list and per-task status, and observe that only one task runs at a time.

The work must be validated by:

- Running unit tests (`cargo test`).
- Running the rendered smoke test (per `AGENTS.md`): `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`.
- Adding at least one automation-level “real test” script under `test/run_1/` that exercises the new Gen3D task APIs against a running Gravimera process.


## Progress

- [x] (2026-03-22 10:05 CST) Drafted this ExecPlan from `docs/todo.md`.
- [x] (2026-03-22 14:25 CST) ExecPlan: expanded detailed design for Gen3D sessions/queue and HTTP task endpoints.
- [x] (2026-03-22 12:55 CST) Fix: pipeline mode must author motion when required (movable drafts must have `move` coverage).
- [x] (2026-03-22 13:07 CST) UI: Meta panel remove Copy/Edit/Fork; add Close button.
- [x] (2026-03-22 13:25 CST) UI: Double-click instance also opens Prefabs + selects item + pops Preview overlay (when `ObjectPrefabId` exists).
- [x] (2026-03-22 13:59 CST) UI: Preview overlay adds `Modify` and `Duplicate`; info area taller.
- [x] (2026-03-22 16:05 CST) UI: Prefabs panel shows Gen3D working/waiting indicators and new-build placeholder; rename `Gen3D` → `Generate`.
- [x] (2026-03-22 12:55 CST) UI: Gen3D panel remove “Clear Prompt”; unify “Clear” to clear both text+images; merge Build/Continue into one button labeled `Build|Edit|Stop`.
- [x] (2026-03-22 19:52 CST) Automation: Add Gen3D task queue endpoints (list + status) and allow running Gen3D tasks without switching to Build Preview.
- [x] (2026-03-22 19:52 CST) Validation: `cargo test`, rendered smoke test, and a `test/run_1/...` automation script.
- [x] (2026-03-22 19:55 CST) Commit(s): clear, scoped messages per milestone.
- [x] (2026-03-22 21:10 CST) Fix: Preview `Duplicate` loads all package defs from disk (no “Missing prefab def … referenced by …/prefabs”); duplicate now copies `gen3d_source_v1/`. Added `POST /v1/prefabs/duplicate` + a real test script.


## Surprises & Discoveries

- Observation: Gen3D smoke/motion validation does not treat “missing move slot coverage” as a motion validation failure, so pipeline mode can finish without authored motion.
  Evidence: `src/gen3d/ai/orchestration.rs::build_gen3d_smoke_results` only gates on `motion_validation.ok`, which can be true when there are zero `move` slots.

- Observation: The Automation HTTP API already has `GET /v1/gen3d/status`, but it is documented as requiring Build Preview and does not expose a multi-task queue abstraction.
  Evidence: `docs/automation_http_api.md` “Gen3D endpoints” section.


## Decision Log

- Decision: Treat “movable unit without any `move` slot coverage” as a hard non-finish condition in pipeline mode, and trigger `llm_generate_motion_authoring_v1` deterministically (bounded) before finishing.
  Rationale: This matches the existing agent-step prompt contract (“if movable and has_move=false, author motion before finishing”) and prevents silent motionless units in pipeline mode.
  Date/Author: 2026-03-22 / codex

- Decision: Label the merged Gen3D primary button as `Edit` when the current session is seeded (edit/fork), otherwise label it as `Build` (and always label as `Stop` while running).
  Rationale: This matches `docs/todo.md` (“fresh build → Build; seeded build → Edit”) and avoids implying that a stopped fresh build is an “Edit”.
  Date/Author: 2026-03-22 / codex

- Decision: Keep README changes minimal; detailed workflow and API semantics belong in `docs/`.
  Rationale: Repo policy in `AGENTS.md`.
  Date/Author: 2026-03-22 / codex


## Outcomes & Retrospective

- Gen3D now supports multiple sessions/panels with a single serialized runner (FIFO), including Prefabs UI indicators + new-build placeholders.
- Automation API now exposes Gen3D task queue endpoints (`/v1/gen3d/tasks*`) that run without switching to Build Preview, plus a `test/run_1/...` real test script.
- Documentation (`docs/automation_http_api.md`, `docs/todo.md`) is updated to match the new workflow and endpoints.


## Context and Orientation

Key code locations:

- Todo list: `docs/todo.md`
- Gen3D pipeline orchestrator: `src/gen3d/ai/pipeline_orchestrator.rs`
- Gen3D smoke/motion validation: `src/gen3d/ai/orchestration.rs` (`build_gen3d_smoke_results`) and `src/gen3d/ai/motion_validation.rs`
- Gen3D UI panel: `src/gen3d/ui.rs`, `src/gen3d/images.rs`, `src/gen3d/state.rs`
- Meta panel UI: `src/motion_ui.rs`
- Prefabs + Preview overlay UI: `src/model_library_ui.rs`
- Double-click selection handling: `src/rts.rs` + `src/motion_ui.rs::record_click_and_check_double`
- Automation HTTP API: `src/automation/mod.rs` + `docs/automation_http_api.md`

Terms:

- “Prefabs panel”: the in-game list panel implemented by `src/model_library_ui.rs`.
- “Preview overlay”: the modal-like overlay for a selected prefab (also in `src/model_library_ui.rs`).
- “Meta panel”: the unit inspection/action panel (`src/motion_ui.rs`).
- “Gen3D panel”: the Gen3D workshop UI (`src/gen3d/ui.rs`) and AI job (`src/gen3d/ai/...`).
- “Pipeline mode”: deterministic Gen3D orchestrator (`Gen3dAiMode::Pipeline`) implemented in `src/gen3d/ai/pipeline_orchestrator.rs`.


## Plan of Work

### 1) Fix pipeline motion authoring

In `src/gen3d/ai/pipeline_orchestrator.rs`:

- Add a deterministic “motion required” predicate:
  - If the draft root has mobility (ground/air) and the planned component attachment graph has no `move` channel slots, consider motion authoring required.
- In the QA stage, when QA is otherwise ok but motion is required, call `llm_generate_motion_authoring_v1` (bounded by `motion_authoring_attempts`), invalidate previous smoke/validate flags, and re-run QA before finishing.
- Update completion gating (`run_complete_enough_for_pipeline_finish`) to require move slot coverage for movable drafts.

Acceptance: pipeline mode cannot finish a movable draft without at least one `move` slot, and does not prematurely finish immediately after motion authoring without rerunning QA.

### 2) Meta panel: remove Gen3D Copy/Edit/Fork; add Close button

In `src/motion_ui.rs`:

- Remove the “Gen3D” section with Copy/Edit/Fork buttons from the Meta list builder.
- Add a small Close button in the top-right of the Meta panel header.
- Wire Close to the same behavior as Escape (stop meta speak bubble if active, close the panel).

Acceptance: Meta panel no longer shows Copy/Edit/Fork; Close button hides the panel.

### 3) Double-click instance opens Prefabs + Preview (and Meta for units)

In `src/rts.rs` selection click handling:

- Detect double-clicks for any picked object (unit or build object) using the existing double-click timer.
- On double-click:
  - If it’s a unit, open Meta panel as before.
  - If the entity has `ObjectPrefabId`, open the Prefabs tab (`TopPanelUiState.selected = Models`), select that prefab, and request Preview overlay open for that prefab.

In `src/model_library_ui.rs`:

- Do not discard `pending_preview` when the panel is closed; keep it until the panel becomes visible so the double-click path can “open then preview”.

Acceptance: double-clicking a placed prefab instance reliably opens Prefabs and pops Preview overlay (and Meta for units).

### 4) Preview overlay: Modify + Duplicate; taller info

In `src/model_library_ui.rs` Preview overlay builder:

- Add `Modify` button:
  - Seeds a Gen3D edit-overwrite session for the prefab id and switches to Gen3D workshop (BuildScene Preview) (or queues when busy, per task-queue design).
- Add `Duplicate` button:
  - Creates a new prefab id copy. Prefer preserving Gen3D provenance when possible (copy gen3d_source/edit bundle when present), else copy defs with an id remap.
- Increase overlay height so the info scroll area shows more text.

Acceptance: Preview overlay shows the two new buttons; Duplicate results in a new prefab entry in Prefabs list.

### 5) Prefabs panel: Generate button, working/waiting indicators, placeholder

In `src/model_library_ui.rs`:

- Rename the header button from `Gen3D` to `Generate`.
- Clicking `Generate` should bring the player to a fresh Gen3D build context (switch to BuildScene Preview and open a fresh Gen3D session/panel; if another task is running, the new session exists but its Build is queued).
- Add thumbnail overlays:
  - Working indicator: active Gen3D task editing that prefab, or active new build placeholder.
  - Waiting indicator: queued task for that prefab/placeholder.
- Insert a placeholder row in the list immediately after a new-build task is started; replace it after the task saves a prefab.

Acceptance: Prefabs list shows real-time Gen3D status and new-build placeholder behavior.

#### Design details: Gen3D sessions + single-runner queue

The core requirement is “multiple Gen3D panels exist, but only one Gen3D task runs at a time”. The existing Gen3D implementation is single-session (`Gen3dWorkshop` + `Gen3dAiJob` + `Gen3dDraft` resources). We will keep those resources as the **active session** (the one shown in the Gen3D Workshop UI), and add a new resource that holds **inactive sessions** (other panels) plus a serialized “task queue”.

Definitions:

- “Session / panel”: a set of Gen3D UI state + draft + job state (prompt, images, status log, seeded edit metadata, etc.). A session can exist without running.
- “Task”: a session that the user (or HTTP) requested to run (Build/Edit clicked). Tasks have a queue state: `idle` (not queued), `waiting`, `running`, `done`, `failed`, `canceled`.
- “Runner”: at most one session whose `Gen3D` job is actively running. The runner keeps progressing even when the active UI session changes.

Data model (new resource):

- Add `Gen3dTaskQueue` resource (in `src/gen3d/`), which tracks:
  - `active_session_id`: which session is currently loaded into the global Gen3D resources (`Gen3dWorkshop`, `Gen3dAiJob`, `Gen3dDraft`) and therefore shown in the Gen3D panel UI.
  - `running_session_id`: optional session id for the currently running task (must be unique).
  - `queue`: ordered list of session ids that are `waiting`.
  - `sessions`: per-session metadata (kind, associated prefab id if any, last known status/error, timestamps).
  - `inactive_states`: the full session state (`Gen3dWorkshop`, `Gen3dAiJob`, `Gen3dDraft`) for sessions that are not currently active.

Session kinds we need for the todo:

- `NewBuild`: no prefab id (creates a placeholder while waiting/running until saved).
- `EditPrefab { prefab_id }`: seeded edit-overwrite session for a Gen3D-saved prefab id.

Queue runner behavior:

- When the user clicks Build/Edit for the active session:
  - If no session is currently running, start the task immediately (becomes `running`).
  - If another session is running, mark this session `waiting` and append to `queue` (the running task continues).
- When a running session finishes (`job.running=false` and `job.build_complete=true`), automatically start the next session in `queue` (first-in-first-out), without requiring BuildScene Preview to be active.
- If a queued task fails, mark it `failed` and continue to the next task.

Prefabs panel representation:

- “Working” indicator is shown when a prefab has an associated session and it is not `waiting` (either the active edit session, or the running edit session).
- “Waiting” indicator is shown when a prefab has an associated session that is `waiting`.
- Placeholder rows are derived from sessions of kind `NewBuild` that are `waiting` or `running` **and** do not yet have a saved prefab id:
  - The placeholder is inserted at the top of the list (most recent).
  - It has the same indicator logic (working if running, waiting if queued).
  - When the session saves a prefab (auto-save or manual), the placeholder disappears and the real prefab package appears via the normal on-disk prefab scan.

Prefab click behavior:

- Clicking a prefab with an associated session selects that Gen3D session and switches to Gen3D Workshop (BuildScene Preview).
- Dragging still spawns the prefab instance as before (drag threshold remains the gate). Placeholders are not spawnable/previewable; clicking them only opens the Gen3D session.

### 6) Gen3D panel UX updates

In `src/gen3d/ui.rs` and `src/gen3d/images.rs`:

- Remove the “Clear Prompt” button and handler.
- Make a single “Clear” button appear when either prompt text or images are present; clicking clears both and resets the session.
- Merge Build/Continue into one main button:
  - When running: label `Stop` and stop the run.
  - When not running and a seeded Edit/Fork session exists: label `Edit` and run (first click starts the seeded run; subsequent clicks start a fresh run dir / cache folder).
  - When not running and no session: label `Build` and start a new build.

Acceptance: no “Clear Prompt” button exists; Clear works for both text+images; only one main build/edit/stop button exists.

### 7) Automation HTTP API: Gen3D task queue + status

In `src/automation/mod.rs` and `docs/automation_http_api.md`:

- Add endpoints to manage a Gen3D task queue (list + per-task status; enqueue build/edit/fork).
- Ensure tasks can run while staying in BuildScene Realm (no need to open the Gen3D workshop UI / no need to switch scenes).
- Ensure at most one task runs at a time (others remain waiting).

Acceptance: automation can enqueue multiple tasks, poll list/status, and observe serialized execution.

#### API design details

We will keep the existing “single-session” endpoints (`/v1/gen3d/prompt`, `/v1/gen3d/build`, etc.) for interactive/manual workflows, but add a task-oriented API so automation can run multiple Gen3D requests deterministically:

- `GET /v1/gen3d/tasks`
  - Returns a list of tasks (queued + running + recently completed) with stable `task_id`s and an explicit `state` field (`waiting|running|done|failed|canceled`).
  - Each task includes:
    - `kind`: `build` or `edit_from_prefab` or `fork_from_prefab`
    - `prefab_id_uuid` when applicable (edit/fork)
    - `run_id` when started
    - `status` and `error` (last known UI status/error string)
    - `result_prefab_id_uuid` when saved (new build or fork)
- `POST /v1/gen3d/tasks/enqueue`
  - Enqueues a new Gen3D task without requiring BuildScene Preview or local UI.
  - Request body (v1):
    - `kind`: `"build"` | `"edit_from_prefab"` | `"fork_from_prefab"`
    - `prompt`: string (required for build; optional for edit/fork; uses existing prompt limits)
    - `prefab_id_uuid`: string (required for edit/fork)
  - Response:
    - `task_id`
- `GET /v1/gen3d/tasks/<task_id>`
  - Returns a single task’s status (same shape as an entry in `/v1/gen3d/tasks`).

Error handling requirements (contract-first):

- Enqueue must validate inputs and return actionable errors:
  - bad UUIDs, missing required fields, prompt over limits, edit/fork for non-Gen3D-saved prefabs, etc.
- The runner must enforce the single-runner constraint deterministically:
  - never start a second run while one is running; tasks must remain `waiting`.

Testing expectation:

- Add a “real test” script that starts Gravimera with automation enabled and mock Gen3D backend (`mock://gen3d`), enqueues two tasks, steps frames while polling `/v1/gen3d/tasks`, and asserts that:
  - exactly one task is `running` at a time
  - both tasks reach `done`
  - at least one resulting prefab id is produced (for build/fork tasks)


## Concrete Steps

Commands (from repo root):

1. Unit tests:

    cargo test

2. Rendered smoke test (required by `AGENTS.md`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3. Automation “real test” (to be added under `test/run_1/`):


## Plan Revisions

- (2026-03-22 12:55 CST) Marked pipeline-motion fix and Gen3D panel UX work as complete in `Progress`, and recorded the `Build` vs `Edit` button-label decision. This keeps the ExecPlan aligned with current implementation.
- (2026-03-22 13:07 CST) Marked the Meta-panel close-button change as complete in `Progress`. This keeps the plan’s UI checklist aligned with the implemented behavior.
- (2026-03-22 13:25 CST) Marked the double-click Prefabs+Preview workflow as complete in `Progress`. This keeps the plan aligned with the updated selection behavior.
- (2026-03-22 13:59 CST) Marked the Preview overlay `Modify`/`Duplicate` actions as complete in `Progress`. This keeps the plan aligned with the new prefab workflow affordances.

    - Start Gravimera with `--automation`.
    - Enqueue two Gen3D tasks via HTTP.
    - Step frames while polling task status until both complete.


## Validation and Acceptance

Acceptance criteria mapped to `docs/todo.md`:

- Pipeline mode generates motion when required and cannot finish a movable unit without `move` coverage.
- Meta panel: Copy/Edit/Fork removed; Close button works.
- Double-clicking an object opens Prefabs + selects prefab + opens Preview overlay (and Meta if unit).
- Preview overlay: Modify + Duplicate exist; info area taller.
- Prefabs panel: shows working/waiting indicators; Generate button exists; placeholder behavior works.
- Gen3D panel: Clear Prompt removed; Clear clears both images+text; one Build/Edit/Stop button works.
- HTTP: can run Gen3D tasks without opening the Gen3D UI; tasks list + status APIs exist.


## Idempotence and Recovery

- UI changes are idempotent (safe to rebuild panels repeatedly).
- Gen3D task queue endpoints must reject invalid requests with actionable errors, and must not start a second Gen3D run while one is active.
- If a queued task fails, it should be marked failed and the runner should proceed to the next task (unless explicitly configured to stop-on-failure).


## Artifacts and Notes

- Keep automation test artifacts under `test/run_1/` (config, logs, screenshots, etc.) per `AGENTS.md`.
- Keep README minimal; update `docs/automation_http_api.md` and any relevant `docs/gen3d/...` docs as needed.
