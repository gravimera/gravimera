# Gen3D: Language-agnostic attack intent + non-blocking motion-validation warnings

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gen3D runs currently have two failure modes that show up together in “attacking creature” edits:

1) The system’s “does the prompt require attack capability?” logic is a small English-only keyword heuristic. Non‑English prompts (including Chinese) do not trigger it, so the run can finish with attack motion clips but without the root “attack profile” that gameplay needs to actually attack.

2) Warn-level motion validation findings (example: `attack_self_intersection`) are currently surfaced as `capability_gaps` in `qa_v1`, which makes agents/pipeline treat them like action items. This leads to repeated `llm_generate_motions_v1` ↔ `qa_v1` loops despite `qa_v1.ok=true` and `errors=0`.

After this change:

- “Attack required by prompt” becomes **language-agnostic** and **non-heuristic**, so a Chinese prompt like “会攻击的双头龙…” correctly requires attack capability.
- Warn-only motion validation findings remain visible as `warnings`, but they no longer appear as `capability_gaps`, so agents/pipeline stop trying to “fix” them indefinitely.

How to see it working (after implementation):

- Start a Gen3D seeded edit run on a prefab that is movable but lacks a root attack profile, with a non‑English prompt that clearly requests attacks (Chinese is the motivating example).
- Run `qa_v1` after motion authoring. If attack capability is required, `qa_v1` must report an **error** until the draft has a root attack profile. Once the profile is present, `qa_v1.ok=true` may still include warn-only motion validation warnings, but `capability_gaps` must be empty (or at least must contain only severity=`error` blockers).
- Observe that the run does not spend many passes repeatedly re-authoring motion just to chase warn-level findings.


## Progress

- [x] (2026-03-19) Investigated run cache showing repeated motion-authoring ↔ QA loop and documented root causes.
- [x] (2026-03-19) Drafted this ExecPlan.
- [x] (2026-03-19) Implemented prompt-intent extraction (generic, language-agnostic) and plumbed into smoke/QA.
- [x] (2026-03-19) Made motion-validation warn findings non-blocking in `capability_gaps` (errors only).
- [x] (2026-03-19) Added unit tests for the two behaviors.
- [x] (2026-03-19) Updated documentation to clarify `capability_gaps` semantics (errors-only) and prompt-intent behavior.
- [x] (2026-03-19) Ran `cargo test` and the rendered smoke test (`cargo run -- --rendered-seconds 2`).


## Surprises & Discoveries

- Observation (pre-fix): `attack_required_by_prompt` was computed by an English-only keyword heuristic.
  Evidence (pre-fix): `src/gen3d/ai/orchestration.rs::build_gen3d_smoke_results` lowercased the raw prompt and checked a small set of substrings (`"can attack"`, `"weapon"`, `"gun"`, …).

- Observation (pre-fix): Warn-level `motion_validation` issues were surfaced as `capability_gaps`, not only as `warnings`.
  Evidence (pre-fix): `src/gen3d/ai/agent_tool_dispatch.rs::build_capability_gaps_from_smoke_v1` converted every `smoke.motion_validation.issues[]` entry into a `{"kind":"motion_validation_error", ...}` gap regardless of severity.

- Observation: The agent prompt summary prints `capability_gaps=N` prominently for `qa_v1` tool results, which nudges agent behavior into a “fix gaps” loop.
  Evidence: `src/gen3d/ai/agent_prompt.rs` prints `capability_gaps={capability_gaps}` for `qa_v1` results and also prints the first warning example.

- Observation: `smoke.attack_present` means “root attack profile exists”, not “attack animation exists”.
  Evidence: In smoke results, `attack_present` is computed from `draft.root_def().attack.is_some()` (see `src/gen3d/ai/orchestration.rs`). Motion channels (like `attack_primary`) are summarized separately in `state_summary.motion_coverage`.

- Observation: In the motivating run cache (`~/.gravimera/cache/gen3d/bee02033-4c02-494a-ac12-1458700996e5/attempt_0/pass_15/smoke_results.json`), `attack_required_by_prompt=false` and `attack_present=false` even though the user prompt asks for attacks and `attack_primary` motion is authored.
  Evidence: That smoke file shows `mobility_present=true`, `attack_present=false`, `attack_required_by_prompt=false`, plus a warn-only `attack_self_intersection` issue.


## Decision Log

- Decision: Replace the English keyword heuristic with a generic, language-agnostic “prompt intent” derived from an LLM structured output (computed once per run and cached).
  Rationale: A hardcoded keyword list is not generic and fails for non-English prompts. Gen3D already depends on LLM calls; a small, strict structured-output intent extraction is consistent with the project direction and handles any language.
  Date/Author: 2026-03-19 / user + assistant

- Decision: Treat warn-level motion validation findings as informational only: keep them in `qa_v1.warnings`, but do not include them in `qa_v1.capability_gaps`.
  Rationale: The Gen3D agent system prompt already says “do not chase warn-only motion_validation issues”, but surfacing them as “capability gaps” contradicts that and causes loops. Capability gaps should represent blockers (severity=`error`) or items with deterministic fixits that the engine expects the agent/pipeline to apply.
  Date/Author: 2026-03-19 / user + assistant

- Decision: Keep the existing JSON field name `attack_required_by_prompt` for now, but change its implementation to be intent-driven (not keyword-driven).
  Rationale: Backwards compatibility is not a hard requirement, but reusing the field minimizes code churn across smoke/QA consumers. If the field becomes misleading later, we can rename in a follow-up once the new intent struct is stable.
  Date/Author: 2026-03-19 / user + assistant


## Outcomes & Retrospective

- (TBD) After implementation, summarize the observed reduction in passes on real run caches and confirm the new behavior with both unit tests and a manual Gen3D run.


## Context and Orientation

Gen3D’s “smoke” and “QA” outputs are the core signals used by both the agent-step orchestrator and the deterministic pipeline to decide what to do next.

- Smoke results are built in `src/gen3d/ai/orchestration.rs::build_gen3d_smoke_results(...)`. Smoke includes:
  - `attack_required_by_prompt` (currently heuristic),
  - `mobility_present` and `attack_present` derived from root draft fields,
  - `motion_validation` issues and `ok`.

- `qa_v1` is implemented in `src/gen3d/ai/agent_tool_dispatch.rs::execute_qa_v1`. It runs:
  - `validate_v1` (structural validation),
  - `smoke_check_v1` (smoke + motion validation),
  - then aggregates issues into `errors` and `warnings`.
  - It also builds and returns `capability_gaps` via `build_capability_gaps_from_smoke_v1(...)`.

- The agent prompt that the LLM sees includes a compact “Recent tool results” summary. This summary is built in `src/gen3d/ai/agent_prompt.rs` and prints `capability_gaps=N` for QA results, which makes “gaps” psychologically “must fix”.

Run artifacts for debugging live under:

    ~/.gravimera/cache/gen3d/<run_id>/attempt_0/pass_N/

Key artifacts include:

    smoke_results.json
    qa.json
    gen3d_run.log

The motivating run id for this ExecPlan is:

    bee02033-4c02-494a-ac12-1458700996e5


## Plan of Work

### 1) Add a cached, explicit “prompt intent” to Gen3D job state

Introduce a small struct on `Gen3dAiJob` (file: `src/gen3d/ai/job.rs`) that represents what the user asked for in terms of high-level capabilities.

Name suggestion (final name is not important; consistency is):

- `Gen3dPromptIntentV1` with at least:
  - `requires_attack: Option<bool>` (None means “unknown / not inferred yet”)
  - `requires_mobility: Option<bool>` (optional, but useful for future consistency)
  - `evidence: Option<String>` (short, user-safe explanation for logs/debugging; keep it bounded)
  - `model: Option<String>` + `computed_at_ms: Option<u64>` (optional observability)

Store it on `Gen3dAiJob` as `job.prompt_intent: Gen3dPromptIntentV1` and ensure it is reset when starting a new run (new Build, new seeded Edit/Fork session). For resume/continue, preserve it.

Expose it in `state_summary` (built in `src/gen3d/ai/agent_prompt.rs`) so the agent and debugging logs can tell whether intent was computed and what it decided.

### 2) Implement a generic intent extractor using strict structured output

Add a small LLM-backed call that converts `job.user_prompt_raw` (+ optional image summary) into `Gen3dPromptIntentV1`.

Important constraints:

- This must be language-agnostic: it should work for Chinese prompts without any keyword lists.
- It must be strict JSON with `additionalProperties=false` and explicit booleans.
- It must be cached: compute at most once per run unless the user prompt changes.
- It must be safe on failure: if the call fails, leave `requires_attack=None` and continue; downstream logic must behave sensibly when intent is unknown.

Integration point (recommended):

- Compute intent early in the run, before the first time we rely on `attack_required_by_prompt` in smoke/QA gating. A practical place is the run start entrypoints in `src/gen3d/ai/orchestration.rs` (new Build / seeded edit start), or as a first pipeline stage in `src/gen3d/ai/pipeline_orchestrator.rs` (if pipeline mode is enabled).

Avoid computing intent inside `qa_v1` itself. `qa_v1` is currently deterministic and called frequently; making it spawn LLM calls would increase cost and add surprise latency.

### 3) Make smoke/QA use intent instead of prompt heuristics

Update `src/gen3d/ai/orchestration.rs::build_gen3d_smoke_results` so:

- It no longer uses the substring keyword heuristic.
- It sets `attack_required_by_prompt` from `job.prompt_intent.requires_attack.unwrap_or(false)` (or equivalent).
- It continues to report `attack_present` as “root attack profile exists”.

If `requires_attack` is unknown (`None`), keep `attack_required_by_prompt=false`, and consider adding a smoke `issues[]` warning indicating “prompt intent unknown; attack requirement not enforced” so it is visible in logs but not treated as a blocker.

### 4) Make motion-validation warnings non-blocking in `capability_gaps`

Update `src/gen3d/ai/agent_tool_dispatch.rs::build_capability_gaps_from_smoke_v1`:

- When iterating `smoke.motion_validation.issues[]`, only include entries with `severity="error"` as `capability_gaps`.
- Continue to include existing “hard blockers” gaps like:
  - missing move channel for movable units,
  - missing collider for movable units,
  - missing root mobility/attack when `attack_required_by_prompt=true`.

This ensures `qa_v1.capability_gaps` represents “you must do something” rather than “here is some quality information”.

### 5) Tighten prompt summary so it matches the new meaning of “capability gaps”

Once warn-only motion issues are no longer included as gaps, the existing prompt summary formatting in `src/gen3d/ai/agent_prompt.rs` should become less misleading automatically. Still, verify the following:

- `qa_v1` summary lines should show `capability_gaps=0` when QA is ok and only warn-level motion validation exists.
- The first warning example should remain, but it should not be the only reason the agent keeps iterating. If we still observe looping, consider additionally suppressing warn examples for `source="motion_validation"` when `qa.ok=true` (optional follow-up; not required unless loops persist).

### 6) Tests and regression coverage (no OpenAI API calls)

Add unit tests that do not call external services:

1) Capability gap filtering:
   - Construct a smoke JSON with `motion_validation.issues=[{severity:\"warn\", kind:\"attack_self_intersection\", ...}]` and assert that `build_capability_gaps_from_smoke_v1(...)` does not produce any `motion_validation_error` gaps.
   - Construct a smoke JSON with an error-level motion issue (example: `hinge_limit_exceeded` from existing tests) and assert it still produces a gap and includes deterministic fixits when available.

2) Smoke uses intent:
   - Add a small helper to build smoke results that accepts a `Gen3dPromptIntentV1` directly in tests (or set `job.prompt_intent` and call the smoke builder through the normal path).
   - Verify that a Chinese prompt can produce `attack_required_by_prompt=true` when intent says `requires_attack=true` (this validates “language-agnostic” at the integration layer without needing to call the model).

Optional but high value:

- Add a regression test around the motivating scenario: a movable draft without root attack profile + `requires_attack=true` should produce a smoke/QA error about missing attack capability. This protects the gameplay correctness, not only the string parsing.

### 7) Documentation updates

Update the docs that describe how `capability_gaps` is meant to be used, especially:

- `docs/execplans/gen3d_deterministic_pipeline.md` (it currently suggests using `capability_gaps` for motion validation without distinguishing warn vs error).

Document the new behavior:

- `capability_gaps` should be interpreted as **blockers** (severity=`error`) and/or items with deterministic fixits.
- Warn-level motion validation findings are informational and appear only under `warnings`.

### 8) Validation and acceptance (repo-level)

From repo root:

1) Run unit tests:

    cargo test

2) Run the required rendered smoke test (non-headless) to ensure the game starts:

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

Acceptance criteria:

- A Gen3D run prompted in Chinese for an attacking creature can no longer “silently” skip attack capability requirement because of language.
- `qa_v1.capability_gaps` is empty when the only remaining motion validation findings are warn-level (e.g. `attack_self_intersection`).
- The agent/pipeline no longer loops on motion authoring just to chase warn-level motion-validation findings.


## Idempotence and Recovery

This change is safe to iterate on:

- The new intent inference should be cached per run; if caching is incorrect, add a “force recompute” debug knob that is only enabled in developer builds (optional).
- Capability gap filtering is a pure change to classification; it does not mutate drafts. If something breaks, the previous behavior can be restored by reverting the filter (but this should not be necessary once tests are added).


## Artifacts and Notes

Motivating run artifact paths (for manual verification during implementation):

    /Users/flow/.gravimera/cache/gen3d/bee02033-4c02-494a-ac12-1458700996e5/attempt_0/pass_15/smoke_results.json
    /Users/flow/.gravimera/cache/gen3d/bee02033-4c02-494a-ac12-1458700996e5/attempt_0/pass_15/qa.json

The last smoke results in that run show:

- `attack_required_by_prompt=false` (due to English-only heuristic)
- `attack_present=false` (root attack profile missing)
- `motion_validation` warn-only `attack_self_intersection`

This ExecPlan’s intent is to make that class of run either:

- enforce attack capability correctly (so QA errors until fixed), and
- avoid turning warn-only motion-validation into “capability gaps” that cause iterative thrash.
