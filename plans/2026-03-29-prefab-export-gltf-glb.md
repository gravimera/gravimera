# Prefab Export glTF/GLB (fix colors + mirror) + Switch mirror fix

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This plan must be maintained in accordance with `PLANS.md` in the repository root.

## Purpose / Big Picture

After this change, exporting prefabs for Blender (and other glTF-capable tools) is both higher-fidelity and more robust:

- The Prefabs panel button reads **Export glTF/GLB** and exports **both** a `.gltf` (JSON + `.bin`) and a `.glb` (binary) for each selected prefab into a chosen folder.
- Exported models preserve **per-part colors/material factors** correctly (no “everything is gray” due to mesh/material caching).
- Exported models handle mirrored/copy components without appearing “transparent” (backface-culling artifacts).
- The in-game **Switch** (form transform) transition no longer shows mirrored parts disappearing; mirrored leaves use the correct mirrored-winding meshes during the transition.
- The Automation HTTP API offers a matching export endpoint that writes both formats to a directory and returns the created paths.

How a human verifies it works:

- In rendered mode: open Prefabs → Manage → select a Gen3D prefab that contains lots of colored parts and mirrored copies → click **Export glTF/GLB** → pick an output folder → open the exported `.glb` in a glTF viewer or Blender and confirm colors and mirrored parts look correct.
- In rendered mode: select an entity that has mirrored parts; press Tab (or use copy/switch flow) and confirm mirrored parts do not disappear during the morph animation.

How automation verifies it works:

- Start the game with `--automation`.
- Call the export endpoint with an `out_dir` and a prefab id.
- Verify the `.glb`, `.gltf`, and `.bin` files exist and the `.glb` starts with `glTF` magic.

## Progress

- [x] (2026-03-29 09:40 CST) Review existing prefab glTF/GLB exporter + Switch transition path; confirm root causes for color loss and mirror culling issues.
- [x] (2026-03-29 10:15 CST) Update the exporter to output both `.gltf`+`.bin` and `.glb`, and update UI copy to “Export glTF/GLB”.
- [x] (2026-03-29 10:15 CST) Fix GLB/glTF export material correctness by separating geometry caching from mesh+material instancing.
- [x] (2026-03-29 10:15 CST) Fix mirrored/copy export rendering by removing the exporter’s mirrored-winding “double fix”.
- [x] (2026-03-29 10:20 CST) Fix Switch (form transform) mirrored disappearance by using mirrored-winding meshes for negative-determinant leaf transforms (and avoid pairing mirrored/non-mirrored leaves as “same type”).
- [x] (2026-03-29 10:22 CST) Update Automation HTTP API + docs + test script to match the new export behavior.
- [x] (2026-03-29 10:26 CST) Run required rendered smoke test and a real automation test.
- [x] (2026-03-29 10:30 CST) Commit changes with clear messages.

## Surprises & Discoveries

- Observation: Exported Gen3D prefabs contained many distinct `color_rgba` values on primitive parts, but the exported `.glb` only had a few materials and looked mostly gray.
  Evidence: In `~/.gravimera/.../prefabs/*.json` there were >100 unique colors, while the exported `.glb` had 4 materials and 4 meshes.
- Observation: The “mirror/copy looks transparent” symptom in glTF viewers was not alpha; it was face culling caused by a mirrored-winding “double fix” (negative determinant scale + inverted winding).
  Evidence: Exported glTF materials were `OPAQUE` and source colors had alpha 1.0.
- Observation: The same mirror disappearance happens only during the in-game Switch morph animation and disappears when the final prefab visuals spawn.
  Evidence: User report (2026-03-29).

## Decision Log

- Decision: Export **both** `.gltf` (JSON + `.bin`) and `.glb` into the chosen output folder for each prefab.
  Rationale: Some tools prefer `.glb` convenience, while others want editable `.gltf` + `.bin`. Exporting both avoids forcing a choice and makes debugging easier.
  Date/Author: 2026-03-29 / Codex

- Decision: Fix “missing colors” by caching **geometry** separately from **mesh+material** in glTF.
  Rationale: In glTF, material assignment lives on `mesh.primitive.material`, so geometry reuse must not implicitly reuse materials.
  Date/Author: 2026-03-29 / Codex

- Decision: For export, keep negative scales as-authored and **do not** invert winding for mirrored nodes.
  Rationale: glTF consumers (Blender / common viewers) already handle negative determinant transforms; inverting winding on top causes faces to be culled (“transparent” look). Runtime Bevy visuals still need mirrored winding because Bevy does not automatically adjust culling for negative determinant transforms.
  Date/Author: 2026-03-29 / Codex

## Outcomes & Retrospective

- (2026-03-29 10:27 CST) Prefab export now writes `.glb` plus `.gltf`+`.bin` per prefab id, preserves per-part colors, and avoids mirrored backface-culling artifacts in common glTF viewers. Switch (form transform) transitions now apply mirrored-winding meshes consistently so mirrored parts do not disappear mid-animation. Automation export test and rendered smoke test pass.

## Context and Orientation

Relevant repository pieces:

- Prefabs UI: `src/model_library_ui.rs` contains the “Prefabs” panel and the manage-mode export buttons. It uses background threads for `rfd` dialogs and export work, then polls a job receiver resource to show toasts.
- Exporter: `src/prefab_glb.rs` exports `.glb` plus `.gltf`+`.bin`. It builds a glTF 2.0 scene with hierarchy, primitive meshes, material factors, and baked TRS animations per channel.
- Runtime visuals and mirroring:
  - Normal prefab visuals (`src/object/visuals.rs`) compute “mirrored” from negative determinant scales and use `PrimitiveMeshCache::get_or_create_mirrored_winding` to invert indices so Bevy renders correctly.
  - Switch morph visuals (`src/object_forms.rs`) flatten the prefab into leaf meshes and spawn them during the transition, but currently do not apply mirrored winding, so mirrored leaves can disappear during the transition.
- Automation HTTP API: `src/automation/mod.rs` defines routes under `/v1/*` and documents them in `docs/automation_http_api.md`. There is currently a `POST /v1/prefabs/export_gltf_glb` endpoint (and `POST /v1/prefabs/export_glb` as an alias) plus a tracked Python test in `test/run_1/prefab_export_glb_api/run.py`.

Terms used here:

- **glTF**: the JSON-based glTF 2.0 format (`.gltf`) plus external binary buffers (`.bin`) and optional images.
- **GLB**: the binary container form of glTF (`.glb`) containing both JSON and BIN chunks.
- **Negative determinant scale / mirrored transform**: a local transform whose scale vector has an odd number of negative components so `scale.x * scale.y * scale.z < 0`. Many renderers must either flip triangle winding or flip culling for these.

## Plan of Work

### 1) Export both `.gltf`+`.bin` and `.glb`

In `src/prefab_glb.rs`:

- Rename the public API to reflect exporting both formats (for example `export_prefabs_to_gltf_glb_dir`).
- For each prefab id, choose a stable base filename like `<Label>_<prefab_uuid>` and write:
  - `<base>.glb`
  - `<base>.gltf`
  - `<base>.bin`
- The `.gltf` must set `buffers[0].uri` to `<base>.bin` (a relative filename).
- Return a report that includes all generated paths so the HTTP API can return them.

### 2) Fix color/material correctness (geometry vs mesh+material caching)

In `src/prefab_glb.rs`:

- Introduce a geometry cache keyed by procedural mesh parameters (`MeshKey` + optional primitive params). Geometry cache entries hold the accessor indices for POSITION/NORMAL and optional indices accessor.
- Introduce a mesh cache keyed by `(geometry_key, material_key_hash)`. This cache creates a glTF `Mesh` object that references the cached geometry accessors but sets the correct material index.
- Ensure that different `color_rgba` values produce distinct materials and that meshes can be reused across many nodes without losing material variety.

Acceptance check (manual/automation): exporting a colorful Gen3D prefab should produce many more than 4 materials when the source has many unique colors.

### 3) Fix mirrored export “transparent” faces

In `src/prefab_glb.rs`:

- Remove the exporter’s mirrored-winding handling (stop calling Bevy `mesh.invert_winding()` based on a mirrored flag).
- Keep negative scales in node transforms unchanged.

### 4) Fix Switch morph mirrored disappearance

In `src/object/visuals.rs`:

- Make `PrimitiveMeshCache::get_or_create_mirrored_winding` available to other modules within the crate (e.g. `pub(crate)`), or add a small `pub(crate)` helper that wraps it.

In `src/object_forms.rs`:

- When resolving leaf meshes for morph animation, detect negative determinant transforms and replace the mesh handle with the mirrored-winding variant, matching the runtime visuals path.
- If a leaf morph pair has a mirrored sign mismatch (one side mirrored, the other not), treat it as a “different type” morph (fade out/in) rather than trying to morph the same mesh, to avoid half-the-animation being wrong.

### 5) Update UI label and behavior

In `src/model_library_ui.rs`:

- Change the button text from “Export GLB” to “Export glTF/GLB”.
- Update toasts and internal job naming/struct names if needed so they are not misleading.
- The export job should call the new exporter function that writes both formats.

### 6) Update Automation API + docs + tests

In `src/automation/mod.rs`:

- Rename or replace `POST /v1/prefabs/export_glb` with an endpoint whose semantics match exporting both formats (implemented as `POST /v1/prefabs/export_gltf_glb`, while keeping `POST /v1/prefabs/export_glb` as an alias).
- Request body remains `out_dir`, `prefab_id_uuids`, optional `fps` and `move_units_per_sec`.
- Response includes the created paths for `.gltf`, `.bin`, and `.glb`.

In docs:

- Update `docs/automation_http_api.md` to document the new endpoint and response.
- Update `docs/prefab_export_gltf_glb.md` so it describes exporting both glTF and GLB and the output layout.

In tests:

- Update `test/run_1/prefab_export_glb_api/run.py` to call `POST /v1/prefabs/export_gltf_glb` and assert all expected files exist.

### 7) Validation and commits

Run:

- `python3 test/run_1/prefab_export_glb_api/run.py` (updated script) and expect it prints `OK: artifacts at ...`.
- Required rendered smoke test:
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`
  - Expect the game starts rendered and exits without crashing.

Commit after the changes with clear messages.

## Concrete Steps

Work from repo root `.../gravimera`:

    cargo check
    python3 test/run_1/prefab_export_glb_api/run.py
    tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2

## Validation and Acceptance

Acceptance is met when all of the following are true:

- Prefabs panel shows a manage-mode button labeled **Export glTF/GLB**.
- Exporting a prefab produces `.gltf`, `.bin`, and `.glb` files per prefab id.
- Exported models preserve per-part colors (not all-gray when source has varied colors).
- Exported mirrored parts do not appear transparent/missing in common glTF viewers.
- In-game Switch morph animation does not make mirrored parts disappear during the transition.
- Automation endpoint exports both formats and the updated test script passes.

## Idempotence and Recovery

- Export is non-destructive and may overwrite existing files with the same names in the output directory.
- If an export fails, fix the error and re-run export; no on-disk state beyond the output folder is required.

## Artifacts and Notes

- Keep a short note in `Surprises & Discoveries` if new validator/tool compatibility issues appear (for example, glTF validator warnings).

---

Plan update (2026-03-29 10:27 CST): Updated endpoint/doc names to `POST /v1/prefabs/export_gltf_glb` and `docs/prefab_export_gltf_glb.md` to match the implemented behavior, and marked completed milestones after running `cargo check`, the automation export test script, and the rendered smoke test.
