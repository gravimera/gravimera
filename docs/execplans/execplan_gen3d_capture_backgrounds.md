# Gen3D: Fix “transparent-looking limb” renders by supporting capture backgrounds

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Right now, some Gen3D screenshot renders can make light-colored geometry (like a white arm) look “transparent” because the model is brightly lit and the capture background is also very light. This is a rendering/preview contrast problem, not an AI “alpha transparency” output problem.

After this change, Gen3D screenshot captures will support deterministic “background presets” (at least a higher-contrast option), and the documented `render_preview_v1.background` argument will actually affect the images that get written to the Gen3D cache. This makes it easy for both humans and the AI reviewer to disambiguate “washed out / low contrast” from “actually transparent”.

You can see it working by producing a Gen3D render that previously looked washed-out, then re-rendering with a darker or mid-gray background and observing that the limb silhouette is clearly opaque.

## Progress

- [x] (2026-02-16 00:00Z) Wrote initial ExecPlan for fixing background handling and improving contrast in Gen3D capture renders.
- [ ] Implement a background preset type and parser used by capture code.
- [ ] Wire background presets into `start_gen3d_review_capture` camera setup.
- [ ] Honor `render_preview_v1.background` by passing it into capture.
- [ ] Change pass-snapshot screenshots to use a contrast-friendly background preset by default.
- [ ] Add unit tests that prove camera clear color changes with the preset.
- [ ] Update docs (`README.md`, and optionally `gen_3d.md`) to reflect the real behavior.
- [ ] Run `cargo test` and the headless smoke test to confirm the game starts.
- [ ] Commit the changes with a clear message.

## Surprises & Discoveries

- Observation: The tool docs for `render_preview_v1` already show a `background` argument, but the implementation ignores it.
  Evidence: In `src/gen3d/ai/agent_tool_dispatch.rs`, the handler parses `background` into a variable named `_background` and never uses it.

- Observation: All Gen3D capture cameras use a fixed very-light clear color, which makes bright geometry blend into the background.
  Evidence: `src/gen3d/ai/mod.rs` (`start_gen3d_review_capture`) sets `Camera.clear_color` to `Color::srgb(0.93, 0.94, 0.96)`. `src/gen3d/preview.rs` uses the same clear color for the user-visible preview camera.

- Observation: The “transparent-looking right arm” in the reported cache run is an appearance issue, not an actual alpha/transparency output issue.
  Evidence: The cached component JSON for the arm uses alpha `1.0` for its colors, and the left arm is produced via copy tooling, so both arms are fully opaque in the draft definition.

## Decision Log

- Decision: Implement “background” as a small enum-like preset that deterministically maps to a camera clear color (solid colors only for this change).
  Rationale: It fixes the bug (ignored arg) and addresses the contrast issue with minimal surface area, without adding any heuristic/content-dependent logic.
  Date/Author: 2026-02-16 / assistant

- Decision: Keep the user-visible Gen3D preview panel background unchanged for now, but render pass-snapshot screenshots with a more contrast-friendly preset.
  Rationale: Pass snapshots are debugging artifacts under `~/.gravimera/cache/gen3d/.../pass_N/`; prioritizing legibility there reduces confusion when inspecting runs, while keeping the in-game preview appearance stable.
  Date/Author: 2026-02-16 / assistant

## Outcomes & Retrospective

Not implemented yet.

Expected at completion: `render_preview_v1.background` changes the PNG backgrounds, pass snapshot screenshots are easier to read for light models, and unit tests demonstrate that the capture cameras’ clear color is set from the selected preset.

## Context and Orientation

Gen3D is a workshop mode that builds a draft object out of primitive shapes and attachments, then uses screenshot renders as review inputs and as cache/debug artifacts.

Key terms used in this plan:

“Clear color” means the color the camera uses to clear the render target before drawing any geometry. In practice, it is the solid “background color” behind the model in the screenshots.

“Review capture” means the offscreen screenshot pipeline that spawns one camera per view angle, renders into an `Image` render target, and writes `.png` files into the Gen3D cache directory.

“Pass snapshot screenshots” are the `pass_front.png`, `pass_left_back.png`, etc. images written under a run’s `attempt_*/pass_*/` directory when `gen3d_save_pass_screenshots` is enabled (it defaults on outside tests). These are not a user UI feature; they are debugging artifacts.

Important files and code locations:

- Tool docs for render preview: `src/gen3d/agent/tools.rs` (`TOOL_ID_RENDER_PREVIEW` description includes `background`).
- Tool implementation: `src/gen3d/ai/agent_tool_dispatch.rs` (the match arm for `TOOL_ID_RENDER_PREVIEW` parses `background` but ignores it).
- Capture implementation: `src/gen3d/ai/mod.rs` (`start_gen3d_review_capture` spawns cameras; currently uses a fixed clear color).
- Pass snapshot capture: `src/gen3d/ai/agent_step.rs` (`maybe_start_pass_snapshot_capture` calls `start_gen3d_review_capture` with prefix `"pass"`).
- User-visible preview camera: `src/gen3d/preview.rs` (fixed clear color; out of scope to change behavior, but we may reuse constants for consistency).

## Plan of Work

First, introduce a small “background preset” type in a place accessible to both `src/gen3d/ai/mod.rs` and the agent tool handler(s) (for example `src/gen3d/ai/agent_tool_dispatch.rs`). The simplest place is inside the `gen3d::ai` module near the existing `Gen3dReviewView` and capture state types, because `start_gen3d_review_capture` already lives there and is called from agent code via `super::...`.

Define a preset set that is stable and easy to document. At minimum:

- `neutral_studio`: the current behavior, `Color::srgb(0.93, 0.94, 0.96)`.
- `contrast_studio`: a mid-gray background that improves visibility for both very light and very dark geometry, for example `Color::srgb(0.55, 0.56, 0.58)`.
- `dark_studio`: a dark background for maximal contrast with very bright geometry, for example `Color::srgb(0.12, 0.13, 0.15)`.

Also treat `default` as an alias for `neutral_studio` for backward compatibility with any existing agent traces or user experiments.

Second, change `start_gen3d_review_capture` to accept a background preset parameter and use it when constructing the `Camera { clear_color: ... }` component. This is the core behavioral change that affects all capture call sites (tool renders, auto-review renders, and pass snapshots), but we will keep the call sites’ chosen default behaviors explicit.

Third, wire the `render_preview_v1.background` argument through the tool handler. Replace the unused `_background` parsing in `src/gen3d/ai/agent_tool_dispatch.rs` with a real parse and pass the chosen preset into `start_gen3d_review_capture`. If parsing fails (unknown string), return a tool error that lists the accepted values so the agent can recover deterministically.

Fourth, improve pass snapshot screenshot legibility by changing `maybe_start_pass_snapshot_capture` to call `start_gen3d_review_capture` with `contrast_studio` instead of the neutral background. This specifically targets the reported “arm looks transparent” debugging experience, without altering the in-game preview panel.

Fifth, add unit tests that prove the background wiring works without needing a GPU render. The test should build a minimal `World`, create `Commands` and `Assets<Image>`, call `start_gen3d_review_capture` twice with different background presets, apply the queued commands, and then query spawned camera entities to assert their `Camera.clear_color` is the expected `ClearColorConfig::Custom(...)`. This test is purely structural: it asserts the camera component is configured correctly, which is the core of the fix.

Sixth, update docs so they reflect reality. At minimum, update `README.md` to mention that Gen3D debug renders and the `render_preview_v1` tool support background presets (and name the accepted strings). Optionally also update `gen_3d.md` because it is explicitly “current implementation” documentation for Gen3D.

Finally, run the project’s tests and the required smoke test, then commit with a clear message.

## Concrete Steps

All commands below run from the repo root: `/Users/flow/workspace/github/gravimera`.

1) Reconfirm the current problem in code (before editing):

    rg -n "let _background" src/gen3d/ai/agent_tool_dispatch.rs
    rg -n "ClearColorConfig::Custom\\(Color::srgb\\(0\\.93, 0\\.94, 0\\.96\\)\\)" src/gen3d

2) Implement the background preset type and parsing.

   Edit `src/gen3d/ai/mod.rs` near the capture/view types to add something like:

   - `enum Gen3dCaptureBackgroundPreset { NeutralStudio, ContrastStudio, DarkStudio }`
   - `impl Gen3dCaptureBackgroundPreset { fn parse(id: &str) -> Result<Self, String>; fn clear_color(self) -> Color; fn id(self) -> &'static str }`

   Parsing should normalize the identifier. Prefer using the existing normalization helper used elsewhere in the module tree (for example, `normalize_identifier_for_match`) so inputs like `"dark-studio"` and `"dark_studio"` behave the same.

3) Wire the preset into capture.

   Edit `src/gen3d/ai/mod.rs`:

   - Change the signature of `start_gen3d_review_capture(...)` to include a `background: Gen3dCaptureBackgroundPreset`.
   - In the camera spawn, replace the hard-coded `Color::srgb(0.93, 0.94, 0.96)` with `background.clear_color()`.

   Update every call site in `src/gen3d/ai/mod.rs`, `src/gen3d/ai/agent_tool_dispatch.rs`, and `src/gen3d/ai/agent_step.rs` to pass an explicit preset.

4) Honor `render_preview_v1.background`.

   Edit `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_RENDER_PREVIEW` handler:

   - Replace the unused `_background` binding with real parsing.
   - Default when the arg is missing should be `neutral_studio` (so the current behavior is preserved).
   - Pass the preset to `super::start_gen3d_review_capture(...)`.

   If parsing fails, return an immediate tool error like: “Unknown background '<value>'. Expected one of: neutral_studio, contrast_studio, dark_studio.”

5) Improve pass snapshot screenshots.

   Edit `src/gen3d/ai/agent_step.rs` in `maybe_start_pass_snapshot_capture(...)`:

   - Pass `Gen3dCaptureBackgroundPreset::ContrastStudio` when calling `super::start_gen3d_review_capture(...)`.

   This is the part that directly improves the “cache inspection” experience.

6) Update tool documentation.

   Edit `src/gen3d/agent/tools.rs`:

   - In the `render_preview_v1` description, add a sentence that lists accepted `background` values and states the default is `neutral_studio`.

7) Add tests.

   Preferred approach: add `#[cfg(test)] mod tests` in `src/gen3d/ai/mod.rs` (or a nearby module within `gen3d::ai`) that:

   - Builds a minimal `Gen3dDraft` with a root `ObjectDef` and at least one primitive part so capture doesn’t early-return.
   - Creates `World`, inserts `Assets<Image>`, constructs `Commands`, calls `start_gen3d_review_capture` with a known preset, applies commands, and asserts the spawned cameras have the expected `clear_color`.
   - Repeat with another preset.

   If you need any fixture files (unlikely for this test), store them under `tests/` per `AGENTS.md`.

8) Run validation commands:

   - Unit tests:

        cargo test

     Expect: “test result: ok. … passed …”

   - Headless smoke test (AGENTS.md requirement; use an isolated data dir):

        tmpdir=$(mktemp -d)
        GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --headless --headless-seconds 1

     Expect: process exits successfully with no panic/crash.

9) Update docs:

   - `README.md`: mention background presets for Gen3D capture renders (at least for developers/debugging), and name the accepted strings.
   - Optionally `gen_3d.md`: add a short note under Cache / Debugging Artifacts that pass screenshots use a contrast-friendly background and that `render_preview_v1.background` is supported.

10) Commit:

   - Use a clear message, for example:

        git status
        git commit -am "gen3d: honor render_preview background + improve pass screenshots"

   If new files were added, `git add` them explicitly before committing.

## Validation and Acceptance

Acceptance is based on observable behavior and tests:

Tool correctness: A unit test proves that calling `start_gen3d_review_capture` with different background presets results in spawned capture cameras having different `Camera.clear_color` values.

End-to-end developer experience: In any Gen3D run where a limb previously looked “transparent” in `pass_*.png` screenshots due to low contrast, the same type of model should now have clearly readable pass screenshots because the pass snapshot capture uses the `contrast_studio` background preset.

Robustness: `cargo test` passes and the headless smoke test starts and exits without crashing.

Documentation: `README.md` reflects the supported `render_preview_v1.background` values and the existence of background presets for capture renders.

## Idempotence and Recovery

This change is safe to repeat and easy to roll back.

If tests fail after wiring the new parameter through `start_gen3d_review_capture`, revert to the last commit and re-apply changes in smaller steps: first add the preset type, then change `start_gen3d_review_capture`, then update call sites one-by-one.

If a background preset choice is controversial (for example changing pass snapshots to `contrast_studio`), it can be reverted independently because it is a single call-site change in `maybe_start_pass_snapshot_capture`.

No persistent user data formats should change; this only affects how capture cameras are configured and what pixels end up in the cache PNGs.

## Artifacts and Notes

Example `render_preview_v1` tool call args after this change (for contrast checking):

    {
      "views": ["front", "left_back", "right_back", "top", "bottom"],
      "image_size": 768,
      "include_motion_sheets": false,
      "overlay": "none",
      "background": "dark_studio",
      "prefix": "review_dark"
    }

Expected cache outputs:

- For pass snapshots: `~/.gravimera/cache/gen3d/<run_uuid>/attempt_0/pass_N/pass_front.png` etc will be rendered with the contrast background.
- For tool renders: filenames follow the existing `prefix` behavior and will reflect the chosen background only by the pixels (unless the caller encodes it into `prefix`).

## Interfaces and Dependencies

Define these stable interfaces in code:

- A background preset type with a parse function and a deterministic clear-color mapping:

    enum Gen3dCaptureBackgroundPreset { ... }

    impl Gen3dCaptureBackgroundPreset {
        fn parse(id: &str) -> Result<Self, String>;
        fn clear_color(self) -> Color;
        fn id(self) -> &'static str;
    }

- Update capture entrypoint to accept the preset:

    fn start_gen3d_review_capture(
        commands: &mut Commands,
        images: &mut Assets<Image>,
        run_dir: &Path,
        draft: &Gen3dDraft,
        include_overlay: bool,
        file_prefix: &str,
        views: &[Gen3dReviewView],
        width_px: u32,
        height_px: u32,
        background: Gen3dCaptureBackgroundPreset,
    ) -> Result<Gen3dReviewCaptureState, String>

Keep the implementation generic and deterministic. Do not introduce any content-dependent heuristics (for example, automatically choosing a background based on model colors). The goal is to give humans and the agent an explicit, reliable control knob.
