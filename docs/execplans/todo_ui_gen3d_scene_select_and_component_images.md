# Implement `docs/todo.md` (scene double-click selection, Prefabs ESC close, Gen3D UI rearrange, component image passing)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

Note: `AGENTS.md` references `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md`, but those files are not present in this working tree (the `docs/agent_skills/` directory exists). This plan therefore embeds the needed “prompt ↔ tool contract” checks where relevant.


## Purpose / Big Picture

`docs/todo.md` currently lists several UX and Gen3D correctness items that make the “scene ↔ Prefabs ↔ Gen3D” loop smoother and make component generation more accurate when users supply reference images.

After this change, a player can:

1. Double-click any prefab-backed object in the scene and jump to the Prefabs panel with that prefab selected and scrolled into view, without an automatic preview overlay popping up.
2. Dismiss the Prefabs panel with the Escape key.
3. Use a cleaner Gen3D UI:
   - No “Preview” title label above the preview.
   - No “Collision” toggle in the preview panel.
   - Status overlay appears on the left side; the small “run stats” overlay appears on the right side of the preview.
   - A top-right `Exit` button exits the Gen3D workshop; Escape also exits.
4. When the user provides reference images, those images are also sent to per-component LLM generation calls, but bounded to avoid context bloat:
   - Send at most 2 images per component generation call.
   - If a reference image exceeds a fixed maximum resolution, downsample to a smaller image and use that for component generation calls.


## Progress

- [x] (2026-03-23 00:45 CST) Draft ExecPlan from `docs/todo.md` and code inspection.
- [ ] Implement Prefabs selection without preview on scene double-click (and scroll into view).
- [ ] Implement Escape-to-close Prefabs panel.
- [ ] Rearrange Gen3D UI (remove Preview label + Collision toggle; move Status overlay left; move run stats right; add Exit button + ESC exit).
- [ ] Send user images to component LLM calls (max 2; downsample large images; update manifests and prompts).
- [ ] Update docs (`docs/todo.md`, any relevant UI/Gen3D docs) to match behavior.
- [ ] Validation: `cargo test`, rendered smoke test, and existing “real tests” under `test/run_1/`.
- [ ] Commits: scoped messages per milestone.


## Surprises & Discoveries

- Observation: Current scene double-click behavior explicitly calls `ModelLibraryUiState::request_preview`, which opens the Preview overlay. This conflicts with the current `docs/todo.md` requirement to not show the preview panel on scene double-click.
  Evidence: `src/rts.rs` double-click handler calls `ui.model_library.request_preview(prefab_id)`.

- Observation: Prefabs list selection state is currently derived from the Preview overlay state (`ModelLibraryUiState.preview`), so selecting without opening the preview requires a dedicated selection field.
  Evidence: `src/model_library_ui.rs` uses `state.preview.as_ref().map(|p| p.prefab_id)` in list styling + scroll-into-view logic.

- Observation: Per-component LLM generation calls currently send zero images (`image_paths = Vec::new()` / `sent_images=false`), even when `job.user_images` is non-empty.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` (`TOOL_ID_LLM_GENERATE_COMPONENT`), `src/gen3d/ai/agent_component_batch.rs`, and legacy component generation loops in `src/gen3d/ai/orchestration.rs`.


## Decision Log

- Decision: Add a dedicated “selected prefab id” field to `ModelLibraryUiState` (selection without opening Preview) and update the existing scroll-into-view + styling systems to use it.
  Rationale: Keeps selection, scrolling, and preview overlay behavior consistent without requiring the preview overlay to be open.
  Date/Author: 2026-03-23 / codex

- Decision: Implement component-generation image bounds as deterministic rules:
  - Pick the first N images in user-provided order (N=2) for every component generation call.
  - Downsample any image larger than a fixed max dimension to that bound, preserving aspect ratio, and write it under the Gen3D attempt `inputs/` folder.
  Rationale: Deterministic, generic, and avoids “pick best image” heuristics while still preventing runaway request sizes.
  Date/Author: 2026-03-23 / codex


## Outcomes & Retrospective

(To be filled in as milestones complete.)


## Context and Orientation

Relevant files and responsibilities:

- `docs/todo.md`: the source checklist for this work.
- Scene selection / double-click:
  - `src/rts.rs`: click + double-click selection logic; currently opens Prefabs preview on double-click.
- Prefabs (Model Library) UI:
  - `src/model_library_ui.rs`: Prefabs panel UI, selection marking, preview overlay spawning, and scroll-into-view logic.
  - `src/workspace_ui.rs`: top toolbar tabs; opening/closing Prefabs panel is driven by `TopPanelUiState.selected == Models`.
- Gen3D UI:
  - `src/gen3d/ui.rs`: Gen3D workshop UI layout (Preview label, Collision toggle, side/status overlay placement, top-left Realm toggle).
  - `src/app_plugins.rs`: schedules Gen3D UI systems (toggle + collision toggle, etc).
- Gen3D AI / tool execution:
  - `src/gen3d/ai/agent_tool_dispatch.rs`: tool implementations including `llm_generate_component_v1` and `llm_generate_components_v1`.
  - `src/gen3d/ai/agent_component_batch.rs`: async component batching.
  - `src/gen3d/ai/orchestration.rs`: legacy per-component generation loop + input caching into attempt folders.

Definitions (plain language):

- “Preview panel” (Prefabs): the large Prefab preview overlay in `model_library_ui` that spawns a preview camera + metadata view.
- “Gen3D Preview label”: the literal `Text::new("Preview")` marker shown above the Gen3D preview panel.
- “Status panel”: the collapsible Gen3D overlay currently positioned on the right side and opened by the `≡` button.
- “Running status”: the small “Run time / Tokens …” overlay inside the Gen3D preview panel (currently positioned top-left inside the preview).


## Plan of Work

### 1) Scene double-click selects Prefab without opening Preview

In `src/model_library_ui.rs`:

- Add a `selected_prefab_id: Option<u128>` field to `ModelLibraryUiState`.
- Add a method `select_prefab(prefab_id: u128)` that:
  - sets `selected_prefab_id`,
  - clears `search_focused` (so keyboard navigation doesn’t get stuck in the search box).
- Update list item styles and selection marks to use `selected_prefab_id` (not `preview.prefab_id`).
- Update `model_library_scroll_selected_item_into_view` to scroll based on `selected_prefab_id`.
- Ensure opening the preview overlay also sets `selected_prefab_id` to that prefab id (so selection remains consistent).

In `src/rts.rs`:

- In the double-click handler, replace `request_preview(prefab_id)` with `select_prefab(prefab_id)` while still opening the Prefabs tab.

Acceptance:

- Double-clicking a prefab-backed entity opens the Prefabs panel, highlights the correct prefab, and scrolls to it.
- The Prefabs Preview overlay does not open automatically from this action.

### 2) Escape closes Prefabs panel

In `src/workspace_ui.rs` (or a small new system in an appropriate module that already owns `TopPanelUiState`):

- When `TopPanelUiState.selected == Some(TopPanelTab::Models)` and Escape is pressed:
  - If the Prefabs preview overlay is open, allow the existing preview ESC handler to close it first.
  - Otherwise, clear `TopPanelUiState.selected = None` so the Prefabs panel closes.

Acceptance:

- Pressing Escape closes the Prefabs panel (and closes preview overlay first if it is open).

### 3) Rearrange Gen3D UI

In `src/gen3d/ui.rs`:

- Remove the top-left Realm/Preview toggle button (`Gen3dToggleButton` + label update logic).
- Remove the `Text::new("Preview")` marker from the preview panel.
- Remove the Collision toggle UI row (`Gen3dCollisionToggleButton`) and its label update code.
- Move the status overlay (`Gen3dSidePanelRoot`) to the left side:
  - change `right: 12px` → `left: 12px` (keep top/bottom/width).
  - move the `≡` toggle button near that overlay (recommended: top-left).
- Move the run stats overlay inside the preview panel to the right side (top-right inside preview).
- Add an `Exit` button on the top-right of the Gen3D workshop:
  - Clicking exits BuildScene Preview back to BuildScene Realm.
  - Pressing Escape in Gen3D also exits (unless an image viewer overlay is open; that overlay keeps its own Escape-to-close behavior).

In `src/app_plugins.rs`:

- Remove scheduled systems for the deleted buttons (Realm toggle + collision toggle).
- Add scheduling for the new Gen3D Exit button system and Gen3D Escape-to-exit system.

Acceptance:

- Gen3D workshop shows no “Preview” label and no collision toggle.
- Status overlay is on the left; run stats overlay is on the right of the preview.
- `Exit` button exists top-right; ESC exits Gen3D workshop.

### 4) Component generation calls send bounded reference images

In `src/gen3d/ai/orchestration.rs`:

- Extend `cache_gen3d_inputs` to also produce a `component_reference_images` list:
  - choose at most 2 images (first in order).
  - if an image is above a max dimension, write a downsampled JPEG into the attempt `inputs/` folder and use that path.
  - record the decision in `inputs_manifest.json` (so runs are debuggable).
- Store those processed paths in `Gen3dAiJob` (e.g. `user_images_component: Vec<PathBuf>`).

In `src/gen3d/ai/agent_tool_dispatch.rs`:

- For `TOOL_ID_LLM_GENERATE_COMPONENT` and `TOOL_ID_LLM_GENERATE_COMPONENTS`, pass `job.user_images_component.clone()` into `spawn_gen3d_ai_text_thread(...)` instead of `Vec::new()`.

In `src/gen3d/ai/agent_component_batch.rs` and any legacy component loops in `src/gen3d/ai/orchestration.rs`:

- Replace `image_paths = Vec::new()` with `job.user_images_component.clone()`.
- Set `sent_images=true` when the list is non-empty (keep existing logging/reporting consistent).

In `src/gen3d/ai/prompts.rs`:

- Update `build_gen3d_component_system_instructions()` to explicitly mention that reference images may be provided and should be used as guidance (without changing schema).

Acceptance:

- When the user supplies images, component generation requests include up to 2 images.
- Oversized images are downsampled once per attempt and reused, reducing request sizes.
- When the user supplies no images, component generation requests send none (unchanged behavior).


## Concrete Steps

All commands are run from the repository root (`/Users/flow/workspace/github/gravimera`).

1. Implement each milestone, running `cargo test` for fast feedback.
2. Run “real tests” scripts (rendered + automation) after code changes:

   - `python3 test/run_1/gen3d_tasks_queue_api/run.py`
   - `python3 test/run_1/gen3d_tasks_queue_seeded_api/run.py`
   - `python3 test/run_1/prefab_duplicate_api/run.py`
   - `python3 test/run_1/gen3d_fresh_build_preview_hidden/run.py`

3. Run the rendered smoke test (per `AGENTS.md`) at the end of the work:

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`


## Validation and Acceptance

Acceptance is met when:

- Scene double-click opens Prefabs and selects + scrolls, without showing the Prefabs preview overlay.
- Escape closes the Prefabs panel.
- Gen3D UI matches `docs/todo.md` (Preview label removed, Collision toggle removed, status overlay left, run stats right, Exit + ESC exits).
- Component generation calls include bounded reference images when provided.
- `cargo test` passes.
- All scripts under `test/run_1/*/run.py` listed above pass in rendered automation mode.
- Rendered smoke test runs for 2 seconds without crash or fallback to headless.


## Idempotence and Recovery

- All changes are source-only and can be re-run by repeating builds/tests.
- Test scripts create isolated run folders under their own `test/run_1/.../tmp/` directories and should not be committed.


## Artifacts and Notes

- Update `docs/todo.md` by checking off completed items.
- If any UI behavior is not automatable via HTTP, record a short manual QA checklist in `docs/real_tests/` and keep it in sync with the todo.

