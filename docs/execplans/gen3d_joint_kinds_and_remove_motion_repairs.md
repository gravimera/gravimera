# Gen3D: remove `fixed` joints, prefer `ball`/`free`, and remove deterministic motion repair tools

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds. This document must be maintained in accordance with `PLANS.md` at the repository root.

## Purpose / Big Picture

Gen3D motion authoring currently struggles with hinge-constrained joints (`hinge_off_axis` QA errors) and a legacy `fixed` joint kind that reduces degrees-of-freedom (DoF) and adds confusing semantics. There are also deterministic “motion repair” tools that can change motion in surprising ways.

After this change:

- Gen3D plan + tool contracts no longer include a `fixed` joint kind. Inputs that still contain `"fixed"` are treated as `"free"` for safety, but outputs will never emit `"fixed"`.
- Plan prompting defaults to higher-DoF joint kinds (`ball` and `free`) and treats `hinge` as an opt-in for cases that truly require 1-DoF constraints.
- The deterministic motion repair tools (`recenter_attachment_motion_v1`, `suggest_motion_repairs_v1`) are removed from the Gen3D tool surface and from QA fixit generation, so motion issues are addressed by re-authoring via LLM tools or explicit DraftOps/PlanOps edits instead of silent programmatic “repairs”.

You can verify it works by running a Gen3D build and observing:

- `get_scene_graph_summary_v1` no longer reports `joint_kind="fixed"` (it becomes `free`).
- Tool lists no longer include the removed motion repair tools.
- A run can still complete end-to-end, and the game starts via the standard smoke command.

## Progress

- [x] (2026-03-24) Add this ExecPlan file.
- [x] (2026-03-24) Remove `fixed` from joint kind contract (schemas/prompts), while accepting `"fixed"` as an alias of `"free"` for deserialization.
- [x] (2026-03-24) Update motion validation to remove `fixed_joint_rotates` (since `fixed` no longer exists as a constraint).
- [x] (2026-03-24) Remove deterministic motion repair tools from the tool registry and tool dispatch (`recenter_attachment_motion_v1`, `suggest_motion_repairs_v1`).
- [x] (2026-03-24) Remove any prompt guidance and QA fixits that recommend the removed tools; keep guidance focused on re-authoring motion or explicit ops.
- [x] (2026-03-24) Update tests/regressions affected by the new joint kind set and tool removal.
- [x] (2026-03-24) Run `cargo test` + rendered smoke; commit.

## Surprises & Discoveries

- Observation: The working tree references `docs/agent_skills/tool_authoring_rules.md` and `docs/agent_skills/prompt_tool_contract_review.md` in `AGENTS.md`, but those files are not present (only `docs/agent_skills/SKILL_agent.md` exists).
  Evidence: `ls docs/agent_skills` shows only `SKILL_agent.md`.

## Decision Log

- Decision: Treat legacy `"fixed"` joint kind values as an alias of `"free"` at deserialization time.
  Rationale: Keeps older prefabs/runs loadable while eliminating `fixed` from the forward contract; matches the goal of “more freedom to animate”.
  Date/Author: 2026-03-24 / Codex CLI agent.

## Outcomes & Retrospective

This change removes a source of Gen3D QA friction (`fixed` joints and deterministic motion repairs), pushing the system toward higher-DoF joints (`ball`/`free`) and LLM re-authoring instead of programmatic fixes. Tests and a rendered smoke run validate that the game still starts and the refactor is coherent.

Implemented in commit `9d01d8d`.

## Context and Orientation

Gen3D joint kinds and motion tooling are defined across a small set of files:

- `src/gen3d/ai/schema.rs`: `AiJointKindJson` and `AiJointJson` represent attachment joint metadata used by planning and validation.
- `src/gen3d/ai/structured_outputs.rs`: JSON-schema-like structured output definitions for LLM-backed tools (controls what joint kinds the model is allowed to emit).
- `src/gen3d/ai/prompts.rs`: System instructions for plan generation and motion authoring, including the human-readable schema excerpt that must match structured outputs and Rust parsing.
- `src/gen3d/ai/motion_validation.rs`: Deterministic motion validation, including hinge axis checks and previously `fixed_joint_rotates`.
- `src/gen3d/ai/orchestration.rs`: Builds `get_scene_graph_summary_v1` output and includes stringification of joint kinds.
- `src/gen3d/agent/tools.rs`: Tool registry (tool list presented to the agent).
- `src/gen3d/ai/agent_tool_dispatch.rs`: Tool execution and also generation of QA “capability gaps” and fixits.
- `src/gen3d/ai/agent_prompt.rs`: The high-level agent policy text; must not recommend removed tools.

Joint kinds in this repo are metadata on each attachment edge (`attach_to.joint.kind`):

- `hinge`: 1-DoF rotation about `axis_join` (strict; off-axis rotation triggers `hinge_off_axis`).
- `ball`: rotational joint with unconstrained axis (higher DoF; reduces hinge-off-axis failures).
- `free`: no constraint metadata; intended for maximum freedom (and now also the legacy “fixed” alias).

## Plan of Work

First, change the joint kind enum to drop `Fixed` and accept `"fixed"` as an alias of `Free` when parsing. Then update every prompt/schema surface that enumerates joint kinds to remove `"fixed"` and add explicit guidance to prefer `ball`/`free` and use `hinge` only when necessary.

Second, remove the motion-validation behavior that assumes “fixed joints should not rotate”, because that constraint is no longer modeled.

Third, remove the deterministic motion repair tools from the tool surface:

- Remove their entries from `src/gen3d/agent/tools.rs`.
- Remove tool dispatch handlers from `src/gen3d/ai/agent_tool_dispatch.rs`.
- Remove QA fixit generation that recommends those tools.
- Remove agent prompt guidance that tells the agent to call those tools.

Finally, update tests/regressions that refer to the removed joint kind or tools, run Rust tests, run the required rendered smoke command, and commit.

## Concrete Steps

All commands are run from the repository root (`/Users/flow/workspace/github/gravimera`).

1. Run focused tests during iteration:

   - `cargo test -q gen3d::ai::motion_validation`
   - `cargo test -q gen3d::ai::agent_tool_dispatch`

2. Before finalizing, run the full suite:

   - `cargo test`

3. Required game smoke run (rendered; not headless):

   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

4. Commit:

   - `git status`
   - `git commit -am "gen3d: remove fixed joint kind and motion repair tools"`

## Validation and Acceptance

Acceptance requires:

- `cargo test` passes.
- The rendered smoke run starts and exits without crashing.
- Tool listing no longer includes `recenter_attachment_motion_v1` or `suggest_motion_repairs_v1`.
- `get_scene_graph_summary_v1` attachment edges never report `joint_kind="fixed"`.

## Idempotence and Recovery

These changes are safe to re-run. If tests fail:

- Re-run the specific failing test module with `cargo test <module_or_test_name>`.
- If schema/prompt mismatches occur, ensure the joint kind enum list matches in all three places: Rust enum (`schema.rs`), structured outputs (`structured_outputs.rs`), and prompt text (`prompts.rs`).

## Artifacts and Notes

(Paste any critical failure transcripts here as indented blocks if needed.)

## Interfaces and Dependencies

At the end:

- `crate::gen3d::ai::schema::AiJointKindJson` must serialize to only `hinge`/`ball`/`free`/`unknown` and must accept `"fixed"` as input alias mapping to `free`.
- The Gen3D tool registry (`crate::gen3d::agent::tools::Gen3dToolRegistryV1`) must not expose the removed tool IDs.
- Tool dispatch (`crate::gen3d::ai::agent_tool_dispatch`) must not accept the removed tool IDs.
