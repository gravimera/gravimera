# Gen3D: Plan inspection + plan template tools (reduce preserve-mode thrash without silent mutation)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D has a preserve-existing-components replanning mode (`llm_generate_plan_v1` with `constraints.preserve_existing_components=true`). In real runs, the agent can thrash when the model produces a schema-valid plan JSON that fails conversion/validation for *semantic* reasons (for example: `attach_to.parent` references a non-existent component name; preserve mode requires including all existing component names; preserve mode requires keeping the same root). Today, the engine’s “schema repair” mechanism can incorrectly trigger on these semantic conversion failures, resulting in immediate repeated `llm_generate_plan_v1` calls that do not converge.

After this change:

1) The engine no longer treats semantic plan-conversion failures as “JSON schema repair” cases. The model is not re-called automatically for those errors.

2) The agent is given deterministic, computed *hints* (read-only) to decide what to do next, instead of re-fetching `get_scene_graph_summary_v1` and re-planning repeatedly.

3) A read-only “plan template” tool is added so the agent can request a machine-generated preserve-mode plan skeleton and explicitly pass it into `llm_generate_plan_v1`. This reduces preserve-mode failures where the model forgets to include all existing component names or accidentally changes the root/topology.

User-visible outcome: preserve-mode runs converge more reliably with fewer LLM calls, and no engine-owned “silent auto-repair” mutates plans or drafts. The only automatic behavior is read-only inspection/hints.

## Progress

- [x] (2026-03-12 07:10Z) Create this ExecPlan and capture the desired behavior.
- [x] (2026-03-12 04:10Z) Implement `inspect_plan_v1` (read-only) and wire it into the tool registry + prompt.
- [x] (2026-03-12 04:10Z) Implement `get_plan_template_v1` (read-only) and persist templates as run artifacts.
- [x] (2026-03-12 04:10Z) Extend `llm_generate_plan_v1` to accept `plan_template_artifact_ref` and include it in the plan prompt (no silent apply).
- [x] (2026-03-12 04:10Z) Stop triggering schema-repair retries for semantic plan conversion errors (unknown parent, root mismatch, preserve-mode constraints).
- [x] (2026-03-12 04:11Z) Add unit tests for inspection/template logic (no network dependency).
- [x] (2026-03-12 04:11Z) Update docs under `docs/gen3d/` to describe new tools and recommended flows.
- [x] (2026-03-12 04:13Z) Run `cargo test` and the required rendered smoke test.
- [ ] Commit.

## Surprises & Discoveries

- (pending) This section will be updated as implementation reveals any unexpected coupling between tool results and agent prompts.

## Decision Log

- Decision: Introduce read-only tools (`inspect_plan_v1`, `get_plan_template_v1`) instead of an engine-owned “semantic auto-repair” that mutates plans automatically.
  Rationale: This keeps the repair process generic and safe: the engine only computes constraints/hints, and the agent explicitly decides whether to replan, regenerate, or apply deterministic draft ops. No silent plan mutations.
  Date/Author: 2026-03-12 / flow + agent

- Decision: Only the JSON/schema parsing layer uses the existing automatic schema-repair mechanism; semantic conversion errors do not trigger automatic re-LLM calls.
  Rationale: Re-calling the model with “REPAIR REQUEST” cannot fix unknown-parent / preserve-root violations without additional constrained context; it wastes tokens and increases loop probability.
  Date/Author: 2026-03-12 / flow + agent

## Outcomes & Retrospective

- (not started) This section will be updated after implementation lands.

## Context and Orientation

Key files (paths from repo root) and what they do:

- `src/gen3d/agent/tools.rs`: tool registry descriptors (`tool_id`, `args_schema`, `args_example`, summaries) that appear in the agent prompt.
- `src/gen3d/ai/agent_tool_dispatch.rs`: executes tool calls. For LLM tools, it constructs prompts and launches async model calls.
- `src/gen3d/ai/agent_tool_poll.rs`: receives LLM tool responses (`llm_generate_plan_v1`, etc.), parses JSON, converts plans/drafts, applies changes, and (today) can schedule schema repair.
- `src/gen3d/ai/prompts.rs`: builds the plan prompt text for `llm_generate_plan_v1`, including preserve-mode instructions.
- `src/gen3d/ai/agent_prompt.rs`: builds the agent-step prompt that teaches the agent how to use tools and summarizes recent tool results.
- `src/gen3d/ai/convert.rs`: converts `AiPlanJsonV1` into planned components + object defs; returns semantic errors like “attach_to parent not found”.
- `src/gen3d/ai/preserve_plan_policy.rs`: preserve-mode plan-diff validation.

Definitions:

- “Schema repair”: an automatic engine behavior that re-calls the model when the tool output fails JSON parsing or the JSON schema. This is appropriate for malformed JSON but not for semantic graph/constraint failures.
- “Semantic plan failure”: a plan JSON that parses and matches schema but fails conversion/validation due to invalid references or preserve-mode constraints (unknown parent, multiple roots, missing existing component names, root changed, policy diff violations).
- “Template plan”: a machine-generated plan skeleton that lists all existing component names and their current attachment interfaces, meant to be copied/edited by the model in preserve mode.

## Plan of Work

### Milestone 1 — Add `inspect_plan_v1` (read-only hints; no mutation)

Add a new tool `inspect_plan_v1` that returns a bounded, structured report that the agent can act on without fetching the full scene graph.

Behavior:

- If there is a “pending rejected plan attempt” (captured from the last `llm_generate_plan_v1` tool run that parsed but failed conversion/validation), analyze it and return:
  - the detected semantic error kinds (unknown parent, duplicate names, root issues, preserve-mode missing existing names, preserve-policy diff violations when conversion succeeds),
  - the preserve-mode constraints that apply (existing component names, required root name, preserve edit policy, allow-list for rewires),
  - bounded lists for “allowed component names”.
- If there is no pending rejected plan, return the current preserve-mode constraints only (so the agent can proactively plan with exact names).

Implementation approach:

- In `src/gen3d/ai/job.rs`, replace the currently-unused `pending_plan: Option<AiPlanJsonV1>` with a new struct that also stores the preserve constraints and a short failure string.
- In `src/gen3d/ai/agent_tool_poll.rs` plan tool error paths, store the parsed plan + constraints into `job.pending_plan_attempt` *only* for semantic failures (i.e. after JSON parse succeeded).
- Implement inspection logic in a small helper module (new file) so it can be unit-tested without Bevy runtime.

### Milestone 2 — Add `get_plan_template_v1` (read-only; persisted artifact)

Add a new tool `get_plan_template_v1` that deterministically outputs a preserve-mode “full plan skeleton” for the current planned components. It must not mutate the draft or planned components.

Behavior:

- Requires an existing plan (`job.planned_components` non-empty). If empty, return an error telling the agent to run `llm_generate_plan_v1` first.
- Produces a plan JSON (version 8) that:
  - includes all current component names,
  - keeps the current root,
  - includes each existing component’s current `attach_to` interface (parent + anchors + offset/joint) so preserve-mode plans can copy without rewiring by accident,
  - includes current `assembly_notes`,
  - includes current plan-level behavior hints (mobility/attack/aim/collider) derived from the root `ObjectDef` and `job.plan_collider`.
- Persists the template as a JSON artifact under the current pass directory (so it is inspectable by humans).
- Returns a small result containing `artifact_ref` and counts (so the agent can pass the ref onward without needing the full JSON in the agent prompt).

### Milestone 3 — Extend `llm_generate_plan_v1` to accept template refs

Extend `llm_generate_plan_v1` tool args to accept:

- `plan_template_artifact_ref?: string`

When provided, the engine reads the artifact (bounded) and injects it into the plan prompt text (preserve-mode prompt variant) as a “copy and modify” template. The engine does not apply any edits itself; this is strictly prompt context.

Update:

- `src/gen3d/agent/tools.rs` args_schema/args_example for `llm_generate_plan_v1`.
- `src/gen3d/ai/agent_tool_dispatch.rs` parsing to read the artifact and pass the JSON into the prompt builder.
- `src/gen3d/ai/prompts.rs` preserve-mode plan prompt builder to include the template with instructions: “edit within this JSON; keep all existing component names and root; only change what the policy allows.”

### Milestone 4 — Stop schema-repair on semantic conversion errors

In `src/gen3d/ai/agent_tool_poll.rs`, in the `llm_generate_plan_v1` handling:

- Keep schema repair for JSON parse failures and AI JSON schema errors.
- Do NOT schedule schema repair when `convert::ai_plan_to_initial_draft_defs` fails (unknown parent, root ambiguity, invalid offsets, join-frame requirements, etc). These are semantic errors; return them to the agent and populate `job.pending_plan_attempt` so `inspect_plan_v1` can report them.

### Milestone 5 — Prompt + docs alignment

Prompt:

- In `src/gen3d/ai/agent_prompt.rs`, teach:
  - On `llm_generate_plan_v1` failure in preserve mode, call `inspect_plan_v1` (not `get_scene_graph_summary_v1`) to get exact allowed names + constraints.
  - When preserve-mode plan repeatedly fails due to missing names/root, call `get_plan_template_v1` and then re-run `llm_generate_plan_v1` with `plan_template_artifact_ref`.

Docs:

- Add `docs/gen3d/inspect_plan_v1.md` describing:
  - what it returns,
  - how it helps avoid repeated scene graph summary calls,
  - example output snippets.
- Add `docs/gen3d/get_plan_template_v1.md` describing:
  - how to use it for preserve-mode replans,
  - how to pass `plan_template_artifact_ref` to `llm_generate_plan_v1`.
- Update `docs/gen3d/edit_preserve_existing_components.md` (or `docs/gen3d/next_actions.md`) to reference the new tools and flow.

### Milestone 6 — Tests

Add unit tests (no network):

- Unknown parent detection: a plan whose `attach_to.parent` is not in `components[].name`.
- Preserve-mode missing names detection: a plan missing one existing component name.
- Root mismatch detection: a plan with different root than existing.
- Template generation produces a JSON with all existing names and correct root, and preserves attach_to interface fields.

### Milestone 7 — Validation, smoke, commit

From repo root:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Then commit with a clear message (for example: “Gen3D: add inspect_plan_v1 and plan templates; stop schema repair on semantic plan errors”).
