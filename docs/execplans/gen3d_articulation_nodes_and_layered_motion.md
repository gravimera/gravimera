# Gen3D: articulation nodes inside components + layered motion families

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D can currently author motion only on component attachment edges. That works for whole-arm, whole-head, or whole-wheel motion, but it cannot express smaller internal motion regions when a component is generated as one rigid object. The most visible failure is facial expression: if the head is one component, the motion tool has no generic way to address eyelids, brows, jaw, or mouth regions separately.

After this change, Gen3D gains a generic internal rig abstraction called an articulation node. An articulation node is a named motion handle inside one generated component. It binds one or more primitive parts, defines a pivot and local basis in component-local space, and can be targeted by motion authoring just like existing attachment edges. The motion tool will no longer be limited to “child component only”; it can author clips for attachment edges, the root edge, and articulation nodes.

This plan also adds layered motion families. In plain language, a motion family is a bucket of channels that can be composed together instead of fighting for a single slot. The initial implementation uses two families:

- `base`: the normal body motion selected from gameplay state (`attack`, `action`, `move`, `idle`, `ambient`)
- `overlay`: an extra forced channel, typically a named motion, that composes on top of `base`

The user-visible outcome is that a generated prefab can keep its normal body motion while also playing a local named overlay motion on selected internal regions. In the Gen3D preview this means a user can select a named channel such as `blink` or `smile` and still see the base idle/move body motion underneath. In saved prefabs and spawned gameplay objects, the runtime understands the same layered part clips because articulation-node motion is expanded into ordinary part animations during Gen3D application.

How to see it working:

1. Build a Gen3D prefab with a prompt that explicitly needs internal articulation inside a component, for example a stylized robot head with eyelids and a jaw.
2. Inspect the run artifacts and confirm the component draft contains `articulation_nodes`, and later motion authoring targets those nodes.
3. In the Gen3D preview, switch to a named overlay channel and confirm the selected local expression/micro-motion composes on top of the base body motion instead of replacing it.
4. Save the prefab, seed an edit session from that prefab, and confirm the articulation-node metadata survives via the Gen3D edit bundle and can be used again by later Gen3D passes.

## Progress

- [x] (2026-03-29 21:35Z) Wrote this ExecPlan and captured the initial design goal.
- [x] (2026-03-29 21:45Z) Audited the current motion contract and confirmed the bottleneck is Gen3D contract/application, not the entire runtime.
- [x] (2026-03-29 21:52Z) Chose the smallest viable architecture: articulation nodes are stored in Gen3D state and expanded into ordinary part clips during application, rather than introducing new runtime node entities.
- [x] (2026-03-29) Implemented articulation-node data structures in Gen3D component drafts, planning state, parsing, edit bundles, and query outputs.
- [x] (2026-03-29) Generalized motion authoring from `edges[]` to generic `targets[]` for root edges, attachment edges, and articulation nodes.
- [x] (2026-03-29) Added layered motion families to part animation slots, serialization, runtime selection, and motion validation.
- [x] (2026-03-29) Updated the mock backend and focused tests so the new contracts are exercised end to end.
- [x] (2026-03-29) Ran `cargo test -q`, the rendered smoke test, and a real Automation HTTP API scenario in `test/run_1`.
- [x] (2026-03-29) Updated Gen3D docs and this plan with the final contract notes and real-run outcomes.

## Surprises & Discoveries

- Observation: the runtime is already broader than the current Gen3D motion contract. Any `ObjectPartDef` can carry `animations`, and `spawn_object_visuals_inner` adds a `PartAnimationPlayer` to any part with animation slots, not only object-ref attachment edges.
  Evidence: `src/object/registry.rs` defines `ObjectPartDef { animations: Vec<PartAnimationSlot>, ... }`, and `src/object/visuals.rs` inserts `PartAnimationPlayer` whenever `!part.animations.is_empty() || apply_aim_yaw`.

- Observation: the current Gen3D motion tool contract is explicitly attachment-edge-only.
  Evidence: `src/gen3d/ai/prompts.rs::build_gen3d_motion_authoring_system_instructions` says “author explicit per-edge animation clips” and requires the model to target child components in `edges[].component`; `src/gen3d/ai/agent_motion_batch.rs` rejects root targets and only mutates `attach_to.animations`.

- Observation: the current runtime chooses one effective channel per part, with fallback order `attack` → `action` → `move` → `idle` → `ambient`, plus an optional forced-channel override. It does not compose a second independent family.
  Evidence: `src/object/visuals.rs::update_part_animations` first checks `ForcedAnimationChannel`, then picks a single slot from the ordered channel list and applies only one sampled transform (or `fallback_basis`).

- Observation: saved prefabs do not need articulation-node metadata at runtime if motion application expands node-authored clips into ordinary part-local clips before saving.
  Evidence: saved prefabs already persist ordinary part animation slots through `src/realm_prefabs.rs` and `src/scene_store.rs`; the preview/runtime consume only those slots.

- Observation: seeded edit sessions already have a natural persistence path for Gen3D-only metadata through `gen3d_edit_bundle_v1`.
  Evidence: `src/gen3d/ai/edit_bundle.rs` stores `planned_components`, root/attachment animations, and motion authoring metadata, then rehydrates that state when seeding edits.

- Observation: the live provider naturally drifted from the intended plan contract by emitting `rig.articulation_nodes`, `rig.named_motions`, and keyed-object `anchors` maps.
  Evidence: the real Automation HTTP run under `test/run_1` produced a raw plan response in `attempt_0/steps/step_0001/tool_plan_pipe_llm_generate_plan_v1_s1_a0_responses_raw.txt` that was structurally useful but misplaced motion metadata under `rig` and sometimes represented array-valued fields as maps.

- Observation: deterministic parser-side normalization was enough to salvage that live-provider drift without adding heuristics or a face-specific fallback.
  Evidence: `src/gen3d/ai/parse.rs` now hoists `rig.articulation_nodes` into `components[].articulation_nodes`, drops unsupported `rig.named_motions`, and normalizes keyed object maps into the required arrays before strict schema parsing.

## Decision Log

- Decision: articulation nodes will be stored in Gen3D state (`AiDraftJsonV1`, `Gen3dPlannedComponent`, edit bundles, query outputs) and expanded into ordinary part animation slots when applying motion.
  Rationale: this gives Gen3D a stable rig abstraction without forcing a large runtime scene-graph refactor. Saved/runtime prefabs stay simple because they only need ordinary part clips.
  Date/Author: 2026-03-29 / assistant

- Decision: motion authoring will move from the attachment-edge-specific `edges[]` schema to a generic `targets[]` schema.
  Rationale: the current `edges[].component` naming hardcodes one kind of target. A generic target model is required to support root motion and internal articulation nodes without face-specific exceptions.
  Date/Author: 2026-03-29 / assistant

- Decision: the first layered implementation will use exactly two motion families, `base` and `overlay`.
  Rationale: two families are enough to prove useful composition immediately and map cleanly to the current preview/runtime state. This is the smallest change that solves “named expression overlays on top of body motion” without inventing a larger scheduler up front.
  Date/Author: 2026-03-29 / assistant

- Decision: the `overlay` family will be driven by `ForcedAnimationChannel` in the runtime and preview, while `base` continues to be selected from gameplay activity state.
  Rationale: this reuses existing preview and runtime plumbing and gives a concrete end-to-end path to validate layered motion through the current UI and Automation HTTP API.
  Date/Author: 2026-03-29 / assistant

- Decision: articulation nodes will describe explicit bindings to parts by stable `part_id`, not by implicit geometry heuristics.
  Rationale: Gen3D has a repository rule against heuristic object-specific algorithms. Explicit node bindings keep the system generic and deterministic.
  Date/Author: 2026-03-29 / assistant

## Outcomes & Retrospective

- Implemented the chosen architecture: articulation nodes live in Gen3D planning/edit state and component drafts, and articulation-targeted motion is expanded into ordinary part clips before runtime/save.

- The runtime now composes two animation families per part:
  - `base` from gameplay state
  - `overlay` from `ForcedAnimationChannel`

- The planning contract was tightened to keep plans structural:
  - plan-time `components[].articulation_nodes[]` reserve node ids + local bases only
  - component drafts provide the explicit `bind_part_indices`
  - named motion clips are authored later by the motion tool, not inside plans

- Real validation completed successfully with the user’s configured provider from `~/.gravimera/config.toml`.
  - Script: `PORT=18792 NO_PROXY=127.0.0.1,localhost test/run_1/scripts/run_real_gen3d_http_test.sh`
  - Result: success
  - Run id: `c3312809-6913-4790-a3e0-7afdfa279a23`
  - Run dir: `/Users/flow/workspace/github/gravimera/test/run_1/home/cache/gen3d/c3312809-6913-4790-a3e0-7afdfa279a23`
  - Saved prefab id: `ed77bc79-4aaa-40c2-93d3-e28b2e180381`
  - Saved edit bundle: `/Users/flow/workspace/github/gravimera/test/run_1/home/realm/default/prefabs/ed77bc79-4aaa-40c2-93d3-e28b2e180381/gen3d_edit_bundle_v1.json`

- The successful live run demonstrated the intended end-to-end behavior:
  - the plan completed without schema-repair failure
  - the generated component draft included `articulation_nodes`
  - motion authoring emitted `targets[].kind == "articulation_node"` plus `family="overlay"` for named local channels like `blink` and `jaw_open`
  - save + seeded edit preserved articulation metadata and reused the same prefab id for overwrite

## Context and Orientation

This repository’s Gen3D flow has three layers relevant to this feature.

The first layer is component generation. The component LLM returns `AiDraftJsonV1` in `src/gen3d/ai/schema.rs`, parsed by `src/gen3d/ai/parse.rs`, then converted into `ObjectDef` component defs by `src/gen3d/ai/convert.rs::ai_to_component_def`. Today that schema includes anchors, collider, and primitive parts, but it does not include any internal motion rig information.

The second layer is Gen3D planning/edit state. `src/gen3d/ai/job.rs` defines `Gen3dPlannedComponent`, which currently stores component placement, anchors, contacts, root-edge motion, and attachment-edge motion. `src/gen3d/ai/edit_bundle.rs` persists this planned state into `gen3d_edit_bundle_v1.json` so seeded edit sessions can resume with the same structure. `src/gen3d/ai/draft_ops.rs::query_component_parts_v1` exposes per-component part snapshots to later edit tools.

The third layer is motion application and playback. `src/gen3d/ai/agent_motion_batch.rs` applies LLM motion authoring by mutating `attach_to.animations`, then `src/gen3d/ai/convert.rs::sync_attachment_tree_to_defs` copies those attachment slots into `ObjectDef` object-ref parts. At runtime, `src/object/visuals.rs::update_part_animations` picks one slot per part and applies it to the spawned visuals. Motion QA mirrors that behavior in `src/gen3d/ai/motion_validation.rs`.

Key terms used in this plan:

- Component: one generated object definition in Gen3D. A unit is assembled from multiple components linked by `attach_to`.
- Primitive part: one low-level geometric element inside a component (`cuboid`, `cylinder`, `sphere`, `cone`).
- Articulation node: a named internal motion handle inside one component. It has a stable id, a pivot and basis in component-local space, and a list of bound parts identified by stable `part_id`.
- Motion family: a compositing bucket for channels. In this plan `base` is the normal body family and `overlay` is an extra forced family composed on top.
- Target: one motion authoring destination. After this change a target may be the root edge, an attachment edge, or an articulation node.

Files that will be edited:

- `src/gen3d/ai/schema.rs`: add articulation-node draft and motion-target schemas.
- `src/gen3d/ai/structured_outputs.rs`: update component and motion structured-output schemas.
- `src/gen3d/ai/parse.rs`: parse and validate the new schemas.
- `src/gen3d/ai/prompts.rs`: teach component generation and motion authoring about articulation nodes and motion families.
- `src/gen3d/ai/job.rs`: store articulation nodes on planned components.
- `src/gen3d/ai/convert.rs`: ingest articulation nodes from component drafts, keep them in planned state, and expand node-authored motion into part slots.
- `src/gen3d/ai/agent_motion_batch.rs`: apply generic motion targets instead of attachment-edge-only outputs.
- `src/gen3d/ai/draft_ops.rs`: expose articulation nodes in `query_component_parts_v1`.
- `src/gen3d/ai/edit_bundle.rs`: persist articulation-node state through seeded edits.
- `src/object/registry.rs`, `src/object/visuals.rs`, `src/scene_store.rs`, `src/realm_prefabs.rs`: add layered motion family support to ordinary part animation slots and serialization.
- `src/gen3d/ai/motion_validation.rs`: mirror the new layered composition.
- `src/gen3d/ai/openai.rs` and tests: update the mock backend and test fixtures for new schema contracts.
- `docs/gen3d/README.md` and `docs/gen3d/pipeline_walkthrough.md`: document the new model and validation flow.

## Plan of Work

The implementation proceeds in four connected slices.

The first slice adds articulation nodes to Gen3D component state. In `src/gen3d/ai/schema.rs`, define an articulation-node draft schema under `AiDraftJsonV1`, with stable `node_id`, optional `parent_node_id`, `transform` in component-local space, and `bind_part_indices` referencing the component draft’s `parts[]`. Parsing in `src/gen3d/ai/parse.rs` must reject duplicate node ids, unknown parent references, cycles, unknown part indices, and duplicate part bindings. Conversion in `src/gen3d/ai/convert.rs::ai_to_component_def` must map `bind_part_indices` to the generated stable `part_id`s and store the resulting articulation nodes on `Gen3dPlannedComponent`. This slice also updates `src/gen3d/ai/edit_bundle.rs` to persist/reload articulation nodes and updates `src/gen3d/ai/draft_ops.rs::query_component_parts_v1` to report them back to the agent as structured metadata alongside the part snapshots.

The second slice generalizes motion authoring to generic targets. Replace the current `edges[]`-based motion schema in `src/gen3d/ai/schema.rs` and `src/gen3d/ai/structured_outputs.rs` with a `targets[]` schema. Each target must include a `kind` field with exactly one of:

- `root_edge`: the implicit draft-root → root-component edge
- `attachment_edge`: the parent → child component attachment edge, identified by child `component`
- `articulation_node`: an internal node inside a component, identified by `component` + `node_id`

Every authored slot also gains a required `family` field set to `base` or `overlay`. `src/gen3d/ai/prompts.rs` must rewrite the motion prompt to describe these target kinds, define when `family=overlay` is appropriate, and print articulation-node summaries in the motion user text. `src/gen3d/ai/agent_motion_batch.rs` must validate the new contract and route application by target kind. Attachment and root targets continue to mutate the existing planned attachment/root slot vectors. Articulation-node targets must be expanded into ordinary part animation slots on the affected component defs: for each bound part and each authored keyframe delta in node space, compute the equivalent part-local delta and write ordinary `PartAnimationSlot`s to the part. This expansion must be deterministic and must not rely on geometry heuristics.

The third slice adds layered motion families to ordinary part playback. In `src/object/registry.rs`, extend `PartAnimationSlot` with a required family enum (`base` or `overlay`). Update every constructor and persistence path in `src/scene_store.rs`, `src/realm_prefabs.rs`, and `src/gen3d/ai/edit_bundle.rs`. In `src/object/visuals.rs::update_part_animations`, keep the current base-family selection order for gameplay state, but independently choose at most one overlay-family slot using `ForcedAnimationChannel` when a matching overlay slot exists. Compose the sampled transforms in a fixed order:

1. base transform / fallback basis
2. selected base-family slot if any
3. selected overlay-family slot if any

The overlay family must not suppress the base family. `src/gen3d/ai/motion_validation.rs` must mirror this exact selection and composition order so QA sees the same behavior as runtime. `src/object/registry.rs` helper methods that enumerate channels or compute durations must include overlay-family channels as discoverable channels without changing the existing canonical order of `idle`, `move`, `action`, and `attack`.

The fourth slice updates tests, docs, and real validation. Unit tests should cover articulation-node parsing/validation, node-to-part expansion math, layered-family playback selection, and seeded-edit persistence. The mock backend in `src/gen3d/ai/openai.rs` must be extended so pipeline tests can emit articulation nodes for a relevant prompt and motion targets for an articulation node overlay channel. Documentation in `docs/gen3d/README.md` and `docs/gen3d/pipeline_walkthrough.md` must describe articulation nodes, target kinds, and motion families without overloading the top-level README. Finally, real validation must be scripted under `test/run_1` using the Automation HTTP API and a rendered game process configured from `~/.gravimera/config.toml`.

## Concrete Steps

All commands are run from the repository root: `/Users/flow/workspace/github/gravimera`.

During implementation:

1. Edit the files listed in `Context and Orientation`.
2. Run focused tests repeatedly while the contracts are changing.

   Suggested commands:

       cargo test gen3d::ai::parse -q
       cargo test gen3d::ai::prompts -q
       cargo test gen3d::ai::pipeline_orchestrator_tests -q
       cargo test gen3d::ai::regression_tests -q
       cargo test object::visuals -q

3. Run the full test suite before finalizing:

       cargo test

4. Run the required rendered smoke test:

       tmpdir=$(mktemp -d)
       GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2

5. Run a real Automation HTTP API scenario from files under `test/run_1`.

   The expected layout is:

       test/run_1/
         config.toml
         home/
         logs/
         responses/
         scripts/

   The test script should:

   - read the user’s existing provider settings from `~/.gravimera/config.toml`
   - write an isolated automation-enabled config to `test/run_1/config.toml`
   - start Gravimera in rendered mode with `GRAVIMERA_HOME=test/run_1/home`
   - call `/v1/health`, `/v1/mode`, `/v1/gen3d/prompt`, `/v1/gen3d/build`
   - step frames while polling `/v1/gen3d/status`
   - inspect `run_dir` artifacts to confirm articulation-node and motion-target artifacts exist
   - call `/v1/gen3d/save`
   - seed `/v1/gen3d/edit_from_prefab` and confirm the edit bundle still contains articulation-node metadata

6. Commit once tests and docs are complete:

       git status
       git add <changed files>
       git commit -m "Add gen3d articulation nodes and layered motion"

## Validation and Acceptance

This feature is complete only when all of the following are true.

For schema and parsing:

- A component draft with valid articulation nodes parses successfully.
- Duplicate node ids, cyclic node parents, out-of-range part bindings, or duplicate part bindings fail with actionable errors.

For motion authoring:

- The motion prompt and schema allow targets of kind `root_edge`, `attachment_edge`, and `articulation_node`.
- Applying an `articulation_node` target writes ordinary part animation slots to the affected component parts.
- A motion authoring result that uses `family=overlay` on an articulation node is accepted and persisted.

For runtime and QA:

- `update_part_animations` composes `base` plus `overlay` instead of letting overlay replace base.
- `motion_validation` mirrors the same selection/composition order.
- The Gen3D preview can select a named overlay channel while base idle/move body motion still plays underneath.

For persistence:

- `gen3d_edit_bundle_v1.json` stores and reloads articulation nodes.
- Saving a prefab preserves the expanded ordinary part animation slots needed for runtime playback.
- Seeding an edit session from that saved prefab restores articulation-node metadata from the edit bundle.

For tests:

- `cargo test` passes.
- The rendered smoke test runs for two seconds without crashing.
- The real HTTP scenario in `test/run_1` completes and produces saved JSON/log evidence showing articulation-node metadata in the run artifacts and seeded edit bundle.

## Idempotence and Recovery

This work should be implemented additively and safely.

The Automation HTTP validation must use an isolated home directory under `test/run_1/home` so rerunning the test does not pollute the user’s normal `~/.gravimera` data. The generated `test/run_1/config.toml` may be overwritten on each run. If a previous test server is still running, stop it before rerunning the script or choose a new local port. If a real Gen3D build stalls or fails due to provider/network issues, keep the run artifacts under `test/run_1/responses/` and rerun only the script after fixing the environment; no source rollback should be necessary.

Because the repository does not currently require backward compatibility for Gen3D source/edit formats, it is acceptable to evolve Gen3D-only JSON contracts. However, persisted runtime prefab formats (`scene.grav` and realm prefab JSON) should continue to default missing new fields safely so existing test assets and built-ins still load.

## Artifacts and Notes

Important implementation note for articulation-node expansion:

When a node with world-in-component transform `N` drives a bound part with base transform `P`, and the authored node-space delta at time `t` is `D(t)`, the equivalent ordinary part-local transform is:

    delta_part(t) = inverse(P) * N * D(t) * inverse(N) * P

This conversion is what lets articulation nodes remain a Gen3D authoring abstraction while the saved/runtime prefab continues to use ordinary part animation slots.

Important implementation note for layered playback:

The first version of layering should stay small. One part may have:

- zero or one selected `base` slot
- zero or one selected `overlay` slot

The overlay slot composes after the base slot. If no overlay slot matches the forced channel, playback remains exactly the current base-only behavior.

Expected real-test evidence to capture in this plan after implementation:

    - HTTP `GET /v1/gen3d/status` response containing a real `run_dir`
    - `prompt_intent.json` or motion artifacts showing named channels used in the run
    - generated component artifact showing `articulation_nodes`
    - motion artifact showing `targets[].kind == "articulation_node"`
    - seeded edit bundle excerpt showing articulation nodes persisted

## Interfaces and Dependencies

No new external crates are required.

At the end of this plan, the following interfaces must exist.

In `src/gen3d/ai/schema.rs`, define articulation-node draft types and generic motion-target types with stable names. The motion-authoring slot type must include a required family enum:

    pub(crate) enum AiMotionTargetKindJsonV1 {
        RootEdge,
        AttachmentEdge,
        ArticulationNode,
        Unknown,
    }

    pub(crate) enum AiAnimationFamilyJsonV1 {
        Base,
        Overlay,
        Unknown,
    }

In `src/gen3d/ai/job.rs`, `Gen3dPlannedComponent` must contain articulation-node metadata sufficient to rehydrate seeded edits and build motion prompts:

    pub(super) struct Gen3dPlannedArticulationNode {
        pub(super) node_id: String,
        pub(super) parent_node_id: Option<String>,
        pub(super) transform: Transform,
        pub(super) bound_part_ids: Vec<u128>,
    }

In `src/object/registry.rs`, `PartAnimationSlot` must carry a motion family enum so runtime and persistence can distinguish `base` from `overlay`.

In `src/gen3d/ai/agent_motion_batch.rs`, motion application must accept generic targets and expand articulation-node clips into ordinary part animation slots deterministically.

In `src/object/visuals.rs`, runtime playback must independently select a base-family slot and an overlay-family slot and compose them in a fixed order.

Revision note: this initial version of the plan records the chosen “expand articulation nodes into part clips” architecture because the audit showed it is materially smaller and safer than adding new runtime node entities, while still delivering the user-visible capability the feature needs.
