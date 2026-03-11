# Gen3D Edit Sessions: Preserve Existing Components

This document describes the deterministic “preserve existing components” behavior used when editing/continuing a Gen3D draft seeded from an existing prefab.

## Preserve Mode (Default for Edit/Fork Seeds)

When a Gen3D session is seeded from an existing prefab (Edit/Fork), the engine enables preserve mode by default so small edits (e.g. “add a hat”) do not regenerate the entire object:

- `get_state_summary_v1` includes `preserve_existing_components_mode: true`
- `llm_generate_plan_v1` defaults to preserve behavior unless you explicitly pass `constraints.preserve_existing_components=false`

## Preserve Mode (Plan Tool)

Preserve mode is requested by calling:

- `llm_generate_plan_v1` with `constraints.preserve_existing_components=true`

When preserve mode is enabled **and** the current draft already contains generated geometry, the engine applies additional guardrails and merges instead of overwriting:

- The new plan **must** include **all existing component names** (no renames/deletes).
- The new plan **must** keep the same **root component** name.
- The plan is validated against a **preserve edit policy** (selected via `constraints.preserve_edit_policy`):
  - `additive` (default): existing components’ attachments are frozen; only additive changes are allowed (add components / add anchors).
  - `allow_offsets`: keep the same attachment interfaces, but allow changing `attach_to.offset` for existing components.
  - `allow_rewire`: allow rewiring only an explicit allow-list (`constraints.rewire_components`); all other existing components remain frozen.
- The engine merges plan metadata into the existing draft **without overwriting existing primitive/model geometry** for already-generated components.
- Existing anchor frames are preserved for existing anchor names; the plan may add new anchors (new names) deterministically.

If the plan violates the guardrails (including the selected edit policy), `llm_generate_plan_v1` returns a tool error and the draft is left unchanged.

## Preserve Mode (Troubleshooting plan failures)

When preserve-mode replanning fails, prefer deterministic “hints/tools” that avoid repeated scene dumps:

- If `llm_generate_plan_v1` fails with a **semantic** error (unknown parent/root, missing required existing names, policy rejection), call:
  - `inspect_plan_v1` (read-only) to get computed constraints (allowed names/root/policy) and structured error kinds.
- If preserve-mode replanning keeps failing, call:
  - `get_plan_template_v1` (read-only) to write a plan JSON template artifact, then
  - re-run `llm_generate_plan_v1` with `plan_template_artifact_ref` set to that artifact ref.

Note: if the model output is invalid JSON or fails the plan JSON schema, the engine may attempt an automatic “schema repair” retry of `llm_generate_plan_v1`. In preserve mode, this retry includes the same preserve-mode constraints/context (and the template, if provided) so it does not accidentally “forget” existing component names.

Related docs:

- `docs/gen3d/inspect_plan_v1.md`
- `docs/gen3d/get_plan_template_v1.md`

Examples (tool args):

- Additive (default; safest for “add a hat”):

  - `{"constraints":{"preserve_existing_components":true,"preserve_edit_policy":"additive"}}`

- Allow offset changes without rewiring:

  - `{"constraints":{"preserve_existing_components":true,"preserve_edit_policy":"allow_offsets"}}`

- Allow rewiring a specific subset (explicit allow-list required):

  - `{"constraints":{"preserve_existing_components":true,"preserve_edit_policy":"allow_rewire","rewire_components":["neck","head"]}}`

Notes:

- If you want to reposition parts without replanning, prefer `apply_draft_ops_v1`:
  - `SetAttachmentOffset` (move an existing component along its attachment edge),
  - `SetAttachmentJoint` (edit degrees-of-freedom metadata for an attachment edge; useful for hinge axis fixes),
  - `SetAnchorTransform` (adjust a join frame), and
  - primitive part ops (add/remove/update geometry).
- If you want to change animation clips, prefer `llm_generate_motion_authoring_v1` (or `apply_draft_ops_v1` animation slot ops).

## Preserve Mode (Generation Tools)

After a preserve-mode plan is applied, `get_state_summary_v1` includes:

- `preserve_existing_components_mode: true`

When this flag is true:

- `llm_generate_components_v1` behaves as **missing-only** unless `force=true` is provided (prevents accidental regeneration of already-generated components).
- `llm_generate_component_v1` will skip already-generated components unless `force=true`.
- Regen budgets still apply when `force=true` is used.
- `force=true` regeneration is additionally **QA-gated**: the engine refuses force-regeneration unless the latest `qa_v1` reports errors (`validate.ok=false` or `smoke.ok=false`). If QA is clean (or has not been run), fix placement/assembly via `llm_review_delta_v1` / `apply_draft_ops_v1` instead of regenerating geometry.

### QA-gated regen requests (agent visibility)

In preserve mode, higher-level tools (most commonly `llm_review_delta_v1`) may request regeneration of already-generated components (for example, to restyle geometry).

When the QA gate is closed (latest QA is clean or unknown), these regen requests are **not actionable** as `force=true` regeneration. The engine surfaces them explicitly in `get_state_summary_v1`:

- `pending_regen_component_indices`: actionable generation work (missing components, or regen that is currently permitted).
- `pending_regen_component_indices_blocked_due_to_qa_gate`: regen requested for already-generated components, but blocked by the QA gate.

Deterministic recovery options:

1) Prefer deterministic edits (`apply_draft_ops_v1`) and/or review-delta tweak actions (`llm_review_delta_v1`) for placement/attachment fixes.
2) If you truly intend a style/geometry rebuild in a seeded edit session, disable preserve mode by calling `llm_generate_plan_v1` with `constraints.preserve_existing_components=false`, then regenerate the affected components **without** `force`.

## Tool Note: Querying Parts by Index

`query_component_parts_v1` accepts either:

- `component`: the component name, or
- `component_index`: a **0-based** index into the current planned components list.
