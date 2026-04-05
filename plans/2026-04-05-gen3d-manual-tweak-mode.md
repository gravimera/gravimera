# Gen3D: Manual Tweak Mode (interactive prefab micro-editing)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document follows `PLANS.md` from the repository root and must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D Edit runs a deterministic pipeline that mostly edits a draft via prompts + tools (DraftOps, PlanOps, QA, etc.). This change adds a *separate*, fully manual workflow for micro-edits that does not require additional prompts or LLM calls.

After this change, a user can click **Manual Tweak** inside the Gen3D workshop (Edit session) to enter a micro-edit mode where they:

- Click to select a **primitive part** (a single generated “piece”) within the model preview.
- Adjust transform (move/rotate/scale) and color deterministically.
- Use Undo/Redo to recover from mistakes.
- (Deform) Drag generic control points (FFD box / lattice) to deform geometry, similar to “character creator / face sculpt” systems.
- Save the modified draft as a snapshot so it appears in the 3D Models library.

The result must be directly observable in the rendered UI, and the edits must persist through Save Snapshot, prefab export, and reload.

## Progress

- [x] (2026-04-05 00:00+08) Traced Gen3D preview + DraftOps plumbing (`src/gen3d/preview.rs`, `src/gen3d/ai/draft_ops.rs`) and confirmed part IDs are stable (`gravimera/gen3d/part/<component>/<idx>`).
- [x] (2026-04-05) Added this ExecPlan and started implementation on a single feature branch.
- [x] (2026-04-05) Manual Tweak UI mode + exit semantics (toggle button + `Esc` exits tweak mode first).
- [x] (2026-04-05) Part picking + selection highlighting in Gen3D preview (click-to-select primitive parts).
- [x] (2026-04-05) MVP edits: move/rotate/scale/recolor + Undo/Redo, applied via DraftOps (`apply_draft_ops_v1`).
- [x] (2026-04-05) Deform edits (FFD): sculpt toggle + control cage handles, persistence, export, and size/bounds consistency.
- [x] (2026-04-05) Updated docs (`docs/controls.md`, `docs/gen3d/README.md`) and passed required rendered smoke test.
- [x] (2026-04-05) Committed Milestone A (Manual Tweak MVP): `fb51113`.

## Surprises & Discoveries

- Observation: Gen3D already has a strong deterministic edit surface (`apply_draft_ops_v1`) that updates `planned_components`, syncs attachment trees, recalculates component sizes, snapshots assembly state, and increments `assembly_rev`.
  Evidence: `src/gen3d/ai/draft_ops.rs::apply_draft_ops_v1`.

- Observation: Gen3D preview rebuilds the UI model when `assembly_rev` changes, which means manual edits can be implemented entirely as DraftOps calls.
  Evidence: `src/gen3d/preview.rs::gen3d_apply_draft_to_preview` rebuild gating on `ui_applied_assembly_rev`.

## Decision Log

- Decision: Manual edits (transform/color/deform) are applied through `apply_draft_ops_v1` (DraftOps) rather than directly mutating draft structures in UI code.
  Rationale: DraftOps already enforces atomicity, revision gating, attachment sync, and consistent component size updates. This keeps the edit model deterministic and debuggable.
  Date/Author: 2026-04-05 / Codex

- Decision: “Drag vertices” is implemented as a generic FFD control-cage deform (not per-mesh raw vertex editing).
  Rationale: A control-cage is closer to “face sculpt” UX, works on any primitive mesh, keeps UI usable, and can be persisted compactly and deterministically.
  Date/Author: 2026-04-05 / Codex

- Decision: Manual Tweak targets `primitive` parts only (Gen3D-generated pieces). Imported `model` parts are out of scope for the first implementation.
  Rationale: Primitive parts are already fully described in prefab JSON and DraftOps. Imported model editing requires an asset pipeline and is much higher risk.
  Date/Author: 2026-04-05 / Codex

- Decision: Edits affect only the live Gen3D draft until the user clicks **Save Snapshot**.
  Rationale: This matches current Gen3D workshop semantics and keeps persistence explicit.
  Date/Author: 2026-04-05 / Codex

- Decision: First delivery prioritizes “good, stable panel editing” over viewport gizmos; viewport gizmos can be added later without changing persistence formats.
  Rationale: This reduces input routing complexity and helps ship a reliable baseline quickly.
  Date/Author: 2026-04-05 / Codex

## Outcomes & Retrospective

- Delivered a deterministic, fully manual edit surface inside Gen3D that does not require new prompts or LLM calls.
  - Transform + recolor edits apply via DraftOps with `atomic=true` + `if_assembly_rev` gating.
  - Undo/Redo works by applying deterministic inverse DraftOps.
- Implemented a generic deformation mechanism for procedural primitives (FFD control cage).
  - Persists in prefab JSON (`primitive.deform`) and round-trips through save/load.
  - Runtime uses a deform-aware mesh cache; export applies the same deform before writing GLB buffers.
- Updated user-facing docs for workflows and controls.

Validation:

- `cargo test -q gen3d` passes locally.
- Required rendered smoke test passes: `cargo run -- --rendered-seconds 2` (with temporary `GRAVIMERA_HOME`).

Known limitations / follow-ups:

- Manual Tweak currently edits `primitive` parts only (not imported `model` parts).
- Sculpt is FFD-based (control points), not true per-vertex editing; additional sculpt UX (symmetry, falloff, per-axis handles) can be added later without changing persistence.

## Context and Orientation

Key implementation files:

- Gen3D preview + UI rebuild: `src/gen3d/preview.rs`
- Gen3D workshop UI: `src/gen3d/ui.rs`, `src/gen3d/state.rs`
- Deterministic edits (DraftOps): `src/gen3d/ai/draft_ops.rs`
- Prefab persistence: `src/realm_prefabs.rs`, spec `docs/gamedesign/34_realm_prefabs_v1.md`
- Prefab export (glTF/GLB): `src/prefab_glb.rs`, `docs/prefab_export_gltf_glb.md`
- Controls docs: `docs/controls.md`

Terms:

- **Draft**: the current Gen3D in-memory prefab graph (root prefab + component prefabs).
- **Primitive part**: a prefab `part` where `kind="primitive"` (Gen3D-generated pieces).
- **Part ID**: stable u128/UUID stored in prefab JSON (`parts[].part_id`); Gen3D uses deterministic IDs derived from `gravimera/gen3d/part/<component>/<idx>`.
- **FFD (Free-Form Deformation)**: a generic deformation that moves mesh vertices based on a small set of control points.

## Plan of Work

This work is split into two milestones to reduce risk:

### Milestone A: Manual Tweak Mode (MVP)

1. Add a `Gen3dManualTweakState` resource to track:
   - enabled flag
   - selected part (part_id UUID + resolved component name)
   - active tool (select/move/rotate/scale/color/deform)
   - undo/redo stacks (per-edit snapshots)

2. UI wiring:
   - Add a `Manual Tweak` button (visible only when a draft exists and Gen3D is idle).
   - When enabled, show an `Exit Tweak` button and a small tool panel.
   - Input gating: `Esc` exits tweak mode when tweak is enabled and prompt is not focused.

3. Part picking:
   - Compute a ray from the preview cursor (same projection path used by component overlay).
   - Enumerate candidate primitive parts in the UI preview model.
   - Use deterministic ray-vs-OBB tests to select the closest hit.
   - Highlight the selected part in preview (simple overlay/outline).

4. Edits:
   - Move/Rotate/Scale via panel fields and small step hotkeys.
   - Recolor via color picker/swatches.
   - Apply edits via `apply_draft_ops_v1` (`update_primitive_part` + `set_transform` / `set_primitive`).

5. Undo/Redo:
   - Each edit records the “before” state and can be reversed by a single DraftOps application.
   - `Ctrl/Cmd+Z` undo, `Ctrl/Cmd+Y` redo (when tweak mode is enabled).

### Milestone B: Deform (FFD)

1. Persistence model:
   - Extend the prefab primitive visual definition to carry an optional deform payload (v1).
   - Ensure it round-trips through `src/realm_prefabs.rs`.

2. Rendering:
   - Apply deform to the generated primitive mesh at spawn time.
   - Cache deformed meshes keyed by (mesh key, params, deform payload) to avoid rebuilding every frame.

3. Size/bounds:
   - Update size/bounds computations used by Gen3D to account for deform so selection, grounding, and export match runtime visuals.

4. Export:
   - Update prefab glTF/GLB export to apply deform before writing buffers.

## Concrete Steps

Workdir: repository root.

Run tests:

  cargo test -q

Required rendered smoke test (do not use `--headless`):

- Bash-style (as referenced in `AGENTS.md`):
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

- PowerShell equivalent:
    $tmp = New-Item -ItemType Directory -Force -Path (Join-Path $env:TEMP ("gravimera_smoke_" + [guid]::NewGuid().ToString("N")))
    $env:GRAVIMERA_HOME = (Join-Path $tmp.FullName ".gravimera")
    cargo run -- --rendered-seconds 2

## Validation and Acceptance

Manual Tweak Mode is accepted when:

- In Gen3D workshop (Edit session), user clicks `Manual Tweak` and sees:
  - an obvious “in tweak mode” indicator, and
  - an `Exit` affordance.
- User can select a primitive part by clicking it in the preview, and the selection is visually obvious.
- User can change move/rotate/scale/color and see immediate preview updates.
- Undo/Redo works and does not corrupt the draft.
- `Save Snapshot` persists the edits; the resulting model in the 3D Models panel matches what was previewed.
- (FFD) Deforms persist, export, and reload consistently (no mismatch between in-game visuals and exported GLB).

## Idempotence and Recovery

- Entering/exiting tweak mode is idempotent (no duplicate gizmos/entities).
- DraftOps applications are atomic; any invalid edit must return an actionable error and leave the draft unchanged.

## Interfaces and Dependencies

This change must keep the Gen3D pipeline deterministic. Any new persistence fields or DraftOps schema changes must be mirrored in:

- `src/realm_prefabs.rs` (save/load JSON),
- `docs/gamedesign/34_realm_prefabs_v1.md` (spec update),
- `src/prefab_glb.rs` (export consistency),
- and relevant docs under `docs/` (controls + Gen3D workflow).
