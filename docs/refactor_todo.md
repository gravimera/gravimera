# Refactor TODOs (Backlog)

This document tracks **refactor candidates** found while reviewing the repo (2026-02-27).

Guidelines:

- Keep items **small + reversible** (prefer mechanical splits/extractions).
- Prefer **shared helpers** over copy/paste (but keep module boundaries clear).
- For Gen3D, preserve the rule: **no heuristic algorithms** (deterministic, schema-driven behavior only).
- When a refactor changes behavior, update/expand docs under `docs/` (keep `README.md` concise).

## 1) Cross-cutting: HTTP/OpenAI + curl helpers

- [x] **Deduplicate curl/OpenAI helper code**
  - Why: `Gen3D` and `SceneBuildAI` re-implement the same patterns (temp auth header file, HTTP status marker parsing, output text extraction, curl invocation patterns).
  - Where:
    - `src/gen3d/ai/openai.rs` (e.g. `TempSecretFile`, curl helpers)
    - `src/scene_build_ai.rs` (similar helpers duplicated)
  - Done when: shared helper module is used by both call paths; behavior stays identical; no secrets are passed on argv.

- [x] **Unify “threaded request result” plumbing**
  - Why: repeated `Arc<Mutex<Option<Result<...>>>>` + progress tracking patterns in both Gen3D and SceneBuildAI.
  - Where:
    - `src/gen3d/ai/mod.rs` + `src/gen3d/ai/agent_loop/mod.rs`
    - `src/scene_build_ai.rs`
  - Done when: a small reusable abstraction handles “spawn worker thread + shared result + progress updates” without changing semantics.

## 2) Gen3D AI module structure

- [x] **Split `src/gen3d/ai/agent_loop/mod.rs` into focused modules**
  - Why: very large file (state machine + tool execution + rendering capture + orchestration); hard to navigate/review.
  - Suggested split (example): `agent_state.rs`, `agent_step.rs`, `tool_dispatch.rs`, `render_capture.rs`, `progress.rs`, `metrics.rs`.
  - Done when: file size reduced substantially; public surface stays minimal; compile/tests unchanged.

- [x] **Reduce “god module” surface in `src/gen3d/ai/mod.rs`**
  - Why: mixes types/state, rendering helpers, job lifecycle, batch logic, and misc helpers.
  - Done when: `mod.rs` becomes mostly re-exports + small glue; heavy logic lives in submodules.

## 3) Automation HTTP API routing

- [x] **Refactor the big route match into per-feature handlers**
  - Why: `src/automation/mod.rs` has a large `(method, path)` match and huge handler signature; repeated JSON parse/error boilerplate.
  - Done when: routing is table-driven or split by feature area (`/v1/scene_sources/*`, `/v1/gen3d/*`, `/v1/animation/*`, etc.) with a small shared `AutomationContext`.

## 4) Camera orbit / screenshot helpers

- [x] **Extract shared orbit/screenshot/render-target utilities**
  - Why: orbit transform + render target creation exist in multiple places with small differences.
  - Where (examples):
    - `src/gen3d/preview.rs` (orbit + render target)
    - `src/gen3d/ai/orchestration.rs` (orbit + capture helpers)
    - `src/scene_build_ai.rs` (orbit + capture helpers)
  - Done when: a shared helper module is used in all three sites; behavior is unchanged (same camera angles, distances, timeouts, and formats).

## 5) Config parsing and structure

- [ ] **Reduce duplication in `src/config.rs` parse pipeline**
  - Why: many `parse_*` functions + duplicated “apply all parses” sequences; hard to add fields without missing a call site.
  - Options:
    - Convert to typed TOML via `toml` + `serde` (preferred if acceptable).
    - Or: keep current parser but centralize parse/apply list and group into sub-configs (`automation`, `gen3d`, `openai`).
  - Done when: adding a config field requires changing one obvious place; behavior remains deterministic.

## 6) App wiring / plugins

- [ ] **Modularize `src/app.rs` systems into plugins**
  - Why: `app.rs` is dense with `add_systems` calls; difficult to see feature boundaries and scheduling.
  - Done when: each feature area (scene store, automation, gen3d, object visuals, etc.) owns its plugin wiring, with ordering constraints documented in code.

## 7) Repo hygiene: tests, fixtures, and ignored dirs

- [ ] **Normalize “test” directory usage**
  - Why: repo contains `test/`, `tests/`, and `game_test/` with overlapping purposes; `.gitignore` currently ignores `tests` even though `tests/*` is tracked.
  - Done when: there is one canonical place for fixtures/artifacts, and ignore rules match what’s tracked.

- [ ] **Move ad-hoc runtime artifacts out of versioned dirs**
  - Why: `test/gen3d_real/*` includes logs/artifacts that look like outputs; keep only minimal fixtures under version control.
  - Done when: fixtures live under a clear `tests/fixtures/...` (or similar) and outputs go to ignored temp dirs.

## 8) Documentation alignment

- [ ] **Fix README drift vs current behavior**
  - Why: some README statements appear to disagree with newer Gen3D docs (e.g., fixed-joint rotation handling).
  - Done when: README is accurate and high-level; deeper details live in `docs/`.
