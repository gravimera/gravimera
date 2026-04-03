# Rebuild Scene Builder as GenScene

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `/Users/lxl/projects/aiprojects/gravimera/PLANS.md`. Save this ExecPlan as `/Users/lxl/projects/aiprojects/gravimera/plans/2026-04-02-genscene-rebuild-v1.md` and validate it with `/Users/lxl/.codex/skills/create-plan/validate-plan.sh` before implementation continues.

## Objective

Deliver a replacement for the legacy Scene Builder so that users can generate a scene from a prompt via a GenScene panel that looks and behaves exactly like Gen3D, including immediate scene creation, asset placement, and close-locking while a build is running.

## Purpose / Big Picture

Replace the legacy Scene Builder with a new GenScene flow. Users will click Generate in the Scenes panel to open a GenScene panel whose layout is identical to Gen3D. They provide a prompt, and GenScene creates a new scene immediately, then selects or generates terrain and 3D models, places them according to the prompt, and updates the GenScene preview.

While a build is running, the GenScene panel must not close. Once the build is no longer running (Stop or completion), the panel may close.

When GenScene triggers GenFloor or Gen3D, the corresponding Terrain and 3D Models lists must immediately show new items using the same placeholder and queue logic as the GenFloor or Gen3D panels.

## Progress

- [x] (2026-04-02) Plan file saved and validated with the create-plan validator.
- [x] (2026-04-02) Remove legacy Scene Builder UI/runtime and Scene Build API.
- [x] (2026-04-02) Add Scenes panel Generate button and GenScene UI shell that is layout-identical to Gen3D.
- [x] (2026-04-02) Implement GenScene asset selection, generation, placement, and preview pipeline with GenFloor and Gen3D placeholder integration.
- [x] (2026-04-02) Add GenScene Automation API endpoints and tests under `test/run_1`.
- [x] (2026-04-02) Update docs and run smoke test.
- [x] (2026-04-03) Commit changes.

## Implementation Plan

- [x] 1. Remove the legacy Scene Builder UI, runtime, and Scene Build API routes so no old systems or tabs remain, including the files that define the Scene Builder UI and Scene Build AI runtime.
- [x] 2. Remove Scene Build resources and systems from app setup and workspace tab wiring so the build UI and systems are no longer registered in the app lifecycle.
- [x] 3. Add a Generate button next to Import in the Scenes panel and wire it to open GenScene in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs` and `/Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs`, keeping it enabled in normal and manage modes.
- [x] 4. Create a `gen_scene` module with workshop state, job phases, plan schema, and preview renderer, copying the Gen3D node tree layout so the panels are visually identical and the close lock triggers while running.
- [x] 5. Build an asset catalog from realm and built-in terrain and prefabs, drive a strict JSON planning call, store plan artifacts, and surface actionable errors on parse or validation failures.
- [x] 6. Trigger GenFloor jobs through the same entry points used by the Terrain panel so placeholder items appear immediately and terrain identifiers can be captured for placement.
- [x] 7. Trigger Gen3D jobs through the existing task queue so model placeholders appear immediately, then collect generated prefab identifiers without switching to the Gen3D preview scene.
- [x] 8. Apply terrain and model placements through scene source patches, update the GenScene preview after each step, and compute preview focus using placed prefab extents.
- [x] 9. Add automation endpoints for GenScene prompt, build, status, and stop in `/Users/lxl/projects/aiprojects/gravimera/src/automation/mod.rs`, plus a new `test/run_1/genscene_api` test mirroring the GenFloor API test flow.
- [ ] 10. Update `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md`, `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md`, and add `/Users/lxl/projects/aiprojects/gravimera/docs/gen_scene/README.md`, then run the rendered smoke test and commit the changes. (Docs updated and smoke test passed; commit pending.)

## Verification Criteria

- The Scenes panel shows a Generate button immediately to the right of Import, and clicking it opens the GenScene panel.
- The GenScene panel layout matches the Gen3D panel layout, including the prompt bar, preview panel, and button column positions and sizing.
- Starting a GenScene build creates a new scene immediately, switches the active scene to it, and prevents closing the panel while the build is running.
- While GenScene is running, the close button and Escape do not close the panel; after Stop or completion, closing works normally.
- Triggering GenFloor or Gen3D from GenScene produces immediate placeholder entries in the Terrain and 3D Models lists, matching the behavior of their own panels.
- Running the rendered smoke test command `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2` completes without a crash.
- The new automation endpoints accept prompt and build requests and report `running=true` during a run and `running=false` when complete or stopped.

## Potential Risks and Mitigations

- Risk: Removing legacy Scene Builder systems could break hidden dependencies. Mitigation: search for all references to the removed types and ensure compilation succeeds before continuing.
- Risk: GenScene layout diverges from Gen3D due to missed nodes or styles. Mitigation: copy the Gen3D UI node tree verbatim and only swap identifiers and labels.
- Risk: Placeholder items do not appear when GenScene triggers GenFloor or Gen3D. Mitigation: reuse the exact GenFloor job start and Gen3D task queue entry points that the library panels watch.
- Risk: LLM plan parsing fails and blocks builds. Mitigation: surface actionable errors to the UI, keep the scene created, and allow a retry without restarting the app.

## Alternative Approaches

- Keep the legacy Scene Builder and add GenScene as a parallel panel, then remove it later after a migration period. This would reduce risk but violates the no-backwards-compatibility requirement.
- Implement a heuristic matching pipeline for assets instead of LLM planning. This would be faster but conflicts with the requirement that Gen3D and GenScene avoid heuristic algorithms.
- Auto-generate all assets without attempting to match existing ones first. This would simplify logic but would not honor the preference for using existing terrain and models.

## Surprises & Discoveries

- Observation: The repo references `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md` but those files do not exist in this working tree. This plan embeds the needed contract-first guidance instead of referencing missing docs.
  Evidence: `/Users/lxl/projects/aiprojects/gravimera/docs/execplans/gen3d_deterministic_pipeline.md:72`.
- Observation: The plan validation script `./.forge/skills/create-plan/validate-plan.sh` does not exist in this repository; the validator exists under the Codex skill directory.
  Evidence: `ls /Users/lxl/.codex/skills/create-plan` shows `validate-plan.sh`.
- Observation: GenScene planning needed a dedicated AI artifact prefix; the mock AI backend only understood `genfloor` until a new `gen_scene` prefix was added.
  Evidence: `src/gen3d/ai/openai.rs` mock response routing is keyed on `artifact_prefix`.

## Decision Log

- Decision: Auto-name the generated scene and auto-switch to it when Build starts.
  Rationale: Matches the requirement that the new scene appears immediately in the Scenes list.
  Date/Author: 2026-04-02 / Codex.

- Decision: GenScene can close only when not running (Stop or completion).
  Rationale: Running is the only blocking state; both Stop and completion transition to non-running.
  Date/Author: 2026-04-02 / Codex.

- Decision: GenScene UI layout must be identical to Gen3D.
  Rationale: Explicit user requirement; reuse the exact node tree and sizing from Gen3D UI.
  Date/Author: 2026-04-02 / Codex.

- Decision: Use the same runtime pathways as GenFloor and Gen3D to surface list placeholders.
  Rationale: The Terrain and 3D Models lists already derive placeholders from GenFloor and Gen3D job and task queue states.
  Date/Author: 2026-04-02 / Codex.

- Decision: Include both realm assets and built-in prefabs or default terrain when matching existing assets.
  Rationale: The asset lists are user-visible and built-ins are part of the available catalog.
  Date/Author: 2026-04-02 / Codex.

- Decision: Use a generic, LLM-driven asset selection and placement plan with strict JSON parsing, avoiding keyword heuristics.
  Rationale: Aligns with the no-heuristic rule for gen3d and genScene.
  Date/Author: 2026-04-02 / Codex.

- Decision: Provide GenScene Automation HTTP API endpoints under `/v1/gen_scene` and remove `/v1/scene_build`.
  Rationale: No backwards compatibility requirement and keeps automation parity.
  Date/Author: 2026-04-02 / Codex.

- Decision: Use `/Users/lxl/.codex/skills/create-plan/validate-plan.sh` to validate this ExecPlan because the repository-local validator path is missing.
  Rationale: The required validation tool exists in the Codex skill directory and is functionally equivalent.
  Date/Author: 2026-04-02 / Codex.

- Decision: Hide the GenScene Save Snapshot button but keep the underlying save request path functional when invoked.
  Rationale: The UI must not show the button, but the save capability should remain available.
  Date/Author: 2026-04-03 / Codex.

## Outcomes & Retrospective

Completed. Rendered smoke test passed and changes were committed.

## Context and Orientation

The Scenes panel UI is defined in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs:340` and the import button interactions are in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:453`. Scene creation and name validation live in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:840` and `/Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:1933`.

The legacy Scene Builder UI is implemented in `/Users/lxl/projects/aiprojects/gravimera/src/scene_authoring_ui.rs:1` and its build actions are wired to the Scene Build AI runtime at `/Users/lxl/projects/aiprojects/gravimera/src/scene_authoring_ui.rs:1031`. The Scene Build AI runtime is defined in `/Users/lxl/projects/aiprojects/gravimera/src/scene_build_ai.rs:322`, registered as a resource in `/Users/lxl/projects/aiprojects/gravimera/src/app.rs:593`, and its systems are wired in `/Users/lxl/projects/aiprojects/gravimera/src/app_plugins.rs:633`. These will be removed.

The Automation HTTP API currently exposes Scene Build endpoints in `/Users/lxl/projects/aiprojects/gravimera/src/automation/mod.rs:2800` and the docs describe them in `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md:1784`. These will be replaced with GenScene endpoints.

Gen3D’s UI layout is the visual reference for GenScene. The preview panel, prompt bar, and button layout are in `/Users/lxl/projects/aiprojects/gravimera/src/gen3d/ui.rs:1213`. Terrain data is stored via `FloorDefV1` in `/Users/lxl/projects/aiprojects/gravimera/src/genfloor/defs.rs:213`, and existing terrain packages are enumerated via `/Users/lxl/projects/aiprojects/gravimera/src/realm_floor_packages.rs:42`. Prefabs are enumerated via `/Users/lxl/projects/aiprojects/gravimera/src/realm_prefab_packages.rs:43`, and descriptors are loaded via `/Users/lxl/projects/aiprojects/gravimera/src/prefab_descriptors.rs:385`. Model list placeholders come from Gen3D task queue state in `/Users/lxl/projects/aiprojects/gravimera/src/model_library_ui.rs:1860`.

Scene sources and patch application are handled via `SceneSourcesPatchV1` and `scene_run_apply_patch_step` in `/Users/lxl/projects/aiprojects/gravimera/src/scene_sources_patch.rs:10` and `/Users/lxl/projects/aiprojects/gravimera/src/scene_runs.rs:188`. Terrain selection is stored per scene via `/Users/lxl/projects/aiprojects/gravimera/src/scene_floor_selection.rs:31`.

## Plan of Work

First, remove the legacy Scene Builder and Scene Build pipeline so there is no conflicting UI or runtime. Delete the modules from `/Users/lxl/projects/aiprojects/gravimera/src/lib.rs`, remove resources from `/Users/lxl/projects/aiprojects/gravimera/src/app.rs`, and delete the systems wired in `/Users/lxl/projects/aiprojects/gravimera/src/app_plugins.rs`. Remove Scene Build automation routes and request structs from `/Users/lxl/projects/aiprojects/gravimera/src/automation/mod.rs`, and update `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md` to describe GenScene instead. Remove the obsolete Scene Build workspace tab path in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs` and its usage in `/Users/lxl/projects/aiprojects/gravimera/src/scene_store.rs`.

Next, add the Generate button to the Scenes panel, positioned immediately to the right of the Import button in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_ui.rs`, and implement the Generate button interactions in `/Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs`. The button should open the GenScene panel and stay enabled in normal and manage modes.

Then, create a new GenScene module under `/Users/lxl/projects/aiprojects/gravimera/src/gen_scene` with a GenScene workshop UI state, a GenScene job state machine, a GenScene plan structured output type, and a preview renderer. The GenScene UI layout must be identical to Gen3D, so copy the node hierarchy, sizing, and arrangement from `/Users/lxl/projects/aiprojects/gravimera/src/gen3d/ui.rs` so the prompt bar, preview panel, and button column match exactly. Replace Gen3D-specific widgets with GenScene equivalents but keep the structure unchanged. The close button must refuse to close while the job is running, and can close once running is false after Stop or completion.

The GenScene pipeline must be LLM-driven and generic. Build an asset catalog of existing terrain and prefabs, including realm packages and built-ins. Use terrain labels and prefab descriptors to describe the catalog. Feed the catalog and prompt into a strict JSON tool output schema that instructs the model to prefer existing assets, specify generation prompts when needed, and output placements with coordinates, yaw, and scale. Avoid keyword heuristics in selection or placement. Parse the JSON strictly and surface actionable errors on parse or validation failure.

When the plan calls for new terrain, use the same GenFloor runtime path as the Terrain panel so the Terrain list shows an immediate placeholder. When the plan calls for new 3D models, use the same Gen3D task-queue workflow as the 3D Models panel so the list shows queued or working placeholders. After Gen3D completes and saves the prefab, collect the prefab identifier for placement.

After assets are ready, apply placements as a scene sources patch and call the scene run apply step to update sources and compile the scene. The preview panel should render the active realm scene using a dedicated camera and render target while GenScene is open. Compute focus and extents from placed instances using prefab sizes so the preview is centered. Update the preview after each step.

Add GenScene Automation API endpoints in `/Users/lxl/projects/aiprojects/gravimera/src/automation/mod.rs` with request types for prompt, build, and stop, plus a status response that reports running state, phase, run identifier, scene identifier, and last error. Update `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md` accordingly. Store test assets and configs under `/Users/lxl/projects/aiprojects/gravimera/test/run_1/genscene_api` and reuse the pattern in `/Users/lxl/projects/aiprojects/gravimera/test/run_1/genfloor_api/run.py`.

Finally, update documentation. Keep `/Users/lxl/projects/aiprojects/gravimera/README.md` concise and put detailed instructions in `/Users/lxl/projects/aiprojects/gravimera/docs/gen_scene/README.md`. Update `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md` to remove Scene Build workspace references and add the Generate button behavior.

## Concrete Steps

Run the plan validator from `/Users/lxl/projects/aiprojects/gravimera` using `/Users/lxl/.codex/skills/create-plan/validate-plan.sh plans/2026-04-02-genscene-rebuild-v1.md` and ensure it reports validation passed before proceeding.

After implementing changes, run the rendered smoke test from `/Users/lxl/projects/aiprojects/gravimera` using `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2` and confirm the app starts without crashing.

Run the GenScene automation test from `/Users/lxl/projects/aiprojects/gravimera` using `python3 test/run_1/genscene_api/run.py` and confirm it reports a successful run and status transition.

Commit after updates with `git add -A` followed by `git commit -m "Rebuild scene builder as GenScene"`.

## Idempotence and Recovery

Running GenScene multiple times should always produce a new scene id and never overwrite an existing scene unless explicitly chosen. If a build fails midway, the new scene should remain in the list and be deletable via the Scenes panel. If a stop is requested, GenScene should cancel any in-flight AI calls via cancel flags and leave the scene in a partially built but valid state.

## Artifacts and Notes

Each GenScene run should write its plan and step artifacts under the scene’s run directory, for example scenes/<scene_id>/runs/gen_scene_<uuid>/plan.json, and should include the final applied scene sources patch for auditability. Keep artifacts compact and JSON-only; avoid markdown.

## Interfaces and Dependencies

Define a new GenScene module rooted at `/Users/lxl/projects/aiprojects/gravimera/src/gen_scene/mod.rs` that re-exports a GenScene workshop resource with fields for open state, prompt, status text, error text, running flag, close lock flag, run identifier, and active scene identifier. The same module should define a GenScene job state machine with phases for idle, planning, generating floor, generating models, applying, done, failed, and canceled. The GenScene plan must be JSON-serializable and include a version field, a terrain choice that is either an existing floor identifier or a GenFloor prompt, an asset list containing existing prefab identifiers or Gen3D prompts, and placement entries with x, z, yaw degrees, optional scale, and optional count.

Add automation API structs and endpoints in `/Users/lxl/projects/aiprojects/gravimera/src/automation/mod.rs` including requests for prompt, build, and stop, plus a status response with running, run identifier, phase, message, scene identifier, and error fields. Update docs in `/Users/lxl/projects/aiprojects/gravimera/docs/automation_http_api.md` and `/Users/lxl/projects/aiprojects/gravimera/docs/controls.md` to match the new flow, and keep `/Users/lxl/projects/aiprojects/gravimera/README.md` concise while placing detailed instructions in `/Users/lxl/projects/aiprojects/gravimera/docs/gen_scene/README.md`.

Plan update note (2026-04-02): Marked the plan validation step complete after the validator passed.
