# `docs/todo.md` comprehensive test cases (real + manual)

This document maps each item in `docs/todo.md` to **real tests** (automation HTTP API scripts under `test/run_1/`) and to a **manual QA checklist** for UI-only behavior.

## Quick run (recommended)

Run the automation “real tests”:

- `python3 test/run_1/gen3d_tasks_queue_api/run.py`
- `python3 test/run_1/gen3d_tasks_queue_seeded_api/run.py`
- `python3 test/run_1/prefab_duplicate_api/run.py`

Run the rendered smoke test (per `AGENTS.md`):

```bash
tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2
```

## Coverage matrix

### 1) Gen3D pipeline mode must author motion (`move`)

Automated:

- `test/run_1/gen3d_tasks_queue_seeded_api/run.py`
  - Enqueues a seeded edit task and asserts the saved prefab contains authored motion coverage for `channel="move"` (regression guard for pipeline-mode finishing with no motion).

Manual QA:

1. Generate a movable unit via Prefabs → `Generate`.
2. Confirm it animates movement after generation (not a static “slide”).
3. Use Preview → `Modify` and confirm the edited unit still has move animation.

### 2) Meta panel: remove Gen3D Copy/Edit/Fork buttons; add Close button

Automated:

- N/A (UI composition).

Manual QA:

1. Enter Build Realm.
2. Double-click a unit selection circle to open Meta panel.
3. Confirm **no** Copy/Edit/Fork buttons exist.
4. Confirm a **Close** button exists in the top-right corner and closes the panel.

### 3) Double-click instance opens Prefabs + selects prefab + pops Preview (and Meta for units)

Automated:

- N/A (double-click is local input).

Manual QA:

1. Place/spawn an instance that has `ObjectPrefabId` (a prefab-backed unit or build object).
2. Double-click it:
   - If it’s a unit: Meta opens.
   - Prefabs panel opens (Models tab selected), correct prefab highlighted.
   - Preview overlay opens for that prefab.

### 4) Preview overlay: `Modify` + `Duplicate`; taller info section

Automated:

- Modify behavior (seeded edit-overwrite semantics):
  - `test/run_1/gen3d_tasks_queue_seeded_api/run.py` (`kind=edit_from_prefab`)
- Duplicate behavior (new prefab id + full package copy + no missing-def warnings):
  - `test/run_1/prefab_duplicate_api/run.py` (`POST /v1/prefabs/duplicate`)

Manual QA:

1. Open a prefab Preview overlay from Prefabs panel.
2. Confirm `Modify` and `Duplicate` buttons exist.
3. Confirm the info section shows more text than before (taller scroll area).
4. Click `Modify`:
   - Gen3D workshop opens in Build Preview.
   - It is seeded from that prefab (button label should be `Edit` when seeded).
5. Click `Duplicate`:
   - A new prefab appears in the list (new UUID).
   - No warning like `Missing prefab def ... referenced by .../prefabs/<id>/prefabs` is emitted.

### 5) Prefabs panel: multi-session UI + single-runner task queue + indicators + placeholder + `Generate` rename

Automated:

- Single-runner + FIFO queue:
  - `test/run_1/gen3d_tasks_queue_api/run.py`
- Seeded edit + fork semantics (overwrite vs new-id):
  - `test/run_1/gen3d_tasks_queue_seeded_api/run.py`

Manual QA:

1. Click `Generate` twice quickly to start 2 builds.
2. Confirm:
   - Only one task is “working” at a time; the other is “waiting”.
   - A placeholder row appears immediately after starting a new build.
   - Placeholder shows working/waiting indicator.
3. While a task is running, click another prefab item that has a queued edit:
   - It should show its associated Gen3D panel/session.
   - Prefabs list name shows `Editing…:` (green) while running and `Queued…:` (yellow) while queued.
4. While an edit is running (e.g., Hippo), open another prefab edit (e.g., Santa) and open its Gen3D panel:
   - The preview shows the selected prefab, not the running one.
5. Open a queued session in the Gen3D panel:
   - Status summary shows `State: Queued (position N of M)`.
   - Primary button label reads `Queued` and is visually disabled.
   - Prefabs list shows a yellow `↻` queue indicator (same glyph as generating, different color, no rotation).
   - `Cancel queue` button is visible; clicking it removes the session from the queue.
6. Confirm the header button reads `Generate` (not `Gen3D`).

### 6) Gen3D panel UX: clear + merged Build/Edit button; HTTP task APIs

Automated:

- HTTP task queue APIs (`/v1/gen3d/tasks*`) without switching to Build Preview:
  - `test/run_1/gen3d_tasks_queue_api/run.py`
  - `test/run_1/gen3d_tasks_queue_seeded_api/run.py`

Manual QA:

1. Open Gen3D workshop.
2. Confirm there is **no** “Clear Prompt” button.
3. Add text + images to the prompt input; confirm the textbox `Clear` affordance appears.
4. Click `Clear`; confirm it clears both text and images.
5. Confirm the primary action button label:
   - Fresh session: `Build`
   - Seeded session: `Edit`
   - While running: `Stop`
6. Confirm `Save Snapshot` button visibility:
   - Visible only while generating.
   - Hidden while queued or completed (auto-save already handled).
   - In an Edit run, clicking `Save Snapshot` creates a new prefab item (forked snapshot).
