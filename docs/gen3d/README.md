# Gen3D (AI model generation + deterministic editing)

Gen3D is Gravimera’s in-game AI modeling workflow: it generates a component plan, drafts
per-component primitive geometry, authors optional motion clips, and iterates with deterministic
validation/QA.

This doc focuses on the **workflow**, the **pipeline state machine**, and the **tool contracts**
Gen3D uses.

## Setup

1. Create a local config:

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

2. Edit `~/.gravimera/config.toml`:

- Set your AI provider in `[openai]` / `[mimo]` / `[gemini]` / `[claude]`
- (Optional) Configure Gen3D behavior under `[gen3d]`
- (Optional) Configure AI request timeouts under `[ai]`
  - `request_timeout_secs = 240` (default). For Gen3D this is a “first-byte timeout” (how long we wait for the provider to start sending the response body).

## Run artifacts

By default, each Gen3D run writes artifacts under:

- `<root_dir>/cache/gen3d/<run_id>/` (default `<root_dir>` is `~/.gravimera/`)
  - `attempt_<n>/steps/step_####/...` (per-tool-call artifacts in pipeline order)
  - `info_store_v1/` (KV + events + blobs metadata)

## Reference images

When the player provides reference photos, the engine:

- Caches the originals under `attempt_*/inputs/images/` (best-effort copy).
- Produces downsampled “component reference images” under `attempt_*/inputs/component_reference_images/`.

These downsampled images are the ones sent to Gen3D’s LLM-backed steps (when images are available),
including:

- `prompt_intent` (attack requirement classification)
- `llm_generate_plan_v1`
- `llm_generate_plan_ops_v1`
- `llm_generate_component_v1` / `llm_generate_components_v1`

DraftOps-specific artifacts:

- `attempt_*/steps/step_*/draft_ops_suggested_last.json` — latest `llm_generate_draft_ops_v1` suggestion
  payload.
- `attempt_*/steps/step_*/apply_draft_ops_last.json` — latest `apply_draft_ops_v1` result (`diff_summary`,
  `rejected_ops`, etc).

## Orchestration

Gen3D uses a deterministic pipeline state machine (pipeline-only).

High-level flow:

- Create sessions: plan → generate components → QA loop → (optional) render/review-delta → finish
- Seeded Edit/Fork sessions: edit strategy → (optional) preserve-mode plan ops → capture scoped part
  snapshots → DraftOps suggest+apply → QA loop → finish

For a step-by-step walkthrough (including the exact tool ids/args and where prompt text is persisted),
see:

- `docs/gen3d/pipeline_walkthrough.md`

Config note:

- Legacy `[gen3d].orchestrator = "pipeline"` is accepted but ignored.
- `[gen3d].orchestrator = "agent"` is rejected (agent-step orchestrator removed).

## Status panel

The left-side Gen3D Status panel shows a compact run summary plus a scrolling step log.

- Summary:
  - `Pipeline: current/total | label` shows the current user-facing pipeline step for the active route
    (create, preserve edit, optional review).
  - `Reuse: copied X | mirrored Y` counts successful reuse operations from actual tool results.
- Log:
  - Active component and motion batch lines include live task counts:
    `tasks: running X | queued Y | total Z`
  - Finished batch lines keep the total task count in the result summary.

## Preview inspection

The main Gen3D preview panel now has a lightweight component-inspection overlay for the full
component tree, not just the first root layer.

- Hover a visible component in the preview image to frame it and show a small info card.
- When parent and child frames overlap, picking prefers the more specific nested component instead
  of collapsing everything to the enclosing torso/body shell.
- Use the `Inspect` → `Explode` toggle in the preview stats box to separate visible components in
  the preview only, including nested object-ref roots.
- In explode mode, zoom stays centered on the exploded assembly instead of pulling back toward the
  original assembled draft center.
- Hovering uses a flatter segmented frame rather than a filled selection box so bright components
  stay readable.
- With the cursor over the preview, use `W` / `A` / `S` / `D` or the arrow keys to pan the preview
  view.
- In explode mode, separated components show their names directly in the preview. Hover still shows
  the fuller info card.

This is preview-only inspection UI. It does not modify the draft, saved prefab, or component
attachments.

## Component identity

Gen3D component definitions keep the component `name` as their object `label`. Internal identity
for reuse, snapshots, and deterministic edit tooling comes from the stable object id namespace
`gravimera/gen3d/component/<name>`, not from a label prefix.

## DraftOps-first primitive editing (seeded edits)

## Post-build edit flow

When a Build run finishes and auto-save succeeds, the session is promoted to an Edit session. The
Build button becomes Edit, and subsequent runs overwrite the same prefab id (the same behavior as
clicking Edit from a prefab preview). If auto-save is skipped or fails, the session remains a Build
session.

For edit requests like “make the wings larger” or “keep this component but make internal parts
animatable” where regeneration is not required, Gen3D prefers **in-place DraftOps edits**:

1. Capture editable part snapshots (per-component): `query_component_parts_v1`
   - The pipeline prefers a small scope (selected via `llm_select_edit_strategy_v1`).
2. Ask the model for DraftOps suggestions: `llm_generate_draft_ops_v1` (`scope_components=[...]`)
3. Apply atomically with revision gating:
   - Pipeline: `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev=<current>`
   - Manual/debugging: `apply_last_draft_ops_v1` (applies the latest `draft_ops_suggested_last.json`)
     or `apply_draft_ops_from_event_v1` (applies a specific suggestion `event_id`)

Key safety rules:

- Always apply DraftOps with `atomic=true` so invalid ops don’t partially accumulate.
- Always apply with `if_assembly_rev` so stale suggestions cannot apply to a changed assembly.

### DraftOps application helpers (manual/debugging)

The engine provides deterministic “apply-by-reference” tools:

- `apply_last_draft_ops_v1`: applies the latest `attempt_*/pass_*/draft_ops_suggested_last.json`.
- `apply_draft_ops_from_event_v1`: applies DraftOps from a specific Info Store `tool_call_result`
  `event_id` for `llm_generate_draft_ops_v1`.

Both tools apply via `apply_draft_ops_v1` with `atomic=true` and `if_assembly_rev` gating.

## Articulation nodes and motion targets

Gen3D plans and component drafts now cooperate on `articulation_nodes[]` inside a single
component.

At plan time:

- `components[].articulation_nodes[]` reserves generic internal motion handles (`node_id`,
  optional `parent_node_id`, `pos`, `forward`, `up`)
- plan JSON is structural only; it does not bind primitive parts and it does not author named
  motion clips

At component-draft time:

Each articulation node is generic internal rig metadata:

- `node_id`: stable motion handle id
- `parent_node_id`: optional parent articulation node
- `pos` + `forward` + `up`: node-local basis in component-local space
- `bind_part_indices`: explicit primitive-part bindings

This is a Gen3D authoring/edit abstraction, not a new runtime entity type. When motion authoring
targets an articulation node, Gen3D expands that node-space motion into ordinary per-part animation
slots before saving/runtime playback.

Motion authoring no longer uses attachment-only `edges[]`. It uses generic `targets[]`:

- `root_edge`
- `attachment_edge`
- `articulation_node`

Each authored slot also declares a motion `family`:

- `base`: normal gameplay-selected body motion (`attack`, `action`, `move`, `idle`, `ambient`)
- `overlay`: a forced named channel that composes on top of `base`

In preview/runtime today, `ForcedAnimationChannel` first tries the same named channel in `base`,
then composes any matching `overlay` slot on top. This lets named local expression channels like
`blink` or `jaw_open` override the default base motion when they were authored as `base`, while
still supporting additive overlays without replacing the body’s normal motion.

Gen3D now persists accepted motion-authoring JSON per channel inside the live job state, workspace
state, snapshots, and saved edit bundle metadata. When a later review-delta or seeded edit
regenerates one component, the engine rebuilds attachment object refs from `planned_components`
first, then replays the stored channels that target that component. This keeps attachment offsets
and internal articulation-node part animations aligned with the regenerated geometry instead of
dropping named expressions during component regen.

`query_component_parts_v1` now returns both `parts[]` and `articulation_nodes[]` so seeded edits can
reason about internal rig handles deterministically.

Seeded edits can now mutate articulation nodes directly through DraftOps:

- `upsert_articulation_node`: add a new node or update an existing node in place
- `remove_articulation_node`: remove a leaf node
- `rebind_articulation_node_parts`: replace the part bindings of an existing node

These ops edit only Gen3D rig metadata. They do not create geometry by themselves. If the desired
internal region does not already exist as primitive parts, the edit still needs primitive-part edits
or regeneration first. One primitive part may belong to only one articulation node at a time, and
`remove_primitive_part` is rejected while an articulation node still binds that part.

## Motion slots: per-slot basis + per-edge fallback basis

When a component is attached (`attach_to`), the child’s placement is controlled by:

- `attach_to.offset` (the authored base transform in the join frame)
- optional per-edge animation slots (`attach_to.animations[]`)
- `attach_to.fallback_basis` (a constant transform applied when no channel slot matches)

At runtime, the selected slot composes transforms as:

```
animated_offset(t) = attach_to.offset * slot.spec.basis * delta(t)
```

Where:

- `delta(t)` is sampled from the animation clip (`loop`/`once`/`ping_pong`/`spin`)
- `slot.spec.basis` is a constant per-slot transform applied between the base offset and the delta

When **no** channel slot matches, the edge falls back to:

```
animated_offset = attach_to.offset * attach_to.fallback_basis
```

For ordinary part playback, the runtime now evaluates two independent families in order:

1. `base`
2. `overlay`

So the final sampled transform is:

```
part_transform(t) = base_transform * selected_base_family_delta(t) * selected_overlay_family_delta(t)
```

When no overlay slot matches the forced channel, playback falls back to base-only behavior.

### Why basis exists (stable edits)

In preserve-mode edits, Gen3D can legally change `attach_to.offset` (when
`constraints.preserve_edit_policy = "allow_offsets"`). Without a separate basis, changing the base
offset changes the coordinate basis used to apply deltas, which makes already-authored channels
(idle/move/action/attack) look “wrong”.

To keep existing animations visually stable, when an attachment’s interface is unchanged and
`attach_to.offset` changes from `old_offset` → `new_offset`, the engine rebases preserved slot bases:

```
basis_new = inverse(new_offset) * old_offset * basis_old
```

This keeps `new_offset * basis_new` equal to `old_offset * basis_old` (so the animation looks the
same), without rewriting keyframes.

The same rebasing rule is applied to both:

- every slot’s `slot.spec.basis`
- the edge’s `attach_to.fallback_basis` (used only when no slot matches)

## Debugging

Useful tools:

- KV: `info_kv_list_keys_v1`, `info_kv_get_v1`
- Events: `info_events_list_v1`, `info_events_get_v1`, `info_events_search_v1`
- Draft diffs: inspect `apply_draft_ops_last.json` and `draft_ops_suggested_last.json` under the
  pass folders.

### Structured-output robustness

Some “human notes” fields in LLM JSON are normalized before parsing:

- `review_delta_v1.summary`, `review_delta_v1.notes_text`
- `gen3d_motion_authoring_v1.notes_text`

They may be returned as `string | string[] | null`; arrays are joined with `\n`, and empty/whitespace-only
strings become `null`.

When an LLM call is retried within a component/motion batch, the previous parse/apply error is appended
to the next prompt under `Previous attempt error:` so the model can self-correct.

See also: `docs/execplans/gen3d_deterministic_pipeline.md`.
