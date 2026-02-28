# Gen3D: Copy Component Subtree Tool (Symmetric Limbs / Wheels)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repo includes `PLANS.md` at the repository root. This ExecPlan must be maintained in accordance with `PLANS.md`.

## Purpose / Big Picture

Gen3D frequently generates only one symmetric limb (one wheel, one leg, one arm), or generates symmetric limbs with inconsistent geometry. The AI already has `copy_component_v1`, but copying an entire limb chain (a “subtree” of attached components) is still tedious and error-prone because the agent must call `copy_component_v1` repeatedly for each link.

After this change, the Gen3D agent can copy an entire attached component subtree in one tool call. This makes symmetric models (cars with 4 wheels, quadrupeds with 4 legs, humanoids with 2 arms) faster, cheaper, and more consistent.

You can see this working by generating a multi-limb object (e.g. a warcar) and observing in the Gen3D cache that the agent uses the new tool and that multiple limbs appear in renders without separate per-limb generation calls.

## Progress

- [x] (2026-02-04 01:45Z) Write this ExecPlan.
- [x] (2026-02-04 01:55Z) Implement `copy_component_subtree_v1` tool (engine-side) and expose it in the tool registry.
- [x] (2026-02-04 01:55Z) Improve `copy_component_v1` with `anchors=preserve_target` option for detached copies (prevents attachment-frame flips).
- [x] (2026-02-04 01:56Z) Add unit tests for subtree copy behavior (copies geometry while preserving target anchors and attachment refs).
- [x] (2026-02-04 01:59Z) Update agent prompt guidance and tool docs, update `README.md`, run `cargo test` and a headless smoke run, commit.

## Surprises & Discoveries

- Observation: `copy_component_v1` detached copies currently overwrite the target anchors with the source anchors, which can unintentionally change assembly join frames (e.g. wheel spin axis flips).
  Evidence: `src/gen3d/ai/copy_component.rs` detached mode applies delta to the source anchors and writes them into the target def.

- Observation: In some sandboxed environments, binding an ephemeral localhost TCP port for the automation API smoke test can fail with `PermissionDenied`.
  Evidence: `tests/automation_api_smoke.rs` can panic on `TcpListener::bind("127.0.0.1:0")` with `Operation not permitted`.

## Decision Log

- Decision: Copy subtrees by structural matching (parent/child relationships and attachment anchor names), not by string name heuristics.
  Rationale: Component names are AI-controlled and can vary; attachment structure is the reliable invariant for symmetric chains.
  Date/Author: 2026-02-04 / Codex

- Decision: Default subtree copy preserves the target anchors and target attachment refs.
  Rationale: Anchors define the “interface” used by attachments; preserving them avoids changing placement/axes while still letting geometry become consistent.
  Date/Author: 2026-02-04 / Codex

- Decision: `copy_component_subtree_v1` initially supports only `mode=detached`.
  Rationale: Linked copies require leaf components; a subtree root usually has children and would fail. Detached subtree copies are the safe default for “fill missing limbs”.
  Date/Author: 2026-02-04 / Codex

## Outcomes & Retrospective

This change adds two key robustness improvements:

1. `copy_component_v1` can preserve target anchors in detached mode (`anchors=preserve_target`) so symmetric copies do not accidentally change join frames.
2. `copy_component_subtree_v1` provides a single-call way to copy a full limb chain subtree from one generated limb to other planned limbs.

This should reduce common “only one leg/wheel exists” failures and reduce generation variance for symmetric parts. If future runs show subtree mapping errors, the next refinement should be to include clearer shape-mismatch reporting (paths) and to optionally tolerate target extra children.

## Context and Orientation

Gen3D represents models as a graph of “components” (each is an `ObjectDef`) joined by attachments. Each component can contain primitive parts and attachment `ObjectRef` parts to its children.

Relevant code:

- `src/gen3d/ai/agent_step.rs` + `src/gen3d/ai/agent_tool_dispatch.rs`: Executes tool calls from the AI agent (`*_v1` tool protocol).
- `src/gen3d/agent/tools.rs`: Tool registry: ids, list, and describe payloads.
- `src/gen3d/ai/copy_component.rs`: Current `copy_component_into` and `detach_component_copy` helpers.
- `src/gen3d/ai/convert.rs`: `resolve_planned_component_transforms` computes assembled transforms using anchors/attachments.

Terms:

- “Component subtree”: a planned component plus all descendants reachable via `attach_to.parent == this_component.name`.
- “Anchors”: named local frames in a component used to connect attachments; changing anchors changes assembly transforms.
- “Attachment refs”: `ObjectPartDef` entries of kind `ObjectRef` with `attachment` set; these encode parent->child links.

## Plan of Work

### Milestone 1: Extend copy helper to support anchor preservation

In `src/gen3d/ai/copy_component.rs`:

1. Add an argument to `copy_component_into` to choose how to handle anchors when copying geometry:
   - `CopySourceAnchors` (current behavior) or
   - `PreserveTargetAnchors` (recommended for symmetric copies).
2. For `Gen3dCopyMode::Detached`, when `PreserveTargetAnchors` is selected:
   - Copy only the source geometry primitives (with optional delta applied).
   - Keep the target component’s existing anchors unchanged.
   - Preserve the target’s attachment refs (current behavior already does this).

### Milestone 2: Add the subtree copy tool

Add a new Gen3D tool id:

- `copy_component_subtree_v1`

Expose it in `src/gen3d/agent/tools.rs` list + describe.

Implement it in `src/gen3d/ai/agent_tool_dispatch.rs` under the tool call match:

Inputs (args):

- `source_root`: component name or index
- `targets`: list of component names or indices (each is a target root)
- Optional `mode`: `detached` (default) or `linked`
- Optional `anchors`: `preserve_target` (default) or `copy_source`

Behavior:

1. Build a child adjacency list from `job.planned_components` using each child’s `attach_to.parent`.
2. For stable mapping, sort each parent’s children by a stable key derived from attachment info:
   - `(child.attach_to.parent_anchor, child.attach_to.child_anchor, child.name)`
3. Compute the source subtree traversal order using DFS from `source_root`.
4. For each target root, traverse the target subtree in parallel using the same child index order. If the target subtree shape is incompatible (missing child at a path), return a tool error with a short explanation.
5. For each matched (source, target) component pair, call the extended `copy_component_into` helper with the chosen anchor mode.
6. Re-resolve transforms (`resolve_planned_component_transforms`), update root def, write `assembly_snapshot`, and bump `assembly_rev` (same as `copy_component_v1` does).

### Milestone 3: Update prompts and docs

In `src/gen3d/ai/agent_prompt.rs` system instructions, add explicit guidance:

- “For symmetric limb chains (legs/arms), generate ONE chain, then call `copy_component_subtree_v1` to fill the others.”
- Mention `anchors=preserve_target` as the default safe setting.

In `README.md`, add a short note in the Gen3D tool list describing the new tool.

### Milestone 4: Tests

Add unit tests that:

1. Create a tiny planned component graph: `body -> leg_a -> foot_a` and `body -> leg_b -> foot_b`.
2. Generate geometry for `leg_a` and `foot_a`, leave `leg_b`/`foot_b` empty.
3. Run subtree copy from `leg_a` to `leg_b` and assert:
   - `leg_b` and `foot_b` now have primitive parts.
   - `leg_b` and `foot_b` still preserve their original anchors (when `preserve_target`).
   - `leg_b`’s attachment ref to `foot_b` remains (subtree copy must not delete/replace attachments).

## Concrete Steps

Run all commands from the repository root:

1. `cargo fmt`
2. `cargo test`
3. Smoke test: `cargo run -- --headless --headless-seconds 1 --config /Users/flow/.gravimera/config.toml`

## Validation and Acceptance

This change is accepted when:

1. `cargo test` passes.
2. The headless smoke run exits without crash.
3. The new tool appears in `list_tools_v1` and has a `describe_tool_v1` entry.
4. The unit test proves subtree copy fills symmetric limbs without changing target anchors/attachments.

## Idempotence and Recovery

All steps are safe to repeat. If the subtree tool produces incorrect mapping, tests should catch it. Roll back by reverting the commit(s) that introduce the tool and helper changes.

## Artifacts and Notes

Keep any future debug evidence in `gen3d_cache/<run_id>/` (agent_trace + render images) and reference that directory in tool feedback entries when reporting issues.
