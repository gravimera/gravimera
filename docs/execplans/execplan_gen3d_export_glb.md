# Gen3D: Export generated prefabs to game-ready `.glb` (glTF 2.0) + Blender/Unity editable bundles

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D currently generates animated prefabs made of primitive parts (cuboid/sphere/cylinder/cone + a few param primitives) composed by anchors/attachments, and saves them into the in-engine `ObjectLibrary`. Those prefabs are usable inside Gravimera, but they cannot be exported as standard 3D files.

After this change, a user can export a Gen3D-generated prefab as a **standards-compliant glTF 2.0 binary** (`.glb`) that is directly usable in games (mesh triangles + PBR materials + animation clips). If the user wants an “editable” deliverable, the exporter also produces a bundle designed for DCC/game tools:

- **Blender**: an editable `.blend` generated from the `.glb` via a deterministic Blender CLI script.
- **Unity**: a Unity-friendly `.fbx` generated via the same Blender CLI script (and a `README` with import guidance).

This must work from three entry points:

1. In-game (Gen3D UI): export the current draft or the last saved prefab.
2. Local Automation HTTP API (`/v1/...`): export by `prefab_id_uuid` so agents/tools can batch export.
3. CLI tool (`model-tool`): export from a prefab id without needing a GPU or interactive UI.

You can see it working by:

- Generating a model in Gen3D with visible movement (wheels spin, legs swing).
- Exporting `runtime.glb` and opening it in a glTF viewer / Blender: the mesh appears correctly oriented and scaled, and animation clips (`idle`, `move`, `attack_primary`, `ambient`) play.
- (Optional) Producing `editable.blend` and `unity.fbx` and importing into Blender/Unity to confirm the hierarchy and clips survive.

## Progress

- [x] (2026-02-15 04:31Z) Write this ExecPlan and keep it current.
- [ ] Add a versioned “Prefab → glTF/GLB” export module with deterministic output.
- [ ] Add animation baking/export for all Gen3D animation drivers and clip types.
- [ ] Add in-game Gen3D “Export” UI + status feedback + output folder conventions.
- [ ] Add Automation API endpoint(s) to export by `prefab_id_uuid` and download a `.zip` bundle.
- [ ] Restore `model-tool` with export subcommands (headless, no GPU required).
- [ ] Add tests: exporter unit tests + glTF parse/validation + regression fixtures in `tests/`.
- [ ] Add optional Blender CLI conversion pipeline (`.glb` → `.blend` + `.fbx`) with deterministic settings.
- [ ] Update docs (`README.md`, `docs/automation_http_api.md`, `gen_3d.md`) and run smoke test.
- [ ] Commit implementation.

## Surprises & Discoveries

- None yet. Update this section during implementation with evidence (test output, screenshots, import logs).

## Decision Log

- Decision: Export glTF 2.0 **binary** (`.glb`) as the primary artifact, plus an “editable bundle” produced by optional Blender CLI conversion.
  Rationale: `.glb` is widely supported, self-contained, and carries node animations. Blender CLI is a practical way to generate `.blend`/`.fbx` without writing bespoke exporters for those formats.
  Date/Author: 2026-02-15 / Codex + user

- Decision: Export **one glTF animation per channel** (`ambient`, `idle`, `move`, `attack_primary`) rather than trying to reproduce Gravimera’s runtime “channel selection” logic in-file.
  Rationale: glTF animations are static clips; game engines choose which clip to play. This maps cleanly to Gravimera’s channel concept and keeps the file usable outside the engine.
  Date/Author: 2026-02-15 / Codex + user

- Decision: For distance-driven drivers (`MovePhase`, `MoveDistance`), bake to time using an explicit **export-time speed** (m/s) and store the original driver semantics in glTF `extras` so external runtimes can re-drive if desired.
  Rationale: glTF animation time is seconds-only, but Gen3D uses meters as the driver unit for locomotion. Baking makes the file immediately playable in typical engines; metadata preserves the “correct” semantics for advanced consumers.
  Date/Author: 2026-02-15 / Codex + user

## Outcomes & Retrospective

- Not started. Update at major milestones and on completion.

## Context and Orientation

This plan exports **prefabs** as defined by the in-engine object system.

Key code locations (paths are repo-root-relative):

- `src/object/registry.rs`: The prefab data model:
  - `ObjectDef` (anchors + parts).
  - `ObjectPartDef` with `ObjectPartKind::{Primitive,ObjectRef,Model}`.
  - Attachments: `AttachmentDef { parent_anchor, child_anchor }` on a part.
  - Animations: `PartAnimationSlot { channel, spec }` on a part with:
    - drivers: `Always`, `MovePhase`, `MoveDistance`, `AttackTime`
    - clips: `Loop { duration_secs, keyframes }` and `Spin { axis, radians_per_unit, axis_space }`

- `src/object/visuals.rs`: Runtime evaluation reference:
  - Attachment composition: `resolve_attachment_transform*` composes `parent_anchor * offset * child_anchor^-1`.
  - Animation evaluation: `update_part_animations` computes `animated = base * delta(t)` then applies attachment composition.

- `src/gen3d/save.rs`: How Gen3D “Save” clones the current draft into fresh UUID prefab ids and inserts defs into `ObjectLibrary`.

- `src/automation/mod.rs` + `docs/automation_http_api.md`: The existing local Automation HTTP API patterns (JSON `{ok:...}` envelope, optional bearer token).

- `src/model_tool.rs`: Placeholder CLI tool entrypoint that will be restored for import/export utilities.

Important definitions for this plan:

- Prefab: An `ObjectDef` graph (root `ObjectDef` referencing other `ObjectDef`s by `ObjectRef` parts) stored in `ObjectLibrary`.
- Anchor: A named local-space transform on an `ObjectDef` used as an attachment “socket” frame.
- Attachment: A rule that aligns a child object’s anchor frame to a parent object’s anchor frame, with an additional offset transform (`ObjectPartDef.transform`) expressed in the parent anchor frame.
- Channel: A named animation bucket (`ambient`, `idle`, `move`, `attack_primary`). Gravimera chooses which channel plays at runtime; the exporter will emit separate glTF clips per channel.
- Driver units: For `Always` and `AttackTime`, the driver unit is seconds. For `MovePhase` and `MoveDistance`, the driver unit is meters traveled (see `src/locomotion.rs` where `LocomotionClock.t` advances by distance).

## Plan of Work

Implement this as an internal “export pipeline” that can be called from UI, automation, and CLI without duplicating logic.

### Milestone A — Define a stable exporter interface (no glTF details yet)

1. Add a new module `src/export/` with a public surface that does not mention glTF internals:

   - `src/export/mod.rs` defines:
     - `ExportSource` (by `prefab_id: u128`, and “current Gen3D draft” in rendered mode).
     - `ExportProfile`:
       - `Runtime` (game-ready, optimized)
       - `Editable` (preserve hierarchy + anchors + metadata)
     - `ExportFormat`:
       - `Glb` (always supported)
       - `BundleZip` (zip containing `*.glb` plus optional conversions and metadata)
     - `ExportOptions`:
       - output directory and base name
       - profile/format selection
       - animation bake settings (sample rate; speed mapping for meter-driven animations)
       - “include anchors as empty nodes” toggle (default: on for `Editable`, off for `Runtime`)
       - “merge static meshes” toggle (default: on for `Runtime`, off for `Editable`)
       - “emit skin” toggle (optional advanced compatibility mode; see Milestone F)
     - `ExportResult` (paths written; warnings)
     - `export_prefab(options, library, root_prefab_id) -> Result<ExportResult, String>`

2. Add a small “deterministic output” policy:
   - Sort any hash-map iteration results.
   - Use stable naming rules (defined in Milestone C) so repeated exports are byte-identical when inputs/options are identical.
   - Write a single `export_manifest.json` into the output directory describing versions/options and source ids.

Acceptance for Milestone A:

- Code compiles with a stub exporter implementation.
- Call sites can be wired later without changing the exporter API.

### Milestone B — Build a complete glTF 2.0 `.glb` writer

Implement glTF output in `src/export/gltf/` (folder), isolated from UI/automation/CLI.

1. Add Rust dependencies (exact versions chosen at implementation time; keep minimal):
   - `gltf-json` for building a correct glTF JSON tree (types + serialization).
   - `byteorder` (or a tiny internal helper) for writing the `.glb` container header and chunk framing.
   - `zip` for bundle output (only if `BundleZip` is implemented in Rust).
   - `gltf` (read/parse) as a dev-dependency for tests to validate exported output.

2. Implement `.glb` writing (self-contained; no external tools):
   - GLB is a binary container with:
     - 12-byte header: magic `glTF`, version 2, total length.
     - JSON chunk: chunk length, chunk type `JSON`, UTF-8 JSON bytes padded to 4-byte alignment with spaces.
     - BIN chunk: chunk length, chunk type `BIN`, raw buffer bytes padded to 4-byte alignment with zeros.

3. Implement a small “buffer builder” utility:
   - Append typed arrays (u16/u32/f32) to the BIN blob with 4-byte alignment.
   - Return (byte_offset, byte_length).
   - Create matching glTF `bufferView` and `accessor` entries with correct `componentType`, `type`, `count`, and min/max for positions.

Acceptance for Milestone B:

- A unit test can build a trivial glTF (one triangle mesh, one node) and write a `.glb`.
- The test re-parses it with the `gltf` crate successfully.

### Milestone C — Prefab traversal → glTF node graph (static pose)

Implement “prefab graph to glTF nodes” with two profiles: `Runtime` and `Editable`.

1. Implement a deterministic traversal of a prefab graph:
   - Input: `ObjectLibrary` + `root_prefab_id`.
   - Output: a tree of “export nodes” where each node corresponds to:
     - an object instance (an `ObjectDef` node),
     - a part instance (a primitive part node or a child object node),
     - optional anchor empty nodes (editable profile).
   - Detect and reject cycles (the engine already guards this at spawn time; exporter should error early).

2. Implement attachment composition exactly like runtime:
   - Use the same math as `src/object/visuals.rs`:
     - If a part has `attachment`, compute:
       - `local = parent_anchor * offset * inverse(child_anchor)`
     - Else:
       - `local = offset` (the part’s own transform).
   - Use the same “robust decomposition” approach as the runtime:
     - Multiply matrices.
     - Decompose to translation/rotation/scale (TRS).
     - If any component is non-finite, return an error describing the prefab path that failed.

3. Node naming and metadata rules (stable and editor-friendly):
   - Root node: `Gen3D_<prefab_id_short>` (8 hex chars from the high bits) and include the prefab label in `extras`.
   - Object nodes: `obj_<label>_<id_short>`.
   - Primitive nodes: `part_<kind>_<index>` plus `extras.gravimera.part` containing:
     - `source_prefab_id_uuid`
     - `source_part_index`
     - `source_part_id_uuid` (if present)
     - `channel_slots` summary (names only)
   - Anchor empty nodes (editable profile): `anchor_<name>` with `extras.gravimera.anchor = { name: ..., }`.

4. Mesh merging policy (runtime profile):
   - Goal: reduce draw calls without breaking animation.
   - Rule: only merge primitives that are:
     - in the same object node,
     - not animated in any channel,
     - not under an animated parent chain (to avoid needing per-vertex skinning),
     - and share the same material.
   - Implementation strategy:
     - Build a per-object “static batch” mesh per material by transforming each primitive’s unit mesh vertices by the primitive’s local TRS and appending into one vertex/index buffer.
     - Keep animated parts as separate nodes/meshes.

Acceptance for Milestone C:

- Exporting a saved Gen3D prefab produces a `.glb` that loads and looks correct in Blender (static pose).
- Runtime profile has fewer meshes/nodes than editable profile for the same prefab.

### Milestone D — Geometry export for all Gen3D primitives (materials + UVs + normals)

Export must output triangle meshes with consistent winding, normals, and UVs.

1. Primitive geometry sources:
   - For unit primitives, generate meshes the same way `src/setup.rs` does (Bevy shape primitives):
     - `Cuboid::new(1,1,1)`
     - `Cylinder::new(0.5, 1.0)`
     - `Cone::new(0.5, 1.0)`
     - `Sphere::new(0.5)`
     - `Capsule3d::new(0.25, 0.5)`
     - `ConicalFrustum { radius_top: 0.25, radius_bottom: 0.5, height: 1.0 }`
     - `Torus::new(0.25, 0.5)`
   - For param primitives, generate the Bevy mesh using the param values from `PrimitiveParams`.

2. Extract Bevy mesh vertex attributes:
   - Required: `POSITION`, `NORMAL`.
   - Strongly recommended: `UV_0` (so the editable export can be textured later).
   - Indices: support both u16 and u32; choose u32 if vertex count exceeds 65535.

3. Material mapping (Primitive color → glTF PBR):
   - Create one glTF material per unique `(base_color_rgba, unlit)` pair.
   - Use `pbrMetallicRoughness` with:
     - `baseColorFactor = [r,g,b,a]`
     - `metallicFactor = 0.0`
     - `roughnessFactor = 0.92` (match `src/object/visuals.rs`)
   - Alpha mode:
     - if `a < 1.0`: `BLEND`
     - else: `OPAQUE`
   - If `unlit == true`, attach `KHR_materials_unlit` extension.

4. Coordinate system and units:
   - Gravimera uses Bevy transforms; treat 1 unit = 1 meter.
   - glTF nodes are right-handed with Y-up. Do not apply axis conversion unless tests show mismatch.
   - If Blender/Unity imports reveal a systematic axis mismatch, fix it by inserting a single root “axis correction” node (never by per-node hacks), and document it in `export_manifest.json`.

Acceptance for Milestone D:

- All primitive kinds used by Gen3D export successfully.
- The exported glTF parses, and Blender shows correct shading (normals) and UVs exist.

### Milestone E — Animation export (channels, drivers, baking)

Export glTF animations so they play correctly in typical engines with no custom runtime code.

1. glTF animation structure (what to build):
   - A glTF `animation` is a collection of “channels”.
   - Each channel targets one node property: translation, rotation, or scale.
   - Each channel references a sampler:
     - input accessor: key times in seconds (f32).
     - output accessor: TRS values (Vec3 for translation/scale, Vec4 quaternion for rotation).
     - interpolation: use `LINEAR` (default).

2. Export one glTF animation per Gravimera channel name:
   - `ambient`, `idle`, `move`, `attack_primary`.
   - Only include nodes that actually have a slot for that channel.
   - Name animations exactly by channel for predictable usage.

3. Driver-unit → seconds mapping (critical):
   - `Always`: driver unit is seconds; no conversion.
   - `AttackTime`: driver unit is seconds since attack start; no conversion.
   - `MovePhase` and `MoveDistance`: driver unit is meters traveled.
     - Convert to seconds using an explicit export option `move_speed_mps`:
       - `seconds = meters / move_speed_mps`.
     - Default `move_speed_mps`:
       - If root prefab has `mobility.max_speed`, use it.
       - Else use `1.0`.
     - Write the chosen value into `export_manifest.json` for reproducibility.

4. Slot evaluation (how to generate TRS keyframes):
   - For each animated node:
     - Start with the node’s “base” local transform (the exported static pose).
     - Evaluate the slot’s delta transform as Gravimera does:
       - Loop clip: interpolate between keyframe deltas and wrap at duration.
       - Spin clip: compute delta rotation from axis + axis-space + radians-per-unit.
     - Apply `speed_scale` and `time_offset_units` exactly as `src/object/visuals.rs`:
       - `t_units = driver_units * speed_scale + time_offset_units`.
     - Compose `animated = base * delta(t_units)`.
   - Important: For attached nodes, the exported node base already includes anchor alignment. Therefore, evaluate animation in the same space as the runtime:
     - Treat `base_transform` as the part’s offset transform (before anchor composition),
     - then compose with anchors to get the final node-local transform.
   - To avoid subtle mismatches, re-use the same helper math (or copy it) from `src/object/visuals.rs` in the exporter:
     - `mul_transform(a,b)`
     - `resolve_attachment_transform_with_offset(...)`

5. Baking strategy (avoid heuristic sampling):
   - Loop clips: use the author-provided keyframe times directly; also emit a final key at `duration` matching the first key to encourage seamless looping in DCC tools.
   - Spin clips: bake at a fixed sample rate `spin_sample_fps` (export option, default 60) for a computed loop duration:
     - If radians-per-unit is finite and non-zero, choose a loop that completes exactly one rotation:
       - `loop_units = 2π / |radians_per_unit|`
       - convert `loop_units` to seconds using the driver mapping above
     - Clamp loop duration to a reasonable range (export options, default min 0.25s, max 10s) and record clamping in warnings.

6. Preserve semantics in metadata:
   - For every exported animated node, include `extras.gravimera.animation_slots` describing:
     - channel name
     - driver kind (including that Move* are meter-driven)
     - speed_scale and time_offset_units
     - clip kind and parameters
   - This allows advanced consumers to ignore baked timing and re-drive by distance in their own engine if desired.

Acceptance for Milestone E:

- A Gen3D wheel model exported to `.glb` spins when playing the `move` clip in Blender (time-based).
- A Gen3D gait model exported to `.glb` shows leg swing in `move` and not in `idle`.

### Milestone F — Optional “skinned export” compatibility mode (Unity-friendly rigs)

Some engines/pipelines prefer skeletal rigs over transform-animated node hierarchies. Provide an optional mode that converts rigid parts into a skinned mesh driven by bones:

1. When `ExportOptions.emit_skin == true`:
   - Create one bone node per exported part node.
   - Generate one combined mesh where each vertex is weighted 1.0 to the bone corresponding to its original part.
   - Export inverse bind matrices and a `skin` that binds the mesh node to the bone hierarchy.

2. Keep the same animation clips, but target the bone nodes instead of mesh nodes.

Acceptance for Milestone F:

- Unity imports the `.glb` (via a glTF importer) and plays animations through bone transforms without losing hierarchy.

### Milestone G — Entry points: Gen3D UI, Automation API, and `model-tool`

1. Gen3D UI (rendered mode):
   - Add an **Export** button in the Gen3D side panel (near Save).
   - Export sources:
     - “Current Draft” (exports the current in-memory draft without forcing Save).
     - “Last Saved Prefab” (uses the last saved `prefab_id_uuid` tracked in `Gen3dAiJob` / workshop status).
   - Output location:
     - `~/.gravimera/exports/gen3d/<timestamp>_<prefab_id_short>/`
   - UI feedback:
     - Show “Exporting…” state and report written paths; surface warnings (e.g., clamped spin duration).

2. Automation API:
   - Add `POST /v1/prefab/export` (general, not Gen3D-specific) in `src/automation/mod.rs`.
   - Request body shape:

       {"prefab_id_uuid":"...","profile":"runtime|editable","format":"glb|bundle_zip","out_dir":"/abs/path","base_name":"MyModel","move_speed_mps":3.0,"spin_sample_fps":60,"emit_skin":false}

   - Response body shape:

       {"ok":true,"out_dir":"...","written":[".../runtime.glb",".../editable.glb",".../export_manifest.json"],"warnings":[...]}

   - Optional: `POST /v1/prefab/export_download` that returns `application/zip` bytes for clients that can’t read local disk.

3. CLI (`model-tool`):
   - Restore `src/model_tool.rs` into a real CLI with:
     - `model-tool export --prefab-id <uuid> --out <dir> --profile runtime|editable [--bundle unity|blender|both]`
     - `model-tool export --scene-dat <path> --instance-id <uuid>` (resolve prefab from a scene instance)
   - Ensure it runs without a GPU:
     - It should not require Bevy rendered mode.
     - It should read `scene.dat`/prefab data and generate meshes internally.

Acceptance for Milestone G:

- A user can export via UI, via HTTP, and via CLI, and all paths produce identical `.glb` bytes for identical inputs/options.

### Milestone H — Blender/Unity editable bundle conversion (optional, but supported)

Provide optional conversion via Blender if it is installed.

1. Add a tool script under `tools/blender/`:
   - `tools/blender/gravimera_convert_glb.py` which:
     - imports the exported `editable.glb`,
     - ensures animations are present as named actions,
     - optionally organizes nodes into collections,
     - saves a `.blend`,
     - exports `.fbx` with Unity-friendly axis settings.

2. Add a tiny Rust wrapper `src/export/blender.rs` that:
   - detects `blender` in `PATH`,
   - runs `blender --background --python tools/blender/gravimera_convert_glb.py -- --in ... --out-blend ... --out-fbx ...`,
   - captures stdout/stderr into the export directory for debugging.

3. Bundle layout for users:
   - `runtime.glb`
   - `editable.glb`
   - `editable.blend` (optional; requires Blender)
   - `unity.fbx` (optional; requires Blender)
   - `export_manifest.json`
   - `README_export.txt` (import instructions for Blender + Unity)

Acceptance for Milestone H:

- Running the conversion script on CI/local produces `.blend` and `.fbx` without interactive UI.
- Unity imports the `.fbx` with separate clips and correct scale/orientation.

### Milestone I — Tests, fixtures, docs, and validation

1. Tests (must be deterministic; keep fixture files under `tests/` and use a dedicated test folder for artifacts):
   - Unit tests for:
     - `.glb` container writer (alignment, chunk sizes).
     - Accessor correctness (counts, component types).
   - Golden/fixture tests:
     - Export a tiny prefab (two primitives + one attachment + one loop animation) into a temp directory.
     - Parse with `gltf` crate and assert:
       - expected node count/names exist,
       - meshes have positions/normals,
       - animations exist with expected channel names.
   - If fixture `.glb` files are committed, keep them small and clearly versioned (and document regeneration steps).

2. Docs updates:
   - `README.md`: Document export entry points (UI/API/CLI) and the meaning of runtime vs editable exports.
   - `docs/automation_http_api.md`: Add the new export endpoints.
   - `gen_3d.md`: Add “Export” workflow details and where files are written.

3. Validation commands:
   - `cargo test`
   - Smoke test: `cargo run -- --headless --headless-seconds 1`

Acceptance for Milestone I:

- Tests pass, and the smoke test starts and exits cleanly.
- Manual spot-check: export a model with motion and confirm the exported clips behave in Blender.

## Concrete Steps

All commands below run from the repo root.

During implementation, keep `Progress` up to date and commit frequently.

1. Add exporter modules and stubs:

    rg -n \"mod export\" -S src
    ls src

2. Implement `.glb` writer and add a unit test that re-parses output:

    cargo test export_glb

3. Wire automation endpoint and manually test it (rendered or headless as appropriate):

    cargo run -- --automation --automation-bind 127.0.0.1:8791 --headless --headless-seconds 0
    curl -s -X POST http://127.0.0.1:8791/v1/prefab/export -H 'Content-Type: application/json' -d '{...}'

4. Smoke test:

    cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

Acceptance is defined as user-visible behavior and tool usability:

1. UI: In Gen3D mode, Export writes files to `~/.gravimera/exports/gen3d/...` and reports the paths in the status panel.
2. `.glb` correctness:
   - The file parses with the `gltf` crate.
   - Blender imports it without warnings about missing buffers/accessors.
3. Animation correctness:
   - Clips exist for `ambient`, `idle`, `move`, `attack_primary` when authored by the prefab.
   - A wheel spin model shows rotation in `move`.
4. Determinism:
   - Exporting the same prefab twice with identical options yields byte-identical `.glb`.
5. Editable bundle (optional):
   - If Blender is installed, `.blend` and `.fbx` are generated and import correctly into Blender/Unity.

## Idempotence and Recovery

- Export is idempotent if `out_dir` is empty or if the exporter is allowed to overwrite:
  - Default: create a new timestamped folder per export.
  - If `out_dir` already exists, write to `out_dir/.tmp_*` and atomically rename on success to avoid half-written bundles.
- On failure, leave the previous successful export intact and write an `export_error.txt` with a short message.

## Artifacts and Notes

- Always write `export_manifest.json` with:
  - exporter version (bump when output semantics change),
  - source `prefab_id_uuid`,
  - export options (including chosen `move_speed_mps`),
  - warnings (clamps, unsupported parts).

- For unsupported inputs (e.g., `ObjectPartKind::Model` referencing non-glTF scenes), fail with a clear error that lists the prefab path to the offending part. Do not silently drop geometry.

## Interfaces and Dependencies

At the end of implementation, these stable interfaces should exist:

- `crate::export::export_prefab(...) -> Result<ExportResult, String>` (primary entry point).
- `crate::export::gltf::write_glb(path, root, bin) -> Result<(), String>` (low-level writer).
- `POST /v1/prefab/export` implemented in `src/automation/mod.rs`.
- `model-tool export ...` implemented in `src/model_tool.rs`.

Dependencies to add (minimal set; lock versions in `Cargo.lock`):

- `gltf-json` (writer types)
- `gltf` (tests only, parse/validate)
- `zip` (bundle zip output; optional if bundle is directory-only)
- `byteorder` (optional; can be replaced by manual little-endian writes)
