# ExecPlan 08: Human Scene Sources UI (Import/Compile/Regen/Validate/Author Layers)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this milestone, humans can use an in-game UI to author and iterate on **scene sources** without needing to run HTTP calls manually.

The UI is not a “town generator” and does not embed domain heuristics. It is a generic authoring panel that exposes the same deterministic pipeline as the Automation API:

- Import scene sources from a `src/` directory
- Compile procedural layers into instances
- Regenerate a single layer (scoped regeneration)
- Validate using a scorecard (hard gates)
- Export pinned instances back to sources
- Author new procedural layers by entering explicit parameters (v1: `grid_instances`, `polyline_instances`)

This makes the scene generation pipeline debuggable and usable for both AI agents (via HTTP) and humans (via UI).

## Progress

- [x] (2026-02-14) Create the initial ExecPlan.
- [x] (2026-02-14) Add a “Scene” toggle button and a scene sources panel UI in rendered mode.
- [x] (2026-02-14) Implement UI actions: import, reload, compile, validate, export pinned.
- [x] (2026-02-14) Implement UI authoring forms for v1 layer kinds: `grid_instances` and `polyline_instances`.
- [x] (2026-02-14) Prevent gameplay input from interfering while the panel is open (mouse/keyboard capture).
- [x] (2026-02-14) Update docs (`README.md`, `docs/controls.md`) and run `cargo test` + headless smoke boot, then commit.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (fill in as discovered)

## Decision Log

- Decision: Provide a panel-driven UI rather than a new editor “mode”.
  Rationale: Keeps risk low and avoids disrupting the existing Build/Play/Gen3D mode flow; the UI can be iterated on as a tool surface.
  Date/Author: 2026-02-14 / Codex

- Decision: Author new layers via explicit parameter forms (not implicit “smart” tools) in v1.
  Rationale: The engine must remain generic; human UI should expose parameters explicitly, not bake in hidden layout rules.
  Date/Author: 2026-02-14 / Codex

## Outcomes & Retrospective

- Shipped an in-game Scene Sources panel (rendered mode):
  - Top-left **Scene** toggle button.
  - Import/reload/compile/validate/export pinned actions.
  - Per-layer regenerate buttons (scoped regeneration).
  - Basic authoring forms for `grid_instances` and `polyline_instances` that write canonical JSON files under `src/layers/`.
  - A clickable prefab list (builtins + any registered prefabs) to populate authoring forms.

What remains:

- Add richer authoring tools (interactive path/region editing, pinning/excludes for procedural layers) as follow-up milestones while keeping determinism + “no heuristics” constraints.

## Context and Orientation

Relevant design/spec references:

- Scene creation goals + regeneration rule: `docs/gamedesign/22_scene_creation.md`
- Scene sources layout: `docs/gamedesign/30_scene_sources_and_build_artifacts.md`
- Procedural layer kinds v1: `docs/gamedesign/33_scene_layer_kinds_v1.md`

Relevant code:

- Scene sources runtime API (import/export/compile/regen/validate): `src/scene_sources_runtime.rs`
- Rendered startup UI spawning: `src/setup.rs`
- Existing UI patterns:
  - General UI helpers: `src/ui.rs`
  - Gen3D in-game UI patterns (buttons, focus, text input): `src/gen3d/ui.rs`

Important constraints:

- No heuristics in generation logic; UI must be a parameterized authoring surface.
- Keep tests green and keep headless mode working.

## Plan of Work

1) Add a new module `src/scene_authoring_ui.rs` that owns:

   - a `SceneAuthoringUiState` resource (panel open/closed, input fields, status/error text),
   - UI marker components (toggle button, panel root, input boxes, action buttons),
   - systems to:
     - toggle panel visibility,
     - handle text input for focused fields (simple append/backspace + paste),
     - run scene sources operations by calling `crate::scene_sources_runtime::*`,
     - rebuild layer list UI (regen buttons) from the currently loaded sources.

2) Wire it into the rendered app in `src/app.rs`:

   - `init_resource::<SceneAuthoringUiState>()`
   - add a startup system to spawn the UI
   - add update systems for interactions and UI text refresh

3) Input capture:

   - When the panel is open, block Build/RTS selection/move inputs so clicking UI doesn’t also place/select/move objects.
   - When a text field is focused, clear `ButtonInput<KeyCode>` in `PreUpdate` after reading the typed text so gameplay bindings don’t trigger.

4) Docs:

   - Add a short “Scene Sources UI” section to `README.md` describing how to open the panel and the typical loop.
   - Add a note to `docs/controls.md` about the “Scene” button/panel.

## Concrete Steps

Run from the repo root:

1) Tests:

   cargo test

2) Headless smoke boot:

   cargo run -- --headless --headless-seconds 1

3) Manual verification (rendered):

   - Run `cargo run`
   - Open the Scene panel
   - Import `tests/scene_generation/fixtures/procedural_layers_v1/src`
   - Compile
   - Click regenerate on a layer and observe changes

## Validation and Acceptance

This milestone is accepted when:

- Rendered mode shows a “Scene” toggle button and a scene sources panel.
- A human can import sources, compile, regenerate a layer, validate, and export pinned instances via UI.
- While the panel is open, clicking inside it does not also trigger world selection/placement, and typing into text fields does not trigger gameplay hotkeys.
- `cargo test` passes and headless smoke boot completes without crashing.

## Idempotence and Recovery

- Import/compile/regenerate/validate must be safe to repeat.
- Export pinned must only export unowned (pinned) instances, leaving layer sources intact.

## Interfaces and Dependencies

Do not add new crates. Reuse:

- Bevy UI patterns already used by `src/gen3d/ui.rs`
- `crate::scene_sources_runtime` for all scene source operations
- `crate::scene_sources::SceneSourcesIndexPaths` to resolve `layers_dir` and `pinned_instances_dir` paths when writing files
