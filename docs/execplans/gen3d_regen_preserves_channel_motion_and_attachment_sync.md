# Gen3D: Preserve per-channel internal motion and attachment sync across component regeneration

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Users can now ask Gen3D to add new expressions or other internal motions to an existing component without losing those motions when a later review-delta regenerates that component. After this change, a seeded edit that adds channels like `smile` or `laugh` keeps those internal part animations after component regen, and attachment offset tweaks made in `planned_components` still survive into the saved prefab defs. You can observe the fix by running the regression test in `src/gen3d/ai/regression_tests.rs`, then by running the rendered HTTP regression in `test/run_1/gen3d_expression_edit_regression/run.py` and confirming the saved prefab still contains the requested named expression channel.

## Progress

- [x] (2026-03-30 09:40Z) Reproduced the failure from the provided run cache and confirmed two distinct bugs: regenerated components were not rebuilding attachment object refs from `planned_components`, and internal articulation-node motion was lost because only the latest motion-authoring blob was retained.
- [x] (2026-03-30 10:10Z) Added durable per-channel motion-authoring persistence to job/workspace/snapshot/edit-bundle state, including legacy bundle fallback from the older single `motion_authoring` field.
- [x] (2026-03-30 10:25Z) Added a shared regenerated-component integration helper that replaces the component def, resolves planned transforms, rebuilds attachment refs from `planned_components`, replays stored motion channels targeting the regenerated component, updates the draft root def, and advances `assembly_rev`.
- [x] (2026-03-30 10:35Z) Switched both the current component batch path and the legacy orchestration regen path to the shared helper so they cannot diverge on save-time state sync.
- [x] (2026-03-30 10:50Z) Added a deterministic regression test that covers the exact failure sequence: author internal `smile` and `laugh`, tweak the head attachment offset, regenerate the head, and assert both the new offset and the internal motion slots survive.
- [x] (2026-03-30 12:10Z) Ran the broader verification stack: targeted regression, full cargo tests, rendered smoke test, a fresh real-provider HTTP build, and the seeded expression-edit HTTP regression under `test/run_1`.
- [x] (2026-03-30 12:35Z) Reviewed the final real-test artifacts, updated this ExecPlan with outcomes, and prepared the change set for commit.

## Surprises & Discoveries

- Observation: the existing component-generation prompt already tells the model to preserve reserved articulation-node ids during regeneration, so the root failure was state integration after regen rather than a missing prompt contract.
  Evidence: `src/gen3d/ai/prompts.rs::build_gen3d_component_user_text` lists required articulation nodes and says they must be emitted again with the same `node_id` values.
- Observation: preserving old attachment `ObjectRef` parts during regen was unnecessary once the engine rebuilt the attachment tree from `planned_components`; keeping the old refs actually made it easier for stale save-time transforms to leak through.
  Evidence: `convert::sync_attachment_tree_to_defs(...)` already clears attachment refs and reconstructs them deterministically from `planned_components`.
- Observation: the seeded real-provider regression now finishes and preserves the requested new channel, but the saved edit bundle still drops a few unrelated root/attach animation slots (`action`, `move`, `cover_face_crying`) that were already drifting in earlier runs.
  Evidence: `test/run_1/gen3d_expression_edit_regression/tmp/run__ddqvtt2/suite_report.json` reports `changed_unrelated_channels_in_bundle=["action","cover_face_crying","move"]`, and the earlier preserved report `test/run_1/gen3d_expression_edit_regression/tmp/run_y5pfavjy/suite_report.json` already showed `["action","cover_face_crying"]`.

## Decision Log

- Decision: keep the existing `motion_authoring` field, but add `motion_authoring_by_channel` as the durable source of truth.
  Rationale: this minimizes churn in current prompt summaries and bundle shape while giving regen a deterministic replay source for every authored channel.
  Date/Author: 2026-03-30 / Codex
- Decision: replay stored motion after a regenerated component is merged instead of inventing heuristic part-mapping logic.
  Rationale: the repository rule for Gen3D is “no heuristics.” Replaying already accepted authored motion against the regenerated articulation-node contract is generic and deterministic.
  Date/Author: 2026-03-30 / Codex
- Decision: route both the current batch path and the legacy orchestration path through one shared regenerated-component helper.
  Rationale: the same stale merge bug existed in both paths, so a single integration helper is the safest way to prevent future drift.
  Date/Author: 2026-03-30 / Codex

## Outcomes & Retrospective

The implementation now preserves internal motion state and attachment-tree sync across component regeneration in code, docs, deterministic tests, and the targeted seeded HTTP edit flow that originally regressed. The seeded real run completed successfully, kept the neck visible in the rendered output, and saved the requested `shy_smile` channel instead of wiping it out after regen.

Verification outcomes:

- `cargo fmt`
- `cargo test -q gen3d_component_regen_preserves_internal_motion_and_attachment_sync -- --nocapture`
- `cargo test -q`
- `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- `bash test/run_1/scripts/run_real_gen3d_http_test.sh`
- `SOURCE_PREFAB_ID=a2df98a5-3ab6-4bd0-9153-8939fc3f60c3 python3 test/run_1/gen3d_expression_edit_regression/run.py`

Residual risk:

- The seeded real regression still changes a few unrelated saved bundle channels even though it only asked for `shy_smile`. This plan fix did not target that older broader edit-roundtrip drift, so it is recorded here but left for a separate follow-up.

## Context and Orientation

Gen3D keeps two representations of an assembly. `planned_components` is the structural plan: component names, anchors, articulation nodes, attachment offsets, and authored attachment/root animation slots. `draft.defs` is the saved/rendered prefab graph: actual `ObjectDef` values, including component primitive parts and attachment `ObjectRef` parts. When a review-delta or edit session regenerates one component, the engine must update both layers consistently.

The relevant files are:

- `src/gen3d/ai/agent_motion_batch.rs`: applies one motion-authoring JSON payload to the current draft.
- `src/gen3d/ai/agent_component_batch.rs`: current component batch regen path.
- `src/gen3d/ai/orchestration.rs`: legacy regen path still used by older flow code.
- `src/gen3d/ai/convert.rs`: deterministic helpers such as `sync_attachment_tree_to_defs(...)` and `update_root_def_from_planned_components(...)`.
- `src/gen3d/ai/job.rs`, `src/gen3d/ai/workspaces.rs`, `src/gen3d/ai/snapshots.rs`, `src/gen3d/ai/edit_bundle.rs`: persisted Gen3D session state.
- `src/gen3d/ai/regression_tests.rs`: deterministic regression coverage.

“Replay motion” in this plan means: clone a stored accepted `AiMotionAuthoringJsonV1`, rewrite only its `applies_to` header to the current run state, and call the same deterministic apply function that motion authoring uses normally. No new heuristic repair layer is introduced.

## Plan of Work

Add a `motion_authoring_by_channel` map to the persisted Gen3D state so the system remembers every accepted authored channel instead of only the latest one. Update motion application so a successful `author_clips` call stores both the latest blob and the per-channel entry. Add a replay helper that selects only the stored channels whose targets mention the regenerated component name, rewrites `applies_to` to the current `attempt/plan_hash/assembly_rev`, and reapplies those channels.

Add a shared regenerated-component integration helper that performs the deterministic merge sequence in one place: update the component def and planned articulation metadata, resolve planned transforms, rebuild attachment refs from `planned_components`, replay stored motion for the regenerated component, update the draft root def, and write the assembly snapshot. Replace the duplicated merge logic in both regen call sites with that helper.

Update the Gen3D docs so the saved-state contract now explains per-channel motion persistence and replay after regen.

## Concrete Steps

From the repository root:

1. Format the Rust code.

       cargo fmt

2. Run the new deterministic regression.

       cargo test -q gen3d_component_regen_preserves_internal_motion_and_attachment_sync -- --nocapture

3. Run the broader Rust verification.

       cargo test -q

4. Run the rendered smoke test required by `AGENTS.md`.

       tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

5. Run the real-provider HTTP regression from `test/run_1`.

       python3 test/run_1/gen3d_expression_edit_regression/run.py

## Validation and Acceptance

Acceptance is met when all of the following are true:

- The deterministic regression in `src/gen3d/ai/regression_tests.rs` passes and proves that after component regen the torso still references the head with the updated offset and the regenerated head still carries `smile` and `laugh` part animation slots.
- `cargo test -q` passes.
- The rendered smoke test starts and exits without falling back to headless mode or crashing.
- The real HTTP regression in `test/run_1/gen3d_expression_edit_regression/run.py` saves an overwrite edit and confirms the requested new named expression channel still exists in the saved prefab package.

## Idempotence and Recovery

The Rust tests and smoke test are safe to rerun. The real HTTP regression uses its own isolated `test/run_1` home directory and copies a source prefab package from the user’s real home, so rerunning it does not mutate the source prefab in `~/.gravimera`. If a verification step fails, inspect the preserved artifacts under `test/run_1/` and the run cache referenced by the test output, fix the issue, and rerun the same command.

## Artifacts and Notes

Important code and artifact touch points:

- `src/gen3d/ai/component_regen.rs`
- `src/gen3d/ai/agent_motion_batch.rs`
- `src/gen3d/ai/regression_tests.rs`
- `docs/gen3d/README.md`
- `docs/gen3d/pipeline_walkthrough.md`

## Interfaces and Dependencies

The implementation must leave these interfaces in place:

- `agent_motion_batch::apply_motion_authoring_for_channel(...)` remains the single deterministic motion-apply function.
- `component_regen::apply_regenerated_component(...)` becomes the shared regen merge helper for both regen paths.
- `Gen3dEditBundleV1` continues to serialize `motion_authoring`, but also serializes `motion_authoring_by_channel` so seeded edits can replay named motion after regen.

Revision note: created this ExecPlan to document the regen-state bugfix, the new per-channel motion persistence contract, and the required verification sequence.
