# Gen3D: Make preserve-mode plan failures actionable (repair context + stronger inspection; enforce preserve policy)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Preserve-mode editing (`llm_generate_plan_v1` with `constraints.preserve_existing_components=true`) is meant to let users add/adjust small parts on an existing prefab without regenerating the entire object. In practice, preserve-mode sessions can fail in two ways that lead to agent thrash:

1) The model returns plan output that is invalid JSON / fails the plan JSON schema. The engine has an automatic “schema repair” retry mechanism, but if that retry prompt does not include preserve-mode constraints (existing component names, root, policy), the model often “forgets” required names and emits a from-scratch plan, creating more semantic failures.

2) The model returns schema-valid JSON that fails semantically (unknown parent names, missing anchors referenced by attachments, missing existing component names, root changed). The agent needs deterministic, computed hints to fix these issues without repeatedly dumping the full scene graph.

After this change:

- Preserve-mode schema-repair retries for `llm_generate_plan_v1` use the correct preserve-mode prompt context (including optional plan templates), so a repair attempt is not accidentally “from scratch”.
- Preserve-mode edit-policy enforcement is actually applied (plan diffs are validated even when the root component is unchanged).
- `inspect_plan_v1` produces more actionable, computed hints (anchor existence checks and name suggestions) so the agent can fix common failures without calling `get_scene_graph_summary_v1`.

User-visible outcome: preserve-mode replans converge with fewer redundant tool calls, and error/hint tooling aligns with the constraints the engine truly enforces.

## Progress

- [x] (2026-03-12 04:40Z) Identify plan schema-repair prompt mismatch for preserve mode.
- [x] (2026-03-12 04:45Z) Update plan schema-repair path to use preserve-mode prompt context and include optional plan templates.
- [x] (2026-03-12 05:00Z) Fix preserve-policy diff validation so it runs even when the root is unchanged.
- [x] (2026-03-12 05:10Z) Extend `inspect_plan_v1` to detect missing anchors referenced by attachments and to provide name suggestions for unknown parents.
- [x] (2026-03-12 05:25Z) Add unit tests for new inspection behaviors (anchor errors + suggestions).
- [x] (2026-03-12 05:29Z) Run `cargo test` and the required rendered smoke test.
- [ ] Commit with a clear message.

## Surprises & Discoveries

- Observation: Plan schema-repair retries were rebuilt using the non-preserve planning prompt even when `constraints.preserve_existing_components=true`.
  Evidence: Preserve-mode repair prompts lacked the “existing component snapshot / preserve policy” sections, making repair attempts prone to omit required existing names and anchors.

- Observation: Preserve-mode policy diff validation could be skipped entirely when the root component existed and remained unchanged.
  Evidence: The code previously used an `else if` chain that returned early after the root check and never ran `validate_preserve_mode_plan_diff` in the common case.

## Decision Log

- Decision: Keep semantic “repair” as read-only hints (`inspect_plan_v1` + templates) rather than engine-owned silent plan mutation.
  Rationale: The engine should not guess intent or mutate plans automatically; it should compute constraints/hints and let the agent explicitly replan.
  Date/Author: 2026-03-12 / flow + agent

- Decision: Improve the schema-repair prompt to match the original tool call’s preserve-mode constraints.
  Rationale: Automatic schema repair is already an engine behavior for malformed JSON/schema. It must not change the planning “mode” or lose the preserve constraints that keep the draft safe.
  Date/Author: 2026-03-12 / flow + agent

## Outcomes & Retrospective

- (2026-03-12) Implemented preserve-mode repair prompt alignment + stronger inspection hints + preserve-policy enforcement; tests and rendered smoke are green. Pending: commit.

## Context and Orientation

Key files (paths from repo root):

- `src/gen3d/ai/agent_tool_poll.rs`: parses LLM tool outputs and applies them. Also schedules the automatic schema-repair retry for malformed JSON/schema outputs.
- `src/gen3d/ai/preserve_plan_policy.rs`: defines preserve-mode policies and computes diff violations.
- `src/gen3d/ai/plan_tools.rs`: implements `inspect_plan_v1` and `get_plan_template_v1`.
- `src/gen3d/ai/prompts.rs`: builds preserve-mode plan prompts and includes existing component snapshots/templates.
- `docs/gen3d/inspect_plan_v1.md`, `docs/gen3d/edit_preserve_existing_components.md`: user-facing tool docs.

Definitions:

- “Schema repair”: automatic retry when tool output fails JSON parsing or JSON schema decoding.
- “Semantic plan failure”: plan JSON parses but is invalid as an attachment graph or violates preserve-mode constraints.
- “Preserve edit policy”: the rules for what changes are allowed to existing components in preserve-mode replans (`additive`, `allow_offsets`, `allow_rewire`).

## Plan of Work

1) Preserve-mode schema-repair prompt alignment

In `src/gen3d/ai/agent_tool_poll.rs`, in the `llm_generate_plan_v1` parse/schema error path, rebuild the repair prompt using:

- preserve-mode prompt (`build_gen3d_plan_user_text_preserve_existing_components`) when preserve mode is enabled and there is an existing plan,
- otherwise the normal planning prompt (`build_gen3d_plan_user_text_with_hints`).

If `plan_template_artifact_ref` was provided on the tool call, attempt to read it (bounded) and include the parsed JSON in the preserve-mode repair prompt as “copy+edit” context.

2) Preserve-policy enforcement correctness

In `src/gen3d/ai/agent_tool_poll.rs`, ensure that when preserve mode is active and the root is unchanged, the code still runs `validate_preserve_mode_plan_diff` and rejects plan diffs that violate the selected preserve policy.

3) Stronger, computed inspection hints

In `src/gen3d/ai/plan_tools.rs` (`inspect_plan_v1` implementation), add:

- `missing_child_anchor`: attachment references a child anchor name that is not `origin` and is not present in that component’s `anchors[]`.
- `missing_parent_anchor`: attachment references a parent anchor name that is not `origin` and is not present in the parent component’s `anchors[]`.
- `unknown_parent.suggestions[]`: a small, deterministic list of existing component names that are likely intended (token/substring match only; no auto-application).

Update docs to reflect these new error kinds and suggestion behavior.

## Concrete Steps

From repo root:

    cargo test
    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Expected:

- `cargo test` succeeds.
- The rendered smoke test starts and exits cleanly (no crash) within ~2 seconds.

## Validation and Acceptance

Acceptance is met when:

- In preserve-mode plan schema parse failures, the schema-repair retry prompt includes preserve-mode constraints (existing component names/root/policy) rather than the from-scratch planning prompt.
- Preserve-mode replans that violate `constraints.preserve_edit_policy` are rejected consistently (no silent acceptance due to skipped diff validation).
- `inspect_plan_v1` reports missing anchor errors and provides non-empty suggestions for common name mismatches (e.g., `dragon_neck` → `neck`).

## Idempotence and Recovery

These changes are safe to apply repeatedly. If `cargo test` or smoke fails:

- Revert the most recent commit and re-run the commands above to confirm the failure is introduced by this change.
- Use `rg` to locate the last changes in `src/gen3d/ai/agent_tool_poll.rs` (plan parse error path and preserve-policy validation) and in `src/gen3d/ai/plan_tools.rs` (inspection errors).

## Artifacts and Notes

- No additional runtime artifacts are created beyond existing Gen3D run cache artifacts.

## Interfaces and Dependencies

- No new external crates are introduced.
- Tool contracts remain read-only for inspection/template helpers; no new mutating tools are added.
