# Todo: unify reasoning_effort config, add Action motion channel, and pass reference images into planning

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D (and other AI-backed flows) currently have multiple “reasoning effort” knobs (a model-level setting plus Gen3D per-step caps), and some steps hard-cap reasoning to low/medium. This makes configuration confusing and prevents “high reasoning everywhere” runs without hunting multiple keys.

Gen3D unit motion generation currently defaults to `idle` + `move` (and sometimes `attack`). We want an additional default unit motion channel named `action` for “operating / doing work” animations (hands moving, levers being pulled, etc.).

When users provide reference images for Gen3D, the engine already produces downsampled “component reference images” for later steps, but prompt-intent classification and plan generation currently don’t receive those images. We want those early planning steps to see the same resolution-handled images so the plan is more accurate.

After this change:

- Config has a single `reasoning_effort` knob for OpenAI-backed reasoning effort, defaulting to `high`, and all AI steps default to using it.
- The runtime and data model support an `action` animation channel, and Gen3D motion authoring can generate it for movable units.
- Gen3D prompt-intent and plan generation include the engine’s resolution-handled reference images when the user provided photos.


## Progress

- [x] (2026-03-23 20:40 CST) Drafted this ExecPlan.
- [x] (2026-03-23) Unify reasoning-effort configuration into a single `reasoning_effort` key and make all steps default to `high`.
- [x] (2026-03-23) Add `action` animation channel + `action_time` driver end-to-end (runtime, serialization, Gen3D prompts/schemas, validation, capture).
- [x] (2026-03-23) Pass resolution-handled reference images into `prompt_intent` and `llm_generate_plan_v1` (agent + pipeline).
- [x] (2026-03-23) Update docs (`config.example.toml`, `docs/object_system.md`, and tool/prompt strings) to match code.
- [x] (2026-03-23) Run `cargo test` and the rendered smoke test (`cargo run -- --rendered-seconds 2` with a temp GRAVIMERA_HOME).
- [x] (2026-03-23) Mark `docs/todo.md` items complete.
- [x] (2026-03-23) Commit changes.


## Surprises & Discoveries

- `render_preview_v1` motion-sheet capture needed end-to-end wiring: blob labels, tool-result JSON, and review-image selection all needed to understand the new `action_sheet.png`.


## Decision Log

- Decision: Use a single OpenAI config key `[openai].reasoning_effort` and remove Gen3D per-step caps.
  Rationale: The todo explicitly asks for a single knob; step-specific caps and hard-coded low/medium caps conflict with “default high everywhere”.
  Date/Author: 2026-03-23 / assistant

- Decision: Model `Action` as an animation **channel** named `action` plus a new driver `action_time` backed by a dedicated `ActionClock`.
  Rationale: Action animations are time-windowed like attacks (not locomotion distance), and we need deterministic sampling for Gen3D sprite-sheet capture; wall-clock (`always`) would make capture nondeterministic.
  Date/Author: 2026-03-23 / assistant


## Outcomes & Retrospective

- Unified all OpenAI-backed “reasoning effort” config to `[openai].reasoning_effort` (default `high`) and removed Gen3D per-step caps so every step can run at the configured effort.
- Added a new runtime animation channel `action` with driver `action_time` backed by `ActionClock`, wired through serialization, Gen3D schemas/prompts, motion validation, and motion-sheet capture (`action_sheet.png`).
- Gen3D prompt-intent and plan generation now receive the engine’s resolution-handled reference images (`user_images_component`) when user photos are provided.
- Validation: `cargo test` passed and the rendered smoke test ran without crashing.


## Context and Orientation

Relevant config and parsing:

- `config.example.toml`: user-facing configuration example.
- `src/config.rs`: config parsing and `AppConfig`/`OpenAiConfig` structs.

Gen3D orchestration & LLM spawning:

- `src/gen3d/ai/agent_loop/mod.rs`: agent-mode prompt-intent spawn (`spawn_agent_prompt_intent_request`).
- `src/gen3d/ai/pipeline_orchestrator.rs`: pipeline-mode prompt-intent spawn.
- `src/gen3d/ai/agent_tool_dispatch.rs`: LLM-backed tool dispatch, including `llm_generate_plan_v1`.
- `src/gen3d/ai/orchestration.rs`: legacy orchestrator and motion-sheet capture (`Gen3dMotionCaptureKind`).

Gen3D prompt/schema contracts:

- `src/gen3d/ai/prompts.rs`: system/user prompts for prompt-intent, plan, motion authoring.
- `src/gen3d/ai/structured_outputs.rs`: Structured Outputs JSON schemas (must match prompts).
- `src/gen3d/ai/schema.rs`: Rust JSON structs/enums for Gen3D AI outputs.
- `src/gen3d/ai/parse.rs` and `src/gen3d/ai/convert.rs`: parsing and application of AI outputs.
- `src/gen3d/ai/draft_ops.rs`: DraftOps schema includes animation slot specs and driver enum list.

Runtime animation + channel selection:

- `docs/object_system.md`: design doc describing animation channels and priority.
- `src/object/registry.rs`: `PartAnimationDriver` enum and `ObjectLibrary` channel helpers.
- `src/object/visuals.rs`: runtime channel selection priority and driver time evaluation.
- `src/types.rs`: components like `AnimationChannelsActive`, `AttackClock` (we will add `ActionClock`).
- `src/locomotion.rs`: updates `AnimationChannelsActive`.
- `src/rts.rs`: debug hotkeys that force channels and insert clocks.
- `src/gen3d/preview.rs`: Gen3D preview dropdown forces channels and inserts clocks.


## Plan of Work

### 1) Unify reasoning-effort configuration

1. In `config.example.toml`, remove Gen3D step-specific `reasoning_effort_*` entries and rename `[openai].model_reasoning_effort` to `[openai].reasoning_effort` (default `high`).
2. In `src/config.rs`:
   - Rename `OpenAiConfig.model_reasoning_effort` → `OpenAiConfig.reasoning_effort`.
   - Remove `AppConfig.gen3d_reasoning_effort_{plan,agent_step,component,review,repair}` fields and their parsing.
3. Update all call sites to use a single effective effort:
   - Gen3D: replace all `cap_reasoning_effort(..., "low|medium|...")` and `cap_reasoning_effort(..., config.gen3d_reasoning_effort_*)` with just the configured effort (from `ai.model_reasoning_effort()` after it is wired to the new `openai.reasoning_effort`).
   - SceneBuild AI: use the renamed `openai.reasoning_effort`.
4. Update tests and mock configs that construct `OpenAiConfig` literals.

### 2) Add `action` motion channel (default for units)

1. Update docs (`docs/object_system.md`) to include channel `action` and priority `attack > action > move > idle > ambient`.
2. Runtime:
   - Add `ActionClock` component and `AnimationChannelsActive.acting` flag in `src/types.rs`.
   - Extend `PartAnimationDriver` with `ActionTime` in `src/object/registry.rs`.
   - Update `src/object/visuals.rs`:
     - Channel selection priority includes `action`.
     - Driver-time evaluation supports `ActionTime` via `ActionClock`.
   - Update `src/locomotion.rs` to compute `acting` from `ActionClock` window (similar to attacks).
   - Update `src/object/registry.rs` helper functions:
     - Update `animation_channels_ordered` to include `action` in the “front” list.
     - Add `channel_action_duration_secs(...)` to support preview/hotkey one-shot behavior.
   - Update `src/rts.rs` and `src/gen3d/preview.rs` to insert and clear `ActionClock` for forced `action` channels (mirrors attack behavior).
3. Gen3D schemas and prompts:
   - Add `action_time` to driver enums in:
     - `src/gen3d/ai/schema.rs` (`AiAnimationDriverJsonV1`)
     - `src/gen3d/ai/structured_outputs.rs` (`schema_motion_authoring`, DraftOps animation slot spec schema)
     - `src/gen3d/ai/prompts.rs` motion authoring system prompt (“driver” and “channel” allowed values, and guidance for when to author `action`).
   - Update parsing/conversion (`src/gen3d/ai/convert.rs`, `src/gen3d/ai/parse.rs`, DraftOps application) to map `action_time` → `PartAnimationDriver::ActionTime`.
4. Motion capture and review artifacts:
   - Extend `Gen3dMotionCaptureKind` to include `Action` and capture an `action_sheet.png` in `src/gen3d/ai/orchestration.rs`.
   - Update `src/gen3d/mod.rs` constants so the max images budget accounts for 3 motion sheets.
   - Update review-image selection and metadata where move/attack sheets are referenced (`src/gen3d/ai/agent_review_images.rs`, `src/gen3d/ai/agent_render_capture.rs`).
5. Pipeline behavior:
   - Update `pipeline_missing_move_slot_coverage(...)` to also require `action` coverage for movable roots (so the pipeline triggers motion authoring when `action` is missing even if QA is otherwise ok).

### 3) Pass reference images into prompt-intent + plan generation

1. In `src/gen3d/ai/agent_loop/mod.rs::spawn_agent_prompt_intent_request`, pass `job.user_images_component.clone()` to `spawn_gen3d_ai_text_thread(...)` instead of `Vec::new()`.
2. In `src/gen3d/ai/pipeline_orchestrator.rs::poll_pipeline_prompt_intent`, pass `job.user_images_component.clone()` similarly.
3. In `src/gen3d/ai/agent_tool_dispatch.rs` for `TOOL_ID_LLM_GENERATE_PLAN`, pass `job.user_images_component.clone()` to `spawn_gen3d_ai_text_thread(...)`.


## Concrete Steps

Work from repo root (`/Users/flow/workspace/github/gravimera`):

1. Edit code + docs as described above.
2. Run tests:

       cargo test

3. Run the rendered smoke test (must not use `--headless`):

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4. Mark `docs/todo.md` items as complete and commit with a clear message.


## Validation and Acceptance

Acceptance checks:

- Config:
  - `config.example.toml` contains `[openai].reasoning_effort = "high"` and no Gen3D step-specific reasoning-effort keys.
  - The game starts without config parse errors for the renamed key.
- Gen3D images:
  - When reference images are provided, the prompt-intent request and plan generation request include the downsampled `job.user_images_component` image paths (observable via run artifacts / agent trace events that log `images: N` for those requests).
- Action motion:
  - Runtime supports channel `action` and driver `action_time` without panics.
  - Gen3D motion authoring schema accepts `action_time` and authored clips apply and persist.
  - Motion-sheet capture produces `action_sheet.png` alongside `move_sheet.png` and `attack_sheet.png`.
- Smoke test:
  - The rendered smoke run starts and exits cleanly.


## Idempotence and Recovery

- All tests and the smoke command are safe to rerun; they write into temp dirs / run caches.
- If config parsing changes break local user config, use `config.example.toml` as the reference and update `~/.gravimera/config.toml` accordingly (no backwards-compat guarantee for this task).


## Artifacts and Notes

- Prefer relying on existing Gen3D run artifacts (`attempt_N/pass_M/*`) to verify image counts and motion-sheet outputs.
- Keep `README.md` minimal; put detailed behavior notes in `docs/` (this file, `docs/object_system.md`, and Gen3D docs).
