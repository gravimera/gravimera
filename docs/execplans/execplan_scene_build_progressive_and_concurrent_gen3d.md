# Progressive Scene Build + Concurrent Gen3D (Shared AI Limiter)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Players should be able to *see* the scene being built progressively instead of waiting for a single “all at once” result. While that build is running, the player should also be able to use Gen3D to create new units/buildings without breaking the scene build workflow.

After this change:

- Pressing **Scene → Build** starts a multi-step scene build where each step applies a patch + compiles immediately, so objects appear in-world progressively.
- The Scene panel shows which step is currently running and brief progress text.
- Gen3D can be used during scene build. Scene build step prompts refresh the prefab catalog every step, so newly created prefabs become available to later steps.
- A shared in-process AI request limiter prevents unbounded concurrent HTTP requests across Gen3D and Scene Build.

## Progress

- [x] (2026-02-15) Add ExecPlan for progressive Scene Build + concurrency.
- [x] (2026-02-15) Add a shared AI request limiter used by Scene Build and Gen3D.
- [x] (2026-02-15) Refactor `src/scene_build_ai.rs` into a multi-step state machine: cleanup → plan → steps → done.
- [x] (2026-02-15) Show step-oriented progress in the Scene UI (`src/scene_authoring_ui.rs`).
- [x] (2026-02-15) Run formatting/tests/smoke and update README usage notes.
- [x] (2026-02-15) Commit with a clear message.

## Surprises & Discoveries

- Observation: (none yet)
  Evidence: (n/a)

## Decision Log

- Decision: Use a multi-step build loop driven by multiple OpenAI calls (plan + per-step patch ops), rather than streaming JSON from a single response.
  Rationale: Step boundaries make the build debuggable, resumable, and easy to persist on disk; Bevy world changes happen only on the main thread.
  Date/Author: 2026-02-15 / Codex

- Decision: Use “layer ownership” to keep step outputs isolated (AI writes only `ai_` layers).
  Rationale: Scene generation must not overwrite human-edited layers, and step isolation improves debugging and allows safe regeneration.
  Date/Author: 2026-02-15 / Codex

- Decision: Implement a shared in-process AI request limiter and use existing config `gen3d.max_parallel_components` to size it (`+1` headroom).
  Rationale: Avoid adding new config parsing right now while still preventing runaway concurrency across multiple jobs.
  Date/Author: 2026-02-15 / Codex

## Outcomes & Retrospective

- Scene Build now runs as a multi-step plan + per-step patch loop that applies + compiles after each step, so world changes appear progressively.
- Scene Build and Gen3D share a global in-process AI permit limiter; both display 'Waiting for AI slot…' while queued.
- Scene Build persists per-run progress (`progress.txt`, `progress.log`) and per-step LLM artifacts under `runs/<run_id>/llm/` for debugging/resume.
- Verified with `cargo fmt`, `cargo test`, and headless smoke (`cargo run -- --headless --headless-seconds 1`).

## Context and Orientation

Relevant parts of the repo:

- `src/scene_build_ai.rs`: current “Scene Builder” AI build logic (one-shot LLM call then apply+compile). Needs refactor to multi-step.
- `src/scene_authoring_ui.rs`: Scene panel UI. Already displays a single-line build progress string.
- `src/scene_runs.rs`: deterministic apply+compile step runner that persists `runs/<run_id>/steps/<NNNN>/` artifacts on disk.
- `src/gen3d/ai/openai.rs`: Gen3D OpenAI HTTP calls (Responses API, uses curl).
- `src/app.rs`: app initialization for rendered/headless mode; good place to size/init global limiter.

Important terms:

- “Scene sources”: file-based, JSON scene representation stored under `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/src/`.
- “AI layers”: layers whose `layer_id` starts with `ai_`. These are owned by generation and may be regenerated.
- “Run”: a persisted folder `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/runs/<run_id>/` containing step artifacts.

## Plan of Work

1. Add a new module `src/ai_limiter.rs` implementing a simple global blocking permit system (Mutex + Condvar). It returns an RAII permit that releases on drop.

2. Initialize the limiter capacity during app startup in `src/app.rs` (both rendered and headless paths). Capacity is derived from the existing config field `gen3d_max_parallel_components` plus one extra slot.

3. Integrate the limiter into Gen3D OpenAI calls:

   - In `src/gen3d/ai/openai.rs`, acquire a permit before running any curl request (at least the POST to `/responses`, and ideally also the GET polling requests).

4. Refactor `src/scene_build_ai.rs` into a multi-step build state machine:

   - When Build starts: create run dir, initialize progress, set job phase to `Cleanup`.
   - `Cleanup`: deterministic step that deletes existing `ai_` layers via `scene_run_apply_patch_step` (so the world clears immediately and the run tracks it).
   - `Plan`: spawn an OpenAI call to produce a JSON plan with N steps (`step_id`, `title`, `goal`). Persist plan artifacts under `runs/<run_id>/llm/plan/`.
   - For each step: spawn an OpenAI call that returns JSON `{ summary, ops[] }` where `ops` matches `SceneSourcesPatchOpV1` JSON (limited to `upsert_layer`/`delete_layer`). Wrap these ops into a `SceneSourcesPatchV1`, apply it via `scene_run_apply_patch_step`, and compile so objects appear.
   - Before spawning each step call, rebuild the prefab catalog from the current `ObjectLibrary` so any newly generated Gen3D prefabs are visible to later steps.
   - Persist per-step LLM artifacts under `runs/<run_id>/llm/step_<NNNN>/`.

5. UI: keep using the existing progress line, but ensure the runtime progress string includes `step i/N` and the step title/phase.

6. Validation and docs:

   - Run `cargo fmt`, `cargo test`, and headless smoke (`cargo run -- --headless --headless-seconds 1`).
   - Update `README.md` to mention progressive build behavior and where to find per-step artifacts.

## Concrete Steps

Run from the repo root:

    cargo fmt
    cargo test
    cargo run -- --headless --headless-seconds 1

Manual rendered-mode check:

    cargo run

Then:

- Open the Scene panel.
- Enter a scene description.
- Press Build.
- Observe the progress line updating through phases and objects appearing progressively.
- While build is running, start a Gen3D job and confirm it still runs; later scene build steps should include the new prefab in the catalog (visible via `runs/<run_id>/llm/step_<NNNN>/user.txt`).

## Validation and Acceptance

Acceptance is met when:

- Pressing **Build** causes the world to update more than once during a single build run (objects appear progressively, not only at the end).
- Scene build produces multiple step folders under `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/runs/<run_id>/steps/` and multiple LLM artifact folders under `.../llm/`.
- Gen3D can be started while scene build is running without crashing or deadlocking.
- Logs show per-phase scene build messages and are written to the configured log sink (see `[log].path`).

## Idempotence and Recovery

- Re-running Build is safe: it creates a new `run_id` and tracks all modifications under that run.
- If the app crashes mid-run, artifacts up to the last completed `steps/<NNNN>/complete.json` remain on disk for debugging.

## Artifacts and Notes

Expected on-disk layout for a run:

- `runs/<run_id>/progress.txt` (latest progress message)
- `runs/<run_id>/progress.log` (append-only progress history)
- `runs/<run_id>/llm/plan/` (plan request/response)
- `runs/<run_id>/llm/step_0001/` etc (step request/response)
- `runs/<run_id>/steps/0001/` etc (apply+compile step artifacts)
