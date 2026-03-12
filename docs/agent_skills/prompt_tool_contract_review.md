# Skill: Prompt ↔ Tool Contract Review

Use this checklist whenever you add/modify tools (especially Gen3D tools). The goal is to prevent common failures where the LLM is instructed to use keys/flows that the engine rejects, or where the new tool behavior exists but is undiscoverable to the agent.

See also:
- Tool authoring rules: `docs/agent_skills/tool_authoring_rules.md`

## Minimum review checklist

1. Tool registry matches runtime parsing

- Update `src/gen3d/agent/tools.rs` (`args_schema` + `args_example`) for any new/changed tool args.
- Ensure examples only use keys that are actually accepted (many tool arg structs use `#[serde(deny_unknown_fields)]`).
- Make sure `one_line_summary` mentions any “must-know” constraints/policies (so it shows up in the tool list).

2. Agent prompt teaches the tool

- Update `src/gen3d/ai/agent_prompt.rs` to explain when to use the tool and any required args (policies/allow-lists/guardrails).
- If a new failure mode exists (tool rejects without a certain arg), the agent prompt should mention it explicitly.

3. Tool-specific prompts/docs are aligned

- If the tool relies on a special contract (e.g. join-frame rules, `rot_frame` requirements, preserve-mode policies), ensure the relevant prompt builder in `src/gen3d/ai/prompts.rs` states it.
- Update docs under `docs/gen3d/` (keep `README.md` clean; put detail in docs).
- Include at least one concrete example snippet for new args/policies.

4. Errors are actionable

- Tool errors should explain what changed (diff/violation) and what to do next (which tool/args to use instead).
- Avoid silent truncation/fallbacks; prefer deterministic errors when inputs are missing/ambiguous.
- Tool results should be actionable and bounded (ids + counts + small examples), not just “keys=[…]” summaries.

5. Tests and smoke

- Add/adjust unit tests for new validation/policy logic (no network/LLM dependency).
- Run the required rendered smoke test:
  - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- Commit changes with a clear message.
