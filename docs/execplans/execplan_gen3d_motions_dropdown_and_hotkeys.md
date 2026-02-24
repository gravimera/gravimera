# Gen3D: Custom motion channels (up to 10) + preview dropdown + digit hotkeys

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Status (2026-02-24):

- Gen3D plans are now **static-only** and do not include `attach_to.animations`.
- This ExecPlan is superseded by `execplan_gen3d_runtime_motion_algorithms.md` (runtime injected motion algorithms + realtime switching).

Gen3D already supports attachment animations via `ObjectPartDef.animations` and the runtime can play the canonical channels (`idle`, `move`, `attack_primary`, `ambient`) based on generic gameplay signals (moving/attacking). However:

- Gen3D AI structured outputs currently restrict animation channels to only those 4 names, so it cannot author “extra motions” like `dance` or `wave`.
- Gen3D plan conversion drops unknown channels, so even if the AI returns them they do not persist into the prefab.
- Gen3D preview UI hardcodes the motion dropdown to Idle/Move/Attack and cannot list all channels present in the draft.
- There is no gameplay input to force a unit to play a specific channel.

Original intent (superseded):

1. Gen3D can **author custom motion channels** (open vocabulary channel strings) in `attach_to.animations` and persist them into prefabs. The engine will **consider up to 10 channels** per unit for UI/hotkeys.
2. Gen3D **always produces** `idle` and `move` channels for mobile units (mobility `ground`/`air`), even if the AI forgets them.
3. Gen3D preview panel motion dropdown lists **all existing motion channels** (up to 10) in the current draft and can play any of them.
4. In gameplay, the player can select one or many units and press **1..9/0** to force the selected units to play the corresponding motion channel (1-based; `0` maps to slot 10).

You can see it working by generating a unit with “generate some motions like dance/wave”, saving it, selecting it in Play mode, and pressing 1..0 to switch channels. The Gen3D preview dropdown should list the same channels.

## Progress

- [ ] (2026-02-17) Write/maintain this ExecPlan.
- [ ] Update Gen3D AI JSON schemas to allow arbitrary channel keys (up to 10) and update prompts to describe custom channels + hotkeys.
- [ ] Update Gen3D plan conversion to keep custom channels and to inject missing `idle`/`move` on mobile units.
- [ ] Add runtime forced-channel playback (`ForcedAnimationChannel`) and gameplay 1..0 hotkeys for selected units.
- [ ] Update Gen3D preview dropdown UI to be data-driven (list up to 10 existing channels).
- [ ] Update `README.md` (and any relevant docs) to document motions, preview dropdown behavior, and digit hotkeys.
- [ ] Run `cargo test` and a headless smoke test (`cargo run -- --headless --headless-seconds 1`).
- [ ] Commit with a clear message.

## Surprises & Discoveries

- (none yet)

## Decision Log

- Decision: Treat “motions” as **animation channels** (string `PartAnimationSlot.channel`), with runtime automatic playback only for canonical channels (`idle`, `move`, `attack_primary`, `ambient`), and player-triggered playback for custom channels via forced override.
  Rationale: Keeps the engine generic (no geometry heuristics) while enabling open-vocabulary motions requested by users.
  Date/Author: 2026-02-17 / Codex + user

- Decision: Gameplay hotkeys use **1..9/0** mapping to the unit’s first 10 channels, ordered with `idle` and `move` first.
  Rationale: Keyboard has 10 reachable digit keys; stable ordering makes it learnable.
  Date/Author: 2026-02-17 / Codex + user

## Outcomes & Retrospective

- (fill in at completion)

## Context and Orientation

Key code locations:

- `src/object/registry.rs`: Prefab definitions including `ObjectPartDef.animations` and `PartAnimationSlot { channel, spec }`.
- `src/object/visuals.rs`: Spawns `PartAnimationPlayer` per part and updates animated transforms each frame (`update_part_animations`), currently selecting channels with fixed priority based on `AnimationChannelsActive`.
- `src/locomotion.rs`: Maintains `LocomotionClock` and sets `AnimationChannelsActive` (moving / attacking) for gameplay.
- `src/gen3d/ai/structured_outputs.rs`: JSON schema used for OpenAI structured outputs (currently restricts animation channel keys).
- `src/gen3d/ai/prompts.rs`: System instructions describing channels to the AI (currently only 4).
- `src/gen3d/ai/convert.rs`: Converts AI plan → initial draft `ObjectDef`s and currently drops unknown channels.
- `src/gen3d/ui.rs`, `src/gen3d/state.rs`, `src/gen3d/preview.rs`: Gen3D preview panel UI + motion selection (currently Idle/Move/Attack enum).
- `src/rts.rs`: Selection + input handling in Play mode (good place to add digit hotkeys).

Definitions:

- Motion / animation channel: A string label in `PartAnimationSlot.channel` that selects which `PartAnimationSpec` is played for a given part.
- Forced channel: A per-entity override that tells the runtime “play this channel if present”.

## Plan of Work

1. **Open-vocabulary channel support in Gen3D AI outputs**
   - Update the structured output schema so `attach_to.animations` can be an object with arbitrary string keys and values of `AiAnimationSpecJson` (or null), and cap it to 10 keys.
   - Update prompts to:
     - Require `idle` and `move` for mobile units.
     - Allow additional channels (e.g. `dance`, `wave`) when requested by the user, up to 10 total.
     - Explain that custom channels are player-triggered in gameplay via 1..0.

2. **Persist custom channels and guarantee `idle`+`move`**
   - In `src/gen3d/ai/convert.rs`, accept any non-empty channel name (trim; recommended snake_case) and keep it when building `PartAnimationSlot`.
   - If the root prefab has mobility and `idle` or `move` is missing across the draft, inject a small generic loop slot on the root attachment so the unit always has baseline motion.

3. **Runtime forced-channel playback**
   - Add a new component `ForcedAnimationChannel { channel: String }` on the root entity.
   - Update `update_part_animations` to choose the forced channel first (if present and the part has a matching slot), otherwise fall back to the existing priority logic.

4. **Gameplay digit hotkeys**
   - Add a system that:
     - Runs in non-Gen3D modes (Play/Build as appropriate; primary goal is Play).
     - When Digit1..Digit9/Digit0 is pressed, computes the selected unit’s available channels (recursive scan) and sets `ForcedAnimationChannel` to the channel at that slot.
     - If the chosen channel uses `attack_time`, also starts an `AttackClock` so the animation advances.

5. **Gen3D preview dropdown**
   - Replace the hard-coded enum dropdown with a data-driven list derived from the current Gen3D draft’s available channels (up to 10).
   - Selecting an item sets the preview model’s `ForcedAnimationChannel` and drives locomotion/attack clocks as needed so the preview animates.

6. **Docs and validation**
   - Update `README.md` to document:
     - Gen3D can generate custom motion channels.
     - Preview dropdown behavior.
     - 1..0 hotkeys for selected units in gameplay.
   - Run tests and a headless smoke run.
   - Commit.

## Concrete Steps

Run from repo root (`/Users/flow/workspace/github/gravimera`):

1. Tests:

    cargo test

2. Smoke:

    cargo run -- --headless --headless-seconds 1

## Validation and Acceptance

Acceptance behaviors (manual):

1. In Gen3D, prompt: “a dancing robot; generate some motions like dance and wave”.
   - Preview dropdown lists channels including at least `idle` and `move`, and also any AI-authored custom channels (up to 10).
   - Selecting a channel in the dropdown visibly changes the motion.

2. Save the unit, switch to Play mode, select it, then press 1..0:
   - The unit switches to the corresponding channel.

3. `cargo test` passes and headless smoke starts/exits without crash.

## Idempotence and Recovery

- If Gen3D outputs too many channels, only the first 10 (ordered) are shown in UI and mapped to hotkeys, but the prefab may still contain more channels.
- Removing `ForcedAnimationChannel` from an entity returns it to automatic channel selection.
