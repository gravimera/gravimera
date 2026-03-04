# Gen3D: “Inherit” Copy (Reuse With Overrides) + Motion-Safe Animation Authoring

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D already has non-heuristic reuse primitives (`reuse_groups`, `copy_component_v1`, `mirror_component_v1`, `copy_component_subtree_v1`, `mirror_component_subtree_v1`) to avoid regenerating repeated parts (wheels, legs, arms). In practice, users still see:

- Copy failures when the planned subtrees are “mostly the same” but not identical in non-geometry fields (different per-leg attachment offsets, per-leg animations, etc.).
- Repeated limbs that are geometrically consistent (copied) but motion-invalid (hinge limits exceeded, stance never plants, legs end up under ground after saving/spawning).
- AI “near-miss JSON” that is valid JSON but not schema-valid, leading to wasted repair loops.

After this change, Gen3D will support a generic concept of **inheritance for repeated parts**: a plan can declare “these targets inherit the same 3D geometry as this source”, while still allowing explicit per-target differences (mount offsets, phase shifts, contact stance, etc.). The engine applies inheritance deterministically and validates the resulting motion against declared joint/contact contracts.

You can see this working by generating a multi-leg creature (8 radial legs, or a quadruped) and observing:

- The cache shows a reuse/inherit plan and deterministic copy application (minimal LLM component generations).
- The saved prefab spawns with feet above/at ground, not sunk below it.
- Motion validation passes (no `hinge_limit_exceeded`, stance contacts exist if declared), and the move gait looks consistent across repeated legs.

This ExecPlan focuses on **generic improvements with a low regression risk**:

- No name-based inference (“leg”, “wheel”, etc.). All behavior is driven by explicit plan fields (anchors, attachments, reuse declarations, joints, contacts) or explicit tool args.
- New schema fields are optional and default to today’s behavior.
- Every behavior change is covered by unit tests and at least one end-to-end rendered Gen3D regression run recorded under `tests/gen3d/cache/gen3d/` (ignored by git).

## Progress

- [x] (2026-02-11) Write and check in this ExecPlan.
- [x] (2026-02-11) Add regression-safety gates + acceptance criteria to this plan.
- [x] (2026-02-11) Implement `time_offset_units` for animation specs (enables deterministic phase offsets without duplicating keyframes).
- [x] (2026-02-11) Make `copy_component_subtree_v1` robust to partially populated targets (deterministic matching by attachment edge keys; clone missing branches when unambiguous).
- [x] (2026-02-11) Update Gen3D prompts + docs to teach “inherit geometry, override offsets/animation is OK” and document `time_offset_units`.
- [x] (2026-02-11) Add/adjust unit tests (animation offset sampling + scene.dat round-trip; subtree copy partial fill).
- [x] (2026-02-11) Run `cargo test` and a headless smoke start.
- [ ] Run a real rendered Gen3D regression (generate → save → move/fire → screenshots) and record run id + findings here (blocked locally without `OPENAI_API_KEY`).
- [x] (2026-02-11) Update docs + `README.md`; commit.

## Surprises & Discoveries

- Observation: A build can succeed at copy/reuse but still fail motion validation due to joint limit mismatch.
  Evidence: Run `~/.gravimera/cache/gen3d/01106523-5481-4d54-acac-7236771d0b24` reports `hinge_limit_exceeded` for all legs on the `move` channel (declared limits around ±35° while the authored swing is ~75°) and `contacts_with_stance = 0` in `attempt_0/pass_2/smoke_results.json`.

- Observation: Review-delta tool outputs can be “almost right” but schema-invalid (wrong field names, duplicated JSON), so no repairs apply.
  Evidence: Same run has `unknown field keyframes, expected driver/speed_scale/clip` in `attempt_0/pass_1/tool_results.jsonl` and duplicated JSON (`}{`) in `attempt_0/pass_2/review_delta_raw.txt`.

- Observation: Users perceive “copy should work” even when per-target differences exist (angles, mount offsets, per-leg animations).
  Evidence: Reported expectation: “legs are mostly the same (3D model), only angles/animations differ; copy should still help”.

- Observation: Rendered end-to-end Gen3D regressions require `OPENAI_API_KEY`.
  Evidence: `tools/gen3d_real_test.py` returns HTTP 400 from Automation `/v1/gen3d/build` when the key is missing: `config.toml: missing openai.token / openai.OPENAI_API_KEY (or env OPENAI_API_KEY)`.

## Decision Log

- Decision: Treat “inherit” as a plan-declared, deterministic operation, not an engine heuristic.
  Rationale: Gen3D must work for arbitrary objects; the plan must explicitly mark which parts repeat and which per-target overrides differ.
  Date/Author: 2026-02-11 / Codex + user

- Decision: Keep inheritance narrowly scoped to geometry and explicitly selected fields; preserve per-target mount interfaces by default.
  Rationale: Repeated limbs often need different parent attachment offsets (radial distribution) and may need different animation phase; those should not block reuse.
  Date/Author: 2026-02-11 / Codex + user

- Decision: Add `time_offset_units` to attachment animation specs and apply it at sampling time as `t = driver_time * speed_scale + time_offset_units`.
  Rationale: A constant phase offset is generic (independent of object type), deterministic, and avoids duplicating keyframes for repeated limbs with staggered gaits.
  Date/Author: 2026-02-11 / Codex

- Decision: Match subtree copy children by explicit attachment edge keys `(parent_anchor, child_anchor)` and allow cloning missing branches even when a target subtree is partially populated.
  Rationale: Per-target offsets/animations and naming differences should not block reuse. Attachment edge keys are the plan’s explicit interface and avoid heuristic name matching.
  Date/Author: 2026-02-11 / Codex

## Outcomes & Retrospective

- Implemented:
  - `time_offset_units` on attachment animation specs (schema + runtime sampling + scene.dat round-trip) to support deterministic phase offsets.
  - More robust `copy_component_subtree_v1` shape handling via attachment edge key matching and partial target subtree expansion when unambiguous.
  - Prompt/tool/docs updates to clarify “inherit geometry, override offsets/animations is OK”.
  - Unit tests covering animation offset sampling and partial subtree expansion.

Remaining:

- Run a rendered end-to-end Gen3D regression with a real OpenAI key and record run id(s) + findings here (locally blocked without `OPENAI_API_KEY`).

## Context and Orientation

Relevant existing mechanisms:

- Plan-level reuse: `reuse_groups` in the Gen3D plan schema (see `src/gen3d/ai/schema.rs` and `src/gen3d/ai/reuse_groups.rs`).
- Copy tools:
  - `copy_component_v1` copies one generated component’s geometry into other planned components.
  - `copy_component_subtree_v1` copies a generated limb chain (root + descendants) into another planned subtree (see `src/gen3d/ai/copy_component.rs`).
- Motion validation and review/repair:
  - Validator outputs are written to `smoke_results.json` in the run cache.
  - The agent may call `llm_review_delta_v1` to repair plan-level rig/animation/contact issues.
- Structured outputs (strict JSON Schema) are requested when supported by the provider to reduce schema repair loops (see `src/gen3d/ai/structured_outputs.rs` and `src/gen3d/ai/openai.rs`).

How Gen3D cache runs are laid out:

- Base directory: `~/.gravimera/cache/gen3d/`
- Run directory: `<base>/<run_id>/`
- Evidence files:
  - Plan/draft/tool artifacts live under `attempt_0/pass_N/`.
  - Motion results typically live at `attempt_0/pass_N/smoke_results.json`.

## Plan of Work

This plan is intentionally generic and contract-driven. It avoids hardcoded “horse gait” heuristics and instead makes reuse + motion correctness explicit in the plan and deterministic in the engine.

### Milestone 1 — Define “inherit” semantics (what can differ without blocking reuse)

Define (in plain language and in schema) which parts of a repeated component/subtree are expected to be identical vs allowed to differ.

Recommended default semantics:

- **Inherited from source** (copied):
  - component geometry (`ObjectDef.parts` excluding attachment `ObjectRef`s)
  - component `ObjectDef.size` (unless explicitly overridden)
  - optionally component anchors (but default should preserve target anchors to keep mount interfaces stable)
- **Preserved per-target** (not copied by default):
  - per-target parent attachment offsets (these live in the parent’s `ObjectRef` part / `attach_to.offset`)
  - per-target attachment animations and joints (also on the parent’s attachment edge)
  - any explicitly declared per-target contact stance schedule

Note: this formalizes the user expectation: “same 3D model, different angle/animation is fine”.

### Milestone 2 — Teach and formalize “inherit geometry, override offsets/animation is OK”

The plan schema already supports `reuse_groups` to declare repeated geometry. The missing piece is **teaching** (and documenting) that reuse works even when per-target values differ:

- Per-target `attach_to.offset` differences (radial distribution, mirrored placement) are expected and do not prevent reuse.
- Per-target attachment animations are expected to differ (phase offsets, stance) while reusing the same underlying geometry.

This is implemented by improving:

- the plan prompt text (`src/gen3d/ai/prompts.rs`) to explicitly state what can differ, and
- the tool docs (`src/gen3d/agent/tools.rs`) so the agent confidently uses copy/reuse even when offsets/animations differ.

### Milestone 3 — Make repeated-limb animation authoring motion-safe by construction

The recurring failure pattern is “joint limits declare ±X but animation swings ±Y”. Prevent this generically:

1) Plan stage must select joint limits and the intended animation amplitude consistently:
   - either increase joint limits to match intended swing, or
   - reduce authored swing to fit limits.

2) Author a single canonical limb clip, then reuse it:
   - Geometry inheritance already reuses the mesh/primitive layout.
   - Motion should also be consistent, with per-target variation restricted to explicit overrides (phase shift, mount offset).

3) Add a non-heuristic way to apply per-target phase offsets without duplicating keyframes:

- Extend animation specs with `time_offset_units` (a constant additive offset in the clip’s time domain, i.e. the same units as `clip.loop.duration_secs` and keyframe `time_secs`).
- Apply it in runtime sampling and in motion validation sampling:
  - `t = driver_time * speed_scale + time_offset_units`
  - This preserves current semantics when `time_offset_units` is absent / null (defaults to 0).

4) Stance must be explicit when declared:
   - If the plan declares stance, ensure the move clip has a real “plant” window (foot motion cancels body motion during stance) or explicitly clear stance if that contract cannot be satisfied.
   - Prefer deterministic validation feedback + explicit repair actions over silent engine heuristics.

### Milestone 4 — Fix “legs under ground” at save/spawn time (generic grounding)

When saving/spawning a generated prefab, place it so it rests on the ground plane.

Generic, deterministic approach:

- If the plan declares ground contacts (anchors), compute the world-space Y of those anchors in the assembled pose at the neutral move phase (or at stance phase if stance exists).
- Apply a root translation so the minimum of those contact Ys is at `y=0` (or slightly above with a small epsilon).
- If no contacts exist, fall back to bounding-box min-Y grounding of the assembled geometry (still deterministic).

This should be done as a placement step, not a heuristic in geometry generation.

### Milestone 5 — Copy tool robustness: make subtree copy useful with “mostly-same” targets

Address the class of failures where targets already contain partial descendants or minor structural differences.

Implement one deterministic strategy and document it:

- For subtree matching, use the explicit attachment interface key `(parent_anchor, child_anchor)` for each edge.
- If a target subtree is missing descendants, clone the missing branches under the target root as long as the existing edges are unambiguous and match the source by that key.
- If ambiguity exists (duplicate keys under one parent, or a target edge key that does not exist in the source), return a clear tool error that names the parent component and the mismatched keys.

The key requirement is: per-target `attach_to.offset` rotations or per-target animations must not prevent geometry inheritance, because those are *expected differences* for radial/mirrored limbs.

### Milestone 6 — Reduce schema/repair churn: structured outputs + better repair prompting

This is already implemented in this repo (structured outputs + repair-loop improvements). When adding `time_offset_units`, extend the structured-output schemas so the model is constrained to emit the correct key.

This avoids non-generic “accept alias key X” fixes and instead improves the likelihood of producing valid output for future schema updates.

## Concrete Steps

All commands should be run from the repo root.

1) (Implementation phase) Run formatting and tests:

    cargo fmt
    cargo test

2) Smoke start:

    cargo run -- --headless --headless-seconds 3

3) Real rendered regression (requires Automation + a valid Gen3D config):

    python3 tools/gen3d_real_test.py --config ~/.gravimera/config.toml --reset-scene --prompt "A voxel donkey with 4 legs. Legs should be repeated geometry with per-leg mount rotations. Include move animation within joint limits and with stance foot plants."

Inspect the resulting run directory under `tests/gen3d/cache/gen3d/<run_id>/` (or `~/.gravimera/cache/gen3d/<run_id>/` depending on config) and confirm:

- Minimal unique component generations + deterministic copies applied.
- `smoke_results.json` has no `hinge_limit_exceeded` and stance/contact metrics are non-zero when stance is declared.
- The saved model spawns above ground and the move animation looks consistent across legs.

## Validation and Acceptance

This change is accepted when:

- A repeated-limb prompt (quadruped and 8-radial-legs) uses inherit/copy deterministically and does not regress by generating every repeated limb separately.
- Copy does not fail solely because targets differ in per-instance mount offsets or per-instance animations.
- Saved prefabs are grounded (no legs under ground) using contacts or bounding-box fallback.
- Motion validation passes for declared joint limits and stance; when it cannot, the system produces an actionable validation error and a deterministic, schema-valid review delta can repair or explicitly clear stance.

## Idempotence and Recovery

- Inheritance/copy application must be deterministic and safe to re-run (either by “fill_missing” semantics or an explicit overwrite mode).
- Any new schema fields must be optional and default to today’s behavior when absent.
- If a provider does not support structured outputs, the system must deterministically fall back to the existing parse/repair path (no hard failures).

## Artifacts and Notes

Record future real-test run ids and findings here (run id, prompt, screenshots, and whether copy/animation grounding issues were resolved).
