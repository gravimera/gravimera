# Fix Gen3D preview playback and add preview image/GIF export

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document follows [PLANS.md](/Users/flow/workspace/github/gravimera/PLANS.md) from the repository root and must be maintained in accordance with that file.

## Purpose / Big Picture

After this change, the Gen3D preview panel must reliably animate the selected motion channel instead of occasionally appearing frozen or failing to restart a preview animation after the draft or session changes. In the same workflow, a user must be able to export preview renders for the current Gen3D draft: static PNG images and animated GIFs for motion channels, using both a new button in the Gen3D preview UI and a local Automation HTTP API.

The result must be directly observable. In Build Preview mode, select different animation channels in the Gen3D preview and confirm the model visibly moves every time. Then click the new export button and observe a generated export directory containing informative filenames and a manifest. From automation, call the new export endpoint, poll its status, and verify the same files are produced.

## Progress

- [x] (2026-03-31 00:26Z) Read `PLANS.md` and traced the Gen3D preview animation, preview UI, and Automation HTTP entry points.
- [x] (2026-03-31 03:39Z) Fixed the preview animation freeze by removing the motion-capture guard from the UI preview ticker and added regression coverage for both motion-capture playback and custom attack-driven channels.
- [x] (2026-03-31 05:04Z) Added a shared Gen3D preview export runtime with a dedicated off-screen preview camera/model root, deterministic channel sampling, ordered PNG/GIF filenames, and manifest generation.
- [x] (2026-03-31 05:24Z) Wired the export runtime to a new preview-panel Export button plus new Automation HTTP start/status endpoints under `/v1/gen3d/preview/export`.
- [x] (2026-03-31 05:08Z) Updated Gen3D/docs/API docs, re-ran focused preview/export tests, and passed the required rendered smoke test (`GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`).
- [ ] Commit the finished change set with a clear message.

## Surprises & Discoveries

- Observation: The Gen3D preview UI already has a dedicated animation ticker in `src/gen3d/preview.rs`, but preview export support does not exist yet. Existing preview-related HTTP endpoints only expose inspection and pan/explode state.
  Evidence: `src/gen3d/preview.rs::gen3d_preview_tick_selected_animation` and `src/automation/mod.rs` routes for `/v1/gen3d/preview`, `/v1/gen3d/preview/components`, `/v1/gen3d/preview/explode`, `/v1/gen3d/preview/pan`, and `/v1/gen3d/preview/probe`.

- Observation: The repository already contains reusable off-screen screenshot capture patterns for Gen3D review renders and prefab thumbnails, which means the export feature can stay generic and avoid window-level screenshots.
  Evidence: `src/gen3d/ai/orchestration.rs::start_gen3d_review_capture` and `src/gen3d/save.rs::start_gen3d_prefab_thumbnail_capture`.

- Observation: The preview scene lights already render both the UI preview layer and the hidden capture layer, so the export feature only needed a temporary camera/model root and did not need to duplicate the light rig.
  Evidence: `src/gen3d/preview.rs::setup_preview_scene` assigns the preview lights to both `GEN3D_PREVIEW_UI_LAYER` and `GEN3D_PREVIEW_LAYER`.

## Decision Log

- Decision: Implement preview export as an asynchronous runtime owned by Gen3D resources instead of a synchronous HTTP route.
  Rationale: Multi-frame capture requires several engine frames and render callbacks. An asynchronous runtime lets the UI button and Automation HTTP API drive the same code path without blocking the main thread.
  Date/Author: 2026-03-31 / Codex

- Decision: Use the existing Gen3D preview render target and orbit/capture helpers as the basis for export, and write GIFs in-process via the `image` crate rather than shelling out to external tools.
  Rationale: The repository already depends on `image`, and this keeps the feature deterministic, portable, and self-contained.
  Date/Author: 2026-03-31 / Codex

- Decision: Drive export captures from a dedicated temporary preview model root on `GEN3D_PREVIEW_LAYER` instead of reusing the visible UI preview root.
  Rationale: This preserves the user-facing preview state while still reusing the existing preview scene lights/orbit framing and keeps automation/UI exports on the same code path.
  Date/Author: 2026-03-31 / Codex

## Outcomes & Retrospective

- The intermittent preview freeze was caused by `gen3d_preview_tick_selected_animation` bailing out whenever the Gen3D AI job was capturing motion sheets. That guard blocked the visible UI preview root from advancing even though motion-sheet capture uses separate off-screen rendering.
- The preview export feature now writes ordered still/GIF bundles plus `manifest.json` from both the preview UI and Automation HTTP API, using a shared runtime that advances across frames and cleans up its temporary entities/render target after completion or failure.
- Validation: `cargo test gen3d::preview -- --nocapture` passed with the new preview/export regression tests, and the required rendered smoke test (`GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`) exited cleanly.

## Context and Orientation

Gen3D lives under `src/gen3d/`. The interactive preview state is stored in `src/gen3d/state.rs` in the `Gen3dPreview` resource. The preview scene, camera orbit behavior, draft application, and animation ticker live in `src/gen3d/preview.rs`. The Gen3D preview panel UI is built and updated in `src/gen3d/ui.rs`.

The actual runtime animation sampler lives in `src/object/visuals.rs::update_part_animations`. It reads three kinds of state from the preview root entity: `AnimationChannelsActive`, `ForcedAnimationChannel`, and time-driver components such as `LocomotionClock`, `AttackClock`, and `ActionClock`. The preview ticker in `src/gen3d/preview.rs` is responsible for driving those components so the preview behaves like gameplay/runtime playback.

Automation HTTP routes live in `src/automation/mod.rs`. Current Gen3D preview routes provide inspection and camera state, but there is no export API yet. The public user-facing documentation for this API lives in `docs/automation_http_api.md`.

Off-screen image capture already exists in two places. `src/gen3d/ai/orchestration.rs` captures Gen3D review renders into PNG files from render targets, and `src/gen3d/save.rs` captures prefab thumbnails. These are the right reference implementations for a new preview export runner because they already handle camera spawning, render-target screenshots, and cleanup.

An “informative filename” in this plan means a filename that includes enough context to understand the exported asset without opening it. At minimum, the filename should encode the motion channel and whether the file is a still or a GIF, and when there are multiple still views it must encode the view name as well.

## Plan of Work

First, isolate the intermittent preview-animation bug in `src/gen3d/preview.rs`. Read how `gen3d_preview_tick_selected_animation` derives active channels and clocks, and compare that against `src/object/visuals.rs::update_part_animations`. Add a focused regression test in `src/gen3d/preview.rs` or the closest module that reproduces the broken state transition. Then patch the preview ticker or related preview rebuild logic so selected preview channels always start and keep advancing correctly after draft rebuilds, channel switches, and one-shot completion.

Next, add a preview export runtime to Gen3D. Extend `src/gen3d/state.rs` with the resource state and marker components needed to represent an in-progress or completed preview export job. The runtime must hold enough information to capture a sequence of frames, write still PNGs, compose GIFs, and report status back to both UI and automation. Reuse preview camera math and render-target capture helpers rather than using the primary window.

Then wire the export runtime into the preview UI. In `src/gen3d/ui.rs`, add a new button in the preview stats/actions area near the animation and inspect controls. The button should start an export to a deterministic default directory under the user’s Gravimera home/cache area and should reflect busy/completed/error state. Update the preview stats text so the user can see where the last export went or why it failed.

After that, expose the same export capability through Automation HTTP. Add request/response structs and routes in `src/automation/mod.rs`. The API should support starting an export with an optional `out_dir` override and an optional channel list override, and it should expose a status endpoint so automation can wait for completion and inspect the manifest. The route handlers must validate that Gen3D preview mode is available and that the preview model exists before starting work.

Finally, document the new API and preview behavior in `docs/automation_http_api.md` and `docs/gen3d/README.md`. Validation must include focused tests for the playback fix, focused tests for export naming/manifest or GIF composition helpers, and the required rendered smoke test. If practical, add a small automation-oriented regression around the new export API under `test/run_1/`.

## Concrete Steps

Work from the repository root:

1. Investigate and patch preview playback.
   Run:
     cargo test gen3d::preview -- --nocapture

2. Implement the export runtime, UI button, and HTTP routes.
   Run targeted tests after each slice:
     cargo test gen3d::preview -- --nocapture
     cargo test gen3d_component_regen_preserves_internal_motion_and_attachment_sync -- --nocapture

3. Validate the Automation HTTP documentation examples against the implementation.

4. Run the required rendered smoke test:
     tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

5. Inspect the git diff, then commit:
     git status --short
     git commit -m "<clear message>"

Expected success indicators:

    test result: ok.

and for the rendered smoke test:

    Running `target/debug/gravimera --rendered-seconds 2`

with no crash before exit.

## Validation and Acceptance

Acceptance for the playback fix:

- In the Gen3D preview panel, selecting `idle`, `move`, `action`, `attack`, or a custom authored motion channel visibly animates the model whenever the draft contains that motion.
- After a Gen3D draft rebuild or session switch, selecting the same preview motion again still plays; the preview does not remain frozen because of stale local state.
- The new regression test fails before the fix and passes after it.

Acceptance for preview export:

- Clicking the new preview export button produces an export directory with informative filenames and a manifest file describing what was written.
- The export directory contains at least one still PNG and one animated GIF when the preview has motion-capable channels.
- `POST /v1/gen3d/preview/export` starts an export job, `GET /v1/gen3d/preview/export` reports progress/result, and the completed response includes the output directory and manifest path.
- The required rendered smoke test still passes.

## Idempotence and Recovery

The export operation must be safe to repeat. If the output directory already exists, create any missing parent directories and either write a new timestamped subdirectory or overwrite only the files belonging to the current export request in a deterministic way. Do not delete unrelated files.

If an export fails partway through, the runtime must record an error status and clean up any temporary cameras/entities it created. A later export request must be able to start fresh without restarting the game.

## Artifacts and Notes

Important artifacts to keep small and inspectable:

- The ExecPlan itself: `plans/2026-03-31-gen3d-preview-animation-export.md`
- Automation API docs: `docs/automation_http_api.md`
- Gen3D user docs: `docs/gen3d/README.md`
- Any real-test or helper output under `test/run_1/` for the new export API

When the export feature is implemented, keep a short manifest example in this section. It should show the naming convention rather than every field in the final JSON.

## Interfaces and Dependencies

The final implementation must expose a Gen3D preview export runtime through explicit repository-local interfaces. The exact names can be adjusted if needed, but the resulting code must provide equivalents of the following:

- A new Gen3D resource in `src/gen3d/state.rs` describing preview export state, including idle/running/completed/error metadata and the output directory.
- A polling system in `src/gen3d/preview.rs` or a nearby Gen3D module that advances preview export capture across frames and writes files.
- A UI button handler in `src/gen3d/ui.rs` that starts the export runtime from the current preview draft.
- Automation routes in `src/automation/mod.rs` for:
  - `POST /v1/gen3d/preview/export`
  - `GET /v1/gen3d/preview/export`

The implementation should keep capture generic by reusing:

- `crate::orbit_capture::create_render_target`
- `crate::orbit_capture::orbit_transform`
- `bevy::render::view::screenshot::Screenshot`
- `image` crate encoders for PNG/GIF composition

Revision note: initial ExecPlan created on 2026-03-31 to cover a preview playback fix plus a shared preview export feature spanning UI and Automation HTTP.
