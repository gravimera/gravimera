# Separate Scene Display Names From Scene IDs

This ExecPlan is a living document. The sections Progress, Surprises & Discoveries, Decision Log, and Outcomes & Retrospective must be kept up to date as work proceeds.

This document must be maintained in accordance with /Users/lxl/projects/aiprojects/gravimera/PLANS.md.

## Objective

Enable scenes to keep a stable scene_id for their directory name while exposing a separate, user-facing display name in the Scenes list. After this change, renaming a scene updates only the display name stored in scene metadata and does not rename the directory. GenScene and manual scene creation should populate the display name without blocking build start, and the UI should always fall back to scene_id when the display name is missing.

## Purpose / Big Picture

Users should be able to rename scenes freely without changing on-disk directories or breaking references. The Scenes list will show a readable display name rather than a raw directory name, and GenScene will assign a short display name from the prompt while keeping an internal scene_id for filesystem paths. This makes scene organization clearer and reduces build-time stalls caused by naming logic.

## Progress

- [x] (2026-04-03 09:40 CST) Draft and validate this ExecPlan.
- [x] (2026-04-03 18:10 CST) Update scene metadata helpers to read and write display names separately from scene_id.
- [x] (2026-04-03 18:15 CST) Replace Scenes list labels with display names and add rename UI that only updates metadata.
- [x] (2026-04-03 18:20 CST) Update GenScene creation to allocate scene_id independently and set display name from the prompt without blocking build start.
- [ ] (2026-04-03 18:25 CST) Add tests and update docs to reflect the new display name behavior (completed: docs updates; remaining: tests).

## Surprises & Discoveries

Observation: Scene metadata already stores a label field alongside scene_id in the scaffolded meta.json, but the Scenes list ignores it and renders the scene_id directly. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/realm.rs:255-271 and /Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:335-405.

Observation: GenScene allocates scene_id by slugifying or translating the prompt, which couples the directory name to the user prompt and can introduce build-time latency. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/gen_scene/job.rs:1223-1290.

## Decision Log

Decision: Use the existing meta.json label field as the display name surfaced in the Scenes list and editable by the user. Rationale: The label already exists in the scene scaffold and is the natural home for a user-facing name. Date/Author: 2026-04-03 / Codex.

Decision: Keep scene_id as the stable directory identifier and never rename directories when a display name changes. Rationale: The user explicitly wants renaming to avoid touching directory names, and this keeps references stable. Date/Author: 2026-04-03 / Codex.

Decision: Display names fall back to scene_id when missing or empty, and the “(Current)” tag appends to the display name rather than replacing it. Rationale: This preserves existing behavior for legacy scenes while improving readability. Date/Author: 2026-04-03 / Codex.

Decision: GenScene should allocate scene_id without LLM calls on the critical path and set display names from the prompt in a separate metadata update. Rationale: Build start should be responsive; prompt-based names can be refined later without blocking. Date/Author: 2026-04-03 / Codex.

Decision: Use UUID-based scene_id values for new scenes (manual add and GenScene) so directory names are stable and decoupled from display names. Rationale: This meets the requirement that directory names follow scene_id while keeping user-facing names editable. Date/Author: 2026-04-03 / Codex.

Decision: Derive the initial display name by trimming the first non-empty prompt line and truncating to a short maximum with “...”. Rationale: Keeps names readable without blocking build start or adding a prompt-translation dependency. Date/Author: 2026-04-03 / Codex.

## Outcomes & Retrospective

Not started.

## Context and Orientation

Scenes are stored under a realm’s scenes directory, and list_scenes enumerates subdirectories to produce scene_id values based on folder names. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/realm.rs:143-169. Scene source scaffolding writes a meta.json containing scene_id and label, but the label is currently set to the same value as scene_id. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/realm.rs:255-271. The Scenes list UI builds items by reading scene_id and rendering it directly, appending “(Current)” when active. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:335-405. Manual scene creation validates and uses the provided name as the directory name, with no separate display name. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:894-931 and /Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs:1977-2001. GenScene’s job creates the scene directory using a prompt-derived scene_id, including LLM translation when needed. Evidence: /Users/lxl/projects/aiprojects/gravimera/src/gen_scene/job.rs:1223-1290.

In this plan, “scene_id” means the directory name under the realm’s scenes folder, while “display name” means the user-facing label stored in scene metadata and shown in the UI. The goal is to keep scene_id stable and editable display names separate.

## Implementation Plan

- [ ] Add helper functions in /Users/lxl/projects/aiprojects/gravimera/src/realm.rs to read and write the scene display name in meta.json, with a fallback to scene_id when the label is missing or empty, and reuse the existing SceneSourcesV1 load/write path.
- [ ] Update the Scenes list rebuild to resolve display names for each scene_id, append “(Current)” to the display name, and keep selection logic and sorting keyed by scene_id and timestamps.
- [ ] Add a rename flow in the Scenes panel that edits only the display name, validates trimmed non-empty input, saves via the new helper, and refreshes the list without touching directories or multi-select mode.
- [ ] Adjust GenScene creation to allocate scene_id independently of the display name and write a short prompt-derived display name to meta.json without blocking build start.
- [ ] Add tests under /Users/lxl/projects/aiprojects/gravimera/test/run_1 that cover display name persistence, rename behavior, and fallback to scene_id, and update any existing scene metadata tests that assume label equals scene_id.
- [ ] Update documentation under /Users/lxl/projects/aiprojects/gravimera/docs to describe display names and rename behavior, keeping README.md concise.

## Plan of Work

First, add metadata helpers in realm.rs to read and write a scene display name from meta.json. This mirrors the existing description helpers and provides a single place to define the fallback rule and validation. Next, update the Scenes list rebuild to load the display name for each scene and render it with the current-scene suffix, keeping all selection logic based on scene_id. Then add a rename workflow in the Scenes panel that edits only the display name, validates it, writes it to metadata, and refreshes the list; this should be designed to work in the normal mode and avoid interference with multi-select mode. After the UI is updated, change GenScene’s scene creation to decouple directory names from display names by allocating scene_id independently and writing a prompt-derived display name to metadata without blocking the build start. Finally, add tests under test/run_1 to exercise display name persistence and rename behavior, and update documentation to describe the new naming behavior.

## Concrete Steps

Work from /Users/lxl/projects/aiprojects/gravimera.

1) Implement the metadata helpers and UI changes described above.
2) Run the rendered smoke test:
   tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2
3) Run any new or updated tests under test/run_1 (for example a new test script for display name behavior).
4) Commit the changes with a clear message. When setting commit timestamps, follow the Asia/Shanghai offset rule: before 14:00 add +10 hours, after 14:00 add +6 hours.

## Verification Criteria

- The Scenes list renders a human-readable display name for each scene and appends “(Current)” to the display name of the active scene.
- Renaming a scene updates the display name immediately without changing the scene directory or scene_id used for selection and storage.
- GenScene creates a new scene directory using scene_id, writes a display name derived from the prompt to metadata, and the Scenes list shows that display name without a build-start hitch.
- Scenes with missing or empty labels fall back to showing scene_id in the list.
- The rendered smoke test completes without crashing.

## Potential Risks and Mitigations

1. Risk: Loading display names for many scenes could slow list rebuilds. Mitigation: Cache labels in memory during a rebuild or read metadata lazily and fall back to scene_id when reads fail.
2. Risk: Display names could be set to empty or whitespace-only values. Mitigation: Enforce trimming and non-empty validation in the rename UI and in GenScene label generation.
3. Risk: GenScene label updates could race with other metadata writes. Mitigation: Centralize label writes in the realm helper and serialize updates via the existing scene source load/write path.

## Alternative Approaches

1. Keep using scene_id in the UI and only add a rename action that renames directories, accepting the filesystem change and potential reference churn. This is simpler but conflicts with the user’s desire to keep directory names stable.
2. Store display names in a separate index file per realm rather than meta.json. This could avoid touching scene metadata but adds a new storage location and requires synchronization logic.

## Idempotence and Recovery

The metadata update steps are safe to run multiple times because they only overwrite the label field in meta.json. If a rename fails mid-way, the directory remains unchanged and the list can revert to the prior display name by reloading metadata. If a display name is invalid, fall back to scene_id to keep the UI usable.

## Artifacts and Notes

The key artifact is the scene meta.json label field, which should remain the single source of truth for display names. Any new test scripts should live under /Users/lxl/projects/aiprojects/gravimera/test/run_1 and clean up temporary realms they create.

## Interfaces and Dependencies

Use SceneSourcesV1 in /Users/lxl/projects/aiprojects/gravimera/src/scene_sources.rs to load and write meta.json, and expose helper functions in /Users/lxl/projects/aiprojects/gravimera/src/realm.rs for reading and writing the display name. UI changes live in /Users/lxl/projects/aiprojects/gravimera/src/workspace_scenes_ui.rs, while GenScene scene creation and label assignment live in /Users/lxl/projects/aiprojects/gravimera/src/gen_scene/job.rs. Documentation updates belong in /Users/lxl/projects/aiprojects/gravimera/docs.

Plan update note (2026-04-03): Shortened Implementation Plan task text to satisfy validator length checks and reran validation; the validator script exits early due to its numbered-task grep under set -e, but all reported checks passed.
Plan update note (2026-04-03): Marked completed implementation steps, recorded decisions for UUID-based scene_id allocation and prompt-derived display names, and noted tests still pending.
