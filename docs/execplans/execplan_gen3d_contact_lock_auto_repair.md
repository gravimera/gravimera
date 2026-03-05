# Gen3D: Deterministic contact-lock auto-repair (make agent runs converge on motion validation)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D’s agent loop relies on `smoke_check_v1` → `motion_validation` to decide whether a generated unit’s motion is acceptable. In some runs, contact-related motion validation errors do not converge because the LLM alternates between:

- fixing slip/lift by adjusting motion, and
- “fixing” slip/lift by clearing contact stances (setting `stance: null`), which triggers `contact_stance_missing` errors and causes thrashing.

After this change:

1) The engine prevents non-convergent thrash by treating `tweak_contact stance:null` as invalid for ground contacts (except the existing wheel case where `move` is a pure `spin` and stance validation is skipped).

2) When `motion_validation.ok == false` due to **contact errors**, the engine applies a deterministic, generic auto-repair that “locks” planted contacts during stance by adding translation deltas to `move` clips on the relevant attachment edges. This runs only when errors exist; if a future model produces valid motion, the guardrail does nothing.

User-visible outcome: the Gen3D agent reaches a stable state where `smoke_check_v1.ok == true` for contact-related motion validation errors, instead of oscillating between `contact_slip` and `contact_stance_missing`.

## Progress

- [x] (2026-03-05 12:41Z) Create this ExecPlan and capture the problem statement.
- [x] (2026-03-05 13:00Z) Add review-delta guardrail: ignore `tweak_contact stance:null` for ground contacts (except `move`=`spin`).
- [x] (2026-03-05 13:00Z) Implement deterministic contact-lock auto-repair for contact errors (stance missing, slip, lift).
- [x] (2026-03-05 13:00Z) Integrate auto-repair into the agent loop’s `smoke_check_v1` so runs converge without extra LLM iterations.
- [x] (2026-03-05 13:00Z) Add regression tests for (a) stance-null guardrail and (b) slip→repaired→ok.
- [x] (2026-03-05 13:00Z) Run Rust tests and required rendered smoke test.
- [ ] Commit.

## Surprises & Discoveries

- Observation: Non-convergence can be a deterministic “policy oscillation”, not a modeling failure.
  Evidence: A run can alternate between `contact_slip` (after motion authoring) and `contact_stance_missing` (after the LLM clears stances via review-delta). Once all stances are cleared, `motion_validation` can no longer evaluate slip/lift and hard-errors, so the LLM re-adds stance, re-triggering slip, and so on.

- Observation: Contact validation assumes forward locomotion even if the prompt is “wriggle”.
  Evidence: `motion_validation` adds an assumed root translation (WORLD +Z) of `cycle_m` meters per cycle, and treats any declared ground contact stance as “planted” in world XZ/Y during stance. A purely rotational “wriggle” without compensating translation will necessarily fail `contact_slip` for planted stances.

- Observation: The best place to make convergence deterministic is the quality gate itself.
  Evidence: Applying the repair inside `smoke_check_v1` makes the agent see post-repair results immediately (and avoids additional review-delta iterations), while keeping the behavior “no-op when already ok”.

## Decision Log

- Decision: Enforce “ground contacts must keep stance” by ignoring `tweak_contact stance:null` for `kind=ground`, except when the component’s `move` clip is a pure `spin`.
  Rationale: Clearing stance does not make motion more correct; it only prevents validation and causes thrash. Wheels are the known generic exception already encoded in `motion_validation`.
  Date/Author: 2026-03-05 / Codex

- Decision: Auto-repair only triggers when `motion_validation` contains **contact-related errors**.
  Rationale: This makes the fix a convergence fallback. It avoids constraining future models that already satisfy validation (no “best-effort override” when the output is already valid).
  Date/Author: 2026-03-05 / Codex

- Decision: Auto-repair uses a generic “contact lock” that cancels measured anchor drift during stance by authoring translation deltas in the attachment join-frame.
  Rationale: This is object-agnostic and depends only on declared contacts + stance schedules + the engine’s existing sampling model, not on heuristics like “legs”, “octopus”, or “horse”.
  Date/Author: 2026-03-05 / Codex

- Decision: Trigger auto-repair from `smoke_check_v1` without bumping `assembly_rev`.
  Rationale: `assembly_rev` is used to validate LLM `applies_to` fields. Bumping it for engine-owned post-processing can invalidate motion roles/authoring and cause extra agent churn. The repair is deterministic and only runs on errors, so treating it as “engine post-pass” keeps the agent stable.
  Date/Author: 2026-03-05 / Codex

## Outcomes & Retrospective

- (2026-03-05) Implemented a convergence guardrail + deterministic repair for contact motion errors:
  - Review-delta can no longer clear ground stances (except wheel `move`=`spin`), preventing stance-missing thrash.
  - `smoke_check_v1` now runs a deterministic contact-lock repair when contact-related **errors** exist, and returns post-repair smoke results with a `motion_auto_repair` summary and artifacts (`motion_auto_repair.json`, `smoke_results_pre_repair.json`).
  - Unit tests cover both behaviors; full `cargo test` and the rendered smoke test pass locally.


## Context and Orientation

Key files and what they do (paths are from repo root):

- `src/gen3d/ai/motion_validation.rs`: Computes `motion_validation` by sampling the move cycle and reporting issues including:
  - `contact_stance_missing` (ground contacts without stance, except `move`=`spin`)
  - `contact_slip` / `contact_lift` (contact anchor moves too much during stance)
- `src/gen3d/ai/orchestration.rs`: Builds `smoke_results.json` via `build_gen3d_smoke_results()` which embeds `motion_validation`.
- `src/gen3d/ai/convert.rs`: Applies `llm_review_delta_v1` actions via `apply_ai_review_delta_actions()`. This is where `tweak_contact stance:null` currently clears stances.
- `src/gen3d/ai/agent_tool_dispatch.rs`: Executes agent tools, including `smoke_check_v1`.

Definitions used here:

- “Ground contact”: a `components[].contacts[]` entry with `kind: ground`, referencing an anchor by name.
- “Stance schedule”: `{phase_01, duty_factor_01}` describing when a contact is considered planted within the move cycle.
- “Contact lock”: a deterministic repair that adjusts `move` clips so the declared planted anchor stays near-constant in world XZ/Y during stance (as measured by `motion_validation`’s model).

## Plan of Work

### Milestone 1 — Guardrail in review-delta application

In `src/gen3d/ai/convert.rs` in `apply_ai_review_delta_actions()`:

- When applying `AiReviewDeltaActionJsonV1::TweakContact { stance: Some(None), ... }`:
  - If the contact is `kind=ground` and the component’s `move` slot is not a pure `spin`, ignore the stance-clear request (do not modify `contact.stance`).
  - Treat ignored stance-clear as “no change”: do not set `result.had_actions = true` for this action.
  - Keep allowing stance clear for the wheel case (`move` is `PartAnimationDef::Spin`) so the existing validator exception remains usable.

This prevents LLM-induced thrash where stances are repeatedly cleared to “fix” slip.

### Milestone 2 — Deterministic contact-lock auto-repair

Implement a deterministic repair in `src/gen3d/ai/motion_validation.rs` (same module as validation so it can reuse sampling helpers) with this behavior:

1) Detect contact-related **errors** using the same sampling model as validation:
   - missing stance on `kind=ground` (except `move`=`spin`)
   - slip/lift over the existing error thresholds (`2× warn`)

2) If any such errors exist, apply a repair:

   - If any ground contacts are missing stance (and not spin), deterministically assign a stance:
     - Sort contacts by `(component_name, contact_name)` and assign `phase_01 = i / N`.
     - Use a conservative generic `duty_factor_01 = 0.5`.

   - For contacts with slip/lift errors, build a per-sample translation correction that cancels drift during stance:
     - Use the existing `SAMPLE_COUNT` and `phase_in_stance()` logic.
     - For each contact, compute the “baseline” anchor world position at mid-stance (same as validator).
     - For each sample inside stance, compute `world_error = baseline - current_position_world`.
     - Apply this as a translation delta on a deterministic attachment edge:
       - Choose the deepest ancestor in the contact’s parent chain that has a non-spin `move` slot; if none exists, use the contact component’s own attachment edge (if it has a parent).
     - Convert `world_error` into join-frame translation using the parent/join rotation at that sample:
       - `join_rot_world = parent_world_rot * parent_anchor_rot * base_offset_rot`
       - `delta_translation_join = join_rot_world.inverse() * world_error`
     - Author a replacement `move` clip (loop) on that edge with keyframes at sample times:
       - Keep the sampled rotation/scale from the existing clip (bake time offsets), but override/add translation so stance samples lock.
       - Use duration `cycle_m` (so one loop per cycle) and `driver=move_phase` with `speed_scale=1`, `time_offset_units=0` for simplicity and determinism.

3) Re-run `motion_validation` after repair and record whether contact errors were resolved (for debug artifacts and tests).

This auto-repair is generic: it uses only declared contacts and the engine’s existing sampling/contract model.

### Milestone 3 — Agent-loop integration

In `src/gen3d/ai/agent_tool_dispatch.rs` in the `TOOL_ID_SMOKE_CHECK` tool implementation:

- After building `smoke_results.json`, if `motion_validation.ok == false` due to contact-related errors:
  - Run the auto-repair once against `job.planned_components`.
  - Sync the updated animations into the draft via `convert::sync_attachment_tree_to_defs(...)`.
  - Rebuild `smoke_results.json` and return the post-repair result.
  - Include a small `motion_auto_repair` summary in the tool result JSON so the agent can explain what happened if needed.

This makes convergence happen at the quality gate (smoke check) without requiring repeated LLM review-delta iterations.

### Milestone 4 — Tests

Add/adjust tests to prove the behavior:

- In `src/gen3d/ai/convert.rs`: update the existing test that currently expects `tweak_contact stance:null` to clear stance; it should now assert that stance remains set for a ground contact without a spin `move` clip, and that `apply.had_actions == false`.

- In `src/gen3d/ai/motion_validation.rs` (or the new repair function’s test module):
  - Construct a minimal root+limb assembly with a ground contact stance and a `move` loop that causes slip.
  - Assert `motion_validation.ok == false` before repair (contains `contact_slip`).
  - Apply the contact-lock repair and assert `motion_validation.ok == true` after.

### Milestone 5 — Validation, smoke test, commit

From repo root:

1) Run unit tests:

    cargo test

2) Run the required UI smoke test (rendered; not headless):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

3) Commit with a clear message describing the guardrail + auto-repair behavior.

## Validation and Acceptance

Acceptance is met when:

- A review-delta that tries to clear `stance` for a ground contact does not clear it (except for `move`=`spin` components).
- A minimal unit test demonstrating `contact_slip` becomes `motion_validation.ok == true` after the deterministic repair.
- In an agent run that previously thrashed on `contact_slip`/`contact_stance_missing`, `smoke_check_v1` returns an `ok: true` result after the repair step instead of oscillating.

## Idempotence and Recovery

- The auto-repair is designed to be safe to re-run: it replaces the `move` clip for selected edges deterministically from the current state and declared contacts.
- If a repair yields undesirable visuals, a safe rollback is to remove the auto-repair hook from `smoke_check_v1` (Milestone 3) while keeping the stance-null guardrail (Milestone 1) to prevent thrash.

## Artifacts and Notes

- `smoke_results.json` already records `motion_validation`. The smoke-check tool should additionally include `motion_auto_repair` metadata when a repair is applied, so cache artifacts clearly indicate that the engine modified motion to satisfy validation.
