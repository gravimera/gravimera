# Gen3D: Rig constraints + motion validation + AI repair loop (avoid “swimming” locomotion)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root; this ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D can already generate component-level animations (via attachment animation channels like `move`), but the AI sometimes authors transforms that violate the player’s intent (e.g. a horse whose gait looks like “swimming”). The root cause is usually not “the model can’t recognize a horse”, but that it emits rotations in the wrong frame (attachment join frames vs component-local axes) or with the wrong degrees-of-freedom.

After this change, Gen3D has an explicit, data-driven “rig contract”:

- Each attachment MAY declare a **joint constraint** (fixed/hinge/ball) expressed in the attachment’s **join frame**.
- Components MAY declare explicit **contacts** (e.g. feet) via named anchors and a stance schedule for movement.
- The engine runs **generic motion validation** by sampling the move cycle and checking:
  - joint limits and axis conformance
  - contact stance plausibility (no wild slip/lift during stance)

When validation fails, the engine asks the AI to **repair** using `llm_review_delta_v1` with structured error messages. If the AI cannot converge, the engine degrades gracefully by disabling only the failing animation channels (identity loop) so the model remains usable and visually non-broken.

User-visible outcome: generated units with locomotion animations are much less likely to have obviously incorrect motion. When they do, the game reports what is wrong in a machine-readable form and either repairs or safely disables only the problematic motion.

## Progress

- [x] (2026-02-08 14:03Z) Create this ExecPlan (design only; no implementation yet).
- [ ] Add plan schema for joints + contacts and update Gen3D prompts accordingly.
- [ ] Implement rig/motion validation and surface results to AI review.
- [ ] Add an AI repair loop and a “disable bad channels” fallback policy.
- [ ] Add diagnostics (optional overlays) and regression tests.
- [ ] Update docs (`gen_3d.md`, possibly `object_system.md`) and run smoke test.
- [ ] Commit.

## Surprises & Discoveries

- Observation: “swimming” locomotion often comes from coordinate-frame mismatch, not animation math.
  Evidence: This has been reproduced in prompt-only creature generations (e.g. “a horse”) where the assembly is structurally fine, but the AI authors `move` keyframes in an unintended frame (e.g. limb “up” ends up pointing forward), producing a paddling-like motion. When reproducing locally, inspect the current run’s Gen3D artifacts (see “Gen3D cache layout” below) and compare `plan_raw.txt` (especially attachment anchor frames + `move` clips) against `move_sheet.png`.

- Observation: the current auto-review already captures motion sheets, but has no contract to validate against.
  Evidence: `src/gen3d/ai/mod.rs` captures `move_sheet.png` and `attack_sheet.png`, and `build_gen3d_review_delta_user_text` includes `smoke_results.json`; however `smoke_check_v1` only checks high-level semantics (attack required, spin axis warning) and does not validate gait plausibility.

## Decision Log

- Decision: Represent joint constraints on `components[].attach_to.joint` (not a separate global joint list).
  Rationale: Joints constrain an existing attachment edge; colocating the constraint with the attachment prevents mismatches and simplifies validation (no separate graph lookup).
  Date/Author: 2026-02-08 / Codex + user

- Decision: Express joint axes in the **parent-anchor join frame**.
  Rationale: Attachment offsets and their animations are already defined in the join frame. Validating (and repairing) hinge motion becomes local and unambiguous.
  Date/Author: 2026-02-08 / Codex + user

- Decision: Represent contacts as named anchors on components plus an explicit stance schedule.
  Rationale: Avoid name heuristics (“hoof”, “foot”) and allow arbitrary creatures and locomotion styles while still enabling generic validation.
  Date/Author: 2026-02-08 / Codex + user

- Decision: Surface rig/motion validation issues via `smoke_results.json` for auto-review.
  Rationale: `llm_review_delta_v1` already receives `smoke_results.json` (both in agent loop and non-agent auto-review). This guarantees the AI sees errors without changing tool ordering.
  Date/Author: 2026-02-08 / Codex

- Decision: Prefer “repair-first, degrade-last” with channel-scoped disabling (identity loop).
  Rationale: Keeps maximum AI freedom for custom motion; prevents shipping obviously broken motion if the AI cannot converge.
  Date/Author: 2026-02-08 / Codex + user

## Outcomes & Retrospective

- (TBD) Once implemented, record whether the validator reduces “swimming” cases and how often repairs converge vs falling back to disabling channels.

## Context and Orientation

Key existing concepts and files:

- Attachment join frame: for an attached `ObjectRef` part, the final transform is resolved as:

    parent_anchor * offset * inverse(child_anchor)

  where `offset` is expressed in the parent anchor join frame.

- Component-level animation: attachment animations are stored on the `ObjectRef` part and update the attachment `offset` each frame; see:
  - `src/object/visuals.rs` (`update_part_animations`, `sample_part_animation`)
  - `src/object/registry.rs` (`PartAnimationSlot`, `PartAnimationDef`, `PartAnimationDriver`)

- Gen3D plan/draft schemas and prompting:
  - `src/gen3d/ai/schema.rs` (AI JSON structs; `deny_unknown_fields`)
  - `src/gen3d/ai/prompts.rs` (plan + component + review delta instructions)
  - `src/gen3d/ai/convert.rs` (plan/draft conversion; attachment offsets; animations)
  - `src/gen3d/ai/mod.rs` (auto-review capture and review request)
  - `src/gen3d/ai/agent_loop/mod.rs` + `src/gen3d/ai/agent_review_delta.rs` (tool-driven agent loop and `llm_review_delta_v1`)

Definitions used in this ExecPlan:

- Join frame: the coordinate frame defined by the parent anchor in the parent’s local space, then transformed into root/world space. In the join frame:
  - +Z is the attachment direction (“parent toward child”) because `parent_anchor.forward` is defined that way by the AI schema.
  - +Y is the join up direction (`parent_anchor.up`).
  - +X is join right (cross(join_up, join_forward)).

- Joint: a constraint describing what relative motion is allowed on an attachment edge (e.g. hinge swing).

- Contact: a named anchor that represents a point intended to touch the ground (or another surface) during locomotion.

Gen3D cache layout (so this plan is machine-independent):

- Base directory: `AppConfig.gen3d_cache_dir` if set; otherwise `default_gen3d_cache_dir()` which resolves to `$GRAVIMERA_HOME/cache/gen3d` (`GRAVIMERA_HOME` defaults to `~/.gravimera`). See `src/paths.rs` and `src/gen3d/ai/mod.rs` (`gen3d_make_run_dir`).
- Run directory: `<base>/<run_id>/` where `run_id` is a UUID.
- Per-run artifacts:
  - `agent_trace.jsonl` lives at the run directory root.
  - Each agent iteration writes to `<run_dir>/attempt_<attempt>/pass_<pass>/`.
  - Most “what happened in this pass” artifacts live in the pass directory (for example `plan_raw.txt`, `scene_graph_summary.json`, `smoke_results.json`, and the review images like `move_sheet.png`).

## Plan of Work

This change is intentionally contract-driven rather than “preset gait” driven. The engine does not hardcode “horse trot”; it enforces the constraints the AI declares.

### Milestone 1 — Extend the plan schema (contracts)

1) Plan JSON: add optional rig fields.

In `src/gen3d/ai/schema.rs`:

- Extend `AiPlanAttachmentJson` with:
  - `joint`: optional object describing a joint constraint for this attachment.

- Extend `AiPlanComponentJson` with:
  - `contacts`: optional list of contacts for this component.

- Add new structs/enums (names are suggestions; keep them stable once chosen):

    - `AiJointJson`:
      - `kind`: `fixed | hinge | ball | free`
      - `axis_join`: `[f32;3]` (required for `hinge`; optional for others)
      - `limits_degrees`: `[min,max]` (optional; hinge uses it)
      - optional fields for ball joints (e.g. `swing_limits_degrees`, `twist_limits_degrees`)

    - `AiContactJson`:
      - `name`: string (stable identifier)
      - `anchor`: string (must exist in `anchors[]`)
      - `kind`: `ground` (start with only this)
      - `stance`: optional object `{ phase_01: number, duty_factor_01: number }` used for `move` validation

Example (plan excerpt; exact field names can differ, but semantics must match):

    {
      "version": 8,
      "rig": { "move_cycle_m": 1.2 },
      "mobility": { "kind": "ground", "max_speed": 6.0 },
      "components": [
        {
          "name": "torso",
          "size": [1.4, 0.85, 2.0],
          "anchors": [
            { "name": "leg_fl_mount", "pos": [-0.45, -0.425, 0.65], "forward": [0,-1,0], "up": [0,0,1] }
          ]
        },
        {
          "name": "leg_fl",
          "size": [0.25, 0.8, 0.25],
          "anchors": [
            { "name": "torso_mount", "pos": [0, 0.4, 0], "forward": [0,-1,0], "up": [0,0,1] },
            { "name": "ground_contact", "pos": [0, -0.4, 0], "forward": [0,-1,0], "up": [0,0,1] }
          ],
          "contacts": [
            {
              "name": "hoof_fl",
              "kind": "ground",
              "anchor": "ground_contact",
              "stance": { "phase_01": 0.0, "duty_factor_01": 0.6 }
            }
          ],
          "attach_to": {
            "parent": "torso",
            "parent_anchor": "leg_fl_mount",
            "child_anchor": "torso_mount",
            "offset": { "pos": [0.0, 0.0, 0.0] },
            "joint": { "kind": "hinge", "axis_join": [1,0,0], "limits_degrees": [-35, 35] }
          }
        }
      ]
    }

2) Prompting updates to make the AI follow the contract.

In `src/gen3d/ai/prompts.rs`, update `build_gen3d_plan_system_instructions()`:

- Define the new fields and state when they are required:
  - If `mobility.kind == "ground"` and you output any `move` animation on an attachment, strongly encourage adding `attach_to.joint` and at least one `contacts[]` entry for the end effector(s) that should touch the ground.
  - If the object is “static” (building/prop), omit rig fields entirely.

- Add explicit guidance that prevents frame confusion:
  - “Joint axes and limits are expressed in the parent-anchor join frame (same frame as `attach_to.offset`).”
  - “Contacts reference anchors and are evaluated in the assembled root frame during validation.”

In `build_gen3d_component_system_instructions()`:

- If the plan includes contact anchors, explicitly require them to be present in the component draft anchors (this is enforced structurally when plan anchors are required, but the prompt should say it).

3) Update `gen_3d.md` to document the new schema sections and the meaning of join-frame joint axes and contact stance schedules.

### Milestone 2 — Implement motion validation (generic, contract-driven)

Add a “rig/motion validation” routine that samples the move cycle and returns structured issues with enough context for `llm_review_delta_v1` to repair.

Key requirements:

- Validation must be deterministic and based on declared contract data (joints/contacts), not on component-name heuristics.
- Output must include `component_id` (UUID string) (and optionally channel names) so `review_delta_v1` can target `tweak_anchor` / `tweak_attachment` / `tweak_contact` precisely.
- Issues should include numeric evidence (angle, slip, lift) and tolerances.
- Validation output must be prompt-budget-friendly: cap issue count (e.g. keep the top 8–16 issues) and order by severity and magnitude (worst offenders first).

Suggested approach:

1) Compute an animated “pose” at sample points.

- Sample N times across one move cycle:
  - If a top-level `rig.move_cycle_m` exists, interpret it as meters-per-cycle and sample in `[0, move_cycle_m)`.
  - Otherwise, infer a cycle from the first `move` animation clip whose `driver == move_phase` and whose `clip.kind == loop` (use `loop.duration_secs`).
    Important: despite the field name, `duration_secs` is interpreted in the driver’s units. For `move_phase`, driver time is meters traveled (`LocomotionClock.t`), so `duration_secs` is effectively meters-per-cycle. See `src/locomotion.rs` and `src/object/visuals.rs` (`PartAnimationDriver::MovePhase`).
    Fallback to `1.0` meter if none exist.

- For each component attachment, build the animated offset for the `move` channel at each sample:
  - base_offset = `attach_to.offset`
  - delta = sampled animation transform for the chosen spec (if any)
  - animated_offset = base_offset * delta (use the same multiplication semantics as runtime; see `mul_transform` in `src/object/visuals.rs`)

- Resolve each component’s world transform by walking the attachment tree using:

    child_world = parent_world * parent_anchor * animated_offset * inverse(child_anchor)

2) Joint validation:

For each attachment with a declared `joint` and a `move` animation:

- Evaluate the **delta** rotation relative to base:

    q_delta = inverse(base_offset.rotation) * animated_offset.rotation

- If `joint.kind == hinge`:
  - Convert `q_delta` to axis-angle.
  - Check axis alignment with `axis_join` (abs(dot) close to 1).
  - If `limits_degrees` is provided, check the signed hinge angle is within bounds.

Return issues like:

- `hinge_off_axis` (error or warn): include `axis_join`, observed axis, alignment score.
- `hinge_limit_exceeded` (error): include observed angle and limits.

3) Contact validation:

For each `contact.kind == ground` with a stance schedule:

- Compute the contact anchor world position at each sample by:
  - anchor_world = component_world * anchor_local
  - contact_pos = anchor_world.translation

- Simulate root movement for “world” contact checks (optional but recommended):
  - root_forward_xz = normalized(root_forward with y=0), fallback to +Z
  - root_translation_at_t = root_forward_xz * t (meters)
  - contact_world = root_translation_at_t + contact_pos

- For the stance interval defined by `phase_01` and `duty_factor_01`:
  - measure max Y deviation within stance (`lift_m`), with an inferred “ground_y” from stance midpoint

Return issues like:

- `contact_lift` (warn or error): include `lift_m`.

4) Surface validation output where the AI can see it.

Extend `build_gen3d_smoke_results()` in `src/gen3d/ai/mod.rs` to include:

- `rig_summary`: extracted joints/contacts counts and cycle length used.
- `motion_validation`: `{ ok, issues: [...] }`

Include `component_id` in each issue to match `review_delta_v1` targeting rules.

### Milestone 3 — AI repair loop + channel-scoped fallback

Goal: if validation fails, the engine requests repair; only if repair does not converge, disable the failing channels.

1) Update `build_gen3d_review_delta_system_instructions(review_appearance: bool, edit_session: bool)`:

- Explicitly state that validation issues are authoritative and should be addressed first.
- Allow disabling any channel (not just `ambient`) via an identity loop as a last resort.

2) Repair policy:

- On motion validation errors:
  - Call `llm_review_delta_v1` with the validation issues and motion sheets.
  - Apply returned deltas.
  - Re-run validation.

- If the same error class persists for the same component/channel for K rounds:
  - Apply an engine-side fallback: replace that channel’s spec with an identity loop.
  - Record the fallback action in cache artifacts (and optionally in tool feedback history).

Important: fallback must be channel-scoped (only disable what is broken).

### Milestone 4 — Diagnostics (optional but high leverage)

Improve AI repair quality by adding “explainable visuals”:

- In Gen3D review captures, optionally render:
  - contact markers (small spheres) at contact anchors
  - joint axis gizmos (RGB axes) at attachment points
  - short numeric overlays (angle / slip) for the worst offenders

This is not required for correctness but tends to dramatically increase the model’s ability to repair.

### Milestone 5 — Tests and regression harness

Add unit tests that do not rely on the OpenAI network backend:

- Pose sampling: given a small synthetic plan (2–3 components) with a hinge and a move loop, verify the validator reports:
  - no issues for within-limit hinge motion
  - an issue when off-axis rotation is introduced

- Contact check: define a contact + stance; verify slip/lift metrics behave as expected.

Prefer placing test scenes/configs under the repo `tests/` folder per `AGENTS.md` if any external artifacts are needed. For pure Rust tests, keep them next to the code (as existing Gen3D tests do).

### Milestone 6 — Documentation and acceptance checklist

Update:

- `gen_3d.md`: plan schema additions + explanations with examples.
- (Optional) `object_system.md`: add a small section describing joint/contact metadata as a contract for animation validation (still data-driven).

Record how to reproduce with a minimal prompt (e.g. “a horse”) and where to inspect:
- `smoke_results.json` for motion issues
- motion sheets (`move_sheet.png`)
- the review delta transcript in `agent_trace.jsonl`

## Concrete Steps

All commands run from the repository root (the directory containing `Cargo.toml`):

1) Run unit tests:

    cargo test

2) Run smoke test (AGENTS.md requirement):

    cargo run -- --headless --headless-seconds 1

3) Manual sanity (optional but recommended):

- In Gen3D, generate a prompt-only creature with move animation (e.g. “a horse”).
- Locate the current run directory (see “Gen3D cache layout” above) and open the most recent pass’s `smoke_results.json`. Confirm it contains `motion_validation`.
- Confirm that if `motion_validation.ok=false`, the system requests a repair and either fixes or disables only the bad channel(s).

## Validation and Acceptance

Acceptance criteria:

1) Schema + prompting:
   - Plans with joints/contacts parse successfully.
   - Older plans without rig fields still parse (fields are optional).

2) Motion validation:
   - `smoke_results.json` includes structured, component-targetable motion validation issues when contract is violated.
   - Issues include `component_id` and enough metrics to be actionably repaired.

3) Repair loop:
   - When motion validation reports errors, the system attempts `llm_review_delta_v1` repair.
   - If repair does not converge, only the failing channel(s) are disabled (identity loop), and the rest of the draft remains intact.

4) Safety:
   - No crashes in headless smoke run.
   - Validation is bounded (finite samples, finite recursion; no cycles).

## Idempotence and Recovery

- Validation and repair are safe to repeat; cache artifacts are per-run and can be deleted.
- If a run gets stuck oscillating, disabling the channel(s) is a deterministic recovery path that keeps the model usable.

## Artifacts and Notes

Expected cache artifacts (existing + new):

- `smoke_results.json` includes `motion_validation`.
- `scene_graph_summary.json` remains the primary “what exists” snapshot.
- `move_sheet.png` and `attack_sheet.png` are included in review inputs; diagnostics overlays (if implemented) appear there.

## Interfaces and Dependencies

At the end of implementation, the following user-facing schema changes should exist (names may vary, but behavior must match):

- Plan JSON additions:
  - `components[].attach_to.joint` (optional)
  - `components[].contacts[]` (optional)
  - optional top-level `rig.move_cycle_m` (optional; strongly recommended when contacts use phase/duty)

And the following engine outputs should exist:

- `smoke_check_v1` output includes `motion_validation` with component/channel-scoped issues and numeric evidence.

---

Revision note (2026-02-08): clarify driver units (`duration_secs` uses driver units), remove machine-specific repo paths, and document Gen3D cache/run directory layout so another agent can follow this plan on a different computer.
