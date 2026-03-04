# Gen3D: Motion-authoring fallback (no “unit with zero animation”)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D’s current default is to generate static geometry and then rely on **runtime motion algorithms** (driven by `interfaces.extra.motion_rig_v1`) for animation. This is great when the generated unit can be mapped into an existing rig kind (biped/quadruped/car/airplane), but when that mapping fails, the unit can end up with **no animation at all**.

After this change, Gen3D should never finish a **movable** unit (mobility = ground/air) in a “no animation” state. When runtime motion mapping is unavailable (or when the prompt implies a non-default/stylized/custom motion), Gen3D automatically asks the LLM for an explicit, structured **motion authoring spec**, then deterministically “bakes” it into `PartAnimationSlot`s on the correct attachment edges (no engine heuristics).

User-visible outcome:

- When a prompt produces a movable creature/unit that does not match an existing runtime rig (snake/octopus/mantis/etc), the saved unit still animates (idle/move and attack when applicable).
- Users do not need new UI controls; the only control surface is the prompt.
- The engine never guesses structure (no name/shape heuristics). The LLM must explicitly name which attachment edges and channels to animate, and the engine only validates + applies.

## Progress

- [x] (2026-03-05 00:10Z) Write this ExecPlan and audit current Gen3D motion + agent loop codepaths.
- [x] (2026-03-05 01:20Z) Add `gen3d_motion_authoring_v1` schema + strict parsing + unit tests.
- [x] (2026-03-05 01:35Z) Add `llm_generate_motion_authoring_v1` tool (registry + dispatch + poll + schema repair).
- [x] (2026-03-05 01:45Z) Implement deterministic baker that writes authored `PartAnimationSlot`s onto planned attachment edges and syncs into `draft.defs`.
- [x] (2026-03-05 01:50Z) Add agent “done” guard: movable unit must have a runtime rig candidate OR authored `move` slots (prevents “unit with zero animation”).
- [x] (2026-03-05 01:55Z) Update `mock://gen3d` backend + mock agent step to cover motion roles + motion authoring tools.
- [x] (2026-03-05 02:00Z) Run `cargo test`.
- [x] (2026-03-05 02:37Z) Run required rendered smoke test.
- [x] (2026-03-05 02:38Z) Run HTTP Gen3D prompts (snake/octopus/mantis) and capture artifacts/transcripts.
- [x] (2026-03-05 02:58Z) Commit.

## Surprises & Discoveries

- Observation: Gen3D plan JSON is static-only (no `attach_to.animations`), but the internal planned attachment type still contains `animations: Vec<PartAnimationSlot>`, and the assembler already copies them into the `ObjectRef` parts in `draft.defs`.
  Evidence: `src/gen3d/ai/job.rs` (`Gen3dPlannedAttachment.animations`) and `src/gen3d/ai/convert.rs` (copies `att.animations` when building `ObjectPartDef::object_ref` parts).

- Observation: The agent “done” gate enforces QA tools and motion validation, but does not reject the case “movable unit has no move/idle animation because no runtime rig was produced.”
  Evidence: `src/gen3d/ai/agent_step.rs` checks `ever_*` QA flags and `last_motion_ok`, but nothing about “animation coverage”.

## Decision Log

- Decision: Add a new Gen3D agent tool `llm_generate_motion_authoring_v1` that produces a strictly-typed JSON spec describing either (a) runtime motion is sufficient, or (b) explicit per-edge animation clips should be authored.
  Rationale: Users cannot toggle UI switches; the prompt must be enough. The engine must not interpret adjectives or infer rig structure; the LLM returns an explicit decision + data and the engine only compiles/validates/applies.
  Date/Author: 2026-03-05 / Codex + user

- Decision: The motion authoring spec targets **attachment edges** by naming the CHILD planned component (`component`), and the engine applies the clips to that component’s `attach_to` edge only.
  Rationale: Edges are the only stable, name-addressable animation targets in current Gen3D. Targeting primitives would require stable part ids, which Gen3D does not currently emit.
  Date/Author: 2026-03-05 / Codex

- Decision: The “baker” is deterministic and contains no inference. It only:
  - validates finiteness and schema constraints,
  - merges/replaces channels explicitly requested by the spec,
  - writes `PartAnimationSlot`s into `Gen3dPlannedAttachment.animations`,
  - and calls existing sync helpers to update `draft.defs`.
  Rationale: Repo rule: Gen3D must support “any object” and cannot rely on heuristics. The LLM provides semantic intent; the engine applies it without guessing.
  Date/Author: 2026-03-05 / Codex + user

- Decision: Add a “done guard” for movable units: if neither a runtime rig is available nor authored clips exist, ignore `done` and force the agent to call `llm_generate_motion_authoring_v1` (and re-run smoke_check).
  Rationale: Prevent shipping units with no animation; keep behavior automatic.
  Date/Author: 2026-03-05 / Codex + user

## Outcomes & Retrospective

- (TBD) Once implemented, record how often motion authoring triggers for “weird” prompts and whether it reduces “static unit” failures without increasing regen loops.

## Context and Orientation

Key concepts:

- Attachment edge: a planned component (child) attaches to a parent via named anchors. In prefab defs this becomes an `ObjectRef` part with an `AttachmentDef`.
- Per-edge animation: `ObjectRef` parts can carry `PartAnimationSlot`s; each slot is keyed by a channel (`idle`/`move`/`attack_primary`/`ambient`) and evaluated by `src/object/visuals.rs`.
- Runtime motion algorithms: engine-injected animations derived from `interfaces.extra.motion_rig_v1` in prefab descriptors, implemented in `src/motion.rs`. These only apply when the descriptor declares a compatible rig.

Relevant files:

- `src/gen3d/agent/tools.rs`: Gen3D tool registry ids and descriptions.
- `src/gen3d/ai/structured_outputs.rs`: JSON schema definitions for strict structured outputs.
- `src/gen3d/ai/schema.rs`: Rust structs used for JSON parsing (`deny_unknown_fields`).
- `src/gen3d/ai/parse.rs`: text → JSON extraction and schema-typed parsing.
- `src/gen3d/ai/prompts.rs`: tool system/user prompt builders.
- `src/gen3d/ai/agent_tool_dispatch.rs`: starts async LLM tool calls.
- `src/gen3d/ai/agent_tool_poll.rs`: parses tool results and applies them to job/draft state.
- `src/gen3d/ai/job.rs`: job state (`planned_components`, `Gen3dPlannedAttachment.animations`, etc).
- `src/gen3d/ai/convert.rs`: syncs planned attachments into `draft.defs` (`sync_attachment_tree_to_defs`).
- `src/gen3d/ai/agent_step.rs`: agent run loop and “done” guardrails.
- `src/gen3d/ai/orchestration.rs`: `build_gen3d_smoke_results` (QA + motion_validation report).

What is currently missing:

- Real-world validation runs for “weird” creatures (snake/octopus/mantis) using the HTTP automation harness **with real OpenAI** (mock runs are recorded below in “Artifacts and Notes”).

## Plan of Work

### Milestone A — Add motion authoring schema + parsing

1) Define a new structured output kind `gen3d_motion_authoring_v1`.

In `src/gen3d/ai/structured_outputs.rs`:

- Add `Gen3dAiJsonSchemaKind::MotionAuthoringV1`.
- Add a new JSON schema function that covers:
  - `version` (1)
  - `applies_to` (run_id/attempt/plan_hash/assembly_rev)
  - `decision` enum: `runtime_ok` | `author_clips` | `regen_geometry_required`
  - `reason` string (brief)
  - `replace_channels` array of channel strings (which channels the baker should replace on targeted edges)
  - `edges` array describing authored clips per child component when `decision=author_clips`

2) Add Rust structs for parsing.

In `src/gen3d/ai/schema.rs`:

- Add `AiMotionAuthoringJsonV1` (deny unknown fields).
- Define minimal animation structs for the tool output (driver + clip + keyframes).
- Keep it strict and compact; reject unknown fields.

3) Add parser entrypoint.

In `src/gen3d/ai/parse.rs`:

- Add `parse_ai_motion_authoring_from_text`.
- Validate limits (max edges, max keyframes per clip, non-finite rejection, channel duplication).

### Milestone B — Add the LLM tool + prompts

1) Tool registration.

In `src/gen3d/agent/tools.rs`:

- Add `TOOL_ID_LLM_GENERATE_MOTION_AUTHORING = "llm_generate_motion_authoring_v1"`.
- Add it to `list()` and `describe()`.

2) Dispatch.

In `src/gen3d/ai/agent_tool_dispatch.rs`:

- Handle `TOOL_ID_LLM_GENERATE_MOTION_AUTHORING` similarly to motion_roles:
  - require components exist,
  - call `spawn_gen3d_ai_text_thread` with expected schema `MotionAuthoringV1`,
  - record artifacts under `tool_motion_authoring_<call_id>_*`.

3) Poll/apply.

In `src/gen3d/ai/agent_tool_poll.rs`:

- Add `Gen3dAgentLlmToolKind::GenerateMotionAuthoring`.
- On success:
  - parse `AiMotionAuthoringJsonV1`,
  - validate `applies_to`,
  - apply baked slots to `job.planned_components[*].attach_to.animations`,
  - call `sync_attachment_tree_to_defs(&job.planned_components, draft)` and bump `job.assembly_rev`.

4) Prompts.

In `src/gen3d/ai/prompts.rs`:

- Add `build_gen3d_motion_authoring_system_instructions()`.
- Add `build_gen3d_motion_authoring_user_text(...)` that includes:
  - effective prompt,
  - applies_to values,
  - mobility/attack summary,
  - attachment edges list (child name, parent name, anchor names, join-frame basis info),
  - current animation slot summary per edge (what exists already).

### Milestone C — Add “no animation” guardrails

1) State summary hints for the agent.

In `src/gen3d/ai/agent_prompt.rs` (`draft_summary`):

- Add a `motion_authoring` status section similar to `motion_roles` so the agent can see whether it already ran for this assembly_rev.
- Add a computed `motion_coverage` summary:
  - whether any `move` slot exists on any attachment edge
  - whether any `idle` slot exists

2) Agent “done” gate.

In `src/gen3d/ai/agent_step.rs`:

- When the agent requests `done`, and the draft is movable (root mobility present):
  - reject `done` if motion coverage is empty (no runtime rig available AND no authored slots).
  - set `workshop.error` with a concrete instruction to call `llm_generate_motion_authoring_v1`, then re-run `smoke_check_v1`.

### Milestone D — Tests and validation

1) Offline unit tests (mock://gen3d).

- Extend the `mock://gen3d` OpenAI backend in `src/gen3d/ai/openai.rs` to return a valid motion-authoring JSON when `artifact_prefix` matches the new tool.
- Update the mock agent step sequence (if needed) so it calls the new tool when required by the new “done” gate.

2) Real-world HTTP automation runs (manual, uses real OpenAI).

Use `test/gen3d_real/config.toml` and `tools/gen3d_real_test.py` to run prompts:

- snake: “A stylized snake unit that slithers; emphasize body wave motion.”
- octopus: “An octopus unit with tentacles that undulate; can move and attack.”
- mantis: “A praying mantis unit that walks trembling and majestic; can attack with scythe arms.”

For each, verify:

- Build completes (`build_complete=true`).
- Saved unit exists and shows non-trivial part motion in idle/move (not only whole-body translation).
- Smoke check reports ok.

## Concrete Steps

All commands run from repo root (`/Users/flow/workspace/github/gravimera`).

1) Unit tests:

    cargo test

2) Required rendered smoke test (must be rendered, not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3) Real Gen3D HTTP tests (requires OpenAI key via env and Automation enabled config):

    python3 tools/gen3d_real_test.py --config test/gen3d_real/config.toml --prompt "A stylized snake unit that slithers; emphasize body wave motion."
    python3 tools/gen3d_real_test.py --config test/gen3d_real/config.toml --prompt "An octopus unit with tentacles that undulate; can move and attack."
    python3 tools/gen3d_real_test.py --config test/gen3d_real/config.toml --prompt "A praying mantis unit that walks trembling and majestic; can attack with scythe arms."

4) If you do NOT have `OPENAI_API_KEY`, run the same HTTP tests with the mock backend (debug builds only):

    python3 tools/gen3d_real_test.py --config test/gen3d_real/config_mock.toml --prompt "A stylized snake unit that slithers; emphasize body wave motion."
    python3 tools/gen3d_real_test.py --config test/gen3d_real/config_mock.toml --prompt "An octopus unit with tentacles that undulate; can move and attack."
    python3 tools/gen3d_real_test.py --config test/gen3d_real/config_mock.toml --prompt "A praying mantis unit that walks trembling and majestic; can attack with scythe arms."

## Validation and Acceptance

Acceptance criteria:

- A Gen3D run that produces a movable unit must not finish with zero animation (no authored slots and no runtime rig path). The agent should be forced to generate motion authoring data until it has a valid animation path.
- The HTTP automation prompts above complete without a crash and produce saved units that visibly animate in `move` and `idle` (and `attack_primary` when attack is present).

## Idempotence and Recovery

- The new tool is safe to re-run: it only replaces channels listed in `replace_channels` on targeted edges.
- If motion authoring produces invalid JSON, tool schema repair is used (consistent with other tools).
- If the LLM requests geometry regeneration (`regen_geometry_required`), the agent must re-plan/regenerate components; this is budget-gated by existing regen budgets.

## Artifacts and Notes

- Record key transcripts here as implementation proceeds:
  - `cargo test` summary
  - rendered smoke test output
  - for each HTTP prompt: `run_id`, `run_dir`, and whether motion was authored vs runtime.

`cargo test`:

- `151 passed; 0 failed` (2026-03-05)

Rendered smoke test:

- OK (2026-03-05) `cargo run -- --rendered-seconds 2`

HTTP automation runs (mock backend; `test/gen3d_real/config_mock.toml`):

- snake prompt → `run_id=428f5650-32b7-470e-8ff5-0b05d6e3f098` (motion authored)
- octopus prompt → `run_id=9d484586-64a0-46ec-932e-944926617da0` (motion authored)
- mantis prompt → `run_id=66306bca-d12a-4050-8ade-16fb936c0f28` (motion authored)

## Interfaces and Dependencies

New/changed interfaces that must exist at the end:

- `src/gen3d/agent/tools.rs`:
  - `pub(crate) const TOOL_ID_LLM_GENERATE_MOTION_AUTHORING: &str = "llm_generate_motion_authoring_v1";`

- `src/gen3d/ai/structured_outputs.rs`:
  - enum variant: `Gen3dAiJsonSchemaKind::MotionAuthoringV1`
  - schema name: `gen3d_motion_authoring_v1`

- `src/gen3d/ai/schema.rs`:
  - `AiMotionAuthoringJsonV1` and supporting types for authored clips.

- `src/gen3d/ai/parse.rs`:
  - `parse_ai_motion_authoring_from_text(&str) -> Result<AiMotionAuthoringJsonV1, String>`

- `src/gen3d/ai/job.rs`:
  - enum variant: `Gen3dAgentLlmToolKind::GenerateMotionAuthoring`
