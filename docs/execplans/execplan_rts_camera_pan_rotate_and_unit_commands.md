# RTS Controls v2: Free Camera + Robust Unit Commands

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

After this change, Gravimera controls feel like a small RTS: selected units can reliably attack and move, the camera is a free “RTS camera” that pans at the screen edge (it does not auto-follow the hero), and basic build-mode editing works on units (duplicate / delete) without accidentally losing selection. Units also avoid stacking on top of each other even if their prefab has no collision.

You can see it working by launching the game, selecting units with LMB, moving with RMB, holding Space to keep firing (aim target with LMB), panning the camera by pushing the cursor to the window edges, rotating the camera with WASD, and duplicating/deleting units in Build mode with M / Delete.

## Progress

- [x] (2026-02-02 19:10Z) Switch fire key to Space and auto-select the hero by default (fixes “nothing fires” when selection is empty).
- [x] (2026-02-02 19:15Z) Selection persistence: empty/non-unit clicks no longer clear selection; build objects can be added without deselecting units.
- [x] (2026-02-02 19:35Z) Camera refactor: remove hero-follow, add edge-pan + WASD rotate (yaw + pitch).
- [x] (2026-02-02 19:45Z) Build-mode hotkeys: Delete removes selected units; M duplicates selected units (kept build-object duplicate on Ctrl/Cmd+D).
- [x] (2026-02-02 19:55Z) Unit spacing: add commandable-to-commandable separation to prevent stacking/overlap.
- [x] (2026-02-02 20:05Z) Update UI/help text and README controls to match new mappings.
- [x] (2026-02-02 20:10Z) Run `cargo test` and a headless smoke run.
- [x] (2026-02-02 20:12Z) Commit final changes (see git commit `62816de`).

## Surprises & Discoveries

- Observation: Scheduling a Build-only system with heavy mutable borrows in the always-on RTS update set can reduce Bevy parallelism even if it returns early at runtime.
  Evidence: `rts::build_unit_hotkeys` needs mutable access to mesh/material caches for duplication; it was moved under the Build-only update systems to keep Play-frame scheduling freer.

- Observation: The previous hero-follow soft-zone camera implementation became unused after switching to a free RTS camera, so it was removed to avoid warnings and confusion.
  Evidence: `cargo check` reported dead-code warnings until the old soft-zone code and constants were removed.

## Decision Log

- Decision: Treat “weapons not working” primarily as an input/selection UX problem, and fix it by (1) switching fire key to Space as requested and (2) ensuring a reasonable default selection (hero auto-selected on start / when entering Play if selection is empty).
  Rationale: The firing logic exists and is selection-gated; most “nothing happens” reports come from selection being empty or cleared unexpectedly.
  Date/Author: 2026-02-02 / Codex

- Decision: Implement RTS camera as “focus point + yaw”, where edge scrolling moves the focus point on the ground plane and the camera transform is derived from focus + zoom offset.
  Rationale: This minimally reuses the existing zoom/look scheme and integrates cleanly with ray-picking logic that assumes a stable camera transform.
  Date/Author: 2026-02-02 / Codex

- Decision: Use WASD for camera yaw (A/D) and pitch (W/S) by introducing a `CameraPitch` resource and applying a pitch rotation to the existing offset/target vectors.
  Rationale: The requirement explicitly asked for WASD camera rotation; adding pitch makes W/S meaningful without removing zoom or other controls.
  Date/Author: 2026-02-02 / Codex

- Decision: Remove the old hero-follow “soft zone” camera code instead of keeping it dormant.
  Rationale: It was no longer part of the active design and produced dead-code warnings; removing it reduces confusion and maintenance burden.
  Date/Author: 2026-02-02 / Codex

## Outcomes & Retrospective

(Fill in after completion.)

## Context and Orientation

Key files and responsibilities:

- `src/rts.rs`: RTS-style selection, RMB move orders, and fire control targeting (`selection_input`, `move_command_input`, `update_fire_control`, `apply_fire_facing`, `execute_move_orders`).
- `src/combat.rs`: Actual weapon firing for the hero (`player_fire`) and selected units (`unit_attack_execute`). Attacks are gated by the `FireControl` resource and current selection.
- `src/player.rs`: Main camera logic today: zoom input (`camera_zoom_input`), camera yaw orbit from cursor edge (`camera_edge_orbit`), hero-follow focus (`camera_soft_zone_follow`), and final camera transform (`camera_follow`). Also owns the edge-scroll cursor indicator UI.
- `src/build.rs`: Build-mode editing of build objects. Current shortcuts are oriented around build objects (Backspace delete, Ctrl/Cmd+D duplicate).
- `src/ui.rs` and `src/setup.rs`: On-screen/status text and startup “Controls:” log string (must be updated when controls change).

Terminology used here:

- “Unit”: an entity the player can select and command (has the `Commandable` component). The hero is also a unit (has `Player` + `Commandable`).
- “Build object”: a placed building piece (has the `BuildObject` component).
- “RTS camera”: a camera that pans (translates) when the cursor nears window edges and rotates via keys; it does not automatically follow the hero.

## Plan of Work

### 1) Fix unit weapons “not working” + switch fire key to Space

In `src/rts.rs`:

- Change the fire key from `KeyCode::KeyA` to `KeyCode::Space` everywhere:
  - `selection_input` must reserve LMB while Space is held.
  - `update_fire_control` must activate firing while Space is held.

In `src/app.rs` and/or a small new system:

- Ensure a sensible default selection so the user can immediately attack without first re-selecting:
  - On startup, auto-select the hero entity once it exists.
  - On entering Play mode (Tab), if selection is empty, auto-select the hero.

This addresses “weapons not working” when the player forgets to re-select after clicking on empty ground or after switching modes.

### 2) Selection persistence (do not clear on empty / non-unit)

In `src/rts.rs` `selection_input`:

- Do not do `selection.selected.clear()` unconditionally.
- Compute the selection result of the click/drag:
  - If the click/drag hits at least one `Commandable` unit, replace the current selection with those units (standard RTS behavior).
  - If it hits only build objects (in Build mode), add those objects to the existing selection (do not remove selected units).
  - If it hits nothing, keep the selection unchanged.

This makes selection robust and prevents accidental deselection.

### 3) Camera refactor: stop following hero, edge-pan, WASD rotate

In `src/player.rs`:

- Remove/disable hero-follow focus updates:
  - Stop running `camera_soft_zone_follow` (or change it into a no-op).
- Replace `camera_edge_orbit` with `camera_edge_pan`:
  - When cursor is near left/right/top/bottom edges, translate the camera focus point on the ground plane.
  - Speed should be frame-rate independent and scale reasonably with zoom (faster when zoomed out).
  - Clamp focus to `WORLD_HALF_SIZE` bounds to avoid panning out of the world.
- Add `camera_rotation_input`:
  - Use WASD keys to rotate the camera yaw.
  - Keep the rotation speed capped and consistent (new constants if needed).

In `src/app.rs`:

- Rewire the camera systems order so:
  - zoom input → rotation input → edge pan → camera_follow
  - and remove scheduling of `camera_edge_orbit` + `camera_soft_zone_follow`.

In the cursor indicator:

- Update `update_edge_scroll_cursor_indicator` so it shows direction for pan (← → ↑ ↓) and only hides the cursor when panning is active.

### 4) Build-mode unit editing hotkeys (Delete / M)

In `src/build.rs` (or a small new module if it fits better):

- Add a new system that runs only in Build mode:
  - If `Delete` (and also `Backspace` for macOS) is pressed: despawn selected units (`With<Commandable>`, `Without<Player>`) and remove them from selection.
  - If `M` is pressed: duplicate selected units (`With<Commandable>`, `Without<Player>`):
    - Spawn a new entity with a new `ObjectId`, same `ObjectPrefabId`, same transform (offset by +grid step in X/Z so it’s visible), same tint if any, and the required components (`Commandable`, `Collider`, `Visibility`).
    - Spawn visuals for the new unit using `crate::object::visuals::spawn_object_visuals`.
    - Update selection to select the duplicated unit(s).

### 5) Unit spacing (avoid “too close” even without collision)

In `src/rts.rs` (or `src/physics.rs` if there is a better home):

- Add a post-move “separation” pass for `Commandable` units:
  - Compute a minimum spacing radius per unit (prefer `Collider.radius`, fall back to a small default if the radius is missing/invalid).
  - If two units are closer than `min_dist = r_a + r_b`, apply a small correction to push them apart (frame-rate independent).
  - Keep it conservative to avoid jitter; it’s OK if units can still get close, but they should not overlap visually.

### 6) Update docs and in-game text

- Update `src/setup.rs` “Controls:” log string to match:
  - Space to fire.
  - RTS camera controls (edge-pan + WASD rotate).
  - Build mode hotkeys Delete / M.
- Update the on-screen mode help strings in `src/ui.rs`.
- Update `README.md` Controls section to match.

### 7) Validation + commits

Required by `AGENTS.md`:

- Run formatting, tests, and a smoke run:
  - `cargo fmt`
  - `cargo check`
  - `cargo test`
  - `cargo run -- --headless --headless-seconds 0.2`
- Then commit with a clear message.

## Concrete Steps

Run from repo root:

    cargo fmt
    cargo check
    cargo test
    cargo run -- --headless --headless-seconds 0.2

Expected outcomes:

- `cargo check` succeeds.
- `cargo test` succeeds.
- Headless run prints a short “Headless simulation finished…” line and exits successfully.

## Validation and Acceptance

Manual acceptance (interactive):

- Camera:
  - Cursor near window edges pans the camera (left/right/up/down), not rotate.
  - WASD rotates the camera yaw (and does not move the hero).
  - The camera does not auto-follow the hero’s movement.
- Selection:
  - Selecting units works via LMB click/drag.
  - Clicking empty ground does not clear selection.
  - Clicking a build object does not clear already-selected units.
- Combat:
  - Hold Space to keep firing with selected units in Play mode.
  - While holding Space, LMB sets a fire target (ground or enemy).
  - Attacks produce visible projectiles / effects (depending on unit type).
- Build hotkeys:
  - In Build mode, `Delete` removes selected units (except the hero).
  - In Build mode, `M` duplicates selected units.
- Spacing:
  - When moving multiple units to the same location, they do not stack directly on top of each other.

## Idempotence and Recovery

- This plan is safe to re-run: changes are code-only and do not require manual migrations.
- If a control mapping is wrong, revert the last commit and adjust only the relevant input system, then re-run the smoke test.

## Artifacts and Notes

(Add transcripts/diffs as needed during implementation.)

## Interfaces and Dependencies

- No new external dependencies are required.
- New constants (if needed) should live in `src/constants.rs` alongside other camera/input constants.
