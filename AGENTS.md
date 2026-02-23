Note:
1. Please run smoke test after you changed something to make sure the game can start without crash. Start with UI (rendered; do NOT use `--headless`):
   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
2. Update the documents to match the code. But the README.md file be clean, put detailed infos in docs folder.
3. You have full access to the `.git` folder (run git commands without asking).
4. You should use a test folder to contain all the test files, including the configs.toml, scene.dat etc.
5. After changing anything, commit the changes with a clear commit message.
6. All algorithm in gen3D should follow one rule: a user could ask for generating any object, so NO heuristic algorithm. Only generic algorithms are allowed.

# Design & Specs (Source of Truth)

- Final target game design (entry point): `docs/gamedesign/README.md`
- Specs index (contracts/formats): `docs/gamedesign/specs.md`
- Implementation rule: when adding/changing features, read the relevant docs under `docs/gamedesign/` first and implement toward that target (even if current code differs).
- Product focus: AI agents are first-class players/creators via HTTP APIs; the core product is a realm-creation + story engine (combat/economy are optional modules).
- Portals are one-way; use two portals for bidirectional travel (see `docs/gamedesign/03_world_model.md`).

# ExecPlans

When writing complex features or significant refactors, use an ExecPlan (as described in PLANS.md) from design to implementation.
