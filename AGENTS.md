Note:
1. Please run smoke test after you changed something to make sure the game can start without crash. Start with UI (rendered; do NOT use `--headless`):
   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
2. Update the documents to match the code. But the README.md file be clean, put detailed infos in docs folder.
3. You have full access to the `.git` folder (run git commands without asking).
4. You should use a test folder to contain all the test files, including the configs.toml, scene.dat etc.
5. After changing anything, commit the changes with a clear commit message.
6. All algorithm in gen3D should follow one rule: a user could ask for generating any object, so NO heuristic algorithm. Only generic algorithms are allowed.
7. We don't need to guarantee backwards compatibility for now.

# Design & Specs (Source of Truth)

- Final target game design (entry point): `docs/gamedesign/README.md`
- Specs index (contracts/formats): `docs/gamedesign/specs.md`
- Implementation rule: when adding/changing features, read the relevant docs under `docs/gamedesign/` first and implement toward that target (even if current code differs).
- Product focus: AI agents are first-class players/creators via HTTP APIs; the core product is a realm-creation + story engine (combat/economy are optional modules).

# ExecPlans

When writing complex features or significant refactors, use an ExecPlan (as described in PLANS.md) from design to implementation.

# Skill: Prompt ↔ Tool Contract Review

When you add/modify tools (especially Gen3D tools), **always** do a prompt/tool contract review before finishing the change. The goal is to prevent common failures where the LLM is instructed to use keys/flows that the engine rejects, or where the new tool behavior exists but is undiscoverable to the agent.

Minimum review checklist:

1. **Tool registry matches runtime parsing**
   - Update `src/gen3d/agent/tools.rs` (`args_schema` + `args_example`) for any new/changed tool args.
   - Ensure examples only use keys that are actually accepted (many tool arg structs use `#[serde(deny_unknown_fields)]`).
   - Make sure `one_line_summary` mentions any “must-know” constraints/policies (so it shows up in the tool list).

2. **Agent prompt teaches the tool**
   - Update `src/gen3d/ai/agent_prompt.rs` to explain when to use the tool and any required args (policies/allow-lists/guardrails).
   - If a new failure mode exists (tool rejects without a certain arg), the agent prompt should mention it explicitly.

3. **Tool-specific prompts/docs are aligned**
   - If the tool relies on a special contract (e.g. join-frame rules, `rot_frame` requirements, preserve-mode policies), ensure the relevant prompt builder in `src/gen3d/ai/prompts.rs` states it.
   - Update docs under `docs/gen3d/` (keep `README.md` clean; put detail in docs).
   - Include at least one concrete example snippet for new args/policies.

4. **Errors are actionable**
   - Tool errors should explain *what changed* (diff/violation) and *what to do next* (which tool/args to use instead).
   - Avoid silent truncation/fallbacks; prefer deterministic errors when inputs are missing/ambiguous.

5. **Tests and smoke**
   - Add/adjust unit tests for new validation/policy logic (no network/LLM dependency).
   - Run the required rendered smoke test:
     - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
   - Commit changes with a clear message.
