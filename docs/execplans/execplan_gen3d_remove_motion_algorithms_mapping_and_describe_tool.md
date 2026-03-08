# Gen3D: Remove Runtime Motion Algorithms, Motion Mapping, and `describe_tool_v1`

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This plan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D motion should be entirely **AI-authored** (explicit animation clips baked into prefab attachment edges). The engine should no longer ship or expose:

1) runtime motion algorithms (walk/wheels/idle/attack generators),
2) “motion mapping” (`motion_roles_v1` → `motion_rig_v1`), or
3) the Gen3D agent tool-schema introspection tool `describe_tool_v1`.

After this change:

- Gen3D always asks the AI to generate customized motion animations when the generated prefab is a **movable unit** (ground/air mobility), so units don’t depend on engine-injected motion.
- The in‑game Meta panel no longer offers any animation/motion algorithm selection UI (because there are no runtime motion algorithms to select).
- Saved prefab descriptors and edit bundles no longer contain `motion_roles_v1` / `motion_rig_v1`.
- The Gen3D agent prompt and tool registry no longer mention or allow `describe_tool_v1` or `llm_generate_motion_roles_v1`.

## Progress

- [x] (2026-03-09 04:34Z) Write ExecPlan for removal work.
- [x] (2026-03-09 05:10Z) Remove runtime motion algorithms (`src/motion.rs`) and all persistence/UI wiring.
- [x] (2026-03-09 05:10Z) Remove motion mapping (`motion_roles_v1` and `motion_rig_v1`) from Gen3D job state, tools, prompts, save pipeline, and specs.
- [x] (2026-03-09 04:34Z) Remove `describe_tool_v1` from Gen3D tool registry, dispatch, prompts, and tests.
- [x] (2026-03-09 05:10Z) Update Meta panel UI to remove animation/motion selection; keep Brain + Gen3D actions.
- [x] (2026-03-09 04:34Z) Update docs to match behavior (keep `README.md` clean; details in `docs/` and `gen_3d.md`).
- [x] (2026-03-09 07:00Z) Validate (tests + rendered smoke test) and commit.

## Surprises & Discoveries

- Observation: Removing runtime motion algorithms is mechanically straightforward (delete `src/motion.rs`, remove Bevy systems, remove `scene.dat` persistence, and remove Meta panel selection UI), but Gen3D still had “motion roles” remnants that prevented compilation.
  Evidence: `cargo check` failed with unresolved `motion_roles` imports/fields and a stale `GenerateMotionRoles` tool kind until those code paths are fully removed.

## Decision Log

- Decision: Treat “motion roles” + “motion rig” as fully removed features and delete their schemas/prompts/save-time derivation rather than keeping dead-code stubs.
  Rationale: The product direction is “AI-authored motion clips baked into prefabs”; keeping mapping scaffolding increases maintenance and is misleading when runtime motion algorithms are removed.
  Date/Author: 2026-03-09 (agent)

## Outcomes & Retrospective

- Runtime motion algorithms removed (no engine-injected idle/move/attack).
- Motion mapping removed (`motion_roles_v1` / `motion_rig_v1`); movable units rely on authored clips (`llm_generate_motion_authoring_v1`).
- `describe_tool_v1` removed; tool introspection is now `list_tools_v1` only.
- Meta panel no longer offers animation selection; it shows a read-only summary plus Brain + Gen3D actions.
- Validation: `cargo test` passed; rendered smoke test ran successfully (`--rendered-seconds 2`).

## Context and Orientation

Key current implementation points (paths are repo-root relative):

- Runtime motion algorithms and motion mapping have been removed. Motion is authored as explicit animation slots baked onto attachment edges.
- The Meta panel UI is implemented in `src/motion_ui.rs` (opened by double-clicking a unit’s selection circle) and contains:
  - Brain module selection (Intelligence service),
  - Gen3D actions (Copy/Edit/Fork),
  - A read-only motion summary (mobility/attack/available channels). No animation selection UI.
- Scene persistence no longer stores per-instance motion algorithm selections in `scene.dat`.
- Gen3D edit bundles no longer contain `motion_roles_v1` / `motion_rig_v1` metadata; only authored clips (`motion_authoring`) remain.
- The Gen3D agent tool registry no longer supports `describe_tool_v1`; `list_tools_v1` is the only tool-introspection surface.

Docs that must match the new behavior:

- Gen3D implementation doc: `gen_3d.md`
- Prefab descriptor spec: `docs/gamedesign/35_prefab_descriptors_v1.md`
- Scene-local prefab package spec (Gen3D edit bundle schema): `docs/gamedesign/39_scene_local_prefab_packages_v1.md`
- ExecPlans index: `docs/execplans/README.md` (should no longer present runtime motion algorithms as an active plan)

## Plan of Work

1) Remove runtime motion algorithms and their selection surface.

   - Delete `src/motion.rs` and remove all uses:
     - `src/app_plugins.rs`: stop registering motion algorithm systems.
     - `src/scene_store.rs`: remove motion-algorithm persistence fields and change detection.
   - Update the Meta panel UI to remove the `idle/move/attack` algorithm selection sections and any code that reads/writes `MotionAlgorithmController`.

2) Remove motion mapping from Gen3D.

   - Remove the motion roles tool (`llm_generate_motion_roles_v1`) and all related state:
     - `src/gen3d/agent/tools.rs` tool ids + registry.
     - `src/gen3d/ai/*` schema/parse/prompt/dispatch/poll/job/workspaces/snapshots/edit_bundle.
   - Remove save-time derivation and persistence of `motion_roles_v1` / `motion_rig_v1` in `src/gen3d/save.rs`.
   - Ensure the agent loop always requests AI-authored motion for movable units:
     - Update `src/gen3d/ai/agent_prompt.rs` rules so `llm_generate_motion_authoring_v1` is required for mobility=ground/air before finishing.
     - Update any engine-side “complete enough” checks and “unfinished checks” messaging to reflect “authored motion required” (no runtime fallback exists).
     - Update the motion authoring prompt (`src/gen3d/ai/prompts.rs`) and schema (`src/gen3d/ai/structured_outputs.rs`) to remove `decision=runtime_ok`.

3) Remove `describe_tool_v1`.

   - Remove the tool id and registry support from `src/gen3d/agent/tools.rs`.
   - Remove tool dispatch support in `src/gen3d/ai/agent_tool_dispatch.rs`.
   - Update the agent prompt in `src/gen3d/ai/agent_prompt.rs` to not mention `describe_tool_v1`.
   - Update tests that referenced `describe_tool_v1` payload compaction.

4) Update docs.

   - `gen_3d.md`: remove runtime motion mapping/algorithm sections; document that Gen3D always asks AI to author motion clips for movable units; remove Meta panel algorithm selection description.
   - `docs/gamedesign/35_prefab_descriptors_v1.md`: remove/mark removed `motion_roles_v1` and `motion_rig_v1` keys; keep `motion_summary` as the supported “what animations exist” summary.
   - `docs/gamedesign/39_scene_local_prefab_packages_v1.md`: remove `motion_roles` from the edit bundle schema and adjust wording around motion authoring.
   - `docs/execplans/README.md`: add this plan as active and mark runtime motion algorithms plan as historical/removed.

## Concrete Steps

All commands run from the repo root.

1) Implement code changes (see Plan of Work).
2) Format and run unit tests:

    cargo fmt
    cargo test

3) Run required rendered smoke test (isolated home dir; do NOT use `--headless`):

    tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

4) Commit with a clear message describing removals (runtime motion, motion mapping, describe tool, Meta panel update).

## Validation and Acceptance

Acceptance is confirmed when:

- `cargo test` passes.
- The rendered smoke test starts and exits without crash.
- The Meta panel opens on double-click for a unit and shows Brain + Gen3D actions but no motion/animation algorithm selection.
- Gen3D tool list no longer includes `describe_tool_v1` or `llm_generate_motion_roles_v1`.
- Gen3D saved prefab descriptors/edit bundles no longer write `motion_roles_v1` / `motion_rig_v1`.

## Idempotence and Recovery

- If compilation fails after deletions, use `rg` to find remaining references to removed identifiers (`MotionAlgorithmController`, `motion_roles_v1`, `describe_tool_v1`) and remove/replace them.
- If the smoke test fails due to graphics environment constraints, capture the exact error output in `Surprises & Discoveries` and keep the code changes as-is; rerun locally with a working display environment.

## Artifacts and Notes

- N/A (to be filled with any important transcripts or errors observed during implementation).

## Interfaces and Dependencies

- Bevy UI: Meta panel remains a Bevy UI panel; only the motion algorithm selection portion is removed.
- Gen3D agent tooling: `list_tools_v1` remains the only tool-introspection surface; `describe_tool_v1` is removed.
- Motion authoring: `llm_generate_motion_authoring_v1` remains the sole path to generate motion for movable units.
