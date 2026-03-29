# Gen3D: deterministic articulation-node DraftOps

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

This plan builds on the shipped articulation-node architecture described in `docs/execplans/gen3d_articulation_nodes_and_layered_motion.md`. That earlier work added articulation nodes to Gen3D planning, component drafts, motion authoring, preview playback, and edit-bundle persistence. This plan adds the missing deterministic edit tools so seeded edit sessions can change articulation nodes without regenerating component geometry.

## Purpose / Big Picture

Gen3D can now generate articulation nodes inside a component, but seeded edit sessions cannot yet modify that internal rig directly. A user who says “keep this component looking the same, but make these internal parts animatable” still lacks a deterministic tool path for adding, removing, or rebinding articulation nodes on an existing component.

After this change, a seeded Edit session can keep the same component geometry while editing only internal articulation metadata when the needed primitive parts already exist. The user-visible result is that Gen3D can preserve a component’s outer look, update its internal motion handles in place, and then reuse those handles in later motion-authoring passes. You can see it working by seeding an edit session from a saved prefab, applying `apply_draft_ops_v1` articulation-node ops, saving again, and confirming the saved `gen3d_edit_bundle_v1.json` reflects the rig changes without regenerating the component.

## Progress

- [x] (2026-03-30 00:28Z) Audited the current seeded-edit path and confirmed the gap: `query_component_parts_v1` exposes `articulation_nodes[]`, but `apply_draft_ops_v1` cannot mutate them yet.
- [x] (2026-03-30 00:31Z) Verified the repo-local prompt/tool contract docs referenced by `AGENTS.md` are not present, so this plan uses a direct source audit of `draft_ops.rs`, `structured_outputs.rs`, `prompts.rs`, and `agent_tool_poll.rs`.
- [x] (2026-03-29 15:30Z) Implemented deterministic DraftOps for articulation-node add/update, remove, and rebind, including component-level articulation validation and bound-part removal guards.
- [x] (2026-03-29 15:30Z) Updated query snapshots, structured-output schema, prompt instructions, and DraftOps validation so LLM-suggested articulation-node ops are accepted or rejected actionably.
- [x] (2026-03-29 15:30Z) Updated Gen3D and Automation HTTP docs to describe in-place articulation-node editing, and verified the real test path exercises remove/upsert/rebind through `/v1/gen3d/apply_draft_ops`.
- [x] (2026-03-29 15:30Z) Ran `cargo fmt`, `cargo test -q`, the required rendered smoke test, and the live provider-backed HTTP validation. Commit is the only remaining step.

## Surprises & Discoveries

- Observation: the missing piece is not persistence or runtime support; it is only the deterministic edit-tool layer.
  Evidence: `src/gen3d/ai/edit_bundle.rs` already persists `articulation_nodes`, and `src/gen3d/ai/draft_ops.rs::query_component_parts_v1` already returns them in edit snapshots.

- Observation: `apply_draft_ops_v1` can already mutate primitive parts, anchors, joints, attachment offsets, and edge/root animation slots, but there is no op kind for articulation nodes.
  Evidence: `src/gen3d/ai/draft_ops.rs::DraftOpJsonV1` currently stops at `remove_animation_slot`.

- Observation: seeded-edit LLM validation performs a second gate in `agent_tool_poll.rs`, so adding new DraftOps requires prompt/schema changes and a matching validation update; changing only `apply_draft_ops_v1` would still cause LLM-suggested ops to be rejected.
  Evidence: `src/gen3d/ai/agent_tool_poll.rs` has explicit allowed-key validation and per-kind semantic checks for DraftOps.

- Observation: removing a primitive part that is still bound to an articulation node would silently leave stale rig metadata today.
  Evidence: `remove_primitive_part` updates the component def size but does not inspect `planned_component.articulation_nodes`.

## Decision Log

- Decision: articulation-node edits will be added to `apply_draft_ops_v1` instead of creating a separate deterministic tool.
  Rationale: articulation nodes are part of the same seeded-edit patch language as primitive edits and attachment edits. Reusing DraftOps preserves atomic application, `if_assembly_rev` gating, actionable diffs, and the existing edit pipeline.
  Date/Author: 2026-03-30 / assistant

- Decision: the new deterministic ops will be `upsert_articulation_node`, `remove_articulation_node`, and `rebind_articulation_node_parts`.
  Rationale: this directly covers the missing generic rig-edit cases: add-or-update a node, remove a node, and change bindings without geometry heuristics.
  Date/Author: 2026-03-30 / assistant

- Decision: articulation-node bindings will keep the same invariant used by generation: one primitive part may belong to at most one articulation node in a component.
  Rationale: the original articulation-node parser already rejects duplicate part bindings across nodes. Keeping the invariant consistent avoids ambiguous later motion application.
  Date/Author: 2026-03-30 / assistant

- Decision: removing a primitive part that is still bound to an articulation node will be rejected with an actionable error instead of auto-unbinding.
  Rationale: explicit failure is safer and keeps the system generic. Auto-unbinding would silently mutate rig semantics.
  Date/Author: 2026-03-30 / assistant

## Outcomes & Retrospective

The feature landed on the existing DraftOps path instead of introducing a separate regeneration-only edit contract. Seeded edits can now add or update articulation nodes, remove leaf articulation nodes, and rebind articulation-node part ownership without regenerating the component shell. The engine validates these edits both at LLM-contract time and at apply time, so invalid node ids, missing parents, duplicate bindings, cycles, and stale bound-part removals fail with actionable errors.

The provider-backed end-to-end test also proved the intended user workflow. Using `test/run_1/scripts/run_real_gen3d_http_test.sh`, Gen3D generated a single-component robot head with internal articulation nodes and named motions, saved it, seeded an edit session from the saved prefab, then successfully applied `remove_articulation_node`, `upsert_articulation_node`, and `rebind_articulation_node_parts` through `/v1/gen3d/apply_draft_ops`. The live responses showed `assembly_rev` advancing from 1 to 4, and the saved `gen3d_edit_bundle_v1.json` still contained the restored articulation node afterward.

Validation completed on the finished tree:

- `cargo fmt`
- `cargo test -q`
- `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- `PORT=18792 NO_PROXY=127.0.0.1,localhost test/run_1/scripts/run_real_gen3d_http_test.sh`

## Context and Orientation

The seeded-edit pipeline already exists. The relevant flow is:

1. `src/gen3d/ai/draft_ops.rs::query_component_parts_v1` produces per-component snapshots containing editable primitive parts plus `articulation_nodes[]`.
2. `src/gen3d/ai/prompts.rs::build_gen3d_draft_ops_*` tells the LLM how to suggest deterministic DraftOps.
3. `src/gen3d/ai/structured_outputs.rs` constrains the LLM output to a strict DraftOps schema.
4. `src/gen3d/ai/agent_tool_poll.rs` re-validates the LLM output against known component names and part ids before accepting it.
5. `src/gen3d/ai/draft_ops.rs::apply_draft_ops_v1` applies the deterministic edits atomically and increments `assembly_rev`.
6. `src/gen3d/ai/edit_bundle.rs` persists the updated `planned_components` state, including articulation nodes, when the user saves.

Important terms used in this plan:

- Articulation node: a generic internal motion handle inside one component. It has a stable `node_id`, an optional `parent_node_id`, a local transform, and explicit `bound_part_ids`.
- DraftOps: the deterministic JSON patch language used by seeded edits. The engine already supports primitive, anchor, attachment, and animation-slot DraftOps.
- Seeded edit: a Gen3D session created from a previously saved prefab by `POST /v1/gen3d/edit_from_prefab`.

Files that need coordinated changes:

- `src/gen3d/ai/draft_ops.rs`: add the new op kinds, validation helpers, diff accounting, and query recipes.
- `src/gen3d/ai/structured_outputs.rs`: extend the strict DraftOps JSON schema.
- `src/gen3d/ai/prompts.rs`: teach `llm_generate_draft_ops_v1` when and how to use the new articulation-node ops.
- `src/gen3d/ai/agent_tool_poll.rs`: extend DraftOps normalization/validation so LLM output using the new ops is accepted or rejected actionably.
- `docs/gen3d/README.md` and `docs/gen3d/pipeline_walkthrough.md`: document that seeded edits can now modify articulation nodes in place.
- `docs/automation_http_api.md`: document the existing `POST /v1/gen3d/apply_draft_ops` endpoint well enough to show articulation-node edits.
- `test/run_1/scripts/run_real_gen3d_http_test.sh`: extend the real test to apply articulation-node DraftOps in a seeded edit session.

## Plan of Work

The first slice extends the DraftOps patch language in `src/gen3d/ai/draft_ops.rs`. Add three new op kinds. `upsert_articulation_node` must add a missing node or update an existing node by `node_id` on a named component, with explicit parent id, transform update, and full `bound_part_id_uuids`. `remove_articulation_node` must remove a node by id, but reject the operation if any other node still lists it as `parent_node_id`. `rebind_articulation_node_parts` must keep the node but replace its part bindings. Add a shared helper that validates a component’s articulation-node list against its current primitive parts: node ids must be unique and non-empty, parents must exist, parent cycles are forbidden, bindings must be non-empty, every bound part id must exist on a primitive part, and no primitive part may be bound by more than one node. Reuse this helper after every articulation-node edit. Also reject `remove_primitive_part` when the part is still bound by a node.

The second slice extends the DraftOps agent contract. Update `src/gen3d/ai/structured_outputs.rs` so `gen3d_draft_ops_v1` allows the three new op kinds with no extra keys. Update `src/gen3d/ai/prompts.rs` so the DraftOps instructions explicitly tell the model to use articulation-node ops when the user wants to make existing internal parts animatable without changing component geometry. The prompt must state that `remove_articulation_node` and `rebind_articulation_node_parts` may only reference existing `node_id` values from the snapshots, while `upsert_articulation_node` may introduce a new `node_id` when needed. Update `src/gen3d/ai/agent_tool_poll.rs` so its DraftOps validator recognizes the new kinds, validates their keys, checks component names, checks part ids against the current snapshot, and checks existing node ids for remove/rebind operations.

The third slice makes the new behavior observable. Extend `query_component_parts_v1` in `src/gen3d/ai/draft_ops.rs` so its `recipes` section includes copy/pasteable examples for articulation-node add/update, rebind, and remove. Update docs in `docs/gen3d/README.md` and `docs/gen3d/pipeline_walkthrough.md` to explain that seeded edits can now adjust internal articulation metadata in place. Add an Automation HTTP API section for `POST /v1/gen3d/apply_draft_ops` because the real test and external agents rely on it. Extend `test/run_1/scripts/run_real_gen3d_http_test.sh` so, after `edit_from_prefab`, it reads the saved edit bundle, chooses a leaf articulation node, removes it through `/v1/gen3d/apply_draft_ops`, recreates it with `upsert_articulation_node`, rebinds it with `rebind_articulation_node_parts`, saves again, and verifies the updated edit bundle still contains articulation nodes.

The fourth slice is validation. Add focused unit tests in `src/gen3d/ai/draft_ops.rs` for upsert, remove, rebind, and the new “cannot remove bound primitive part” rejection. Add prompt tests in `src/gen3d/ai/prompts.rs` that assert the DraftOps instructions mention the new op kinds and the no-regen rig-edit guidance. Then run `cargo fmt`, `cargo test -q`, the required rendered smoke test, and the real HTTP script under `test/run_1`.

## Concrete Steps

All commands run from `/Users/flow/workspace/github/gravimera`.

During implementation:

1. Edit `src/gen3d/ai/draft_ops.rs`, `src/gen3d/ai/structured_outputs.rs`, `src/gen3d/ai/prompts.rs`, `src/gen3d/ai/agent_tool_poll.rs`, docs, and the real test script.
2. Run focused tests while changing DraftOps:

       cargo test -q gen3d::ai::draft_ops
       cargo test -q gen3d::ai::prompts

3. Run the full suite:

       cargo test -q

4. Run the required rendered smoke test:

       tmpdir=$(mktemp -d)
       GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

5. Run the real provider-backed HTTP test:

       PORT=18792 NO_PROXY=127.0.0.1,localhost test/run_1/scripts/run_real_gen3d_http_test.sh

6. Commit the finished work:

       git status --short
       git add <changed files>
       git commit -m "Add gen3d articulation node draft ops"

## Validation and Acceptance

This work is complete when all of the following are true.

For deterministic DraftOps:

- `apply_draft_ops_v1` accepts `upsert_articulation_node`, `remove_articulation_node`, and `rebind_articulation_node_parts`.
- `remove_articulation_node` rejects removal of a node that still has children.
- `remove_primitive_part` rejects removal of a primitive part still bound to an articulation node.
- Applying articulation-node DraftOps increments `assembly_rev` and reports articulation-node changes in the result diff.

For LLM/tool contract alignment:

- The DraftOps structured-output schema accepts the new op kinds.
- The DraftOps system instructions mention the new op kinds and explain when to use them instead of regeneration.
- `agent_tool_poll.rs` accepts valid new ops and rejects malformed ones with actionable errors.

For persistence and end-to-end behavior:

- `query_component_parts_v1` exposes articulation nodes plus sample recipes for articulation-node DraftOps.
- After `/v1/gen3d/edit_from_prefab`, `POST /v1/gen3d/apply_draft_ops` can remove, recreate, and rebind a leaf articulation node.
- After saving, the resulting `gen3d_edit_bundle_v1.json` still contains the expected articulation-node metadata.

For validation commands:

- `cargo test -q` passes.
- The rendered smoke test runs for two seconds without crashing.
- The real HTTP script succeeds against the user’s configured provider.

## Idempotence and Recovery

The new deterministic ops are idempotent when repeated with the same payload and `if_assembly_rev` is updated to the current revision. If an articulation-node edit fails because the assembly revision changed, rerun `query_component_parts_v1` in the active workspace, regenerate or refresh the DraftOps payload, and retry with the new revision. The real test continues using `test/run_1/home` as an isolated sandbox, so reruns do not affect the user’s normal `~/.gravimera` data.

## Artifacts and Notes

The key success artifacts for this plan are:

- `attempt_*/steps/step_*/draft_ops_suggested_last.json` when the LLM suggests articulation-node DraftOps.
- `attempt_*/steps/step_*/apply_draft_ops_last.json` showing articulation-node diffs and the new `assembly_rev`.
- `test/run_1/responses/*.json` proving the Automation HTTP API accepted the new ops.
- `test/run_1/home/realm/default/prefabs/<prefab_id>/gen3d_edit_bundle_v1.json` showing the saved articulation-node metadata after the edit.

## Interfaces and Dependencies

At the end of this plan, `src/gen3d/ai/draft_ops.rs` must support DraftOps JSON with these additional shapes:

    {"kind":"upsert_articulation_node","component":"head","node_id":"visor_upper","parent_node_id":"head_core","set_transform":{"pos":[0.0,0.1,0.2],"rot_quat_xyzw":[0.0,0.0,0.0,1.0]},"bound_part_id_uuids":["..."]}

    {"kind":"remove_articulation_node","component":"head","node_id":"visor_upper"}

    {"kind":"rebind_articulation_node_parts","component":"head","node_id":"visor_upper","bound_part_id_uuids":["...","..."]}

The engine must keep using existing repository-local types and modules:

- `crate::gen3d::ai::job::Gen3dPlannedArticulationNode`
- `crate::gen3d::ai::draft_ops::apply_draft_ops_v1`
- `crate::gen3d::ai::structured_outputs::Gen3dAiJsonSchemaKind::DraftOpsV1`
- `crate::gen3d::ai::prompts::build_gen3d_draft_ops_system_instructions`
- `crate::gen3d::ai::agent_tool_poll::poll_agent_tool`

Revision note: created this follow-on ExecPlan on 2026-03-30 because articulation nodes were already implemented for generation and motion, but seeded edits still lacked deterministic rig-edit ops.
