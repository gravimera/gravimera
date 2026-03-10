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

## Tool Note: Querying Parts by Index

`query_component_parts_v1` accepts either:

- `component`: the component name, or
- `component_index`: a **0-based** index into the current planned components list.
