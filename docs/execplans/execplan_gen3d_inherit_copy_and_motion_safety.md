# Gen3D: “Inherit” Copy (Reuse With Overrides) + Motion-Safe Animation Authoring

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository contains `PLANS.md` at the repo root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D already has non-heuristic reuse primitives (`reuse_groups`, `copy_component_v1`, `copy_component_subtree_v1`) to avoid regenerating repeated parts (wheels, legs, arms). In practice, users still see:

- Copy failures when the planned subtrees are “mostly the same” but not identical in non-geometry fields (different per-leg attachment offsets, per-leg animations, etc.).
- Repeated limbs that are geometrically consistent (copied) but motion-invalid (hinge limits exceeded, stance never plants, legs end up under ground after saving/spawning).
- AI “near-miss JSON” that is valid JSON but not schema-valid, leading to wasted repair loops.

After this change, Gen3D will support a generic concept of **inheritance for repeated parts**: a plan can declare “these targets inherit the same 3D geometry as this source”, while still allowing explicit per-target differences (mount offsets, phase shifts, contact stance, etc.). The engine applies inheritance deterministically and validates the resulting motion against declared joint/contact contracts.

You can see this working by generating a multi-leg creature (8 radial legs, or a quadruped) and observing:

- The cache shows a reuse/inherit plan and deterministic copy application (minimal LLM component generations).
- The saved prefab spawns with feet above/at ground, not sunk below it.
- Motion validation passes (no `hinge_limit_exceeded`, stance contacts exist if declared), and the move gait looks consistent across repeated legs.

## Progress

- [x] (2026-02-11) Write and check in this ExecPlan.
- [ ] Define precise “inherit” semantics (which fields are inherited vs preserved vs overridden).
- [ ] Extend reuse/copy pipeline to support inherit + overrides (engine-side; no heuristics).
- [ ] Add motion-authoring constraints so repeated animations stay within joint limits and produce stance.
- [ ] Add end-to-end Gen3D regression: generate, save to scene, move/fire, capture screenshots, and record run ids.
- [ ] Update docs (`gen_3d.md`, `README.md` if needed), run smoke test, commit.

## Surprises & Discoveries

- Observation: A build can succeed at copy/reuse but still fail motion validation due to joint limit mismatch.
  Evidence: Run `~/.gravimera/cache/gen3d/01106523-5481-4d54-acac-7236771d0b24` reports `hinge_limit_exceeded` for all legs on the `move` channel (declared limits around ±35° while the authored swing is ~75°) and `contacts_with_stance = 0` in `attempt_0/pass_2/smoke_results.json`.

- Observation: Review-delta tool outputs can be “almost right” but schema-invalid (wrong field names, duplicated JSON), so no repairs apply.
  Evidence: Same run has `unknown field keyframes, expected driver/speed_scale/clip` in `attempt_0/pass_1/tool_results.jsonl` and duplicated JSON (`}{`) in `attempt_0/pass_2/review_delta_raw.txt`.

- Observation: Users perceive “copy should work” even when per-target differences exist (angles, mount offsets, per-leg animations).
  Evidence: Reported expectation: “legs are mostly the same (3D model), only angles/animations differ; copy should still help”.

## Decision Log

- Decision: Treat “inherit” as a plan-declared, deterministic operation, not an engine heuristic.
  Rationale: Gen3D must work for arbitrary objects; the plan must explicitly mark which parts repeat and which per-target overrides differ.
  Date/Author: 2026-02-11 / Codex + user

- Decision: Keep inheritance narrowly scoped to geometry and explicitly selected fields; preserve per-target mount interfaces by default.
  Rationale: Repeated limbs often need different parent attachment offsets (radial distribution) and may need different animation phase; those should not block reuse.
  Date/Author: 2026-02-11 / Codex + user

## Outcomes & Retrospective

- (TBD) Record whether inherit+overrides reduces copy failures and improves repeated-limb motion stability across real tests.

## Context and Orientation

Relevant existing mechanisms:

- Plan-level reuse: `reuse_groups` in the Gen3D plan schema (see `src/gen3d/ai/schema.rs` and `src/gen3d/ai/reuse_groups.rs`).
- Copy tools:
  - `copy_component_v1` copies one generated component’s geometry into other planned components.
  - `copy_component_subtree_v1` copies a generated limb chain (root + descendants) into another planned subtree (see `src/gen3d/ai/copy_component.rs`).
- Motion validation and review/repair:
  - Validator outputs are written to `smoke_results.json` in the run cache.
  - The agent may call `llm_review_delta_v1` to repair plan-level rig/animation/contact issues.

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

### Milestone 2 — Add inherit + overrides to the plan schema (deterministic)

Extend the plan schema (or reuse `reuse_groups`) so the plan can declare:

- A source root (component/subtree).
- Targets.
- What to inherit:
  - geometry only (default),
  - geometry + anchors,
  - geometry + anchors + size,
  - (optional future) animation clip reuse.
- Optional per-target overrides that are *explicit*:
  - per-target attachment `move` phase shift (see next milestone),
  - per-target contact stance windows,
  - per-target mount offsets/rotations (already naturally live in `attach_to.offset` and should be planned explicitly).

The engine must apply these without any name heuristics: it should only follow explicit plan fields.

### Milestone 3 — Make repeated-limb animation authoring motion-safe by construction

The recurring failure pattern is “joint limits declare ±X but animation swings ±Y”. Prevent this generically:

1) Plan stage must select joint limits and the intended animation amplitude consistently:
   - either increase joint limits to match intended swing, or
   - reduce authored swing to fit limits.

2) Author a single canonical limb clip, then reuse it:
   - Geometry inheritance already reuses the mesh/primitive layout.
   - Motion should also be consistent, with per-target variation restricted to explicit overrides (phase shift, mount offset).

3) Add a non-heuristic way to apply per-target phase offsets:
   - Option A (preferred): add an explicit `phase_offset_units` (or similar) to `PartAnimationSpec` so the same clip can be reused with different phase starts while using the same driver (`move_phase`).
   - Option B (fallback): add an engine helper that “rotates” loop keyframes by a phase offset and writes a per-target keyframe list deterministically.

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

Potential improvements (choose one deterministic strategy and document it):

- Add a `mode`/flag for subtree inherit:
  - `fill_missing`: copy only into components with `actual_size == None` (current auto-copy behavior), and only clone descendants when the target subtree is empty.
  - `overwrite_geometry`: always copy geometry into the mapped subtree pairs (even if targets were generated earlier), while preserving per-target attachments and mount interfaces.
  - `allow_partial`: if a target subtree is partially populated, match by (parent_anchor, child_anchor, stable order) and copy what matches, leaving non-matching extras untouched; errors must be path-specific and actionable.

The key requirement is: per-target `attach_to.offset` rotations or per-target animations must not prevent geometry inheritance, because those are *expected differences* for radial/mirrored limbs.

### Milestone 6 — Reduce schema/repair churn: structured outputs + better repair prompting

Keep relying on schema-valid JSON:

- Prefer API-level structured outputs where supported (plan, draft, review delta).
- When repairs are needed, ensure the repair prompt never includes concatenated/duplicated prior invalid JSON; the model should output exactly one corrected JSON object matching the tool schema.

This avoids non-generic “accept alias key X” fixes and instead improves the likelihood of producing valid output for any future key name mistakes.

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

