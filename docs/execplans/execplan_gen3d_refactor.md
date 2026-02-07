# Gen3D: Codebase Refactor (Structure + Maintainability)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, the Gen3D implementation is easier to understand, change, and debug without changing gameplay behavior. In particular, the large `src/gen3d/ai.rs` file is split into smaller, purpose-focused modules (prompt building, schema, parsing, OpenAI client, plan/draft conversion, cache artifacts, and orchestration).

You can see this working by:

1. Building the project and running the headless smoke test.
2. Entering Gen3D in the rendered game and performing a Build + Save as usual.
3. Observing: Gen3D behavior remains the same, but the code is now navigable by feature area rather than a single giant file.

## Progress

- [x] (2026-01-31 21:20Z) Create this ExecPlan and locate the Gen3D entrypoints and module boundaries.
- [x] (2026-01-31 22:05Z) Split `src/gen3d/ai.rs` into `src/gen3d/ai/` (orchestration in `src/gen3d/ai/mod.rs`).
- [x] (2026-01-31 22:15Z) Move AI JSON schema structs/enums into `src/gen3d/ai/schema.rs`.
- [x] (2026-01-31 22:25Z) Move OpenAI request/response code into `src/gen3d/ai/openai.rs` (Responses API + Chat Completions fallback).
- [x] (2026-01-31 22:35Z) Move prompt construction into `src/gen3d/ai/prompts.rs`.
- [x] (2026-01-31 22:45Z) Move parsing + tolerance helpers into `src/gen3d/ai/parse.rs`.
- [x] (2026-01-31 22:55Z) Move plan/draft → runtime conversion helpers into `src/gen3d/ai/convert.rs`.
- [x] (2026-01-31 23:05Z) Move unit tests into `openai.rs` / `parse.rs` / `convert.rs` and remove the large test module from `src/gen3d/ai/mod.rs`.
- [ ] Keep all tests passing and keep Gen3D behavior unchanged (remaining: rerun after final cleanups).
- [ ] Run `cargo test` and `cargo run -- --headless --headless-seconds 1`.
- [ ] Update `README.md` “Code layout” (paths changed) and commit.

## Surprises & Discoveries

- Observation: The original `src/gen3d/ai.rs` mixed Bevy orchestration with parse/convert helpers, which made it hard to test or navigate.
  Evidence: Splitting into `src/gen3d/ai/{openai,parse,convert,prompts,schema,artifacts}.rs` allows test-only imports without pulling the Bevy scheduling code.

## Decision Log

- Decision: Keep `Gen3dAiJob` orchestration + Bevy systems in `src/gen3d/ai/mod.rs`, and push pure helpers (schema, parsing, prompts, conversion) into submodules.
  Rationale: Keeps Bevy schedule logic in one place while making the “library-like” code testable and reusable.
  Date/Author: 2026-01-31 / Codex

## Outcomes & Retrospective

(To fill at completion.)

## Context and Orientation

Gen3D lives under `src/gen3d/` and is wired into the Bevy app in `src/app.rs`. The public Gen3D entrypoints are re-exported from `src/gen3d/mod.rs`.

Key files today:

- `src/gen3d/mod.rs`: constants + exports of Gen3D systems.
- `src/gen3d/ai/mod.rs`: Gen3D AI orchestration (`Gen3dAiJob`, Bevy systems, job state machine).
- `src/gen3d/ai/schema.rs`: serde structs/enums for AI plan/draft/review JSON.
- `src/gen3d/ai/prompts.rs`: prompt building helpers (`build_gen3d_*`).
- `src/gen3d/ai/openai.rs`: OpenAI HTTP plumbing (Responses API + polling, Chat Completions fallback).
- `src/gen3d/ai/parse.rs`: tolerant parsing helpers for non-conforming AI JSON outputs.
- `src/gen3d/ai/convert.rs`: plan/draft → runtime conversion helpers (attachments, colliders, animation slots, etc).
- `src/gen3d/ai/artifacts.rs`: cache folder artifacts (logs, JSON snapshots, screenshots).
- `src/gen3d/ui.rs`: Gen3D UI layout and interactions (prompt/status/panels/buttons).
- `src/gen3d/preview.rs`: preview panel render world + orbit controls + collision overlay.
- `src/gen3d/images.rs`: drag/drop images, thumbnails, image viewer, scrollbars.
- `src/gen3d/state.rs`: Gen3D resources and UI state structs.
- `src/gen3d/save.rs`: “Save” button conversion: remap object ids, spawn next to hero, persist to `scene.dat`.

Terminology used here:

- **Plan**: The AI’s high-level component/anchor/attachment tree that defines how components connect (no absolute placement rules).
- **Draft**: The AI’s component geometry expressed as primitives + anchors, which becomes a runtime `ObjectDef`.
- **Artifact**: A debug output written into `gen3d_cache/<run_id>/` such as requests/responses, extracted JSON, screenshots, and logs.

## Plan of Work

This refactor is intentionally behavior-preserving. The goal is to move code without changing logic, except where small adapter glue is needed to break dependency cycles.

### 1) Introduce an `ai/` module directory and keep the public API stable

Change `src/gen3d/mod.rs` to keep `pub(crate) use ai::{gen3d_generate_button, gen3d_poll_ai_job, Gen3dAiJob};` working, but switch `mod ai;` to load from `src/gen3d/ai/mod.rs`.

### 2) Split `ai.rs` into submodules

Create these new files under `src/gen3d/ai/` and move code from `ai.rs` into them:

- `mod.rs`: Bevy systems + orchestration + `Gen3dAiJob` resource public surface.
- `schema.rs`: all `Ai*Json*` structs/enums used for serde parsing.
- `prompts.rs`: `build_gen3d_*_system_instructions()` and `build_gen3d_*_user_text()` helpers.
- `openai.rs`: HTTP request helpers, Responses API polling, Chat Completions fallback, and “extract output text” helpers.
- `parse.rs`: parse helpers that convert output text → JSON structs, plus tolerant parsing adapters.
- `convert.rs`: plan/draft conversion into runtime `ObjectDef` / `Gen3dPlannedComponent` and animation axis conversion glue.
- `artifacts.rs`: writing JSON/text artifacts into the cache folder (requests, responses, extracted plan, assembly snapshot, screenshots list).

When moving code:

- Prefer “move first, then recompile” rather than refactoring logic at the same time.
- Keep function signatures stable where possible to minimize churn.
- If module boundaries create borrow/lifetime issues, prefer passing values (owned strings, simple structs) rather than introducing shared mutable state.

### 3) Keep unit tests close to what they test

If tests currently in `ai.rs` only cover parsing/conversion, move those tests into the module they belong to (`parse.rs` or `convert.rs`), so they compile without needing Bevy orchestration types.

### 4) Validation

Run the repository’s standard checks:

- `cargo test`
- `cargo run -- --headless --headless-seconds 1`

If needed, manually run rendered mode and open Gen3D to ensure the UI still works, but the automated acceptance gate for this ExecPlan is the tests + headless smoke test.

### 5) Documentation

Update `README.md` “Code layout” section if it mentions file paths that change (`src/gen3d/ai.rs` → `src/gen3d/ai/*`).

Commit once the refactor compiles and passes smoke.

## Concrete Steps

From repository root:

1. Create the new module folder and move code in small chunks, recompiling after each chunk:

   - `cargo test`

2. After all moves:

   - `cargo test`
   - `cargo run -- --headless --headless-seconds 1`

## Validation and Acceptance

Acceptance is met when:

- The project compiles and `cargo test` passes.
- The headless smoke test starts and exits cleanly.
- Gen3D public APIs still exist (the exports from `src/gen3d/mod.rs` are unchanged).
- Gen3D cache artifacts still appear in `gen3d_cache/<uuid>/` during a Build.
- No behavior regressions are introduced intentionally (this is a pure refactor).

## Idempotence and Recovery

- This refactor is safe to repeat incrementally as long as each step compiles before proceeding.
- If a module split introduces cyclic dependencies, merge one layer back (e.g., keep `schema` + `parse` together temporarily) and re-split later.

## Artifacts and Notes

(Add any notable compile errors, tricky module boundaries, or file-move gotchas encountered during the refactor.)

## Interfaces and Dependencies

Public Gen3D interface must remain:

In `src/gen3d/mod.rs`:

    pub(crate) use ai::{gen3d_generate_button, gen3d_poll_ai_job, Gen3dAiJob};

The refactor must not change `AppConfig` / OpenAI config formats, nor the `scene.dat` persistence behavior.

## Revision Notes

- (2026-01-31) Updated progress and key-file mapping to reflect the completed module split and test moves, so the ExecPlan remains self-contained and accurate.
