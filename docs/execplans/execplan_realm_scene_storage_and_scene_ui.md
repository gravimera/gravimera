# Realm/Scene Storage + Human-Friendly Scene Panel

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository uses ExecPlans as defined in `PLANS.md` at the repository root. Maintain this document in accordance with that file.

## Purpose / Big Picture

Gravimera currently persists a protobuf `scene.dat` as the runtime “world save” and the in-game Scene panel was oriented toward developer-facing scene-source operations. After this change, a human can manage multiple **realms** and **scenes** from a redesigned Scene panel: pick a realm from a dropdown, pick a scene via tabs, edit a high-level scene description, and run the core load/compile/validate actions without digging into low-level layer authoring UI.

This also migrates existing local saves: if a legacy `scene.dat` exists at `~/.gravimera/scene.dat`, it is treated as the `default` realm + `default` scene and moved into the new realm directory layout on startup (`~/.gravimera/realm/default/scenes/default/build/scene.dat`).

## Progress

- [x] (2026-02-15) Define realm/scene directory layout under `~/.gravimera/realm/` and implement safe migration of legacy `~/.gravimera/scene.dat`.
- [x] (2026-02-15) Introduce an “active realm + scene” resource, and route all scene.dat load/save through it.
- [x] (2026-02-15) Redesign the rendered-mode Scene panel UI: realm dropdown, scene tabs, scene description editor, and simplified action buttons.
- [x] (2026-02-15) Update docs (`README.md`, `config.example.toml`, any user-facing references) for the new default save location.
- [x] (2026-02-15) Validate with `cargo test`, a headless smoke boot, and a short rendered boot (`timeout 5 cargo run`).

## Surprises & Discoveries

- Observation: Bevy UI child rebuilds cannot rely on `EntityCommands::despawn_descendants` / `set_parent` in this repo’s Bevy version; rebuilding uses `Children` + `try_despawn()` and parent `with_children` instead.
  Evidence: compilation errors during initial refactor of `src/scene_authoring_ui.rs`.

## Decision Log

- Decision: Store realm packages under `~/.gravimera/realm/<realm_id>/...` (not `realms/`).
  Rationale: Matches the user requirement (“default realm storage path is under `~/.gravimera/realm` folder”).
  Date/Author: 2026-02-15 / Codex

- Decision: Store per-scene build artifacts under `scenes/<scene_id>/build/`, including `scene.dat`.
  Rationale: Aligns with `docs/gamedesign/30_scene_sources_and_build_artifacts.md` (build outputs are caches; scenes live under a realm).
  Date/Author: 2026-02-15 / Codex

- Decision: Persist the active realm/scene selection to `~/.gravimera/realm/active.json`.
  Rationale: Makes startup deterministic and keeps the user’s last selection stable across restarts without requiring config editing.
  Date/Author: 2026-02-15 / Codex

- Decision: Persist the human-authored scene description into `scenes/<scene_id>/src/meta.json` as `description`.
  Rationale: Description is an authoritative, diff-friendly part of scene sources, and `SceneSourcesV1` already provides canonical load/write.
  Date/Author: 2026-02-15 / Codex

## Outcomes & Retrospective

- Implemented realm/scene-local persistence under `~/.gravimera/realm/` with migration of legacy `~/.gravimera/scene.dat` into the default realm/scene build path.
- Added realm/scene switching that saves the current scene before loading the next.
- Replaced the Scene panel with a human-oriented UI (realm dropdown, scene tabs, description editor, and simplified source actions).

## Context and Orientation

Current relevant code:

- `src/paths.rs` defines `GRAVIMERA_HOME` and realm/scene filesystem helpers under `~/.gravimera/realm/`.
- `src/realm.rs` owns active realm/scene selection, scaffolding for missing `src/` files, and migration of legacy `~/.gravimera/scene.dat` into the default realm/scene.
- `src/scene_store.rs` loads/saves the protobuf `scene.dat` per active realm/scene and applies scene switches after saving.
- `src/scene_authoring_ui.rs` implements the redesigned human Scene panel (realm dropdown + scene tabs + description editor + simplified actions).
- `src/scene_sources_runtime.rs` contains the deterministic “scene sources” compilation pipeline used by the Scene panel actions.
- `README.md` and `config.example.toml` document the realm/scene default save location.

Terms used here:

- Realm: top-level world boundary (owner, ruleset, content roots). In this repo it will be represented as a directory under `~/.gravimera/realm/<realm_id>/`.
- Scene: a sub-world in a realm, selected via UI tabs. In this repo it will be represented as a directory under `.../scenes/<scene_id>/`.
- Build outputs: derived caches for runtime loading; `scene.dat` is treated as one such output.

## Plan of Work

1) Add path helpers for realm/scene locations and implement a safe migration path for the legacy `~/.gravimera/scene.dat` into `~/.gravimera/realm/default/scenes/default/build/scene.dat`.

2) Introduce a resource that represents the active `(realm_id, scene_id)` and a small persistence record (an “active selection” file) so the active scene is stable across restarts. Update `scene_store` to load/save based on the active selection (unless `scene_dat_path` is explicitly set in `config.toml`).

3) Redesign the Scene panel:

   - Realm dropdown: shows available realms (directories under `~/.gravimera/realm/`).
   - Scene tabs: shows available scenes in the selected realm.
   - Scene description editor: multiline text; persisted in the selected scene’s `src/meta.json` as `description`.
   - Simplified actions: “Load Sources”, “Reload”, “Compile”, “Validate”, and “Save Pinned”.

4) Update docs to reflect the new default save location and how the Scene panel is used now.

5) Validate via tests and smoke boots.

## Concrete Steps

Run from repo root:

    cargo test
    cargo run -- --headless --headless-seconds 1
    timeout 5 cargo run

## Validation and Acceptance

Acceptance is:

1) With no config override, starting Gravimera migrates an existing legacy save at `~/.gravimera/scene.dat` into `~/.gravimera/realm/default/scenes/default/build/scene.dat` without data loss (never overwrites an existing destination).

2) In rendered mode, clicking the Scene button shows:

- a realm dropdown that switches realms,
- scene tabs that switch scenes,
- a multiline description field that can be edited and persists across reloads,
- action buttons that execute without crashing.

3) `cargo test` passes, headless boot works, and rendered boot does not crash on startup.

## Idempotence and Recovery

- Migration must be idempotent: running the app multiple times should not repeatedly move/copy data or destroy existing destination data.
- If migration cannot proceed (permission error, destination exists), the app should continue using a safe fallback path and log a warning.

## Artifacts and Notes

- Keep any fixtures used for tests under `tests/`.

## Interfaces and Dependencies

No new external dependencies are required. Use existing:

- Bevy UI for panel controls,
- `SceneSourcesV1` load/write for description persistence inside `meta.json`,
- existing `scene_store` save/load for `scene.dat`.
