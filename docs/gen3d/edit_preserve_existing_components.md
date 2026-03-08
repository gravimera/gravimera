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
- The engine merges plan metadata into the existing draft **without overwriting existing primitive/model geometry** for already-generated components.
- Existing anchor frames are preserved for existing anchor names; the plan may add new anchors (new names) deterministically.

If the plan violates the guardrails, `llm_generate_plan_v1` returns a tool error and the draft is left unchanged.

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
