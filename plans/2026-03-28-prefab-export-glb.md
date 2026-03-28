# Prefab Export (GLB for Blender)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` in the repository root.

## Purpose / Big Picture

Enable users to export prefab(s) from the **Prefabs** panel into a **Blender-friendly** format with good fidelity: a `.glb` (glTF 2.0 binary) file per prefab containing:

- Mesh geometry (procedural primitives used by Gravimera’s prefab system)
- Materials (PBR base color + metallic/roughness, plus unlit where applicable)
- Hierarchy (prefab part tree as nodes)
- Animations (baked TRS keyframes for each animation channel present in the prefab)

This adds a new **Export GLB** button in Prefabs manage mode and a new Automation HTTP API endpoint so tooling can export prefabs programmatically.

How a human verifies it works:

1. Run the game, open Prefabs → Manage → multi-select → Export GLB.
2. Pick an output folder; observe `.glb` files created.
3. Import the `.glb` into Blender; observe meshes/hierarchy and playable animations.

How automation verifies it works:

1. Start the game with `--automation`.
2. Call `POST /v1/prefabs/export_glb` with a prefab id and an output folder.
3. Verify a `.glb` file exists and begins with the `glTF` magic.

## Progress

- [x] (2026-03-28 12:00 CST) Draft ExecPlan with GLB export design, UI wiring, HTTP endpoint, and validation steps.
- [x] Implement a minimal GLB writer (JSON + BIN chunks) and glTF scene builder for prefab nodes/meshes/materials.
- [x] Bake prefab animations into glTF animations per channel.
- [x] Add Prefabs UI: Export GLB button + background dialog/job + toasts.
- [x] Add Automation API: `POST /v1/prefabs/export_glb` (writes `.glb` files to a directory).
- [x] Add docs and a real test script under `test/`.
- [x] Run required smoke test (rendered, 2 seconds).
- [x] Commit with a clear message.

## Surprises & Discoveries

- (none yet)

## Decision Log

- Decision: Export format is **glTF 2.0 binary (`.glb`)**.
  Rationale: Widely supported by Blender; supports PBR materials, hierarchy, and animation in a single portable file.
  Date/Author: 2026-03-28 / Codex

- Decision: Animations are exported as **baked TRS keyframes** at a fixed FPS (default 30) for each animation channel name found in the prefab.
  Rationale: Robust across Gravimera’s runtime animation driver semantics (time/move/action/attack); Blender consumers get standard timelines.
  Date/Author: 2026-03-28 / Codex

- Decision: The exporter supports prefab parts of kinds `object_ref` and `primitive` / `mesh`, but **fails fast** for `ObjectPartKind::Model { scene }`.
  Rationale: Merging arbitrary external scene assets into a single glTF is significantly more complex; the repo currently treats prefab package `materials/` as “reserved”. The exporter should be explicit and actionable when encountering unsupported content.
  Date/Author: 2026-03-28 / Codex

## Outcomes & Retrospective

- (not completed yet)

## Context and Orientation

Relevant code locations:

- Prefabs UI lives in `src/model_library_ui.rs` (the panel titled “Prefabs”).
  - It already supports multi-select manage mode, zip import/export, and background jobs using `rfd` dialogs and `std::thread::spawn`.
- Prefab defs are stored in memory in `src/object/registry.rs` as `ObjectDef`, with parts `ObjectPartDef` and kinds:
  - `ObjectRef` (hierarchy to another object id),
  - `Primitive` (procedural mesh + color/unlit),
  - `Mesh` (mesh key + material key),
  - `Model` (external scene path; currently treated as unsupported for GLB export).
- Prefab package on-disk layout is described by `docs/gamedesign/39_realm_prefab_packages_v1.md`.
  - `materials/` is reserved for future external assets.
- Automation HTTP API is implemented in `src/automation/mod.rs` and documented in `docs/automation_http_api.md`.
  - Existing endpoints follow the pattern “write to a supplied `out_dir` and return JSON”.

Terms used in this plan:

- **Prefab**: an `ObjectDef` root id plus any internal component `ObjectDef`s referenced through `ObjectRef` parts.
- **GLB**: a single-file binary glTF 2.0 container with a JSON chunk and a BIN chunk.
- **Baked animation**: keyframes are sampled at fixed times and written as absolute TRS curves, rather than trying to encode Gravimera’s runtime driver logic into Blender.

## Plan of Work

### 1) Add a core prefab → GLB exporter module

Create a new module `src/prefab_glb.rs` that can export one prefab id to a `.glb` file. It should:

- Build a node tree for the prefab:
  - Root node is the prefab id.
  - Each `ObjectPartDef` becomes a child node under its parent object node.
  - `ObjectRef` parts recursively inline the referenced child object’s parts as children.
  - Attachments (`AttachmentDef`) must be resolved when computing each part node’s base transform (same math as runtime).
  - Detect and reject composition cycles and missing referenced object ids with clear errors.
- Build mesh and material tables:
  - Meshes are generated procedurally from `MeshKey` and `PrimitiveParams` (using Bevy’s mesh generation types) and deduplicated per-file.
  - Mirrored transforms (negative determinant scale) should be handled by generating a mirrored-winding mesh variant (to match runtime visuals).
  - Materials map to glTF PBR base color + metallic/roughness. For `unlit=true`, include `KHR_materials_unlit`.
- Export animations:
  - Discover channel names using `ObjectLibrary::animation_channels_ordered(prefab_id)`.
  - For each channel name, write a glTF animation named after the channel, sampling at fixed FPS (default 30).
  - Duration rules:
    - `attack`: use `ObjectLibrary::channel_attack_duration_secs(prefab_id, "attack")` if present, else 1.0s.
    - `action`: use `ObjectLibrary::channel_action_duration_secs(prefab_id, "action")` if present, else 1.0s.
    - `idle`, `move`, `ambient`, and other channels: default 2.0s, but clamp to `[0.05, 10.0]`.
  - Selection rules should mimic runtime behavior (no randomness):
    - For `attack`/`action`/`move`/`idle` exports, emulate the channel priority fallback (`attack`→`action`→`move`→`idle`→`ambient`).
    - For non-standard channels, force that channel first and then fall back as in idle state.
  - Driver mapping:
    - `Always`, `AttackTime`, `ActionTime`: driver time = seconds.
    - `MovePhase`, `MoveDistance`: driver time = seconds * `move_units_per_sec` (default 1.0).
  - Output curves are absolute TRS per node.
- Write GLB:
  - Assemble `gltf-json` `Root` and a BIN buffer.
  - Write the GLB header + JSON chunk + BIN chunk with correct padding.

Expose a minimal public interface used by UI and Automation:

    pub(crate) struct PrefabGlbExportOptions {
        pub(crate) fps: u32,
        pub(crate) move_units_per_sec: f32,
    }

    pub(crate) struct PrefabGlbExportReport {
        pub(crate) exported: usize,
        pub(crate) out_paths: Vec<std::path::PathBuf>,
    }

    pub(crate) fn export_prefabs_to_glb_dir(
        prefab_ids: &[u128],
        out_dir: &std::path::Path,
        library: &crate::object::registry::ObjectLibrary,
        options: PrefabGlbExportOptions,
    ) -> Result<PrefabGlbExportReport, String>;

### 2) Wire Prefabs UI: add “Export GLB”

In `src/model_library_ui.rs`:

- Add a new manage-mode button next to the existing “Export” (zip) and “Delete” buttons:
  - Label: `Export GLB`
  - Visible only when the panel is open, in Build mode, `BuildScene::Realm`, and `multi_select_mode` is active (same visibility gating as zip export).
- Implement background dialog + export job similar to zip export:
  - On click, require at least one selected prefab; otherwise show a warning toast.
  - Open a folder picker (`rfd::FileDialog::pick_folder`) on a background thread.
  - After folder is chosen, spawn an export thread to run `crate::prefab_glb::export_prefabs_to_glb_dir(...)`.
  - Poll job completion via a resource receiver and show a toast summary.

Register new resources in `src/app.rs` and add new systems in `src/app_plugins.rs` next to the existing zip export systems.

### 3) Add Automation HTTP API endpoint

In `src/automation/mod.rs`, add:

- Discovery entry:
  - `{ "method": "POST", "path": "/v1/prefabs/export_glb" }`
- Route handler:
  - `POST /v1/prefabs/export_glb`
  - Request JSON:

        { "out_dir": "/abs/or/relative/path", "prefab_id_uuids": ["..."], "fps": 30, "move_units_per_sec": 1.0 }

    `fps` and `move_units_per_sec` are optional (defaults as in exporter options).

  - Response JSON (shape):

        { "ok": true, "exported": 1, "out_paths": [".../Label_uuid.glb"] }

The endpoint writes one `.glb` per prefab id in the output directory. It should return `409` on exporter errors (missing defs, unsupported model parts, etc) with an actionable error string.

### 4) Documentation + real test script

Documentation:

- Add `docs/prefab_export_glb.md` describing:
  - UI workflow (Manage → Export GLB)
  - Output layout (`<out_dir>/<Label>_<prefab_uuid>.glb`)
  - Limitations (currently fails for `ObjectPartKind::Model`)
- Update `docs/automation_http_api.md` with the new endpoint.

Test:

- Add `test/run_1/prefab_export_glb_api/run.py` that:
  - Starts the game with automation enabled (rendered), using a per-run `GRAVIMERA_HOME`.
  - Calls `GET /v1/prefabs`, picks a deterministic prefab id (first entry, or a known label like “Human” when present).
  - Calls `POST /v1/prefabs/export_glb` into a run-local output dir.
  - Verifies the `.glb` file exists and begins with magic `glTF`.

### 5) Validation and Acceptance

Required smoke test (per AGENTS.md):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expect: the game starts in rendered mode and exits without crashing.

Exporter acceptance:

- UI: Export GLB produces `.glb` files for selected prefabs, and reports a success toast.
- API: `POST /v1/prefabs/export_glb` returns `{ ok: true }` and writes `.glb` files.
- Import `.glb` into Blender: nodes and animations are present.

### 6) Commit

Commit all changes with a clear message, e.g.:

    Add prefab GLB export (UI + automation API)

## Idempotence and Recovery

- Export is non-destructive.
- Re-running export overwrites existing `.glb` files with the same name in the chosen output folder.
- If export fails due to unsupported content, the error message must identify the prefab id and reason so the user can choose a different prefab or export path.

## Artifacts and Notes

Record in this plan:

- Smoke test (2026-03-28): `cargo run -- --rendered-seconds 2` starts rendered mode, creates a window, and exits without crashing.
- Real API test: `test/run_1/prefab_export_glb_api/run.py` exports a prefab to `.glb` and validates the `glTF` magic.
- Note: output filenames are `<Label>_<prefab_uuid>.glb` for easier human browsing.

Plan update (2026-03-28): Initial plan created for GLB export with baked animations, UI button, and automation endpoint.
