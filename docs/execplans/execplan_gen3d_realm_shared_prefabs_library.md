# Gen3D: Realm-Shared Prefabs Library (thumbnails, short names, search, preview)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

Creators use Gen3D to generate objects and then re-use them while building a realm. Today prefabs are stored per-scene, the Prefabs panel is text-only, and it is hard to browse or preview a realm’s prefab library.

After this change:

- Clicking **Save** in Gen3D writes a prefab package under `~/.gravimera/realm/<realm_id>/prefabs/<root_prefab_uuid>/` and includes:
  - `thumbnail.png`: a deterministic render at “front +30°” angle.
  - `prefabs/<root_prefab_uuid>.desc.json`: a descriptor that includes:
    - `label`: a short name (at most 3 words) derived from the prompt + object facts.
    - `text.short`: a short description derived from the prompt + object facts.
    - `provenance.modified_at_ms`: last-modified timestamp (unix epoch ms) for sorting.
- The **Prefabs** panel (Build → Realm view) shows a realm-wide, searchable list sorted by last modified:
  - thumbnail on the left, short name on the right (fallback to prefab id for old prefabs).
  - typing in the search box filters live and ranks results by relevance.
  - clicking a row opens a preview overlay with a studio render + metadata; it closes via a close button or `Esc`.
  - dragging a row continues to spawn the prefab into the world (click-without-drag does not spawn).
- On startup, any existing scene-local prefab packages under `.../scenes/<scene_id>/prefabs/` are migrated into the realm-level `.../prefabs/` folder.

## Progress

- [x] (2026-03-11) Create ExecPlan and identify touch-points.
- [x] (2026-03-11) Introduce realm-scoped prefab package store + migrate scene-local packages on startup.
- [x] (2026-03-11) Persist short name (`label`, ≤3 words) and `modified_at_ms` in prefab descriptors.
- [x] (2026-03-11) Generate and persist `thumbnail.png` on Gen3D Save.
- [x] (2026-03-11) Update Prefabs panel: search box, thumbnail+name rows, click-to-preview overlay, Esc/exit, sorting.
- [x] (2026-03-11) Update gamedesign docs to match new storage + UI behavior.
- [x] (2026-03-11) Run tests + rendered smoke test and commit.

## Surprises & Discoveries

- Observation: (fill in during implementation)
  Evidence: (fill in during implementation)

## Decision Log

- Decision: Store “last modified” timestamp in `PrefabDescriptorProvenanceV1.modified_at_ms`.
  Rationale: Descriptors are the semantic metadata layer used by tooling and UI; adding timestamps there avoids bumping the prefab-def schema and keeps authoring metadata separate from geometry.
  Date/Author: 2026-03-11 / Codex

- Decision: “30 degree to front” thumbnail angle = yaw `+π/6`, pitch = Gen3D preview default pitch.
  Rationale: Interprets the request as a 3/4 orbit view offset from front while keeping a deterministic, generic camera model.
  Date/Author: 2026-03-11 / Codex

## Outcomes & Retrospective

- Shipped realm-scoped prefab packages (`~/.gravimera/realm/<realm_id>/prefabs/<root_uuid>/...`) and startup migration from legacy scene-local packages.
- Shipped Gen3D Save persistence upgrades: `thumbnail.png` (front+30°), short name (`descriptor.label`, ≤3 words), and `provenance.modified_at_ms` for sorting/recency.
- Shipped Prefabs panel upgrades: search box + relevance sort, thumbnail+name rows, click-to-preview overlay (Esc/exit), and default sort by last modified.

## Context and Orientation

Key current modules and files:

- Legacy scene-local prefab packages: `src/scene_prefabs.rs`
  - Old location: `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/prefabs/<root_uuid>/...` (migrated on startup)
- Realm-scoped prefab packages: `src/realm_prefab_packages.rs`
  - New location: `~/.gravimera/realm/<realm_id>/prefabs/<root_uuid>/...`
- Prefab defs: `src/realm_prefabs.rs` (JSON read/write into a directory)
- Prefab descriptors: `src/prefab_descriptors.rs` (format + load/save helpers)
- Gen3D save pipeline: `src/gen3d/save.rs`
  - Writes prefab defs into a package directory and writes `<prefab_uuid>.desc.json`.
  - Also supports best-effort AI “descriptor meta” (`name` -> `label`, `short`, `tags`).
- Prefabs panel UI: `src/model_library_ui.rs` (Build → Realm)
- Startup realm/scenes scaffold: `src/realm.rs`
- File-system paths: `src/paths.rs`

New capability required:

- Realm-level prefab packages under `~/.gravimera/realm/<realm_id>/prefabs/`.
- Migration on startup: move any `.../scenes/<scene_id>/prefabs/<uuid>/` package directories into realm-level storage.

## Plan of Work

### 1) Realm-scoped prefab packages + migration

Add a realm-level package helper module:

- Create `src/realm_prefab_packages.rs` mirroring `src/scene_prefabs.rs` but without `scene_id`.
  - Provide functions to:
    - locate the realm prefabs root dir and a package dir,
    - save/load prefab defs into the package’s `prefabs/` dir,
    - locate `gen3d_source_v1/` dir and `gen3d_edit_bundle_v1.json`,
    - locate `thumbnail.png` path at package root,
    - list packages by enumerating UUID-named dirs.
  - Provide `migrate_scene_prefab_packages_to_realm(realm_id)`:
    - enumerate `~/.gravimera/realm/<realm_id>/scenes/*/prefabs/<uuid>/`,
    - move each package dir into `~/.gravimera/realm/<realm_id>/prefabs/<uuid>/`,
    - if destination exists, quarantine the scene-local dir under `.../prefabs/_scene_prefabs_conflicts/<scene_id>/<uuid>/` (never delete on conflict).

Update `src/paths.rs`:

- Add:
  - `realm_prefabs_dir(realm_id) -> ~/.gravimera/realm/<realm_id>/prefabs`
  - `realm_prefab_package_dir(realm_id, root_prefab_id) -> .../prefabs/<uuid>`

Wire migration into startup:

- In `src/realm.rs`:
  - in `realm_startup_init` and `ensure_realm_scene_scaffold`, ensure the realm prefabs dir exists and call migration.

Update call sites to use realm store (stop writing/reading scene-local packages):

- `src/gen3d/save.rs`: save packages to realm store.
- `src/gen3d/ai/orchestration.rs`: edit/fork session load should read from realm store.
- `src/model_library_ui.rs`: list realm packages (not scene packages).
- `src/scene_build_ai.rs`: load prefab descriptors from realm store for prompt context.

### 2) Descriptor: short name (≤3 words) + modified timestamp

Extend descriptor format:

- In `src/prefab_descriptors.rs`, add `modified_at_ms: Option<u128>` to `PrefabDescriptorProvenanceV1`.

On Gen3D Save (`src/gen3d/save.rs`):

- Always set `provenance.modified_at_ms = now_ms`.
- Preserve `created_at_ms` when overwriting an existing prefab (read existing descriptor when present).
- Ensure `label` (short name) is present and is at most 3 words:
  - Prefer AI descriptor-meta `name` when available (still clamp).
  - Otherwise derive from first prompt line and clamp.
- Ensure `text.short` (short description) is present (can be longer than 3 words).
- Update `set_descriptor_meta_v1` tool (`src/gen3d/ai/agent_tool_dispatch.rs`) to accept `name` and clamp to ≤3 words.
- Update the descriptor-meta AI prompt (`src/gen3d/ai/prompts.rs`) to require `name` (≤3 words), `short`, and `tags`.

### 3) Thumbnail generation on Save (front +30°)

On each Gen3D Save:

- Write `thumbnail.png` into the prefab package root.
- Spawn a one-off offscreen camera on a dedicated render layer:
  - render only the prefab visuals on that layer,
  - use yaw `+π/6`, pitch = Gen3D preview default pitch,
  - compute distance from prefab size/half-extents using `crate::orbit_capture::required_distance_for_view`,
  - capture via `bevy::render::view::screenshot::Screenshot::image(...)` and `save_to_disk`.
- Track capture completion in a small resource and despawn temporary entities when done.
- Best-effort: if capture fails, log and continue without blocking Save.

### 4) Prefabs panel UI: thumbnails, search, preview overlay, sorting

Update `src/model_library_ui.rs`:

- Add a search input field under the “Prefabs” header.
  - live filtering as the query updates.
- Change row layout:
  - thumbnail square on the left (loads `thumbnail.png`),
  - short name on the right (fallback to UUID if missing).
- Sorting:
  - default (empty query): by `modified_at_ms` desc; fallback to `created_at_ms`; fallback to filesystem modified time.
  - with query: sort by relevance desc then modified desc.
  - relevance scoring: generic string matching across short name, label, tags, and id (no object-specific heuristics).
- Click behavior:
  - click-without-drag opens a preview overlay panel (does not spawn into world).
  - drag continues to spawn into the scene (existing behavior).
- Preview overlay panel:
  - shows a studio render of the prefab (isolated render layer + offscreen camera + simple lights),
  - displays: short name, long description, id, tags, roles, timestamps, and any structured fields that exist in the descriptor,
  - close via a close button and `Esc` key.

### 5) Docs updates

Update docs to match behavior and storage:

- `docs/gamedesign/specs.md`: replace the scene-local prefab package spec entry with realm-level package spec.
- `docs/gamedesign/39_realm_prefab_packages_v1.md`: realm-shared prefab package layout (with migration notes).
- `docs/gamedesign/35_prefab_descriptors_v1.md`: update directory layout, add `modified_at_ms`, tighten short name (`label`) to “≤3 words”, and document thumbnails.
- `docs/gamedesign/02_game_modes_and_loops.md`: Prefabs panel behavior (realm-wide, search, thumbnails, preview).
- Update any other docs referencing scene-local prefabs (e.g. `docs/gamedesign/34_realm_prefabs_v1.md`).

## Concrete Steps

Run all commands from repo root (`/Users/flow/workspace/github/gravimera`).

1) Implement code changes.

2) Run tests:

    cargo test

3) Run the required rendered smoke test (do not use `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4) Commit with a clear message.

## Validation and Acceptance

Manual validation (rendered build):

- Start the game, enter Build → Gen3D Preview.
- Generate an object and click Save.
- Verify a realm-level prefab package exists under:
  - `~/.gravimera/realm/<realm_id>/prefabs/<root_prefab_uuid>/`
  - and contains:
    - `thumbnail.png`
    - `prefabs/<root_prefab_uuid>.desc.json` with:
      - `label` present and ≤3 words
      - `provenance.modified_at_ms` present
- Go to Build → Realm and open the Prefabs panel:
  - list is sorted with the newly saved prefab at the top,
  - each row shows thumbnail (left) + short name (right),
  - typing in the search box filters and reorders results live,
  - clicking a row opens the preview overlay; `Esc` and close button exit it.

## Idempotence and Recovery

- Migration is safe to re-run: it should be idempotent and never delete a destination package.
- On conflict, quarantine is used so packages are not lost.

## Interfaces and Dependencies

- Uses Bevy UI entities/components in `src/model_library_ui.rs`.
- Uses Bevy screenshot capture API (`bevy::render::view::screenshot`) for thumbnail rendering.
- Uses existing `crate::orbit_capture` helpers for deterministic camera orbit math.
