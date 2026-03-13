# Gen3D: Explicit `spin.axis_space` + join-consistent attachment spins

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository has an ExecPlan process described in `PLANS.md` at the repository root. This document must be maintained in accordance with that file.

## Purpose / Big Picture

Gen3D authors `move` wheel spins using a procedural `spin` clip. Today the engine and Gen3D disagree about *which coordinate frame* the `spin.axis` vector is expressed in for attachment edges, which makes it easy to accidentally spin a wheel around the wrong axis (or in the wrong direction).

After completing this plan:

1. `spin` clips can explicitly declare `axis_space: "join" | "child_local"`.
2. Attachment `spin` evaluation is consistent across:
   - Gen3D motion-authoring prompt text,
   - Gen3D motion validation and motion-recenter tools,
   - runtime animation playback.
3. Wheel spins are authored in the join frame (`axis_space="join"`), so the wheel axle is unambiguous and mirrored wheels do not silently flip axes due to anchor rotations.

User-visible verification:

- Run a Gen3D prompt that produces a vehicle with wheels (e.g. “warcar with laser cannon”), start it in the preview, and observe that wheels spin around their axles while moving (not around world-up).
- The rendered smoke test starts without crashing.

## Progress

- [x] (2026-03-14 01:30+0800) Create this ExecPlan.
- [x] (2026-03-14 01:50+0800) Implement `spin.axis_space` end-to-end (types, JSON, scene store).
- [x] (2026-03-14 01:55+0800) Make attachment `spin` playback honor `axis_space` (default `"join"`).
- [x] (2026-03-14 02:05+0800) Update Gen3D tool contracts + prompts + validators to match the same semantics.
- [x] (2026-03-14 02:15+0800) Update specs/docs that describe `spin` to include `axis_space`.
- [x] (2026-03-14 02:20+0800) Add/adjust tests to lock in the semantics (join vs child-local).
- [x] (2026-03-14 02:34+0800) Run rendered smoke test and commit (`faccfec`).

## Surprises & Discoveries

- Observation: Runtime currently applies attachment `spin` in the child-local frame (rebasing via the child anchor), but Gen3D prompts and validators describe/assume join-frame deltas for authored motion.
  Evidence: `src/object/visuals.rs` special-cases `Spin` for attachments, while `src/gen3d/ai/prompts.rs` and `src/gen3d/ai/motion_validation.rs` state/implement join-frame deltas.

- Observation: Scene persistence (`scene.dat`) did not encode any spin-axis space information, so `axis_space="child_local"` could not round-trip through saves.
  Evidence: `src/scene_store.rs` `SceneDatPartAnimationSpin` only had `{axis_x,axis_y,axis_z,radians_per_unit}` prior to this change.

## Decision Log

- Decision: Introduce `axis_space` on `spin` clips instead of changing `axis` meaning implicitly.
  Rationale: Explicitly naming the coordinate space makes the data self-describing and prevents “same numbers, different frame” bugs from recurring across tools/runtime.
  Date/Author: 2026-03-14 / Codex

- Decision: Default `axis_space` to `"join"`.
  Rationale: Gen3D motion authoring and attachment keyframe deltas are defined in the join frame; making the default match that contract is the easiest mental model (“all attachment deltas are join-frame unless explicitly marked otherwise”).
  Date/Author: 2026-03-14 / Codex

- Decision: Persist `axis_space` in `scene.dat` as an explicit enum field on the spin message.
  Rationale: `"child_local"` must survive save/load; adding a protobuf field is deterministic and backward-safe (missing field defaults to join).
  Date/Author: 2026-03-14 / Codex

## Outcomes & Retrospective

- Outcome: `spin` clips gained `axis_space: "join" | "child_local"` across runtime, prefab JSON, Gen3D edit bundles, and `scene.dat`.
- Outcome: Attachment spin playback is now deterministic and matches Gen3D’s authoring/validation contract: join-frame by default, with explicit opt-in for child-local spinners.
- Outcome: Gen3D motion authoring instructions and structured-output schema now require the author to state which axis space they are using for spin clips in plain words.
- Outcome: Added tests for both attachment modes (join vs child-local) and ensured motion validation rebases child-local spins the same way runtime does.

Remaining gaps / future work:

- If we ever add additional clip kinds with axis-like parameters, apply the same design rule: every axis must declare its space.

## Context and Orientation

Key terms (plain language):

- Attachment edge: A parent object part that attaches a child object via `attachment { parent_anchor, child_anchor }` and an `offset` transform (stored as `ObjectPartDef.transform`). The join frame is the parent anchor’s local frame used to author this `offset`.
- Join frame: The coordinate axes at the attachment joint: +X = join_right, +Y = join_up, +Z = join_forward. Gen3D prompts describe authored `delta` transforms in this frame.
- Child-local frame: The child object’s own local axes (before attachment), i.e. “spin around the model’s axis”.

Relevant code (from repository root):

- Runtime animation playback:
  - `src/object/visuals.rs` (`update_part_animations`, attachment transform resolution)
  - `src/object/registry.rs` (`PartAnimationDef::Spin`)
- Prefab JSON format:
  - `docs/gamedesign/34_realm_prefabs_v1.md` (spec)
  - `src/realm_prefabs.rs` (serde mapping)
- Gen3D motion authoring + validation:
  - `src/gen3d/ai/prompts.rs` (motion authoring contract text)
  - `src/gen3d/ai/schema.rs` (structured JSON types; deny-unknown-fields)
  - `src/gen3d/agent/tools.rs` (tool schema strings)
  - `src/gen3d/ai/motion_validation.rs` (computes world transforms; hinge checks)
  - `src/gen3d/ai/motion_recenter.rs` (recenter tool; spin support)
- Saved scene format (protobuf):
  - `src/scene_store.rs` (SceneDat animation encoding)

## Plan of Work

First, add an explicit axis-space enum for `spin` clips:

- Extend `PartAnimationDef::Spin` to carry `axis_space` with values `"join"` and `"child_local"`.
- Update all serialization layers (prefab JSON, Gen3D edit bundle JSON, scene store protobuf) to read/write the new field.

Second, make runtime playback deterministic and consistent:

- For attachment edges:
  - If `axis_space="join"`, apply the spin delta in the same way as keyframe deltas: `animated_offset = base_offset * delta(t)`.
  - If `axis_space="child_local"`, preserve the existing child-local rebasing behavior (using the child anchor).

Third, align Gen3D contracts and validators:

- Update the motion-authoring prompt and tool schema to include `axis_space` on `spin` clips, with guidance:
  - Wheels/hinges: use `"join"` and align the axis with `axis_join`.
  - Child-local spinners (fans/rotors): use `"child_local"` when the spin axis is best described in the child’s own axes.
- Update `motion_validation` to evaluate `spin` the same way runtime does for both axis spaces (including hinge off-axis checks).
- Update `recenter_attachment_motion_v1` to only support spin clips whose `axis_space="join"` on hinge edges (or explicitly document/refuse otherwise).

Finally, update docs/specs and add tests:

- Update `docs/gamedesign/34_realm_prefabs_v1.md` `Spin` spec to include `axis_space`.
- Update Gen3D docs that mention spin semantics (notably `docs/gen3d/recenter_attachment_motion_v1.md`).
- Update/extend tests in `src/object/visuals.rs` and `src/gen3d/ai/motion_validation.rs` to lock the semantics.

## Concrete Steps

Run these commands from the repo root (`/Users/flow/workspace/github/gravimera`):

  - Unit tests (fast confidence):
    - `cargo test -q`

  - Rendered smoke run (required by repo instructions):
    - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`

## Validation and Acceptance

Accept when:

1. A rendered smoke run starts and exits without crashing.
2. A Gen3D vehicle prompt produces wheels that spin around axles while moving (no obvious world-up spinning).
3. Tests cover both `axis_space="join"` and `axis_space="child_local"` behavior on attachment spins.
4. Tool contracts, prompts, validators, and docs agree on the meaning of `axis_space`.
5. Changes are committed with a clear message.

## Idempotence and Recovery

- This change is safe to iterate: it is self-contained in animation clip parsing/evaluation and prompt/schema strings.
- If a mismatch is found (tool vs runtime), use the tests and the prompt cache artifacts under `~/.gravimera/cache/gen3d/` to identify which layer is still interpreting axes incorrectly.

## Artifacts and Notes

- Cache example of the original failure mode (wheel spins authored without an explicit axis space): `~/.gravimera/cache/gen3d/05f555ae-13c1-4665-a584-1985a10c47dd/attempt_0/pass_3/motion_authoring.json` (wheel edges with `clip.kind="spin"` and `axis=[0,0,1]`).
