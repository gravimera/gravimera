# Gen3D preview component hover framing and explode inspection

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

`PLANS.md` is checked into the repository root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, a user looking at the Gen3D preview panel can inspect the generated assembly as named components instead of a single opaque model. Moving the cursor over a component in the preview will draw a clear frame around that component and show a small info card with useful details such as the component name and size. Turning on the new explode switch will push preview components apart in the preview, keep the saved draft unchanged, and show component names directly in the panel so the user can quickly understand the structure of the object.

The behavior is visible in the existing Gen3D workshop in Build Preview mode. Open Gen3D, generate or load a draft with nested components, hover the preview, and confirm that the hovered visible component is framed. Then enable the explode switch and confirm that the preview separates nested components and shows labels for each of them while hover still reveals the full information card.

## Progress

- [x] (2026-03-30 07:24Z) Read `PLANS.md`, the Gen3D preview UI/state/scheduling code, and the object-visual recursion path to define the implementation.
- [x] (2026-03-30 07:24Z) Chose the generic architecture: preview-only metadata on object-ref visual roots, projected overlay UI in the preview panel, and preview-only explode offsets that do not mutate saved Gen3D draft data.
- [x] (2026-03-30 07:27Z) Drafted this ExecPlan before code changes.
- [x] (2026-03-30 07:41Z) Added preview state, UI marker components, the hover frame/info card overlay, the explode toggle, and a fixed label pool in the preview panel.
- [x] (2026-03-30 07:42Z) Added generic `VisualObjectRefRoot` metadata for spawned object-ref visual roots in `src/object/visuals.rs`.
- [x] (2026-03-30 07:54Z) Implemented preview hover picking, projected framing, and preview-only explode offsets in `src/gen3d/preview.rs`.
- [x] (2026-03-30 07:45Z) Added preview helper tests covering local center math, explode fallback direction, cursor mapping, and local AABB ray entry distance.
- [x] (2026-03-30 07:47Z) Updated `docs/gen3d/README.md` and `docs/controls.md` for the new inspection controls.
- [x] (2026-03-30 08:05Z) Ran `cargo fmt`, focused Gen3D tests, full `cargo test -q`, and the required rendered smoke test.
- [x] (2026-03-30 10:32Z) Diagnosed the user-reported regression: hover/explode were incorrectly limited to `depth == 0`, so nested prefabs collapsed to one top-level target.
- [x] (2026-03-30 10:57Z) Reworked preview picking/explode to support all object-ref depths generically, using hierarchy-safe local/world explode offsets and a nested-component hover ranking.
- [x] (2026-03-30 11:12Z) Added preview automation endpoints for component snapshots, explode toggling, and deterministic probe picking.
- [x] (2026-03-30 11:28Z) Added a rendered real HTTP regression under `test/run_1/gen3d_preview_component_inspection/`.
- [ ] Run full validation on the nested-component fix path and commit the finished change with a clear message.

## Surprises & Discoveries

- Observation: the user-visible Gen3D preview is rendered from a dedicated camera into an off-screen image, then displayed as a fitted UI image in the preview panel.
  Evidence: `src/gen3d/preview.rs` creates a `Camera3d` whose `RenderTarget` is the preview image, and `src/gen3d/ui.rs` fits that image inside `Gen3dPreviewPanelImage`.

- Observation: the preview currently has no per-component runtime metadata on object-ref root entities, so there is nothing to query for hover/picking.
  Evidence: `src/object/visuals.rs` inserts `ObjectRefEdgeBinding` and `PartAnimationPlayer` on object-ref edges, but no generic component-inspection marker exists today.

- Observation: the preview panel already supports absolute-position overlay children, so the new frame, toggle, tooltip, and labels can live inside the existing panel instead of requiring a new window or camera.
  Evidence: `src/gen3d/ui.rs` already places the preview stats card as an absolute child of the preview panel.

- Observation: Bevy system parameter validation rejects multiple mutable `Query` parameters that touch the same component types, even when the queried entities are logically distinct overlay widgets.
  Evidence: the first rendered smoke run failed with `error[B0001]` for `Node`/`Visibility`, then again for `Text`, until the overlay system was rewritten to use a single filtered overlay-node query and a single filtered overlay-text query.

- Observation: limiting preview inspection to object-ref metadata with `depth == 0` only works for flat assemblies. In nested prefabs it makes torso/body shells shadow all internal targets and makes explode appear inert.
  Evidence: `src/gen3d/preview.rs` originally filtered both hover and explode to `meta.depth != 0`, while `src/object/visuals.rs` stored first-level children with `depth = 0`.

## Decision Log

- Decision: inspect the full spawned object-ref tree, not only the first root layer.
  Rationale: user-visible Gen3D prefabs can nest components several levels deep. Restricting inspection to the first layer makes the hover card and explode mode fail on exactly the kind of assemblies the feature is supposed to explain.
  Date/Author: 2026-03-30 / Codex

- Decision: keep the hover frame as a 2D projected overlay rather than a 3D highlight mesh or shader effect.
  Rationale: the preview already renders into a UI image. A projected frame is simpler, generic across any object, independent of mesh topology, and naturally lines up with the preview panel where the user is already looking.
  Date/Author: 2026-03-30 / Codex

- Decision: make explode mode preview-only by adding temporary offsets to preview visual roots instead of rewriting `Gen3dDraft` or object definitions.
  Rationale: the feature is an inspection mode, not a modeling edit. The saved prefab and draft data must remain unchanged when the user toggles explode on or off.
  Date/Author: 2026-03-30 / Codex

- Decision: track and remove the previously applied explode offset per top-level preview component before applying the new one.
  Rationale: top-level object-ref roots are not guaranteed to have a runtime animation player, so blindly adding offsets every frame would accumulate drift on static components. A preview-only `Gen3dPreviewAppliedExplodeOffset` component keeps the transform correction generic and non-destructive.
  Date/Author: 2026-03-30 / Codex

- Decision: collapse hover-frame/card nodes into one filtered query and hover-info/label texts into one filtered query.
  Rationale: this satisfies Bevy’s query-validation rules in the rendered app while keeping the update logic in a single system.
  Date/Author: 2026-03-30 / Codex

## Outcomes & Retrospective

The feature is implemented. Gen3D preview now exposes a hover frame and info card for top-level components, an `Inspect` → `Explode` toggle in the preview controls, and always-on component labels during explode mode. The implementation stays generic by using object-ref visual metadata plus preview-space projection math; it does not assume faces, limbs, or any other object-specific structure.
The feature is implemented. Gen3D preview now exposes a hover frame and info card for nested object-ref components, an `Inspect` → `Explode` toggle in the preview controls, and always-on component labels during explode mode. The implementation stays generic by using object-ref visual metadata plus preview-space projection math; it does not assume faces, limbs, or any other object-specific structure.

The follow-up fix also added automation-only preview debug endpoints so rendered HTTP tests can prove nested picking and explode motion against real prefabs instead of relying on a human mouse session. Remaining validation work is to run the full suite and the new rendered regression after the nested-component patch.

## Context and Orientation

Gen3D lives in `src/gen3d/`. The preview camera and preview model rebuild logic live in `src/gen3d/preview.rs`. The Gen3D UI tree, including the preview panel widget, lives in `src/gen3d/ui.rs`. The preview resource and UI marker components live in `src/gen3d/state.rs`. The plugin schedule that decides when Gen3D systems run lives in `src/app_plugins.rs`. Public re-exports for these systems are in `src/gen3d/mod.rs`.

Object visuals are built recursively in `src/object/visuals.rs`. That module takes an `ObjectDef` from `src/object/registry.rs` and spawns entities for primitive parts, model parts, and object references. An object reference means one object contains another object as a child component. In this repository, the root Gen3D draft object is composed primarily of object references, so the spawned object-ref entities are the natural place to attach generic preview metadata.

The Gen3D preview panel is not a direct 3D viewport. The code creates an off-screen render target of `960x540` pixels and shows that texture through a fitted UI image. This matters for hover math. A cursor position in the window must first be converted into the preview image’s local coordinates, then mapped into render-target coordinates for `Camera::viewport_to_world`, and projected back from render-target coordinates into the panel overlay for the hover frame and labels.

The planned feature must remain generic. It cannot assume a face, head, limb, or any other object-specific structure. The only allowed structural rule is the existing object-component graph that comes from the draft itself.

## Plan of Work

Start in `src/gen3d/state.rs` by extending `Gen3dPreview` with the new inspection state. This should include at least an explode-mode toggle and the current hovered component summary so UI update systems can show the right text and visibility. Add marker components for the new preview overlay entities: one root for the overlay layer, one frame node, one hover info card and its text node, one explode toggle button and label, one label-root for explode labels, and per-label marker components that let a rebuild/update system address labels deterministically.

Update `src/gen3d/ui.rs` to add the new overlay widgets inside `spawn_gen3d_preview_panel` where the preview image and stats card already live. Keep the existing preview layout intact. The explode control should be an explicit toggle in the panel, not a keyboard shortcut. The hover frame and info card should be hidden by default. The component label container should also exist from the start so later systems can populate or hide labels without rebuilding the whole workshop root. Add the button interaction system and any small UI update system needed to keep its style and text in sync with preview state.

In `src/object/visuals.rs`, define a new generic component that describes a spawned object-ref root. It must include the preview root entity that owns the spawned tree, the child object id, the parent object id, the recursion depth, and a stable order index from the parent’s part list. Insert it when spawning an `ObjectPartKind::ObjectRef` child entity inside `spawn_object_visuals_inner`. This metadata must not change gameplay or save behavior; it is only a lightweight description of already-spawned structure.

Implement the preview math in `src/gen3d/preview.rs`. Add helpers that:

1. Compute the displayed preview image rectangle inside the preview panel.
2. Convert a window cursor position into preview render-target coordinates.
3. Build a component-oriented bounding box from the component root `GlobalTransform`, object `size`, and `ground_origin_y`.
4. Ray-test that box for picking.
5. Project the box corners back into panel coordinates to produce a hover frame rectangle.
6. Compute a deterministic explode offset for each top-level component using its current center relative to the draft focus, with a stable fallback based on component order when the direction vector is near zero.

Use that math in systems that run after preview rebuild. One system should update transforms on top-level component roots when explode mode changes or when the preview is rebuilt. Another should update hover state and overlay positions every frame while Gen3D preview is visible. The hover system should only inspect top-level components belonging to the user-visible preview root, not the hidden review-capture copy.

Still in `src/gen3d/preview.rs`, add or rebuild the explode labels using the visible top-level components from the current UI preview root. In explode mode, show one short label near each component. On ordinary hover without explode mode, keep only the frame and the info card visible.

Wire the new systems into `src/app_plugins.rs` after `gen3d_apply_draft_to_preview` and after the existing preview image fit logic where appropriate. Export any new systems from `src/gen3d/mod.rs`.

Add tests in `src/gen3d/preview.rs` for the pure math helpers. At minimum, cover the explode-direction fallback, the local center computation from `size` and `ground_origin_y`, and a projected-frame or cursor-mapping helper so the feature has deterministic coverage without depending on a rendered window.

Finally, update `docs/gen3d/README.md` and `docs/controls.md` so the new preview inspection behavior is discoverable. Keep `README.md` concise and move detail into the docs that already describe Gen3D behavior.

## Concrete Steps

All commands below run from the repository root: `/Users/flow/workspace/github/gravimera`.

1. Refresh the planning requirements and inspect the starting code.

       sed -n '1,240p' PLANS.md
       sed -n '1,260p' src/gen3d/state.rs
       sed -n '1,340p' src/gen3d/preview.rs
       sed -n '1,260p' src/gen3d/ui.rs
       sed -n '360,620p' src/object/visuals.rs
       sed -n '640,940p' src/app_plugins.rs

2. Format after code edits.

       cargo fmt

3. Run focused tests while iterating.

       cargo test -q gen3d::preview
       cargo test -q gen3d::ui

   Observed result after implementation:

       running 5 tests
       .....
       test result: ok. 5 passed; 0 failed

       running 1 test
       .
       test result: ok. 1 passed; 0 failed

4. Run the full suite.

       cargo test -q

   Observed result:

       running 333 tests
       ...
       test result: ok. 333 passed; 0 failed

5. Run the required rendered smoke test in a temporary Gravimera home.

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

   Observed result after fixing runtime query conflicts:

       Finished `dev` profile [unoptimized + debuginfo] target(s) in 4.78s
       Running `target/debug/gravimera --rendered-seconds 2`
       ... Creating new window Gravimera ...

6. Inspect the final diff and commit.

       git status --short
       git add docs/execplans/gen3d_preview_component_hover_and_explode.md docs/gen3d/README.md docs/controls.md src/app_plugins.rs src/gen3d/mod.rs src/gen3d/preview.rs src/gen3d/state.rs src/gen3d/ui.rs src/object/visuals.rs
       git commit -m "Add Gen3D preview component hover and explode inspection"

## Validation and Acceptance

Acceptance is behavioral, not just compilation.

First, run the unit and integration tests listed in `Concrete Steps`. The new preview helper tests must fail before the implementation and pass after it.

Then run the rendered smoke test. The application must start, render for two seconds, and exit without crashing.

Finally, verify the user-visible feature manually in the Gen3D workshop:

1. Enter Build mode, open the Object Preview workspace, and open Gen3D.
2. Load or generate a draft with multiple top-level components.
3. Move the cursor over a visible component in the preview panel.
4. Confirm that:
   - a clear frame appears around the hovered component,
   - the info card shows the component name and additional useful data,
   - moving off the component hides the frame and card.
5. Turn on the explode switch.
6. Confirm that:
   - the preview separates top-level components without changing the saved draft,
   - each separated component shows its name,
   - hovering a component in explode mode still shows the full info card,
   - turning explode back off restores the normal assembled preview.

## Idempotence and Recovery

The code changes are additive and safe to repeat. Re-running `cargo fmt`, the test commands, and the smoke test is safe.

If the UI layout or hover math is wrong midway through implementation, the safe recovery path is to keep the generic metadata component in place, simplify the overlay update logic, and rerun the focused preview tests before rerunning the smoke test. No destructive migration is involved because explode mode is preview-only and does not rewrite saved drafts.

## Artifacts and Notes

Important code locations to revisit while implementing:

- `src/gen3d/state.rs`: `Gen3dPreview` resource and UI marker components.
- `src/gen3d/ui.rs`: `spawn_gen3d_preview_panel`, `enter_gen3d_mode`, and button-style systems.
- `src/gen3d/preview.rs`: `gen3d_preview_orbit_controls`, `gen3d_apply_draft_to_preview`, and new preview overlay math/systems.
- `src/object/visuals.rs`: `spawn_object_visuals_inner`.
- `src/object/registry.rs`: `ObjectDef.size` and `ground_origin_y`.
- `src/app_plugins.rs`: Gen3D schedule wiring.
- `docs/gen3d/README.md` and `docs/controls.md`: user-facing documentation.

Observed evidence:

- `cargo test -q gen3d::preview`: 5 preview helper tests passed.
- `cargo test -q gen3d::ui`: 1 UI test passed.
- `cargo test -q`: 333 tests passed.
- Rendered smoke: the app opened a real window and ran for two rendered seconds without crashing after the overlay-query fix.

## Interfaces and Dependencies

The implementation will continue using Bevy UI and Bevy camera APIs already in the repository. No new crate dependency is required.

In `src/object/visuals.rs`, define a generic metadata component with a stable path and fields equivalent to:

    pub(crate) struct VisualObjectRefRoot {
        pub(crate) root_entity: Entity,
        pub(crate) parent_object_id: u128,
        pub(crate) object_id: u128,
        pub(crate) depth: usize,
        pub(crate) order: usize,
    }

In `src/gen3d/preview.rs`, add pure helper functions for:

- computing the local center of an object bounding box from `size` and `ground_origin_y`,
- computing a deterministic explode direction/offset,
- mapping between panel/image coordinates and preview render-target coordinates,
- projecting component box corners into a 2D frame rectangle.

In `src/gen3d/ui.rs` and `src/gen3d/state.rs`, the preview overlay must expose marker components for:

- the explode toggle button and its text,
- the hover frame,
- the hover info card and its text,
- the root that owns explode labels,
- each explode label node and text node.

Revision note: updated after implementation to record the actual runtime query-validation issues, the finished system design, and the commands/results used to validate the feature.
