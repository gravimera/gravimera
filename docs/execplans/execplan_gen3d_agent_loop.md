# Gen3D: Agent Loop (Iterative Review + Smoke Checks + Robust Deltas)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

After this change, Gen3D no longer relies on a single “perfect” AI response that must contain complete and accurate JSON in one shot. Instead, each Build starts an automated “agent loop” that repeatedly:

1. Generates a component plan and component drafts.
2. Assembles the draft in-engine.
3. Runs deterministic validations and lightweight smoke checks.
4. Renders a fixed set of review images (static + motion).
5. Asks a review AI for a strict, machine-appliable delta (“review_delta_v1”) to fix problems.

The player experience remains simple: click **Build** (or **Stop**). The extra iterations are automatic and bounded by budgets, with clear “best effort” messaging if budgets are exhausted.

You can see this working by:

1. Running the game and entering Gen3D.
2. Dropping 0–3 images (<5 MiB each) and/or typing a prompt (e.g. “goblin with spear, can attack” or “small wooden chair”).
3. Clicking **Build** and waiting for it to finish.
4. Observing in the terminal debug logs: multiple “pass” steps (render → validate → review → apply delta).
5. Inspecting the cache folder: `gen3d_cache/<run_id>/attempt_0/pass_0/...` with artifacts, then additional `pass_1/...` if review iterations are enabled.

## Progress

- [x] (2026-01-31 23:35Z) Write this ExecPlan and keep it self-contained.
- [x] (2026-02-01 00:10Z) Add `review_delta_v1` schema + parser + prompts and wire them into the Gen3D loop.
- [x] (2026-02-01 00:10Z) Implement cache layout `gen3d_cache/<run_id>/attempt_N/pass_M/` and store per-pass artifacts in the current `pass_M` directory.
- [x] (2026-02-01 01:30Z) Implement fixed per-pass artifacts and smoke checks:
  - Static renders (7): front, front_left, left_back, back, right_back, front_right, top
  - Motion sprite-sheets: `move_sheet.png`, `attack_sheet.png` (2×2 frames at t=0/25/50/75%)
  - `scene_graph_summary.json` and `smoke_results.json`
- [x] (2026-02-01 00:10Z) Wire the Gen3D agent loop state machine (budgets, replan, regen, and 1-step JSON repair).
- [ ] Validation: `cargo test` and `cargo run -- --headless --headless-seconds 1`.
- [ ] Update `README.md` (doc the agent loop and cache layout) and commit.

## Surprises & Discoveries

- Observation: (to fill)
  Evidence: (to fill)

## Decision Log

- Decision: Use a strict “review delta” action protocol rather than asking the AI to output full final plans repeatedly.
  Rationale: Small typed actions are easier to validate, apply, retry/repair, and keep deterministic.
  Date/Author: 2026-01-31 / Codex

- Decision: Store artifacts under `attempt_N/pass_M/` and keep each pass self-contained.
  Rationale: Replans and retries become debuggable without overwriting prior evidence.
  Date/Author: 2026-01-31 / Codex

## Outcomes & Retrospective

(To fill at completion.)

## Context and Orientation

Gen3D code lives in `src/gen3d/`.

Key files:

- `src/gen3d/ai/mod.rs`: Bevy systems + Gen3D AI orchestration state machine.
- `src/gen3d/ai/openai.rs`: OpenAI Requests (/responses + polling) + Chat Completions fallback, artifact capture.
- `src/gen3d/ai/prompts.rs`: prompt construction for plan/component/review.
- `src/gen3d/ai/schema.rs`: serde JSON structs/enums for plan/draft/review schemas.
- `src/gen3d/ai/parse.rs`: tolerant JSON extraction + parsing helpers.
- `src/gen3d/ai/convert.rs`: plan/draft → runtime conversion helpers.
- `src/gen3d/preview.rs`: preview panel “studio” scene (no floor) and review overlay layer.
- `src/object/visuals.rs`: attachment animation runtime (channels + drivers).
- `src/rts.rs` / `src/locomotion.rs` / `src/combat.rs`: unit movement and attacks for saved Gen3D units.

Terminology:

- **Run**: one click of Build, creating `gen3d_cache/<run_id>/`.
- **Attempt**: a plan-generation attempt. A replan creates `attempt_1/`, etc.
- **Pass**: a review iteration. Each pass produces a render bundle + smoke bundle and requests a review delta.
- **Draft**: a temporary prefab built from primitives and attachments, shown in the Gen3D preview panel.
- **Review delta**: strict JSON describing edits (actions) to apply to the assembled plan/draft.

## Plan of Work

### 1) Add new review delta schema and parser

Update `src/gen3d/ai/schema.rs`:

- Introduce `ReviewDeltaV1` with:
  - `version: u32`
  - `applies_to: { run_id, attempt, plan_hash, assembly_rev }`
  - `actions: Vec<ReviewActionV1>`
- Define `ReviewActionV1` action kinds:
  - `accept`
  - `replan` (includes full new plan JSON)
  - `regen_component` (by `component_id`)
  - `tweak_component_transform` (set/delta pos/rot/scale; rotation basis or quat only)
  - `tweak_anchor`, `tweak_attachment`
  - `tweak_mobility`, `tweak_attack`

Update `src/gen3d/ai/parse.rs`:

- Add `parse_review_delta_from_text()` that:
  - extracts a single JSON object from text,
  - parses into `ReviewDeltaV1`,
  - returns a typed schema error string on missing/invalid fields.

### 2) Update prompts to drive the agent workflow

Update `src/gen3d/ai/prompts.rs`:

- Modify plan prompt to include an explicit intent decision (`category`, `mobility_intent`, `attack_intent`) and stable component ids when possible.
- Replace the old review schema instructions with `review_delta_v1` instructions:
  - “Return ONLY valid JSON”
  - “Do not output markdown”
  - “Actions must reference `component_id`”
  - “No Euler rotations in deltas; use forward/up or quaternion”

### 3) Implement the run/attempt/pass cache layout

Update `src/gen3d/ai/mod.rs` and `src/gen3d/ai/artifacts.rs`:

- Create:
  - `gen3d_cache/<run_id>/run.json` (basic run info, prompt, input file list, config snapshot)
  - `attempt_N/inputs/` (cached images + prompt copy + manifest)
  - `attempt_N/pass_M/` (AI requests/responses, extracted JSON, renders, summaries)
- Ensure existing artifacts written by `openai.rs` are directed into the current `pass_dir`.

### 4) Fixed per-pass artifacts (renders + smoke summaries)

Implement per-pass captures in `src/gen3d/ai/mod.rs`:

- Static renders (always):
  - front, front_left, left_back, back, right_back, front_right, top
- Motion sprite-sheets (always generated; may be blank if no animations exist):
  - `move_sheet.png`: 2×2 captures with `moving=true` and locomotion clocks set to represent time samples.
  - `attack_sheet.png`: 2×2 captures with `attacking_primary=true` and an attack clock set to represent time samples.
- Also write:
  - `scene_graph_summary.json` (component list, bounds, anchors, attachments, declared intent)
  - `smoke_results.json` (validation outcomes + mobility/attack applicability and basic results)

Important constraint: do not implement domain-specific “placement heuristics” (no hard-coded “tree branches must be above trunk”). The engine must only validate and normalize generic correctness; the review AI decides what is “logical”.

### 5) Agent loop and convergence

In `src/gen3d/ai/mod.rs`, replace the current “single-pass build” behavior with:

- Attempt 0:
  - plan → plan-fill (if needed) → generate components (parallel)
  - then for pass = 0..(review_passes):
    - assemble + validate + smoke + render bundle
    - request review delta (with original images + fixed render bundle + text summaries)
    - apply delta; if it requests regen, schedule regen and continue
    - if accept, finish early
  - if budgets exhaust, mark “best effort” in status but still allow Save
- If delta requests `replan` and within budget:
  - start `attempt_1` from scratch, reusing cached inputs

Robustness: add a “JSON repair” sub-step:

- If review delta JSON fails validation, ask AI to return corrected JSON with the exact schema errors, retry a small number of times before failing the run.

Budgets and defaults (configurable in `config.toml`):

- `refine_iterations` is interpreted as `review_passes` (0 disables auto-review).
- `gen3d_max_parallel_components` remains.
- New defaults:
  - `max_replans = 1`
  - `max_regen_per_component = 2`
  - `max_regen_total = 16`

### 6) Validation and docs

- Run:
  - `cargo test`
  - `cargo run -- --headless --headless-seconds 1`
- Update `README.md` to document:
  - agent loop behavior
  - cache folder structure `attempt_N/pass_M/`
  - config knobs (`refine_iterations`, regen caps, max replans)

## Concrete Steps

From repo root (`/Users/flow/workspace/github/gravimera`):

1. Implement schema/prompt/parser changes and compile:
   - `cargo test`

2. Implement cache layout and per-pass artifacts:
   - `cargo test`

3. Implement agent loop and budgets:
   - `cargo test`
   - `cargo run -- --headless --headless-seconds 1`

4. Update `README.md` and commit.

## Validation and Acceptance

Acceptance is met when:

- Gen3D Build creates `gen3d_cache/<run_id>/attempt_0/pass_0/` with:
  - the fixed render bundle (`front`, `left_back`, `right_back`, `top`, `move_sheet`, `attack_sheet`)
  - `scene_graph_summary.json` and `smoke_results.json`
  - AI request/response artifacts for plan/component/review.
- When `refine_iterations > 0`, Gen3D performs at least one review pass and applies a delta or accepts.
- `cargo test` passes and the headless smoke test exits cleanly.

## Idempotence and Recovery

- Re-running Gen3D creates new `<run_id>` folders; prior runs are never overwritten.
- If an AI response is malformed, the repair loop retries without crashing the game.
- If a pass fails hard, the run ends with a clear status and preserved artifacts for debugging.

## Artifacts and Notes

(Add any noteworthy logs, failures, or surprising AI behaviors here as implementation progresses.)

## Interfaces and Dependencies

At the end of this change:

- The review AI response format is `review_delta_v1` and is validated strictly before application.
- Gen3D review requests always include:
  - user prompt text + `image_object_summary` (when reference photos are provided; raw photos are not attached)
  - engine-rendered preview PNGs when `[gen3d].review_appearance = true` (static views, plus motion sheets only when smoke/validation indicates motion issues)
  - `scene_graph_summary.json` and `smoke_results.json` embedded as text
- The code paths remain in `src/gen3d/ai/*` (no re-monolithification into a single file).
