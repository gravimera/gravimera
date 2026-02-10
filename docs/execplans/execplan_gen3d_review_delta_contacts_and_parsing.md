# ExecPlan: Gen3D Review-Delta Robustness + Contact Stance Repairs

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D builds often fail to converge when motion validation reports `contact_slip` / `contact_lift` for “stance” contacts (usually feet). In theory, `llm_review_delta_v1` should repair these by tweaking animations, but in practice the review-delta output is frequently “almost correct JSON” that fails strict parsing (missing required fields like `spec.driver` or `loop.duration_secs`, or using slightly wrong enum spellings). When parsing fails, the engine cannot apply any repairs and the run wastes multiple review passes.

After this change:

1) `llm_review_delta_v1` has an explicit, machine-appliable way to fix stance-related motion failures without regenerating geometry: it can **clear or adjust `contacts[].stance`** via a new `tweak_contact` action.

2) Review-delta parsing becomes more robust by running a **deterministic normalization pass** over the JSON value before strict schema deserialization. This accepts common near-miss outputs (missing `driver`, missing `duration_secs`, driver synonyms like `move_cycle`) without making the schema “loose” or adding heuristics to Gen3D geometry generation.

User-visible outcomes:

- Fewer “auto-review failed (parse error)” build endings.
- Motion-validation failures caused by incorrect stance declarations can be repaired generically (either by fixing the animation or by clearing stance).
- Reduced token/time waste from repeated re-asks when the model output is structurally correct but slightly incomplete.

## Progress

- [x] (2026-02-10) Write and check in this ExecPlan.
- [x] (2026-02-10) Implement `tweak_contact` review-delta action (set/clear stance).
- [x] (2026-02-10) Normalize review-delta JSON to reduce parse failures (driver/duration/enum synonyms).
- [x] (2026-02-10) Update prompts to teach `tweak_contact` and stance repair strategy.
- [x] (2026-02-10) Add unit tests for parsing normalization + contact tweak application.
- [x] (2026-02-10) Run `cargo test` and a headless smoke start.
- [ ] Run a real rendered Gen3D regression and record the run id + results.

## Surprises & Discoveries

- Observation: Disabling a leg’s `move` animation does not automatically fix `contact_slip` when `stance` is present.
  Evidence: Contact slip validation simulates forward root motion; for a planted foot, the limb animation must counteract root translation during stance. Therefore, if the stance contract cannot be satisfied, **clearing stance** is the minimal generic repair.

- Observation: Review-delta outputs frequently omit required fields even when the intent is clear.
  Evidence: Common failures include missing `spec.driver` in `tweak_animation` and missing `loop.duration_secs` in `clip.kind=="loop"`, which causes strict schema parsing failures.

## Decision Log

- Decision: Add an explicit review-delta action (`tweak_contact`) to set/clear stance instead of inferring stance changes automatically from validation errors.
  Rationale: Automatically clearing stance based on validator output would be heuristic behavior in the engine. A review-delta action keeps the decision AI-driven and explicit.
  Date/Author: 2026-02-10 / Codex

- Decision: Implement deterministic JSON normalization in the review-delta parser rather than loosening serde structs (e.g., making `driver` optional everywhere).
  Rationale: Keep the schema strict as the source of truth, but accept common, unambiguous near-miss outputs in a controlled way. This improves robustness without allowing arbitrary unknown shapes.
  Date/Author: 2026-02-10 / Codex

## Outcomes & Retrospective

- Implemented `review_delta_v1` improvements:
  - New `tweak_contact` action can clear or set `contacts[].stance` (enables fixing `contact_slip`/`contact_lift` without regenerating geometry).
  - Review-delta parser now normalizes common near-misses for `tweak_animation`:
    - fills missing `spec.driver` based on `channel`
    - fills or fixes missing/invalid `loop.duration_secs`
    - infers/normalizes `clip.kind`
- Added unit tests:
  - `gen3d::ai::parse::tests::normalizes_review_delta_missing_driver_and_duration`
  - `gen3d::ai::convert::tests::applies_review_delta_tweak_contact_clears_stance`

Remaining work:

- Run a real rendered Gen3D regression to measure impact on actual agent runs and record run ids here.

## Context and Orientation

Key files and concepts:

- Review-delta schema: `src/gen3d/ai/schema.rs` (`AiReviewDeltaJsonV1`, `AiReviewDeltaActionJsonV1`).
- Review-delta parsing: `src/gen3d/ai/parse.rs` (`parse_ai_review_delta_from_text`).
- Applying review-delta actions: `src/gen3d/ai/convert.rs` (`apply_ai_review_delta_actions`).
- Motion validation: `src/gen3d/ai/motion_validation.rs` (`validate_contacts` emits `contact_slip` / `contact_lift` only when a contact has `stance`).
- Prompting:
  - Plan prompt includes `contacts[].stance`: `src/gen3d/ai/prompts.rs` `build_gen3d_plan_system_instructions`.
  - Review-delta prompt schema: `src/gen3d/ai/prompts.rs` `build_gen3d_review_delta_system_instructions`.

Definitions (as used in this repo):

- `contacts[]`: plan-level declaration of named contact points (usually ground contacts) by anchor name.
- `stance`: optional schedule (`phase_01`, `duty_factor_01`) declaring when a contact is intended to be planted during the move cycle.
- `contact_slip`: motion-validation issue indicating the declared contact moves too much in world XZ during stance.
- `tweak_animation`: review-delta action that edits attachment animation specs for one component’s attachment to its parent.

## Plan of Work

1) Add a new review-delta action in the schema: `tweak_contact`.

   - In `src/gen3d/ai/schema.rs`, extend `AiReviewDeltaActionJsonV1` with:
     - `component_id` (UUID string, same targeting scheme as other actions)
     - `contact_name` (string; matches `components[].contacts[].name`)
     - `set` object that can update:
       - `stance` as a tri-state: absent = no change; `null` = clear; object = set
     - optional `reason`

2) Apply `tweak_contact` in the engine.

   - In `src/gen3d/ai/convert.rs` `apply_ai_review_delta_actions`, locate the targeted component + contact and apply `set.stance`.
   - Treat an unknown `component_id` or unknown `contact_name` as a no-op (consistent with other review actions).

3) Harden review-delta parsing with deterministic normalization.

   - In `src/gen3d/ai/parse.rs` `parse_ai_review_delta_from_text`:
     - Before deserializing, traverse the JSON and normalize common near-misses:
       - For `tweak_animation` actions:
         - If `spec.driver` is missing, infer it from `channel`:
           - `move` -> `move_phase`
           - `attack_primary` -> `attack_time`
           - `idle` / `ambient` -> `always`
         - If `spec.driver` is a common synonym (e.g. `move_cycle`), rewrite to canonical (`move_phase`).
         - If `spec.clip.kind=="loop"` and `duration_secs` is missing, set `duration_secs` to:
           - the max `keyframes[].time_secs` (if any, and > 0), else `1.0`.
         - If `spec.clip.kind` is missing but the object looks like a loop/spin (fields present), set it.
     - Keep `deny_unknown_fields` for the actual schema; normalization should only make small, local edits.

4) Update review-delta system instructions to teach `tweak_contact`.

   - In `src/gen3d/ai/prompts.rs` `build_gen3d_review_delta_system_instructions`:
     - Document the new action kind + schema.
     - Add a motion-validation hint:
       - If `contact_slip` / `contact_lift` persist and you cannot author a truly planted foot motion, clear stance for that contact instead of repeatedly tweaking animations.

   - In `build_gen3d_plan_system_instructions`, strengthen the guidance on `contacts[].stance`:
     - Only declare stance when the move animation is authored to keep the contact planted in world space during stance.

5) Add unit tests.

   - In `src/gen3d/ai/parse.rs` tests:
     - Verify a review-delta with `tweak_animation` missing `spec.driver` and/or missing loop `duration_secs` parses successfully after normalization.
   - In `src/gen3d/ai/convert.rs` tests (or a small new test module):
     - Verify `tweak_contact` can clear stance (`"stance": null`) on a planned component contact.

6) Validate and regression test.

   - Run `cargo fmt`, `cargo test`.
   - Run a headless smoke start: `cargo run -- --headless --headless-seconds 3`.
   - Run a real rendered Gen3D regression (requires GPU + config with OpenAI key), using:

       python3 tools/gen3d_real_test.py --config tests/gen3d/config.toml --prompt "A voxel octopus robot with 8 evenly spaced radial legs; legs are repeated; include move animation."

     Confirm in the run artifacts that:
     - Review-delta does not fail with missing driver/duration parsing errors.
     - If contact stance is incompatible, reviewer can clear it using `tweak_contact` and validation passes.

## Concrete Steps

All commands should be run from repo root.

1) Implement code changes + format:

    cargo fmt

2) Unit tests:

    cargo test

3) Headless smoke start:

    cargo run -- --headless --headless-seconds 3

4) (Optional, rendered) Gen3D regression:

    OPENAI_API_KEY=... python3 tools/gen3d_real_test.py --config tests/gen3d/config.toml --reset-scene --prompt "A voxel octopus robot with 8 evenly spaced radial legs; legs are repeated; include move animation."

## Validation and Acceptance

Acceptance is satisfied when:

- `cargo test` passes (including new parser/action tests).
- Headless smoke start runs and exits cleanly.
- In a real rendered run, `llm_review_delta_v1` no longer frequently fails on missing `driver` / missing `duration_secs`, and stance-related motion failures can be repaired using `tweak_contact`.

## Idempotence and Recovery

- Parsing normalization is deterministic; rerunning builds should not change results except by enabling previously-blocked deltas to be applied.
- If normalization causes an unintended behavior, it can be disabled by removing the normalization pass while keeping the schema strict.

## Artifacts and Notes

Record real rendered run ids here once executed.
