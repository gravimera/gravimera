# Gen3D: split motion authoring per-channel (parallel), remove ambient, and enable WebP decoding

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change:

1) Gen3D motion authoring becomes “motion-per-call” instead of “all channels in one call”.
   - A single LLM motion tool call authors **exactly one** animation channel (a “motion”) such as `move`, `action`, `idle`, `attack_primary`, or any user-defined channel name.
   - When multiple channels are needed, the batch tool runs multiple per-channel LLM calls **in parallel** (bounded by the existing Gen3D parallelism limit).

2) The default `ambient` animation channel is removed from Gen3D prompts and validation.
   - Generated prefabs no longer carry an `ambient` channel by default.
   - If always-on motion is desired, it must be authored into standard channels (`idle`, `move`, `action`, `attack_primary`) or into user-defined channels.

3) Gen3D reference images can be dropped as `webp` without Bevy logging:

        WARN bevy_image::image: feature "webp" is not enabled

This matters because Gen3D workflows often use WebP images, and motion authoring latency becomes a dominant wall-time cost once geometry generation is parallelized.

## Progress

- [x] (2026-03-23 19:15 CST) Enable WebP decoding in the Bevy feature set to eliminate the runtime warning and allow WebP drag-and-drop images.
- [x] (2026-03-23) Implement new per-channel + batch motion tools (`llm_generate_motion_v1`, `llm_generate_motions_v1`) and update the agent/pipeline to use them.
- [x] (2026-03-23) Remove `ambient` from motion authoring prompts and motion validation selection logic; update affected tests.
- [x] (2026-03-23) Update docs (`docs/todo.md` + relevant ExecPlans) to match the new tool contract and channel rules.
- [x] (2026-03-23) Add/adjust tests (updated Gen3D unit tests; existing `test/run_1/...` scripts remain valid).
- [x] (2026-03-23) Validation: `cargo test` and rendered smoke test (`tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`).
- [ ] Commit(s): clear, scoped messages per milestone.

## Surprises & Discoveries

- Observation: WebP decoding can fail noisily even when our Gen3D UI accepts `.webp` files (`src/gen3d/images.rs` allows it).
  Evidence: runtime log line in `docs/todo.md` (2026-03-23).

- Observation: The current motion tool validates “movable drafts must have at least one `move` slot” inside the tool result handler.
  Evidence: `src/gen3d/ai/agent_tool_poll.rs` motion authoring path pushes an error if `has_move=false` for movable drafts. This will break per-channel authoring (e.g. authoring `action` first).

## Decision Log

- Decision: Use two tools, mirroring component generation: `llm_generate_motion_v1` (one channel) and `llm_generate_motions_v1` (batch, parallel).
  Rationale: The engine already has a proven “batch tool spawns N parallel LLM calls” implementation (`llm_generate_components_v1` + `agent_component_batch.rs`). Reusing the same pattern avoids invasive changes to the agent step executor while still delivering parallelism.
  Date/Author: 2026-03-23 / codex

- Decision: Keep the existing structured output schema shape (`gen3d_motion_authoring_v1`), but enforce “single-channel” in tool validation.
  Rationale: This minimizes schema churn while still making each LLM call small and channel-specific. It also allows user-defined channel names without needing schema changes.
  Date/Author: 2026-03-23 / codex

## Outcomes & Retrospective

- (Pending) At completion, record: which tools were added/removed, measured wall-time improvements for motion authoring, and any regressions (especially around motion validation and seeded-edit sessions).

## Context and Orientation

### Where motion lives in this repo

Gen3D stores authored motion as animation “slots” on attachment edges:

- Planned component graph: `src/gen3d/ai/schema.rs` (`Gen3dPlannedComponent`, `Gen3dPlannedAttachment`).
- Each attachment has `animations: Vec<PartAnimationSlot>` where:
  - `slot.channel` is a free string (lower_snake_case recommended).
  - `slot.spec.driver` determines which runtime quantity drives time (`always`, `move_phase`, `move_distance`, `attack_time`, `action_time`).
  - `slot.spec.clip` is a small keyframed transform delta or a spin.

Motion authoring tool plumbing:

- Tool ids and user-facing tool list: `src/gen3d/agent/tools.rs`.
- Tool dispatch (starts LLM threads): `src/gen3d/ai/agent_tool_dispatch.rs`.
- Tool poll (parses results and mutates planned components / defs): `src/gen3d/ai/agent_tool_poll.rs`.
- LLM prompt text: `src/gen3d/ai/prompts.rs` (`build_gen3d_motion_authoring_*`).
- Structured output schema: `src/gen3d/ai/structured_outputs.rs` (`schema_motion_authoring`).
- Pipeline orchestrator (deterministic mode): `src/gen3d/ai/pipeline_orchestrator.rs`.
- Motion validation simulation chooses which channel to apply per edge: `src/gen3d/ai/motion_validation.rs` (`choose_slot` in `compute_world_transforms_for_channels`).

### Current problems (from docs/todo.md)

`docs/todo.md` currently asks for:

- “One tool call only generate one motion” and “parallel calls for multiple motions”.
- Remove the default `ambient` channel.
- Allow arbitrary number of motion channels; number keys `1`..`0` trigger the corresponding channel (already implemented in runtime forcing via `ObjectLibrary::animation_channels_ordered_top10`).
- Fix Bevy’s `webp` feature warning.

## Plan of Work

### 1) WebP decoding warning

Edit `Cargo.toml` to enable Bevy’s WebP support (Bevy feature `webp`). Validate via `cargo check` and by running the rendered smoke test. The acceptance condition is: dropping a `.webp` in the Gen3D panel no longer produces the warning and the thumbnail appears.

### 2) Remove `ambient` as a default channel

Update motion-related code paths to stop referencing `ambient`:

- `src/gen3d/ai/prompts.rs`: remove `ambient` from the motion-authoring schema example and from any channel enumerations in prose.
- `src/gen3d/ai/motion_validation.rs`: remove `ambient` from the channel priority list in `choose_slot`.
- `src/gen3d/ai/agent_prompt.rs`: remove `has_ambient` from `state_summary.motion_coverage`.
- Update unit tests that used `ambient` as a placeholder channel:
  - `src/gen3d/ai/draft_ops.rs` upsert animation slot test.
  - `src/gen3d/ai/convert.rs` spinner test.

Acceptance: `cargo test` passes and no runtime logic expects an `ambient` slot.

### 3) Split motion authoring into per-channel calls + parallel batch tool

Add new tool ids and update prompt/tool contracts:

- In `src/gen3d/agent/tools.rs`:
  - Add `TOOL_ID_LLM_GENERATE_MOTION = "llm_generate_motion_v1"`.
  - Add `TOOL_ID_LLM_GENERATE_MOTIONS = "llm_generate_motions_v1"`.
  - Remove or stop listing `llm_generate_motion_authoring_v1` in the public tool registry.
  - Add tool descriptors:
    - `llm_generate_motion_v1` args: `{ channel: string }` (required).
    - `llm_generate_motions_v1` args: `{ channels: string[] }` (required).

- In `src/gen3d/ai/prompts.rs`:
  - Keep the same output schema shape, but add a **required** input concept “target channel”:
    - The system instructions must explicitly say: “Author slots ONLY for channel `<channel>`; set replace_channels=[\"<channel>\"]; every slot.channel must equal `<channel>`.”
  - Update `build_gen3d_motion_authoring_user_text` to include the target channel and current slot counts by channel (so the LLM knows what already exists).

Implementation in tool dispatch/poll:

- In `src/gen3d/ai/job.rs`:
  - Extend `Gen3dAgentLlmToolKind` with:
    - `GenerateMotion { channel: String }`
    - `GenerateMotionsBatch`
  - Add agent state: `pending_motion_batch: Option<Gen3dPendingMotionBatch>`.
  - Add job fields: `motion_queue: Vec<String>`, `motion_in_flight: Vec<Gen3dInFlightMotion>`, `motion_attempts: std::collections::BTreeMap<String,u8>` (or store attempts in the pending batch).

- In `src/gen3d/ai/agent_tool_dispatch.rs`:
  - Implement new tool dispatch branches:
    - `llm_generate_motion_v1`: spawn one LLM thread (similar to the existing motion-authoring tool) and set `pending_llm_tool=GenerateMotion{channel}`.
    - `llm_generate_motions_v1`: initialize a batch state and set `pending_llm_tool=GenerateMotionsBatch` (do not start all threads here; the poller will manage the parallel limit, mirroring `agent_component_batch.rs`).

- Add `src/gen3d/ai/agent_motion_batch.rs` (new), modeled after `agent_component_batch.rs`:
  - Poll in-flight channel requests, parse+validate each result, and apply it to `job.planned_components[*].attach_to.animations`.
  - Start new channel requests up to the parallel limit (`job.max_parallel_components`), with the same “Responses continuation unknown ⇒ parallel=1” safety gate.
  - When all requested channels are completed, return a single `Gen3dToolResultJsonV1` summarizing per-channel success/failure.

- In `src/gen3d/ai/agent_tool_poll.rs`:
  - Add a special-case early branch like the component batch:
    - If `pending_llm_tool==GenerateMotionsBatch`, call `poll_agent_motion_batch(...)` and surface the tool result when done.
  - Update schema repair routing to use the motion-authoring schema for `GenerateMotion{..}` and `GenerateMotionsBatch`.
  - Update the single-motion handler:
    - Enforce “single channel” constraints:
      - `replace_channels` must be exactly `["<requested_channel>"]`.
      - Every slot.channel must equal `<requested_channel>`.
    - Remove the “movable must have a move slot” check, and replace it with:
      - If requested_channel is `move` and the draft is movable, require at least one `move` slot after applying.
      - If requested_channel is `action` and the draft is movable, require at least one `action` slot after applying.

Update the orchestrators / prompts that call motion authoring:

- In `src/gen3d/ai/agent_prompt.rs`:
  - Replace references to `llm_generate_motion_authoring_v1` with:
    - If movable and missing channels, call `llm_generate_motions_v1` with the missing set (`move`, `action`, and optionally `idle`; add `attack_primary` if the unit has an attack profile).
    - For custom user-requested motions, call `llm_generate_motion_v1` with a user-defined channel name.

- In `src/gen3d/ai/pipeline_orchestrator.rs`:
  - Replace the single call to the old motion tool with `llm_generate_motions_v1`:
    - When missing move/action coverage: request exactly the missing channels.
    - When motion_validation fails: request `["move","action"]` (and `attack_primary` if the unit attacks), since the failure is likely in authored deltas.

- In `src/gen3d/ai/openai.rs` mock:
  - Update the artifact_prefix handling to match new prefixes, and ensure the mock returns only the requested channel.

Acceptance:

- A movable draft that is missing both `move` and `action` triggers one batch tool call and produces both channels (tool result shows success for both).
- A user can request a custom channel (e.g. `dance`) via the agent tool call and it is preserved in the prefab animations list.

## Concrete Steps

Run from repo root:

1) Rust checks:

    cargo fmt
    cargo test

2) Rendered smoke test (per AGENTS.md):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3) (Optional) Manual verification of WebP drag-and-drop:

   - Start the game rendered.
   - Open Gen3D panel.
   - Drop a `.webp` file into the window.
   - Observe: thumbnail appears and log does not print the WebP feature warning.

## Validation and Acceptance

Acceptance requires:

- `cargo test` succeeds.
- Rendered smoke test starts and exits cleanly.
- Motion tools:
  - `llm_generate_motion_v1` rejects multi-channel outputs and produces clear actionable errors.
  - `llm_generate_motions_v1` runs multiple channel calls in parallel (visible in status/progress) and applies results without corrupting other channels.
- No remaining references to `ambient` in motion tool prompts/validation logic (except as user-authored free-form channels, which should still work).

## Idempotence and Recovery

- Tool changes are not backward compatible; if the agent/pipeline seems to call an old tool id, update both the prompt and the tool registry together.
- Batch motion authoring is safe to re-run; it only replaces the requested channels and leaves other channels intact.

## Artifacts and Notes

- Keep per-channel artifacts under the Gen3D pass directory:
  - `motion_<channel>_raw.txt`
  - `motion_<channel>.json`
  - `motion_batch_result.json` (summary)

## Interfaces and Dependencies

At end state, the following must exist and be used:

- Tool ids:
  - `llm_generate_motion_v1` (one channel; args require `channel`)
  - `llm_generate_motions_v1` (batch; args require `channels`)

- New poller module:
  - `src/gen3d/ai/agent_motion_batch.rs` with a public `poll_agent_motion_batch(...) -> Option<Gen3dToolResultJsonV1>` analogous to `agent_component_batch.rs`.

- The agent and pipeline must not reference `llm_generate_motion_authoring_v1` anymore.
