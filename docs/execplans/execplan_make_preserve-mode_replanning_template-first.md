# Make Preserve-Mode Replanning Template-First (Stop `llm_generate_plan_v1` Thrash)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Today, in preserve mode (`constraints.preserve_existing_components=true`), the agent can call `llm_generate_plan_v1` without a `plan_template_kv`. That makes the model “restate the whole existing plan” from
partial context, which frequently produces semantic errors (missing existing names, missing anchors, missing `rot_frame`) and triggers expensive repeated replans.

After this change, preserve-mode replanning becomes template-first and self-correcting: the tool contract will refuse preserve-mode replans without a template, the prompt/tool registry will teach the correct
flow, and tests will prevent regressions. The observable effect is fewer plan retries and faster convergence on simple edit requests (e.g. “add a hat”).

## Progress

- [ ] (2026-03-14) Audit current preserve-mode replanning flow and locate the `llm_generate_plan_v1` dispatch + prompt builders.
- [ ] Audit template size/availability constraints (`get_plan_template_v1` max bytes; Info Store requirements) and add a non-deadlocking recovery path.
- [ ] Implement a tool-level gate: preserve-mode replans require `plan_template_kv` when an existing plan is present.
- [ ] Update tool registry + agent prompt text so the gate is discoverable and consistent.
- [ ] Add regression tests for the new gate (offline; no OpenAI calls).
- [ ] Update docs under `docs/gen3d/` to match the new contract.
- [ ] Run `cargo test` and the required rendered smoke test, then commit with a clear message.

## Surprises & Discoveries

- Observation: The agent prompt already *suggests* using `get_plan_template_v1`, but it is optional language (“if keep failing”), and the tool registry summary for `llm_generate_plan_v1` does not mention any
  template requirement. This makes “retry `llm_generate_plan_v1`” the path of least resistance.
  Evidence: `src/gen3d/ai/agent_prompt.rs` preserve-mode section; `src/gen3d/agent/tools.rs` `llm_generate_plan_v1` descriptor.
- Observation: `get_plan_template_v1` can fail for large plans (hard max-bytes) and requires the Info Store to be available; `llm_generate_plan_v1` currently hints that templated replans are optional. If we add
  a hard “template required” gate without addressing these cases, preserve-mode replanning can dead-end on big assemblies or Info Store failures.
  Evidence: `src/gen3d/ai/agent_tool_dispatch.rs` `TOOL_ID_GET_PLAN_TEMPLATE` max-bytes check + Info Store write; `TOOL_ID_LLM_GENERATE_PLAN` `plan_template_kv` read + size checks.

## Regression Risks (and mitigations)

1) **Dead-end on template size**: `get_plan_template_v1` has a strict max size; if it cannot produce a template for a large assembly, and preserve-mode replans are gated on `plan_template_kv`, then preserve-mode
   replanning becomes impossible for those sessions.

2) **Dead-end on storage**: if the Info Store cannot be opened/written/read (misconfig, IO error), then `plan_template_kv` cannot be obtained even when a template JSON would be small enough.

Mitigation principle (from `docs/agent_skills/tool_authoring_rules.md`): gates must be teachable *and* provide actionable recovery paths; avoid introducing deadlocks.

Mitigation options (pick at least one before enforcing the hard gate):

- **Preferred**: make `get_plan_template_v1` *bounded by design* and always able to emit a “lean” template under the budget (e.g. a new version or args that omit text-heavy fields like `assembly_notes` /
  `modeling_notes`, and/or enforce per-field limits). Return `truncated: true/false` + `omitted_fields[]` so the agent knows what was dropped.
- **Secondary**: align the byte budgets and error language (avoid “replan without template” if we are enforcing template-first) and ensure errors mention the next deterministic step (e.g. “call
  `get_plan_template_v1` with `version=2` (lean) / `max_bytes=...`”).
- **Escape hatch** (only if needed): an explicit, opt-in override (a new arg) that allows preserve-mode replanning without a template when the template tool is unavailable. This must be auditable and should be
  discouraged in prompts/registry so it doesn’t become the new default path.

## Decision Log

- Decision: Enforce the “template-first” workflow via tool-contract gating rather than only prompt wording.
  Rationale: `docs/agent_skills/tool_authoring_rules.md` explicitly prefers tool-contract enforcement over prompt micromanagement; tool gates prevent token-burning loops even when the model ignores hints.
  Date/Author: 2026-03-14 / GPT-5.2

## Outcomes & Retrospective

(Empty until implementation.)

## Context and Orientation

Key concepts:

- “Preserve mode” is used in seeded edit/fork sessions to avoid regenerating existing geometry. It is controlled by `llm_generate_plan_v1.constraints.preserve_existing_components=true`.
- `get_plan_template_v1` is a read-only tool that writes an engine-generated plan JSON template to the Info Store and returns a `plan_template_kv` reference. The returned KV key is deterministic:
`ws.<workspace_id>.plan_template.preserve_mode.v1` (see `src/gen3d/ai/agent_tool_dispatch.rs`).
- `llm_generate_plan_v1` is dispatched in `src/gen3d/ai/agent_tool_dispatch.rs` and uses prompt builders in `src/gen3d/ai/prompts.rs`.
- The agent’s system instructions (what the model is told to do) are built in `src/gen3d/ai/agent_prompt.rs`.
- The tool registry shown to the model is in `src/gen3d/agent/tools.rs`.

Relevant existing docs:

- `docs/gen3d/edit_preserve_existing_components.md`
- `docs/gen3d/get_plan_template_v1.md`
- `docs/gen3d/inspect_plan_v1.md`
- Prompt/tool alignment checklist: `docs/agent_skills/prompt_tool_contract_review.md`

## Plan of Work

### 1) Audit current behavior

In the repo root:

- Use ripgrep to locate the `TOOL_ID_LLM_GENERATE_PLAN` handling in `src/gen3d/ai/agent_tool_dispatch.rs`.
- Confirm how `plan_template_kv` is currently parsed and used (it is optional today).
- Confirm how the tool list (`src/gen3d/agent/tools.rs`) describes `llm_generate_plan_v1` and whether it mentions preserve-mode templating (it does not today).

Record the exact spot(s) where we will add a preflight gate.

### 2) Close the dead-end regressions (template size + Info Store)

Before enforcing “template required”, ensure there is a deterministic recovery path when templates are not available:

- Template size: add a bounded “lean” template variant for preserve-mode replans (versioned or arg-gated), and document its byte budget. It should include only the minimal fields required to preserve names,
  anchors, attach relationships, and `rot_frame`-like anchors/frames if applicable.
- Info Store dependency: decide whether we can (a) guarantee Info Store availability for Gen3D sessions, or (b) provide an alternate, explicit ref type if the store is unavailable. Do **not** silently change
  behavior; any alternate must be reflected in the tool contract and registry.

Add tests for these cases so the gate cannot ship without a safe recovery.

### 3) Add a tool-level gate (contract enforcement)

In `src/gen3d/ai/agent_tool_dispatch.rs`, inside the `TOOL_ID_LLM_GENERATE_PLAN` branch, add a preflight check before spawning any model call:

- If `preserve_existing_components == true` AND `job.planned_components` is non-empty (meaning we truly are preserving an existing plan) AND `args.plan_template_kv` is missing:
  - Return `ok=false` immediately with an actionable error string.
  - The error must be short enough to survive prompt truncation (tool results are truncated in the agent prompt); include concrete next steps:
    - “Call `get_plan_template_v1` first”
    - “Retry `llm_generate_plan_v1` with `plan_template_kv` from that result”
    - If template generation fails due to size or storage, include the deterministic alternative chosen in step 2 (e.g. “retry `get_plan_template_v1` with `version=2` (lean)”).

Do not silently call `get_plan_template_v1` internally; that would be a hidden side effect and violates the tool-authoring rules.

Optional but recommended follow-up gate (only if it proves safe during implementation):
- If there is a captured `job.pending_plan_attempt` from a semantic failure, refuse immediate re-calls to `llm_generate_plan_v1` unless a template is provided. This prevents “retry loops” after semantic errors.

### 4) Make the gate discoverable in prompts/tool registry

Update the tool registry and agent instructions so the model can discover the requirement without trial-and-error:

- In `src/gen3d/agent/tools.rs`, update `llm_generate_plan_v1.one_line_summary` to mention:
  - Preserve-mode replanning requires `plan_template_kv` (call `get_plan_template_v1` first).
- In `src/gen3d/ai/agent_prompt.rs`, update the preserve-mode helper text to match the new contract:
  - Replace “If preserve-mode replanning keeps failing…” with a direct rule: preserve mode requires `get_plan_template_v1` + `plan_template_kv`.
  - Keep it contract-oriented (“tool will refuse otherwise”), not heuristic advice.

Ensure these edits stay consistent with `docs/agent_skills/prompt_tool_contract_review.md`.

### 5) Add offline regression tests

Add tests that do not call OpenAI:

- Implement the gate in a small helper function (pure function) if needed so it is easy to test without constructing the full async tool pipeline.
- Add Rust tests that verify:
  - Preserve-mode + existing plan + missing `plan_template_kv` returns an error containing `get_plan_template_v1`.
  - Non-preserve mode does not require `plan_template_kv`.
  - Preserve-mode but with no existing plan (empty `planned_components`) does not crash and behaves as before (or errors with a clear message if we choose to enforce that case too).
- Add tests that verify we do not dead-end on large plans / template unavailability:
  - `get_plan_template_v1` “lean” mode stays under the byte budget for a synthetic large plan (fixture under `test/` if needed).
  - The `llm_generate_plan_v1` gate error message includes the “lean template” recovery path (or the chosen alternative).

If any fixture files are needed, store them under the repo’s `test/` folder (not scattered elsewhere), consistent with the repo instruction about test artifacts.

### 6) Update docs (source of truth for users/agents)

Update `docs/gen3d/edit_preserve_existing_components.md`:

- Change preserve-mode guidance from “use template if replanning keeps failing” to “template is required for preserve-mode replans”.
- Keep troubleshooting sections aligned with the new behavior.

Update `docs/gen3d/get_plan_template_v1.md` as needed to reflect that its output is now required input for preserve-mode replanning.

Keep `README.md` clean; put details only in `docs/`.

### 7) Validation + commit

Run:

- `cargo test` (or a scoped test filter if the suite is large).
- Required rendered smoke test (per repo instructions):
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

Then commit with a clear message, e.g. “gen3d: require plan_template_kv for preserve replans”.

## Concrete Steps

From repo root:

- Locate code:
  - `rg -n "TOOL_ID_LLM_GENERATE_PLAN" src/gen3d/ai/agent_tool_dispatch.rs`
  - `rg -n "LLM: generate plan" src/gen3d/agent/tools.rs`
  - `rg -n "Preserve-mode replanning" src/gen3d/ai/agent_prompt.rs`
- Locate size/availability constraints:
  - `rg -n "MAX_TEMPLATE_BYTES" src/gen3d/ai/agent_tool_dispatch.rs`
  - `rg -n "plan_template_kv is too large|Failed to open Info Store" src/gen3d/ai/agent_tool_dispatch.rs`
- Implement the gate and prompt/registry updates.
- Add tests and run:
  - `cargo test -q`
- Run smoke:
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- Commit:
  - `git status`
  - `git commit -am "gen3d: require plan template for preserve replans"` (or equivalent, after adding new files)

## Validation and Acceptance

Acceptance is met when:

1) Calling `llm_generate_plan_v1` with `constraints.preserve_existing_components=true` in an edit session with an existing plan, but without `plan_template_kv`, immediately returns a tool error that tells the
agent to call `get_plan_template_v1` and retry with the returned `plan_template_kv`, without making any network LLM request.

2) The tool list and agent prompt clearly mention the requirement so an agent can do the right thing on the first attempt.

3) New tests cover the gate and pass offline.

4) The rendered smoke test starts and exits cleanly.

5) The new gate does not introduce a preserve-mode dead-end:
   - If the template tool cannot emit a template due to size, the workflow has a deterministic fallback (e.g. a “lean” template version) that is documented and tested.
   - If the template cannot be stored/loaded, the error message provides an actionable recovery path rather than forcing disable-preserve as the only option.

## Idempotence and Recovery

- The gate is safe to apply repeatedly; it only changes behavior for preserve-mode replans that omit `plan_template_kv`.
- If the gate blocks an unexpected workflow, rollback is a single revert commit; no data migrations are needed.

## Artifacts and Notes

Keep the error string short and concrete (it will be truncated in the agent prompt). Prefer wording like:

  “Preserve-mode replanning requires `plan_template_kv`. Call `get_plan_template_v1`, then retry `llm_generate_plan_v1` with the returned `plan_template_kv`.”

## Interfaces and Dependencies

No new external dependencies. All changes are in existing Gen3D modules:

- Tool dispatch: `src/gen3d/ai/agent_tool_dispatch.rs`
- Tool registry: `src/gen3d/agent/tools.rs`
- Agent system prompt: `src/gen3d/ai/agent_prompt.rs`
- Docs: `docs/gen3d/*.md`
- Tests: existing Rust test framework (unit/integration tests), with fixtures under `test/` if needed.
