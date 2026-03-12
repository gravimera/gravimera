# Gen3D: Make tool contracts discoverable and tool-call errors actionable

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D runs currently lose time (and hit the no-progress guard) when the agent calls tools with empty `{}` args for tools that require required keys (for example `search_artifacts_v1` requires `query`). Today, the agent only sees each tool’s one-line summary by default; it must spend an extra step calling `get_tool_detail_v1` to learn required args, and on failures it only sees an unhelpful “Missing args.*” message. This combination can lead to repeated inspection steps, wasted budgets, and “best effort stop”.

After this change:

1) The agent sees a brief, per-tool args signature in every step prompt (so it can avoid empty-arg calls without spending `get_tool_detail_v1`).
2) When a tool call fails, the “Recent tool results” entry includes contract help (required keys, args schema signature, and an example args object), so the agent can correct itself on the next step.

This change is generic (no object-specific heuristics) and does not modify Gen3D budgets. It improves correctness and reduces inspection loops.

## Progress

- [x] (2026-03-13 07:47Z) Write this ExecPlan and commit it.
- [x] (2026-03-13 07:40Z) Implement tool-list prompt formatting with args signatures (and bounded examples).
- [x] (2026-03-13 07:41Z) Implement generic contract hints for tool-call errors in “Recent tool results”.
- [x] (2026-03-13 07:42Z) Add unit tests proving prompt and error summaries include required contract details.
- [x] (2026-03-13 07:43Z) Update user-facing docs under `docs/` / `gen_3d.md` to reflect the new prompt/error behavior.
- [x] (2026-03-13 07:44Z) Run tests and the required rendered smoke test.
- [x] (2026-03-13 07:47Z) Commit the implementation with a clear message.

## Surprises & Discoveries

- Observation: The Gen3D agent prompt currently lists only `tool_id` + `one_line_summary`, and the tool-result summarizer returns early on error, so the agent sees only `ERROR: <message>` with no contract help.
  Evidence: `src/gen3d/ai/agent_prompt.rs` `build_agent_user_text()` and `summarize_tool_result()` early-return branch.

- Observation: In a real run, the agent can successfully fetch `get_tool_detail_v1` for a tool, but then take other steps before using it, so the schema is no longer in “Recent tool results” when it next attempts the tool call.
  Evidence: Run cache example from March 13, 2026 (run id `28b433e3-3eca-4105-b659-12c5586109d3`) shows `get_tool_detail_v1(search_artifacts_v1)` in pass_24, but later pass_26 calls `search_artifacts_v1` with `{}` again and fails.

## Decision Log

- Decision: Prefer always-on prompt discovery (args signature per tool) over changing no-progress guard accounting for `get_tool_detail_v1` steps.
  Rationale: Making introspection “free” risks infinite loops. Always-on discovery avoids extra steps and reduces guard pressure without changing budgets.
  Date/Author: 2026-03-13 / GPT-5.2

- Decision: Add generic contract hints for *all* tool-call failures by looking up the tool descriptor in the registry at prompt-build time.
  Rationale: This handles more than `search_artifacts_v1` / `read_artifact_v1` without editing every tool implementation and keeps the behavior consistent.
  Date/Author: 2026-03-13 / GPT-5.2

## Outcomes & Retrospective

- Outcome: The agent sees a brief args signature and example per tool by default, avoiding empty-arg calls without spending `get_tool_detail_v1`.
- Outcome: Tool-call failures now include a compact contract hint (expected args signature, required keys, example) in the next step’s “Recent tool results”.
- Outcome: Added unit tests for prompt + error-hint behavior, and updated `gen_3d.md` docs.

## Context and Orientation

Gen3D agent “tools” are registry-defined operations the agent can call. They are described in `src/gen3d/agent/tools.rs` as `Gen3dToolDescriptorV1 { tool_id, one_line_summary, args_schema, args_example }`.

The agent prompt is assembled in `src/gen3d/ai/agent_prompt.rs`:

- `build_agent_system_instructions()` returns the system message (strict JSON, rules).
- `build_agent_user_text()` returns the user message (tool list, state summary, recent tool results).

The no-progress guard stops best-effort builds after N inspection steps or tries where the assembled-state hash does not change. It is implemented in `src/gen3d/ai/agent_step.rs` and is intentionally budgeted (defaults are documented in `config.example.toml`).

Key terms used in this plan:

- “Args signature”: a short, single-line representation of a tool’s expected args, derived from the first line of `args_schema`.
- “Contract hint”: guidance shown to the agent when a tool call fails, including required keys and an example args object.

## Plan of Work

Edit the prompt builder to always include a brief args signature for each tool, and optionally a bounded example args object. This removes the need for `get_tool_detail_v1` for basic correct invocation.

Edit the tool-result summarizer so that when `ok=false`, the prompt includes contract hints for the tool that failed. This is done by looking up the tool descriptor (`args_schema` and `args_example`) from the registry and formatting a bounded hint.

Add unit tests that:

1) Assert the tool list includes args signatures for at least one “required-arg” tool (`search_artifacts_v1`) and one “no-arg” tool (`qa_v1`).
2) Assert the error summary for an `ok=false` tool result includes the expected contract hint (schema + example).

Update docs to reflect the new prompt content and the error help, keeping `README.md` unchanged.

## Concrete Steps

All commands are run from the repository root.

1) Implement prompt formatting:

   - Edit `src/gen3d/ai/agent_prompt.rs` in `build_agent_user_text()`.
   - Change the “Available tools” section to print:
     - `tool_id`
     - `one_line_summary`
     - `args:` followed by the first line of `args_schema` (or `{}`).
     - Optional `example:` with `args_example` truncated to a safe length (do not dump large schemas/examples).

2) Implement error contract hints:

   - In the `!result.ok` branch of `summarize_tool_result()`, append:
     - `required_keys=[…]` derived from the args signature (keys without `?`).
     - `args_schema=<first line>` (truncated).
     - `args_example=<json>` (truncated).
   - If the tool id is unknown (not found in registry), fall back to just the error string.

3) Add tests:

   - Extend `#[cfg(test)]` tests in `src/gen3d/ai/agent_prompt.rs` to build a minimal `Gen3dAiJob::default()` and `Gen3dWorkshop::default()` and call `build_agent_user_text()`.
   - Assert presence of tool signatures and error hints in the resulting text.

4) Update docs:

   - Update `gen_3d.md` (or a new doc under `docs/`) to explain:
     - the prompt includes per-tool args signatures
     - tool-call errors show the relevant args schema signature and example
     - `get_tool_detail_v1` remains for deep/complex tools, but should be less frequently needed

5) Validation:

   - Run unit tests:
     - `cargo test -q`
   - Run the required rendered smoke test (do NOT use `--headless`):
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME=\"$tmpdir/.gravimera\" cargo run -- --rendered-seconds 2`

6) Commit:

   - Commit with a message like: `gen3d: show tool args in prompt + actionable tool-call errors`

## Validation and Acceptance

Acceptance is met when:

1) The generated agent user prompt text always contains `args:` for each tool and clearly distinguishes `{}` tools from tools requiring keys.
2) When a `Gen3dToolResultJsonV1 { ok:false }` is present in recent results, the prompt includes a compact hint showing the tool’s expected args signature and an example.
3) `cargo test` passes and the rendered smoke test completes without crashing.

## Idempotence and Recovery

These changes are safe to re-run:

- Prompt formatting changes are deterministic.
- Tests are additive and can be re-run repeatedly.

If a prompt change unexpectedly bloats token usage, reduce truncation limits for examples (keep args signatures intact).

## Artifacts and Notes

- Key files to edit:
  - `src/gen3d/ai/agent_prompt.rs` (prompt + error summaries)
  - `src/gen3d/agent/tools.rs` (if any args_schema/args_example require cleanup to be accurate)
  - `gen_3d.md` or a doc under `docs/`

## Interfaces and Dependencies

No new external dependencies are required. Use the existing `Gen3dToolRegistryV1` descriptors as the source of truth for tool arg contracts.
