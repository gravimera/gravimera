# Gen3D motion improvement loop (trace + revise, no screenshots)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D can already author motion clips via LLM tools (`llm_generate_motion_v1` / `llm_generate_motions_v1`), but the first draft can be “unnatural” (abrupt, jittery, implausible) or simply not aligned with the user’s intent (“wave” ends up looking like a punch). Today, improving these motions typically relies on image-based review (renders / sprite sheets) or on the QA path (`qa_v1` + motion_validation issues). Both are the wrong fit for an “AI self-improves motion” loop:

- Images are large and consume many tokens.
- QA is safety/validity-oriented; it is not a general “make it feel natural / match intent” refinement loop.

After this change, Gen3D gains a screenshot-free motion refinement mechanism:

1. A new deterministic read-only tool, `motion_trace_v1`, samples the current draft animation for a specific channel and returns a compact numeric summary of what moved, how much, and where the worst moments are (bounded; no per-frame dumps by default).
2. When Gen3D authors a motion channel, it is allowed up to **two** improvement rounds per pass:
   - Trace → (optional) revise the same channel → trace → (optional) revise the same channel.
   - The engine enforces the budget so this cannot loop forever.
3. Revisions are done by the existing LLM motion authoring tool (`llm_generate_motion_v1`) using the trace summary as its “self-observation” signal. No screenshots are required.

How a human can see it working after implementation:

- Run a Gen3D build for a movable unit with motion (or request a custom motion channel like `dance`).
- Observe in the run artifacts (under `~/.gravimera/cache/gen3d/<run_id>/attempt_<n>/pass_<m>/`) that the engine wrote:
  - `motion_trace_<channel>_round0.json` (baseline trace),
  - `motion_trace_<channel>_round1.json` (post-first-revise trace, if a revise occurred),
  - `motion_trace_<channel>_final.json` (final trace for reporting),
  - and motion authoring artifacts already used today (`motion_<channel>_raw.txt`, etc.).
- Observe in status text and/or tool results that motion refinement ran at most twice per pass, then stopped deterministically.


## Progress

- [x] (2026-03-24 22:10 CST) Drafted this ExecPlan (design only; no code changes yet).
- [ ] Add deterministic motion inspection tool(s): `motion_trace_v1` (required) and `motion_probe_v1` (optional but recommended).
- [ ] Add a motion-authoring “no change” decision so the model can stop refining without forcing a rewrite.
- [ ] Implement the 2-round trace→revise improvement loop in the deterministic pipeline orchestrator (and ensure agent-step mode is compatible).
- [ ] Add offline/unit tests (no network) and a rendered smoke test run.
- [ ] Update docs under `docs/gen3d/` to describe the new loop and tool contracts.
- [ ] Commit changes with clear messages.


## Surprises & Discoveries

- Observation: There is already a deterministic, screenshot-free motion measurement tool (`motion_metrics_v1`), but it is focused on locomotion/contact metrics (stride, slip, lift) and is not a general “any channel” motion introspection API.
  Evidence: `src/gen3d/ai/motion_validation.rs::build_motion_metrics_report_v1` output shape centers on ground contacts and cycle distance.

- Observation: The current LLM motion authoring contract requires `decision=author_clips` to emit a non-empty `edges` list; there is no “no-op/no-change” decision. This makes it impossible to run a bounded “revise if needed” loop without either (a) always rewriting the channel, or (b) adding a separate LLM “should revise?” tool.
  Evidence: `src/gen3d/ai/schema.rs::AiMotionAuthoringDecisionJsonV1` lacks `no_change`; `src/gen3d/ai/agent_tool_poll.rs` rejects empty `edges` for `author_clips`.

- Observation: The runtime motion sampler used by motion validation chooses from a fixed priority set of channels (`attack_primary`, `action`, `move`, `idle`, `ambient`, `__base`). Custom channels are not first-class in that selection logic. A trace tool must therefore be “channel-targeted” (apply channel X when present, else fall back) rather than relying on the runtime’s fixed channel priority logic.
  Evidence: `src/gen3d/ai/motion_validation.rs::compute_world_transforms_for_channels` hardcodes the channel order and does not accept an arbitrary channel name.


## Decision Log

- Decision: Add a deterministic numeric motion introspection tool (`motion_trace_v1`) instead of screenshot-based review for this refinement loop.
  Rationale: Numeric traces are small (token-efficient), deterministic, and can describe “what moved / when” for any motion channel without requiring images.
  Date/Author: 2026-03-24 / codex

- Decision: Do not introduce new rig “contracts” (new required joint limits / human-specific constraints) as part of this work.
  Rationale: Users can request any object and any motion; adding human-like constraints would reduce creative freedom. The tracing tools will report generic kinematics and will only use any joint limits/axes if they already exist in the plan.
  Date/Author: 2026-03-24 / codex

- Decision: Bound refinement by enforcing a deterministic per-pass budget: at most 2 trace→revise rounds per channel per pass.
  Rationale: Prevent infinite loops and align with existing Gen3D “two chances” style budgets (for example, limited review-delta rounds).
  Date/Author: 2026-03-24 / codex

- Decision: Extend the motion authoring structured output with a new decision `no_change` so the model can choose to stop refining without rewriting the channel.
  Rationale: This avoids adding a second LLM “should revise?” tool and avoids forcing unnecessary rewrites that can degrade a good motion.
  Date/Author: 2026-03-24 / codex


## Outcomes & Retrospective

- (Pending) Record: how often the loop stops after 0/1/2 rounds in real prompts, typical token cost increase, and any failure patterns (for example: trace output truncation, inability to localize problems without probe).


## Context and Orientation

This section describes how motion is represented and where to implement the change, as if the reader is new to this repository.

### Gen3D motion model (what “motion” means here)

In Gen3D, an object is a set of planned components connected by attachment edges. Each attachment edge can have zero or more animation “slots”. A slot is:

- A `channel` string (examples: `move`, `action`, `idle`, `attack_primary`, or any user-defined string).
- A `driver` that defines the time domain (“what advances time”):
  - `always`: wall-clock seconds,
  - `move_phase` / `move_distance`: movement-driven units,
  - `attack_time` / `action_time`: event-driven seconds since an action started.
- A clip definition (keyframed `loop`/`once`/`ping_pong`, or procedural `spin`).

At runtime (and in some validation code), a chosen slot’s delta transform is composed with the attachment offset and a constant basis:

  animated_offset(t) = attach_to.offset * slot.spec.basis * delta(t)

Key files:

- Tool registry (user-visible tool list): `src/gen3d/agent/tools.rs`
- Tool dispatch and polling (LLM-backed tools): `src/gen3d/ai/agent_tool_dispatch.rs`, `src/gen3d/ai/agent_tool_poll.rs`
- Motion validation and existing deterministic motion computations: `src/gen3d/ai/motion_validation.rs`
- Deterministic pipeline orchestrator (where budgets/looping live): `src/gen3d/ai/pipeline_orchestrator.rs`
- Gen3D docs: `docs/gen3d/README.md`

Artifacts:

- Gen3D run artifacts are written under `~/.gravimera/cache/gen3d/<run_id>/attempt_<n>/pass_<m>/...`.
- The pipeline/agent already writes motion authoring artifacts like `motion_<channel>_raw.txt`.
  This plan adds trace artifacts like `motion_trace_<channel>_round0.json`.

Terminology used in this plan:

- “attempt”: one overall run attempt within a Gen3D run id (retries can create `attempt_1`, `attempt_2`, etc.).
- “pass”: one step in the pipeline/agent loop within an attempt. Pass directories are used to store artifacts for that step.
- “revise”: an LLM motion authoring call that replaces the specified channel on targeted edges.
- “trace”: a deterministic measurement over sampled times for one channel.


## Plan of Work

### 1) Add a deterministic motion trace tool: `motion_trace_v1`

Add a new tool id and descriptor entry in `src/gen3d/agent/tools.rs`.

Tool intent: provide a compact, bounded, screenshot-free “what happened in this animation channel?” report that the LLM can use to refine its own output.

The tool must be:

- Read-only (no draft mutation).
- Deterministic (given the same planned components and the same args, produce the same JSON).
- Bounded (no unbounded per-frame dumps into the tool result; write large arrays to artifacts instead).
- Channel-agnostic (works for any channel string).

Suggested args (strict; unknown keys rejected):

  { version?: 1, channel: string, sample_count?: number, top_k?: number, scope_components?: string[], include?: { anchors?: string[] } }

Suggested result shape (example; adjust fields as implementation dictates, but keep it compact and stable):

  {
    "ok": true,
    "version": 1,
    "channel": "move",
    "rig_summary": {
      "sample_count": 24,
      "edges_with_channel": 6,
      "components_total": 12
    },
    "summary": {
      "top_movers_rotation": [
        { "child_component": "arm_lower_r", "rotation_range_deg": 92.3, "translation_range_m": 0.01 }
      ],
      "top_movers_translation": [
        { "child_component": "hand_r", "translation_range_m": 0.42, "rotation_range_deg": 18.1 }
      ],
      "worst_moments": [
        { "kind": "peak_angular_accel", "child_component": "arm_lower_r", "phase_01": 0.37, "value": 812.0, "units": "deg_per_unit2" }
      ]
    },
    "by_edge_top": [
      {
        "child_component": "arm_lower_r",
        "parent_component": "arm_upper_r",
        "channel_slots": 1,
        "driver": "always",
        "clip_kind": "loop",
        "duration_units": 1.0,
        "metrics": {
          "rotation_range_deg": 92.3,
          "translation_range_m": 0.01,
          "closure_error_rotation_deg": 3.2,
          "closure_error_translation_m": 0.002,
          "peak_angular_speed_deg_per_unit": 180.0,
          "peak_angular_accel_deg_per_unit2": 812.0
        }
      }
    ],
    "artifacts": {
      "trace_json": "motion_trace_move_round0.json"
    }
  }

Important design constraints (to keep this generic and not heuristic):

- The tool should not label things as “unnatural” or “impossible” by comparing against human-specific thresholds.
- It may report constraint violations only when explicit limits exist in the current planned joint (for example, hinge limits declared on that edge).
- For all other cases, report raw kinematics (ranges, peaks, closure error) and let the LLM interpret in the context of the user prompt.

Implementation notes (Rust):

- Prefer implementing the core sampling/metrics code in a new module (for example `src/gen3d/ai/motion_trace.rs`) and call it from tool dispatch. Reuse helper functions from `src/gen3d/ai/motion_validation.rs` when possible, but avoid entangling “QA issue classification” with “trace reporting”.
- Sampling should be “phase-based” so it works with any driver. A simple generic convention is to sample `phase_01` in `[0, 1)` and map it to each slot’s local time units (`t_units`):
  - For keyframed clips: `t_units = phase_01 * duration_units` (for `loop` / `once` / `ping_pong`).
  - For `spin`: treat the sampled unit window as `t_units = phase_01 * 1.0` (and report that convention explicitly in the output).
- The tool must be bounded:
  - Clamp `sample_count` (for example 8..256).
  - Clamp `top_k` (for example 1..16).
  - In the JSON tool result, include only “top edges” (for example top 12 by a generic magnitude score) plus `worst_moments` (top_k).
  - Write the full per-sample series (if needed) into the pass artifact directory, and return only the artifact filename.

### 2) (Optional but recommended) Add a “deep dive at one moment” tool: `motion_probe_v1`

`motion_trace_v1` is intentionally compact. When the LLM needs to localize a problem, it should be able to query a single timestamp/phase and get detailed transforms for a few components without screenshots.

Add `motion_probe_v1` as a read-only deterministic tool that returns (bounded) transforms at a requested `phase_01` for a specific `channel` and a scoped component list.

Suggested args:

  { version?: 1, channel: string, phase_01: number, components: string[], include?: { anchors?: string[] } }

Suggested output:

- For each requested component:
  - local/join/world transform (whichever frames are cheap and stable to provide),
  - requested anchor positions if `include.anchors` is set.

This tool must also be bounded and must not dump all components by default.

### 3) Add a “no change” decision to motion authoring structured output

To make the improvement loop safe and budgeted, the model needs a way to say “this channel is good enough; stop refining” without being forced to emit a rewritten non-empty `edges` payload.

Make a backward-incompatible (allowed per repo policy) schema extension:

- In `src/gen3d/ai/schema.rs`:
  - Extend `AiMotionAuthoringDecisionJsonV1` with a new variant `NoChange` serialized as `no_change`.
- In `src/gen3d/ai/structured_outputs.rs`:
  - Update the structured output schema for motion authoring to allow `decision=no_change`.
- In `src/gen3d/ai/agent_tool_poll.rs`:
  - Accept `decision=no_change` as a successful result when:
    - `replace_channels=[]` and `edges=[]`,
    - `reason` is non-empty (brief justification, for debugging),
    - and `applies_to` still matches the job.
  - Ensure tool results and artifacts still make it clear that a “no change” decision happened (so debugging is possible).

This change is intentionally specific: `no_change` is only for “I decline to mutate anything”.

### 4) Implement the 2-round trace→revise improvement loop (pipeline-first)

The improvement loop should live in deterministic code (pipeline orchestrator) so budgets are enforced regardless of model behavior.

High-level behavior per channel in a single pass:

1. Run baseline trace: `motion_trace_v1` (write `motion_trace_<channel>_round0.json`).
2. Improvement round 1 (budgeted):
   - Call `llm_generate_motion_v1` for the same channel, but include the trace summary in the prompt as “self-observation”.
   - If the result is `decision=no_change`, stop early for this channel.
   - If the result authors clips, apply them (existing behavior), then continue.
3. Run post-round trace: `motion_trace_v1` (write `motion_trace_<channel>_round1.json`).
4. Improvement round 2 (budgeted, identical to round 1).
5. Run one final trace-only report (`motion_trace_<channel>_final.json`) for artifacts/debugging. This final trace must not trigger another revise, so it does not create an infinite loop.

Budget rule:

- The orchestrator must enforce: at most 2 revise rounds per channel per pass.
- When the budget is exhausted, record an Info Store event of kind `budget_stop` explaining which channel hit the limit.

Prompting (LLM motion authoring “improve mode”):

- Update `src/gen3d/ai/prompts.rs` motion authoring prompt builder to include:
  - The target `channel`.
  - A compact summary of existing slots for that channel (which edges currently have it, clip kinds, drivers).
  - The latest `motion_trace_v1` summary (top movers + worst moments + by-edge-top metrics).
  - Clear instructions:
    - If the motion already matches the user’s intent and no obvious problems are visible in the trace, return `decision=no_change` with a short reason.
    - Otherwise, author a revised motion for exactly the requested channel (existing single-channel contract).
    - Do not introduce new channels, do not modify geometry, and do not depend on screenshots.

Note: This is not the QA path. The revise decision is made by the LLM using trace numbers, not by `qa_v1`.

### 5) Ensure agent-step mode remains compatible (no infinite loops)

Agent-step fallback should not be able to accidentally loop motion refinement forever.

Even if the agent prompt is updated to encourage using the improvement loop, the hard guard must be in engine code. The simplest strategy is:

- When in agent mode, still track the same per-pass per-channel refine counters.
- If the agent tries to trigger a third refine round, return a tool error that is actionable (“budget exhausted; proceed without further refinement this pass”) and record a `budget_stop` Info Store event.

### 6) Tests (offline + deterministic)

Add tests that do not require network:

- Unit tests for `motion_trace_v1` output stability and bounding:
  - Construct a small planned-component graph with one attachment and one `loop` clip.
  - Assert that `motion_trace_v1` returns a stable shape and includes expected keys, and that it clamps `sample_count` and `top_k`.
- Unit tests for `decision=no_change` parsing and acceptance:
  - Ensure `agent_tool_poll` accepts `no_change` with empty edges and reject malformed variants.
- Integration-like tests for the pipeline budget:
  - Use the existing mock backend (`mock://gen3d`) to force motion authoring to “always revise” for two rounds.
  - Verify the third attempt is blocked and yields a deterministic `budget_stop` event and no infinite loop.

If any new test assets/config files are needed, store them under a dedicated folder like `test/run_motion_improve_1/` in keeping with repo practice (do not scatter configs in tmp dirs).

### 7) Docs updates

Update `docs/gen3d/README.md` to include:

- The new tool(s) (`motion_trace_v1`, `motion_probe_v1`) and their purpose (“no screenshots; numeric self-observation”).
- The refinement loop behavior and the 2-round budget.
- Artifact filenames produced per pass.

Keep `README.md` at the repo root clean; put the details under `docs/`.


## Concrete Steps

All commands run from the repo root (`/Users/flow/workspace/github/gravimera`).

1. Implement tool(s), schema updates, and pipeline loop per the Plan of Work.

2. Run formatting and tests:

   cargo fmt
   cargo test

3. Run the rendered smoke test (must be rendered; do not use `--headless`):

   tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4. (Manual) Verify motion improvement loop produces artifacts:

   - Start a Gen3D build for a movable unit.
   - Ensure motion authoring triggers (missing move/action, or request a custom channel).
   - Inspect the latest pass directory for `motion_trace_<channel>_*.json` files.


## Validation and Acceptance

This change is accepted when all of the following are true:

1. The new tool(s) are available to Gen3D agent mode (listed in `get_tool_list_v1`) and return deterministic, bounded JSON without screenshots.
2. In pipeline mode, after motion authoring for a channel, the engine runs at most two trace→revise rounds per pass and then stops deterministically (even if the model would otherwise keep revising).
3. `decision=no_change` for motion authoring is supported and prevents unnecessary rewrites.
4. `cargo test` passes.
5. The rendered smoke test starts and exits cleanly.


## Idempotence and Recovery

- `motion_trace_v1` / `motion_probe_v1` are read-only and safe to re-run.
- The improvement loop is bounded. If refinement makes motion worse, re-running the same pass is not expected to be deterministic because LLM output may differ; however:
  - The run artifacts will still contain per-round traces and raw model outputs so regressions can be debugged.
  - The per-pass budget prevents “LLM thrash” from continuing indefinitely.


## Artifacts and Notes

Planned artifact filenames (per pass directory):

- `motion_trace_<channel>_round0.json`
- `motion_trace_<channel>_round1.json` (if a revise occurred)
- `motion_trace_<channel>_final.json`
- Existing motion tool artifacts:
  - `motion_<channel>_raw.txt`
  - Any existing parsed JSON artifacts for motion authoring

If the trace tool writes a larger “full series” file, keep it clearly named and bounded, for example:

- `motion_trace_<channel>_series.json` (not referenced by the agent prompt by default)


## Interfaces and Dependencies

At end state, the following interfaces must exist and be used:

### New Gen3D tools (read-only)

- Tool id: `motion_trace_v1`
  - Args: `{ version?: 1, channel: string, sample_count?: number, top_k?: number, scope_components?: string[], include?: { anchors?: string[] } }`
  - Result: a bounded JSON report with summary + top edges + worst moments; writes an artifact in the pass dir.

- Tool id: `motion_probe_v1` (optional)
  - Args: `{ version?: 1, channel: string, phase_01: number, components: string[], include?: { anchors?: string[] } }`
  - Result: bounded per-component transforms at one sampled moment; writes an artifact in the pass dir.

### Motion authoring structured output

- Extend `AiMotionAuthoringDecisionJsonV1` with `no_change`.
- Accept `no_change` as success when it does not mutate state (`edges=[]`, `replace_channels=[]`) and has a non-empty reason.

### Pipeline budgets

- Add a per-pass, per-channel counter (stored in job state) that enforces at most 2 refine rounds (trace→revise) before stopping with an explicit `budget_stop` Info Store event.

