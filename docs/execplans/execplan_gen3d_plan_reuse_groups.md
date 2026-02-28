# ExecPlan: Gen3D Plan Reuse Groups + Auto-Copy Pipeline

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gen3D can avoid wasted LLM calls and make repeated parts more consistent by letting the PLAN explicitly declare which components (or limb subtrees) are repeated, and by having the engine automatically:

1) generate only the unique components + the declared reuse “source” components, and then
2) fill the remaining repeated components via copy tools.

This is user-visible as:

- Faster Gen3D builds for prompts with many repeated parts (radial legs, mirrored wheels, etc.).
- More consistent repeated parts (because most of them come from deterministic copies).
- Fewer “agent forgot to use copy” failures, because the engine applies the plan’s reuse instructions deterministically.

We also harden the agent loop so it does not mix “plan generation” and “component generation” in a single agent step (which previously caused the agent to pick a generation strategy before it could observe reuse opportunities).

Finally, we run a real rendered Gen3D regression with a prompt that stresses radial legs + animation, save the prefab into the world, and use Automation to move/fire while capturing screenshots.

## Progress

- [x] (2026-02-10) Write and check in this ExecPlan.
- [x] (2026-02-10) Extend plan schema with `reuse_groups` and document it in prompts/tooling docs.
- [x] (2026-02-10) Persist validated reuse groups into job state and expose them in `get_state_summary_v1`.
- [x] (2026-02-10) Enforce “plan step only” in the Gen3D agent loop (prompt rule + engine guard).
- [x] (2026-02-10) Implement reuse-aware batch generation and auto-copy based on `reuse_groups`.
- [x] (2026-02-10) Add unit tests for reuse plan validation + generation scheduling + copy application.
- [x] (2026-02-10) Update `README.md` to mention plan-level reuse + auto-copy.
- [x] (2026-02-10) Run `cargo test`, run a headless smoke start, and commit.
- [x] (2026-02-10) Run a real rendered Gen3D test (radial legs prompt) via `tools/gen3d_real_test.py`, save to scene, move + fire + capture screenshots, and record the run id + any issues.

## Surprises & Discoveries

- Observation: Some providers return SSE text for `/responses` even when we do not request streaming.
  Evidence: Older real test runs failed with “`/responses returned no output text`” and fell back to `/chat/completions` (often 504). Fix implemented in `src/gen3d/ai/openai.rs` to extract SSE `response.output_text.delta` and accept `"type":"text"` parts.

- Observation: Plan tool outputs are often “nearly-correct” but still require repair (and repairs can become very large).
  Evidence: This real test run needed 2 repair attempts for `llm_generate_plan_v1` and produced multi-MB `/responses` artifacts under `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/attempt_0/pass_0/`.

- Observation: Engine-side `auto_copy` works, but agents may still redundantly call copy tools.
  Evidence: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/attempt_0/pass_1/auto_copy.json` shows deterministic copies applied after batch generation, and the same pass also contains explicit copy tool calls in `tool_calls.jsonl`.

## Decision Log

- Decision: Represent plan reuse as explicit `reuse_groups` rather than deriving it from component name patterns.
  Rationale: Name-based grouping is heuristic. An explicit plan field is deterministic and works for any object as long as the plan author (the LLM) marks repeats.
  Date/Author: 2026-02-10 / Codex

- Decision: Apply reuse groups in the engine after batch component generation instead of relying on the agent to remember to call copy tools.
  Rationale: The copy operations are deterministic and based on the plan; engine-side application eliminates an entire class of “agent skipped reuse” regressions and reduces token spend.
  Date/Author: 2026-02-10 / Codex

- Decision: Keep copy semantics simple: default `anchors=preserve_target` and `mode=detached` for plan reuse.
  Rationale: `preserve_target` keeps mount/join frames stable (important for radial limbs). `detached` works for both leaf and non-leaf components and matches subtree copy’s current capabilities.
  Date/Author: 2026-02-10 / Codex

## Outcomes & Retrospective

- `reuse_groups` is now a plan-level, non-heuristic way to declare repeated parts for deterministic reuse.
- Reuse-aware missing-only batching + engine-side auto-copy reduces LLM calls and keeps repeated limbs consistent.
- Real rendered run executed end-to-end (Build → Save → Move → Fire → screenshots) with:
  - run_id `f685579e-25b4-47cf-948a-42f9e09a5c8c` under `tests/gen3d/cache/gen3d/`
  - `auto_copy.json` applied 7 subtree copies / 14 component copies
  - leg roots distributed evenly at ~45° increments (verified via `assembly_transforms.json`)
  - smoke + motion validation OK (`smoke_results.json`)
- Note: the driver used `--build-timeout-secs 1800` and the run finished “best effort” (`build_complete=false` but `draft_ready=true`) while still saving successfully and capturing move/fire sequences.

## Context and Orientation

Gen3D “agent mode” is an internal loop where:

- `llm_generate_plan_v1` produces an assembly plan (components + anchors + attachments + animations).
- `llm_generate_components_v1` produces component geometry drafts.
- Copy tools (`copy_component_v1`, `copy_component_subtree_v1`) deterministically reuse geometry for repeated parts.

Key code locations:

- `src/gen3d/ai/schema.rs`: serde JSON structs for plan/draft/review.
- `src/gen3d/ai/prompts.rs`: the system prompts that define the JSON schema and rules for plan/component/review generation.
- `src/gen3d/ai/mod.rs`: legacy Gen3D flow that applies the plan into `job.planned_components` and builds initial stub `ObjectDef`s.
- `src/gen3d/ai/agent_loop/mod.rs` + `src/gen3d/ai/agent_*.rs`: Gen3D agent loop wiring, tool handling, state summary, and copy tool implementations.
- `src/gen3d/ai/copy_component.rs`: engine-side copy logic for components and subtrees.
- `tools/gen3d_real_test.py`: end-to-end rendered Gen3D driver via Automation HTTP API.
- `src/automation/mod.rs`: Automation endpoints used by the real test (`/v1/mode`, `/v1/gen3d/*`, `/v1/select`, `/v1/move`, `/v1/fire`, `/v1/step`, `/v1/screenshot`).

Terminology (plain language):

- “Component”: a named sub-part of a prefab (e.g., `body`, `leg_0_hip`, `wheel_left`).
- “Attachment”: how a child component mounts to a parent component (parent anchor + child anchor + an offset transform).
- “Reuse group”: a plan-declared instruction that a set of target components (or target subtrees) should inherit geometry from a source component/subtree via copy tools.
- “Auto-copy”: the engine applies reuse groups after generating sources, filling target geometry deterministically.

## Plan of Work

First, extend the plan JSON schema:

- In `src/gen3d/ai/schema.rs`, add an optional top-level array field `reuse_groups`.
- Each entry declares:
  - `kind`: `component` or `subtree`
  - `source`: the component name to copy from (root for subtree copy)
  - `targets`: list of target component names (roots for subtree copy)
  - Optional `anchors` (`preserve_target` / `copy_source`) and `mode` (`detached` / `linked`), with safe defaults.

Second, update plan generation instructions:

- In `src/gen3d/ai/prompts.rs` `build_gen3d_plan_system_instructions`, document `reuse_groups` in the schema and explicitly instruct the model to fill it for repeated parts (numbered sets like `leg_0..leg_7`, mirrored parts, radial limbs).
- Provide a short example in prose so the model reliably emits the field.

Third, persist and validate reuse groups:

- In `src/gen3d/ai/mod.rs` where the plan is applied, store the parsed reuse groups into the job state (new field on `Gen3dAiJob`).
- Validate groups against the current plan’s component names:
  - drop unknown/empty names
  - dedupe targets
  - forbid `source` in `targets`
  - ignore unknown kinds
  - produce human-readable warnings stored in job state for debugging

Fourth, expose reuse plan to the agent:

- In `src/gen3d/ai/agent_prompt.rs` `draft_summary`, include:
  - `reuse_groups` (validated, with defaults applied)
  - `reuse_warnings` (if any)
  - `reuse_generation_plan`: component indices/names to generate first, and ready-to-use copy tool calls that will be applied after generation.

Fifth, enforce a “plan-only” agent step:

- Update `build_agent_system_instructions` (in `src/gen3d/ai/agent_prompt.rs`) to tell the agent not to include `llm_generate_components_v1` in the same step as `llm_generate_plan_v1`.
- Add an engine guard in `execute_agent_actions` (in `src/gen3d/ai/agent_step.rs`): after a successful `llm_generate_plan_v1` tool call, end the step immediately (request the next step) even if the agent included additional actions.

Sixth, implement reuse-aware batch generation + auto-copy:

- In `src/gen3d/ai/agent_tool_dispatch.rs` tool handler for `llm_generate_components_v1` (and its polling/completion path):
  - When the call does not specify explicit `component_indices`/`component_names` and `job` has validated `reuse_groups`, generate only:
    - all missing components that are NOT declared as copy targets (including full target subtrees for subtree reuse), plus
    - all source components/subtrees required by reuse groups.
  - After the batch generation finishes, apply copy operations for all reuse groups to fill missing targets.
  - Never overwrite already-generated targets (only fill targets with `actual_size==None`).
  - Record a compact summary (generated indices, copy count) in the tool result JSON and run log.

Seventh, add unit tests:

- Add a small pure helper that converts reuse groups into:
  - a set of “skip generating these targets” indices
  - a set of “must generate these sources” indices
  - a stable copy execution order
- Unit test that:
  - For a plan with `leg_0..leg_7` and a reuse group from `leg_0` to the others, the generation schedule includes only `leg_0` (and other uniques) and excludes `leg_1..leg_7`.
  - Auto-copy fills `leg_1..leg_7` after `leg_0` is generated.

Eighth, update docs:

- Update `README.md` with a one-line mention that plans can declare reuse groups and that Gen3D will auto-copy repeated parts.

Ninth, run validation and real rendered regression:

- Run `cargo test`.
- Run `cargo run -- --headless --headless-seconds 3` (smoke start, crash-free).
- Build the debug binary (if needed) and run:

    python3 tools/gen3d_real_test.py --config ~/.gravimera/config.toml --prompt "Voxel/pixel-art style octopus robot with 8 evenly spaced radial legs, elevated body, top cannon. Legs are a repeated chain; keep move animation consistent across legs."

- In the produced `run_dir` (printed by the script), inspect:
  - `agent_trace.jsonl` to confirm copy tools were used (or auto-copy ran) and that the batch did not generate all leg components separately.
  - `external_screenshots_*` images (and optional mp4s) to confirm:
    - legs are evenly radial
    - feet contact the ground as expected
    - move animation looks consistent and does not exhibit flipped phases or broken joints

## Concrete Steps

All commands should be run from the repo root.

1) Edit + format:

    cargo fmt

2) Unit tests:

    cargo test

3) Headless smoke start (crash check):

    cargo run -- --headless --headless-seconds 3

4) Real rendered Gen3D regression (requires a working GPU/renderer and a valid OpenAI config):

    python3 tools/gen3d_real_test.py --config ~/.gravimera/config.toml --prompt "An octopus robot with 8 distributed radial legs. A cannon on top. Legs hold the body up high. Voxel/pixel-art style. Use repeated leg geometry."

Expected: the script prints `OK: run_id=... run_dir=... instance_id_uuid=...` and the run dir contains screenshots and agent traces.

## Validation and Acceptance

Acceptance is satisfied when:

- `cargo test` passes.
- The headless smoke start runs and exits cleanly.
- The real rendered Gen3D test completes, saves the prefab into the world, and captures screenshots.
- The run’s `agent_trace.jsonl` shows that repeated components were filled via reuse/copy (either explicit copy tool calls or engine auto-copy), and that we did not spend LLM calls generating every repeated leg component independently.
- No obvious animation regressions are visible in captured screenshots/MP4s (legs animate coherently; no 180° flips caused by anchor frame mistakes; stance/ground contact is reasonable).

## Idempotence and Recovery

- Rerunning the real test is safe; it creates a new Gen3D run directory each time.
- If a run hangs, use Automation `/v1/shutdown` (the python script already attempts this on exit).
- If rendered mode fails and the app falls back to headless mode, the rendered Gen3D test cannot run; resolve GPU/renderer availability first (see `src/app.rs` `render_preflight`).

## Artifacts and Notes

Record run ids and key findings here once the regression is executed:

- Run: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/`
  - Reuse: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/attempt_0/pass_1/auto_copy.json`
  - Render previews: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/attempt_0/pass_1/render_front.png`
  - World screenshots: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/external_screenshots_world/spawn.png`
  - Animation strips: `tests/gen3d/cache/gen3d/f685579e-25b4-47cf-948a-42f9e09a5c8c/external_screenshots_anim/move_anim.mp4`
