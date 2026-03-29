# Native Screen Recording via Shared UI and Automation API

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gravimera already knows how to render frames, take screenshots, and let external tools drive the simulation. What it does not have is a first-class recording system that can be started from the game itself or from the local Automation HTTP API and that writes a real video file directly, without depending on an external `ffmpeg` process.

After this change, a user can press a Record button in the in-game toolbar or call an HTTP endpoint, let Gravimera run or step deterministically, and then stop recording to obtain a native `.mp4` file written by Gravimera itself. The initial shipped scope is video-only in rendered mode. Audio capture is explicitly deferred so the first version remains tractable and testable.

How to see it working after implementation:

1. Start Gravimera in rendered mode and click the new Record button in the top-left toolbar. A visible recording indicator appears. Stop recording and verify that a timestamped `.mp4` file appears under the Gravimera recording directory.
2. Start Gravimera with `--automation --automation-pause-on-start`, call `POST /v1/recording/start`, drive the game with `POST /v1/step`, then call `POST /v1/recording/stop`. The resulting clip should exist on disk and play in a normal video player.


## Progress

- [x] (2026-03-29 20:19 CST) Researched the current rendering, screenshot, and automation architecture and drafted this ExecPlan.
- [ ] Add a dedicated recording module and plugin that captures frames and writes native MP4 output without `ffmpeg`.
- [ ] Expose one shared recording control surface that is used by both in-game UI and the Automation HTTP API.
- [ ] Add recording configuration, documentation, and a rendered real test under `test/`.
- [ ] Run rendered smoke and recording tests, then commit the implementation.


## Surprises & Discoveries

- Observation: The existing Automation screenshot route is too narrow to be the recording implementation. It only captures the primary window, writes asynchronously, and explicitly rejects `include_ui=false`.
  Evidence: `src/automation/mod.rs` defines `ScreenshotRequest` with `include_ui`, and `POST /v1/screenshot` returns `501` when `include_ui=false`; `docs/automation_http_api.md` documents that screenshot saving is asynchronous and not available in headless mode.

- Observation: Gravimera already has multiple examples of offscreen image capture using Bevy render targets and `Screenshot::image(...)`.
  Evidence: `src/orbit_capture.rs` creates copyable target textures, and `src/scene_build_ai.rs` uses `RenderTarget::Image(...)` plus `ScreenshotCaptured` observers to persist generated views.

- Observation: The existing top-left workspace toolbar is the cleanest user-visible place for a record control because it already owns scene/model/floor/play controls and is always visible in rendered mode.
  Evidence: `src/workspace_ui.rs` builds the toolbar row and currently spawns buttons for `Scenes`, `3D Models`, `Terrain`, and `Play`.

- Observation: Gravimera already has deterministic stepping through `POST /v1/step`, which means the recorder can support a simulation-time mode for reproducible API-driven clips instead of only wall-clock recording.
  Evidence: `docs/automation_http_api.md` documents synchronous stepping and paused-time workflows; `src/automation/mod.rs` owns `AutomationStepJob` and paused-time state.

- Observation: There are existing Bevy ecosystem recording helpers, but they do not match Gravimera’s shipping requirements closely enough to be used as the feature itself.
  Evidence: Research on 2026-03-29 found that Bevy 0.18’s `EasyScreenRecordPlugin` is dev-tool oriented and has platform caveats, and `bevy_capture` documents a headless-only MP4 path. Gravimera still needs its own runtime that works from UI and Automation API in normal rendered mode.


## Decision Log

- Decision: Implement recording as a dedicated `RecordingPlugin` and runtime instead of layering repeated calls on top of `/v1/screenshot`.
  Rationale: The screenshot endpoint is asynchronous, window-only, and request-oriented. Recording needs session state, backpressure, status reporting, and background finalization.
  Date/Author: 2026-03-29 / assistant

- Decision: The native output format for V1 is H.264 video in an MP4 container, encoded in-process.
  Rationale: MP4 is the most practical default artifact for users. The implementation plan is to use an in-process H.264 encoder crate plus an in-process MP4 muxer crate so Gravimera writes `.mp4` directly with no `ffmpeg` dependency.
  Date/Author: 2026-03-29 / assistant + user intent

- Decision: Use `openh264` for video encoding and `muxide` for MP4 muxing.
  Rationale: This keeps encoding and container writing inside the process and avoids introducing a shell-out dependency. The plan assumes these crates remain suitable at implementation time; if either proves incompatible during a spike, the replacement must preserve the same external behavior and the plan must be updated immediately.
  Date/Author: 2026-03-29 / assistant

- Decision: V1 is rendered-mode only and returns `501` in headless mode.
  Rationale: The requested feature is game screen recording. The current repo already distinguishes rendered-mode capabilities from headless mode, and screen capture without a rendered frame source is a different problem.
  Date/Author: 2026-03-29 / assistant

- Decision: V1 ships as video-only. Audio recording is explicitly out of scope for the initial milestone set.
  Rationale: The repo has visible screenshot and TTS paths but no obvious general mixed-audio recording bus. Shipping video first satisfies the user request while keeping complexity bounded.
  Date/Author: 2026-03-29 / assistant

- Decision: The internal model supports multiple capture kinds, but the shipped V1 user-facing capture kind is `window`.
  Rationale: The existing screen-recording request is satisfied by recording the rendered window as the user sees it. An internal enum keeps the door open for later `render_target` or world-only recording without requiring a second redesign.
  Date/Author: 2026-03-29 / assistant

- Decision: Recording uses a constant-frame-rate output model with a selectable timebase: `real` for normal UI usage and `virtual` for Automation API deterministic stepping.
  Rationale: UI users expect wall-clock behavior, while API users need reproducible clips driven by `/v1/step`. A timebase choice resolves the tension without needing two separate recorders.
  Date/Author: 2026-03-29 / assistant

- Decision: The default output root is `${GRAVIMERA_HOME}/recordings/<timestamp>_<session-id>/`.
  Rationale: This keeps artifacts isolated, avoids destructive overwrites, and makes it easy for tests to clean up or inspect one session directory at a time.
  Date/Author: 2026-03-29 / assistant

- Decision: The in-game control lives in the existing workspace toolbar, next to the current `Play` button, and a keyboard shortcut is added as a secondary path.
  Rationale: The toolbar is already the primary rendered-mode “session control” row. Adding record there keeps the UI discoverable and minimizes new layout surface.
  Date/Author: 2026-03-29 / assistant


## Outcomes & Retrospective

At this stage, no recording code has been implemented. The useful outcome is that the architecture question is now resolved enough to execute without reopening basic design every few files:

- The feature will be one shared recorder service used by both UI and Automation API.
- The output will be native MP4 written by Gravimera, not PNG sequences plus external `ffmpeg`.
- The first version is intentionally video-only, rendered-only, and window-capture based.
- The plan already defines where the code goes, what the API contract is, how deterministic stepping is handled, and what tests must prove before the feature is considered done.

Implementation still needs a small feasibility spike around the exact encoder and muxer crates. If that spike changes the dependency choice, this document must be revised before broader implementation continues.


## Context and Orientation

Important terms used in this plan:

- A **recording session** is the period between `start` and `stop`. It owns one output directory, one output `.mp4` file, one status object, and one background encoder worker.
- An **encoder** turns raw image frames into compressed H.264 video packets.
- A **muxer** writes those compressed packets into an MP4 container so ordinary video players can open the file.
- A **capture kind** is where the pixels come from. V1 ships `window`, meaning the primary rendered window including UI.
- A **timebase** decides how the recorder advances timestamps. `real` follows wall-clock deltas from the render loop. `virtual` follows Gravimera’s stepped simulation time so API-driven clips can be deterministic.

Key files and how they fit together today:

- `Cargo.toml` defines current dependencies. Recording adds native encoder and muxer crates here.
- `src/lib.rs` is the root module list. A new `recording` module will be added here.
- `src/app.rs` initializes resources and plugins in rendered mode and already adds `AutomationPlugin`.
- `src/automation/mod.rs` owns the local HTTP API and already exposes `/v1/screenshot` and `/v1/step`.
- `src/workspace_ui.rs` builds the top-left workspace toolbar where the new Record button should live.
- `src/app_plugins.rs` wires update systems for rendered UI and gameplay.
- `src/orbit_capture.rs` is the existing helper for offscreen render targets.
- `src/scene_build_ai.rs` shows the repo’s existing pattern for `ScreenshotCaptured`-driven persistence from offscreen renders.
- `src/config.rs` parses persistent config from `config.toml`; recording defaults belong here.
- `docs/automation_http_api.md` documents the HTTP contract and must be updated when endpoints are added.

Two existing behaviors strongly shape the design:

First, the current screenshot endpoint is asynchronous and request-scoped, so it cannot be the recorder by itself. Second, Automation already supports paused deterministic stepping, which means the recorder must be able to advance by simulation time as well as by wall clock.


## Milestones

### Milestone 1: Core Recording Runtime and Native MP4 Writer

At the end of this milestone, Gravimera can start and stop a recording session internally, capture frames from the primary window, and write a valid MP4 file natively without shelling out to `ffmpeg`. There is no UI button or HTTP endpoint yet; the point of this milestone is to prove the runtime, the worker-thread protocol, and the encoder/muxer stack.

The implementation work lives primarily in a new `src/recording.rs` module plus dependency additions in `Cargo.toml`. The recorder owns session state, file paths, frame queue limits, capture scheduling, and background finalization. The worker thread receives raw frames, converts them to the encoder’s required pixel format, emits compressed H.264 access units, and hands them to the MP4 muxer.

Acceptance for this milestone is not “it compiles.” Acceptance is: a small internal test or harness can write a short MP4 to disk, that file contains the expected MP4 top-level boxes (`ftyp`, `mdat`, `moov`), and its size is clearly non-zero. The repo should also still pass the rendered smoke start-up test.

### Milestone 2: Shared Control Surface for UI and Automation API

At the end of this milestone, the same recorder runtime is reachable from both the in-game UI and the local Automation HTTP API. The user can start and stop the same session type from either surface and can inspect status from both surfaces.

The implementation work spans `src/automation/mod.rs`, `src/workspace_ui.rs`, `src/app_plugins.rs`, and the new recording module. The key rule is that UI and HTTP do not each implement recording logic. They only translate input into shared `RecordingControlCommand` values and read shared status snapshots back out.

Acceptance for this milestone is twofold. First, pressing the UI control in rendered mode visibly toggles recording and yields an `.mp4` file on stop. Second, HTTP calls to `POST /v1/recording/start`, `GET /v1/recording/status`, and `POST /v1/recording/stop` produce the same effect without requiring any UI interaction.

### Milestone 3: Deterministic API Recording and Status Polish

At the end of this milestone, Automation users can request `timebase="virtual"` and record reproducible clips while the game is paused and stepped via `/v1/step`. UI users continue to use `timebase="real"` by default.

The implementation work stays in the recorder runtime and Automation API. The recorder must compute frame timestamps from the chosen timebase, reject impossible combinations with clear errors, and expose enough status to let tests wait for finalization cleanly. This is also where the UI receives a small status pill or label such as `REC 00:03.2` and a final toast naming the output path.

Acceptance for this milestone is a rendered real test that starts the game with Automation enabled and paused, records a clip using `virtual` time, advances a known number of frames through `/v1/step`, stops recording, waits until status returns to idle, and verifies that the output MP4 exists and is structurally valid.

### Milestone 4: Documentation, Recovery Rules, and Follow-Up Hooks

At the end of this milestone, the user-facing and contributor-facing docs match the code, recovery behaviors are explicit, and the next extension points are already reserved in the code structure.

The implementation work updates `docs/automation_http_api.md`, adds a dedicated recording doc under `docs/` if the HTTP API section becomes too large, and keeps README changes minimal. The code should also leave an internal `RecordingCaptureKind` enum in place even if V1 only accepts `window`, because that preserves a clean path toward a later `render_target` mode.

Acceptance for this milestone is complete docs, passing tests, a clean rendered smoke start-up, and a commit message that explains the feature clearly.


## Plan of Work

Begin by adding a new `src/recording.rs` module and registering it from `src/lib.rs`. This module owns all recording-specific types so that neither the UI layer nor the HTTP layer becomes a second recording implementation. Add a `RecordingPlugin`, a `RecordingRuntime` resource, a small command type for `start` / `stop` / `cancel`, and a serializable status snapshot for UI and API consumers.

Extend `Cargo.toml` with an in-process H.264 encoder and an in-process MP4 muxer. The implementation should also add a small amount of pixel-format glue for converting Bevy’s RGBA capture bytes into the encoder’s expected planar YUV format. The recorder must enforce even dimensions before encoding because common H.264 4:2:0 pipelines require it.

Wire the plugin into `src/app.rs` beside the existing rendered-mode plugins and resources. Add recording defaults to `src/config.rs`, including output directory, default frames per second, target bitrate, queue capacity, and default timebase. The output root should be relative to `GRAVIMERA_HOME` by default so tests can isolate their artifacts.

Implement capture scheduling inside the recording module instead of reusing `/v1/screenshot`. For V1, the capture source is the primary window. The recorder should schedule at most one in-flight capture when backpressure is present and should surface dropped-frame counts in status if the encoder cannot keep up. The output clip is constant-frame-rate. In `real` mode, frame scheduling follows wall-clock deltas. In `virtual` mode, frame scheduling follows simulation deltas that advance through the Automation stepping system.

Once the core runtime works, expose it through the Automation HTTP API. Add new request types in `src/automation/mod.rs` and route handlers for start, status, stop, and cancel. These routes should return actionable errors such as “already recording”, “no active recording”, “recording requires rendered mode”, or “virtual timebase requires Automation paused/stepped control” when applicable.

Then add the in-game UI controls in `src/workspace_ui.rs`. The default placement is after the existing `Play` button. The button should toggle between `Record` and `Stop` text, receive a clear active style while recording, and optionally pair with a small status label showing elapsed recording time and frame count. Add a hotkey such as `F9` as a secondary control path, but the toolbar button remains the primary visible affordance.

After both surfaces work, add docs and a rendered real test. The test belongs under `test/` and should use a dedicated run directory such as `test/run_1`. The test must launch Gravimera in rendered mode with Automation enabled, record a short virtual-time clip, stop, poll status until finalization finishes, and assert that the output MP4 file exists and is structurally valid.


## Concrete Steps

All commands run from the repository root.

1. Add the new recording module and dependencies.

   - Edit `Cargo.toml`.
   - Add `src/recording.rs`.
   - Register the module in `src/lib.rs`.
   - Wire the plugin from `src/app.rs`.
   - Add recording config parsing in `src/config.rs`.

2. Add the shared runtime and internal tests.

   - Implement session state, command handling, capture scheduling, worker-thread encoding, and finalization in `src/recording.rs`.
   - Add unit tests in `src/recording.rs` or a nearby dedicated test module for config parsing, state transitions, even-dimension normalization, and MP4 structural sanity.

3. Add Automation API routes.

   - Edit `src/automation/mod.rs`.
   - Add `POST /v1/recording/start`.
   - Add `GET /v1/recording/status`.
   - Add `POST /v1/recording/stop`.
   - Add `POST /v1/recording/cancel`.

4. Add in-game UI controls.

   - Edit `src/workspace_ui.rs` to add button components and layout.
   - Edit `src/app_plugins.rs` to schedule any new interaction and status systems.
   - If a toast is used for completion, reuse the existing `UiToastCommand` path instead of inventing a second notification system.

5. Add docs and the rendered real test.

   - Update `docs/automation_http_api.md`.
   - Add a dedicated recording doc under `docs/` if the HTTP API doc becomes too dense.
   - Add `test/recording_real_test.py`.
   - Add `test/recording_test_config.toml`.
   - Store run artifacts under `test/run_1/`.

6. Validate.

   - `cargo test recording -- --nocapture`
   - `python3 test/recording_real_test.py --run-dir ./test/run_1`
   - `rm -rf test/run_1/smoke && mkdir -p test/run_1/smoke`
   - `GRAVIMERA_HOME="$PWD/test/run_1/smoke/home" cargo run -- --rendered-seconds 2`


## Validation and Acceptance

The feature is accepted only when all of the following are true:

- In rendered mode, clicking the toolbar Record button starts a recording session and visibly changes the UI to indicate that recording is active.
- Clicking the same control again stops the session, returns the UI to idle, and leaves behind a playable `.mp4` file in the expected session directory.
- Starting Gravimera with Automation enabled allows:

  - `POST /v1/recording/start` with a JSON body containing at least `fps`, `timebase`, and optional `path`.
  - `GET /v1/recording/status` returning `state`, `output_path`, `frames_submitted`, `frames_encoded`, `dropped_frames`, and `error`.
  - `POST /v1/recording/stop` to finalize the session.
  - `POST /v1/recording/cancel` to abort and discard an unfinished session.

- A real test under `test/` can record a short deterministic clip by using `timebase="virtual"` and driving the simulation with `/v1/step`.
- The final output file is structurally valid as MP4. At minimum, automated validation must confirm that the file exists, has non-trivial size, and contains recognizable `ftyp`, `mdat`, and `moov` boxes.
- The required rendered smoke test still starts the game without crashing.


## Idempotence and Recovery

- Starting a recording while one is already active must return `409` with a clear error instead of implicitly replacing the old session.
- Stopping or canceling when no recording is active must also return `409` with a clear error.
- Each recording session gets its own unique directory. Retrying after failure never overwrites a successful prior recording unless the caller explicitly requested a specific path and overwrite behavior.
- During encoding, the recorder writes to a temporary file name such as `recording.partial.mp4` and renames to the final `.mp4` only after successful finalization. This prevents half-written files from masquerading as successful artifacts.
- If finalization fails, the status endpoint must preserve the error string and the session directory must keep a small manifest describing the failure so the user can inspect what happened.


## Artifacts and Notes

The session directory should contain the final `.mp4` plus a compact manifest JSON. The manifest is not the recording itself; it is operational evidence that makes tests and failure recovery easier. A reasonable V1 shape is:

    {
      "format_version": 1,
      "capture_kind": "window",
      "timebase": "virtual",
      "fps": 30,
      "bitrate_kbps": 6000,
      "width_px": 1280,
      "height_px": 720,
      "frames_submitted": 90,
      "frames_encoded": 90,
      "dropped_frames": 0,
      "state": "completed",
      "output_file": "session.mp4",
      "error": null
    }

The Automation API request and response shapes should stay simple and actionable. A reasonable `start` request is:

    {
      "path": null,
      "fps": 30,
      "bitrate_kbps": 6000,
      "timebase": "virtual",
      "capture_kind": "window"
    }

A reasonable `status` response is:

    {
      "ok": true,
      "state": "recording",
      "output_path": ".../recordings/2026-03-29_2019_abcd/session.mp4",
      "frames_submitted": 42,
      "frames_encoded": 40,
      "dropped_frames": 0,
      "timebase": "virtual",
      "fps": 30,
      "error": null
    }


## Interfaces and Dependencies

In `src/recording.rs`, define the primary runtime surface:

    pub(crate) struct RecordingPlugin;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum RecordingCaptureKind {
        Window,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(crate) enum RecordingTimebase {
        Real,
        Virtual,
    }

    #[derive(Clone, Debug)]
    pub(crate) struct RecordingStartSpec {
        pub(crate) output_path: Option<std::path::PathBuf>,
        pub(crate) fps: u32,
        pub(crate) bitrate_kbps: u32,
        pub(crate) capture_kind: RecordingCaptureKind,
        pub(crate) timebase: RecordingTimebase,
    }

    #[derive(Clone, Debug)]
    pub(crate) enum RecordingControlCommand {
        Start(RecordingStartSpec),
        Stop,
        Cancel,
    }

    #[derive(Clone, Debug, serde::Serialize)]
    pub(crate) struct RecordingStatusSnapshot {
        pub(crate) state: String,
        pub(crate) output_path: Option<String>,
        pub(crate) frames_submitted: u64,
        pub(crate) frames_encoded: u64,
        pub(crate) dropped_frames: u64,
        pub(crate) fps: u32,
        pub(crate) timebase: &'static str,
        pub(crate) error: Option<String>,
    }

    pub(crate) fn recording_status_snapshot(
        runtime: &RecordingRuntime,
    ) -> RecordingStatusSnapshot;

The recording module should also own the worker-thread protocol and must not leak encoder-specific crate types into `src/automation/mod.rs` or `src/workspace_ui.rs`.

In `src/automation/mod.rs`, define request payloads that map directly onto the shared runtime:

    #[derive(Deserialize)]
    struct RecordingStartRequest {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        fps: Option<u32>,
        #[serde(default)]
        bitrate_kbps: Option<u32>,
        #[serde(default)]
        capture_kind: Option<String>,
        #[serde(default)]
        timebase: Option<String>,
    }

The API handlers should call into the shared recording command path and should never create encoder workers directly.

In `src/workspace_ui.rs`, add dedicated components for the toolbar control:

    #[derive(Component)]
    pub(crate) struct WorkspaceRecordButton;

    #[derive(Component)]
    pub(crate) struct WorkspaceRecordButtonText;

The UI systems should read `RecordingStatusSnapshot` and update button text and style accordingly.

Dependencies to add in `Cargo.toml`:

- A native H.264 encoder crate, currently planned as `openh264`.
- A native MP4 muxer crate, currently planned as `muxide`.

If the implementation spike replaces either dependency, the replacement must still satisfy these requirements:

- No external `ffmpeg` or similar shell-out in the feature path.
- Cross-platform support consistent with Gravimera’s rendered targets.
- Ability to write standard `.mp4` output directly from the process.


## Revision Note

Initial version created on 2026-03-29 to capture the agreed design direction before implementation. The main reason for this plan is that native recording touches rendering, Automation API, UI, dependency selection, and test strategy at the same time, so the architecture must be written down before code starts moving.
