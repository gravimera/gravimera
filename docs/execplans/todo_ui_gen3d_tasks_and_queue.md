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
- [x] (2026-03-22 12:55 CST) Fix: pipeline mode must author motion when required (movable drafts must have `move` coverage).
- [x] (2026-03-22 13:07 CST) UI: Meta panel remove Copy/Edit/Fork; add Close button.
- [ ] (2026-03-22) UI: Double-click instance also opens Prefabs + selects item + pops Preview overlay (when `ObjectPrefabId` exists).
- [ ] (2026-03-22) UI: Preview overlay adds `Modify` and `Duplicate`; info area taller.
- [ ] (2026-03-22) UI: Prefabs panel shows Gen3D working/waiting indicators and new-build placeholder; rename `Gen3D` → `Generate`.
- [x] (2026-03-22 12:55 CST) UI: Gen3D panel remove “Clear Prompt”; unify “Clear” to clear both text+images; merge Build/Continue into one button labeled `Build|Edit|Stop`.
- [ ] (2026-03-22) Automation: Add Gen3D task queue endpoints (list + status) and allow running Gen3D tasks without switching to Build Preview.
- [ ] (2026-03-22) Validation: `cargo test`, rendered smoke test, and a `test/run_1/...` automation script.
- [ ] (2026-03-22) Commit(s): clear, scoped messages per milestone.


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

(Fill in at completion.)


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
- Clicking `Generate` should bring the player to a fresh Gen3D build context (switch to BuildScene Preview and clear the Gen3D prompt/images if no active task is running; otherwise create a waiting task panel).
- Add thumbnail overlays:
  - Working indicator: active Gen3D task editing that prefab, or active new build placeholder.
  - Waiting indicator: queued task for that prefab/placeholder.
- Insert a placeholder row in the list immediately after a new-build task is started; replace it after the task saves a prefab.

Acceptance: Prefabs list shows real-time Gen3D status and new-build placeholder behavior.

### 6) Gen3D panel UX updates

In `src/gen3d/ui.rs` and `src/gen3d/images.rs`:

- Remove the “Clear Prompt” button and handler.
- Make a single “Clear” button appear when either prompt text or images are present; clicking clears both and resets the session.
- Merge Build/Continue into one main button:
  - When running: label `Stop` and stop the run.
  - When not running and session is resumable: label `Edit` and resume.
  - When not running and no session: label `Build` and start a new build.

Acceptance: no “Clear Prompt” button exists; Clear works for both text+images; only one main build/edit/stop button exists.

### 7) Automation HTTP API: Gen3D task queue + status

In `src/automation/mod.rs` and `docs/automation_http_api.md`:

- Add endpoints to manage a Gen3D task queue (list + per-task status; enqueue build/edit/fork).
- Ensure tasks can run while staying in BuildScene Realm (no need to open the Gen3D workshop UI).
- Ensure at most one task runs at a time (others remain waiting).

Acceptance: automation can enqueue multiple tasks, poll list/status, and observe serialized execution.


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
