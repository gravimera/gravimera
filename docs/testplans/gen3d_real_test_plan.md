# Gen3D Real (Rendered) Test Plan

This is a living, developer-facing test plan for verifying that **Gen3D works end-to-end in the actual game** (rendered mode):

- Build (LLM-driven) → Save into the world → Move → (optional) Attack → Capture screenshots
- Verify **assembly** (wheels/legs not detached), **animations** (move/attack channels), and **basic gameplay integration** (pathing + combat)

When you find issues, log them in `docs/gen3d_real_test_issues.md` with the run id + cache path.

## Why this exists

Gen3D quality problems often look “fine” in JSON but break in real gameplay:

- wheels spin around the wrong axis (or backwards),
- legs swing but are detached or mirrored incorrectly,
- cannons/turrets rotate constantly due to a wrong channel,
- the saved unit can’t move / can’t attack / crashes the scene,
- components are assembled but drift / intersect badly / float away.

This plan ensures we catch those failures by exercising the real engine loop and reviewing render captures.

## Prerequisites

1) A working `config.toml` with:

- `[automation] enabled=true` (so the test runner can drive the game)
- `[gen3d].ai_service = "openai"` (default) with `[openai]` configured **or** `[gen3d].ai_service = "gemini"` with `[gemini]` configured **or** `[gen3d].ai_service = "claude"` with `[claude]` configured

Config notes:

- `[openai]`, `[gemini]`, and `[claude]` support `base_url`, `model`, and `token` in `config.toml`.
- Tokens can be kept out of the repo and provided via env:
  - OpenAI: `OPENAI_API_KEY`
  - Gemini: `X_GOOG_API_KEY` (or `GEMINI_API_KEY`)
  - Claude: `ANTHROPIC_API_KEY` (or `CLAUDE_API_KEY`)
- For offline/deterministic debug runs, you can use the built-in mock backend by setting `base_url = "mock://gen3d"` and omitting the token (debug/test builds only).
- Reference fixtures: `tests/gen3d/config.toml` (OpenAI), `tests/gen3d/config_gemini.toml` (Gemini), and `tests/gen3d/config_claude.toml` (Claude).

Start from `config.example.toml` (it is safe to copy; `config.toml` is gitignored).

2) Python 3 available (`python3`).

3) Optional: `ffmpeg` (if present, the test runner also writes simple `.mp4` strips from captured frames).

## What the test runner produces

For each Gen3D build run, the engine creates:

- `gen3d_cache/<run_id>/attempt_0/inputs/` (prompt + original images copied into cache)
- `gen3d_cache/<run_id>/attempt_0/pass_*/` (per-pass artifacts: agent trace, render captures, smoke/validate results, etc)

The test runner additionally writes:

- `gen3d_cache/<run_id>/external_screenshots_world/` (spawn + key moments)
- `gen3d_cache/<run_id>/external_screenshots_anim/` (movement frame strip)
- `gen3d_cache/<run_id>/external_screenshots_anim_fire/` (attack frame strip, if attackable)
- `gen3d_cache/<run_id>/driver_status.jsonl` (timeline of `/v1/gen3d/status` polling)

## Canonical prompt suite (4 prompts)

These are the baseline prompts we run after any Gen3D-related engine changes:

1) `A warcar with a cannon as weapon`
2) `A soldier with a gun`
3) `A horse`
4) `A knight on a horse`

## How to run (recommended)

Run the suite via the real (rendered) driver:

    python3 tools/gen3d_real_test.py --config /path/to/config.toml --reset-scene --single-session \
      --prompt "A warcar with a cannon as weapon" \
      --prompt "A soldier with a gun" \
      --prompt "A horse" \
      --prompt "A knight on a horse"

Notes:

- `--reset-scene` deletes the configured `scene.dat` before running, so old saved units don’t overlap new ones.
- `--single-session` keeps the game process alive and saves *all* generated units into the same world, which makes animation/combat review easier.

If you want faster “draft exists” checks (not recommended for animation verification), add `--save-early`.

## What to check (visual + artifacts)

For each run id, review:

1) Build health:

- `GET /v1/gen3d/status` ended with:
  - `build_complete=true`
  - `draft_ready=true`
- No crash in the game log.

2) Assembly correctness:

- `external_screenshots_world/spawn.png`:
  - the unit/building is visible near the hero,
  - it is not clipped into the ground,
  - major components (wheels/legs/body/weapon/head) are present (no missing arm/leg).
- `attempt_0/pass_*/render_*.png` (agent review captures):
  - components are not scattered far away from the root,
  - symmetric parts (wheels/legs/arms) are mirrored as expected,
  - no “mystery block” floating far behind the object.

3) Movement animation:

- `external_screenshots_anim/frame_*.png` (and `move_anim.mp4` if present):
  - Vehicles: wheels spin around their axle (not vertical/world-up) and look wheel-like (not a paper-thin disc).
  - Walkers/animals: legs move plausibly (swing/frequency ties to speed); legs are not swapped/inside-out.
  - The unit moves without getting permanently stuck on nearby buildings.

4) Attack behavior (only if `has_attack=true` in `/v1/state` for that instance):

- `external_screenshots_anim_fire/frame_*.png`:
  - projectiles appear and travel,
  - the weapon/head aims toward the fire target while the body keeps moving (aim vs move split),
  - melee units can attack a direction even with no enemy nearby (fan-shaped reach), and clamp to max aim angle if needed.

5) Engine smoke/validate artifacts (optional but helpful):

- `attempt_0/pass_*/smoke_results.json`:
  - pay attention to spin-axis warnings (`suggested_component_local_axis` can fix wheels/props/turrets).
- `attempt_0/pass_*/validate.json`:
  - ensure there are no schema/anchor/reference errors.
- `attempt_0/pass_*/qa.json` (if `qa_v1` was used):
  - combined `{ ok, validate, smoke, errors, warnings }` summary for the pass.

## Logging issues

When you find an issue, add a new entry in `docs/gen3d_real_test_issues.md` with:

- date/time,
- prompt,
- `run_id` + full cache path,
- what you expected vs what happened,
- the most relevant images (paths under `external_screenshots_*` and/or `attempt_0/pass_*/render_*.png`),
- whether it looks like an engine bug (wrong axis conversion, bad mirroring) or an AI output issue.

Keep entries short but reproducible.

## Improving this plan

When Gen3D capabilities expand (new primitives, new animation channels, new combat kinds), update this doc to include:

- new canonical prompts,
- new “must-check” failure modes,
- new artifact paths produced by the engine/scripts.
