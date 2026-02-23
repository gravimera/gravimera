# Prefab Descriptors (v1)

This spec defines a **realm-shared, text-based prefab description format** used by:

- AI agents (search/selection/planning),
- humans (review/edit in git),
- and future Gen3D “edit existing prefab” workflows.

Prefab descriptors are **semantic metadata**. They complement (not replace) the structural prefab
definition in `docs/gamedesign/34_realm_prefabs_v1.md`.

## Goals

- **Searchable:** agents can select good prefabs without guessing from geometry alone.
- **Editable:** JSON text, stable diffs, mergeable.
- **Versioned:** forward-compatible and migratable.
- **Generic:** no game-specific heuristics; supports “any object”.

## Non-Goals

- Not a rendering/mesh format.
- Not a source of truth for physics/geometry (those live in prefab defs).
- Not a complete “asset pack registry” (pack manifests are separate).

## Directory Layout

Prefab descriptor files live **next to** their prefab definition JSON.

They may be stored either:

- in a realm prefab pack (portable/sharable), or
- in the local model depot (realm-independent library).

- `~/.gravimera/realm/<realm_id>/prefabs/packs/<pack_id>/prefabs/<prefab_uuid>.json`
- `~/.gravimera/realm/<realm_id>/prefabs/packs/<pack_id>/prefabs/<prefab_uuid>.desc.json`
- `~/.gravimera/depot/models/<model_uuid>/prefabs/<prefab_uuid>.json`
- `~/.gravimera/depot/models/<model_uuid>/prefabs/<prefab_uuid>.desc.json`

Notes:

- `<prefab_uuid>` is the prefab id (UUID). The filename should match the document’s `prefab_id`.
- `generated` is the reserved pack id for engine/AI-generated prefabs stored inside a realm pack.

## Descriptor Document: `PrefabDescriptorFileV1`

Descriptors are stored as JSON objects.

Top-level fields:

- `format_version`: integer. **Must be `1`** for this spec.
- `prefab_id`: UUID string. Prefab id this descriptor describes.
- `label`: optional string. Human-friendly name (may differ from prefab-def label).
- `text`: optional text descriptions (short/long).
- `tags`: optional array of strings (open vocabulary).
- `roles`: optional array of strings (open vocabulary; recommended values below).
- `interfaces`: optional interface/contract notes (anchors/channels).
- `provenance`: optional provenance and edit history.

Unknown fields are allowed. Tools should preserve unknown fields when rewriting/canonicalizing.

### `text`

`text` is an object:

- `short`: optional string (1–2 lines; agent-friendly).
- `long`: optional string (multi-paragraph; human-friendly).

### `tags`

`tags` is an array of strings:

- tags must be non-empty after trimming.
- recommended: `lower_snake_case` and stable meaning (“wood”, “ancient_town”, “portal_gate”).
- tags are not enforced by the engine; they exist for search and selection.

### `roles`

`roles` is an array of strings.

Roles are open vocabulary, but recommended values include:

- `unit`
- `building`
- `prop`
- `terrain`
- `projectile`
- `effect`

### `interfaces`

`interfaces` is an object describing the “intentional surface area” of a prefab.

Fields:

- `anchors`: optional array of anchor descriptors:
  - `name`: string (must match an anchor in the prefab definition, or `"origin"`).
  - `meaning`: optional string (open vocabulary; e.g. `"muzzle"`, `"door"`, `"seat"`, `"mount"`).
  - `notes`: optional string
  - `required`: optional bool (default false)
- `animation_channels`: optional array of strings (e.g. `"idle"`, `"move"`, `"attack"`).
- `notes`: optional string (general contract notes).

#### `interfaces.extra.motion_rig_v1` (runtime motion rig contract)

`interfaces.extra` may include an optional `motion_rig_v1` object that declares an explicit,
non-heuristic **motion rig contract** for applying **engine-provided, generic motion algorithms**
at runtime.

Key properties:

- The engine **does not** infer “this model has two legs” from names/shape/geometry.
- Motion algorithms are applied only when the prefab declares a compatible `motion_rig_v1`.
- The contract targets **attachment edges** (parent component → child component) via
  `(parent_object_id, child_object_id, parent_anchor, child_anchor)`.

Top-level fields:

- `version`: integer. Must be `1`.
- `kind`: string. One of:
  - `"biped_v1"`
  - `"quadruped_v1"`
  - `"car_v1"`
- `default_move_algorithm`: optional string. If present, must be one of:
  - `"none"` (use prefab-authored clips)
  - `"biped_walk_v1"`
  - `"quadruped_walk_v1"`
  - `"car_wheels_v1"`
- `move_cycle_m`: optional number (defaults to `1.0`). Used by walk rigs as the cycle length in
  **meters traveled** (driven by `MovePhase`).
- `walk_swing_degrees`: optional number (defaults vary by rig kind). Used by walk rigs as the
  swing amplitude (degrees) around the join’s local +X axis.

Edge reference object (`MotionEdgeRefV1`):

- `parent_object_id`: UUID string (the parent component prefab id).
- `child_object_id`: UUID string (the child component prefab id).
- `parent_anchor`: string (anchor name on the parent; `"origin"` allowed).
- `child_anchor`: string (anchor name on the child; `"origin"` allowed).

##### `kind = "biped_v1"`

Requires a `biped` object:

- `left_leg`: `MotionEdgeRefV1`
- `right_leg`: `MotionEdgeRefV1`

##### `kind = "quadruped_v1"`

Requires a `quadruped` object:

- `front_left_leg`: `MotionEdgeRefV1`
- `front_right_leg`: `MotionEdgeRefV1`
- `back_left_leg`: `MotionEdgeRefV1`
- `back_right_leg`: `MotionEdgeRefV1`

##### `kind = "car_v1"`

Requires a `car` object:

- `wheels`: array of wheel objects:
  - `edge`: `MotionEdgeRefV1`
  - `spin_axis_local`: optional `[x, y, z]` array (defaults to `[1, 0, 0]`)
- `wheel_radius_m`: optional number. If present, the engine uses `radians_per_meter = 1 / wheel_radius_m`.
- `radians_per_meter`: optional number. If present, overrides wheel spin rate directly.

Notes:

- If neither `wheel_radius_m` nor `radians_per_meter` is present, the engine derives wheel radius
  from the wheel component’s AABB size (including mount scale) and uses the rolling relation
  `angle = distance / radius`.

### `provenance`

`provenance` records where the prefab came from and how it evolved.

Fields:

- `source`: optional string (open vocabulary; recommended: `"gen3d"`, `"import"`, `"handmade"`, `"builtin"`).
- `created_at_ms`: optional integer (unix epoch milliseconds).
- `gen3d`: optional object (present when `source = "gen3d"`):
  - `prompt`: optional string (the user’s intent; may be multi-line).
  - `style_prompt`: optional string
  - `run_id`: optional string (links to Gen3D run artifacts)
  - `extra`: arbitrary JSON (best-effort, tool-specific). Recommended keys when present:
    - `attempt`: integer (Gen3D attempt index)
    - `pass`: integer (Gen3D pass index)
    - `plan_hash`: string (hash of the plan context)
    - `assembly_rev`: integer (monotonic assembly revision)
    - `plan_extracted`: object (a compact plan extract written by Gen3D; similar to the `plan_extracted.json` artifact)
- `revisions`: optional array of revision entries:
  - `rev`: integer (monotonic)
  - `created_at_ms`: integer
  - `actor`: string (e.g. `"human"`, `"agent:object"`)
  - `summary`: string

## Validation Rules (Non-Exhaustive)

- `format_version` must equal `1`.
- `prefab_id` must be a valid UUID string.
- Any string list (`tags`, `roles`, `animation_channels`) must contain only non-empty trimmed strings.
- `interfaces.anchors[].name` must be non-empty.

The engine may load descriptors best-effort and ignore invalid entries.

## Canonical JSON

For stable diffs, writers should:

- sort JSON object keys recursively,
- pretty-print,
- end files with a trailing newline,
- sort+dedup `tags`, `roles`, and `animation_channels` lexicographically.

## Relationship to Prefab Selection

Agents should select prefabs using a combination of:

- descriptor semantics (`label`, `text`, `tags`, `roles`, `interfaces`), and
- derived facts from the prefab definition (size, mobility/attack presence, anchor names).

Descriptors are optional; when missing, agents fall back to prefab-def `label` and derived facts.

## Gen3D Descriptor Enrichment (Best-Effort)

Gen3D writes descriptor files for saved models. In addition to filling standard fields (label/roles/anchors/animation_channels/provenance), Gen3D may:

- Populate `text.long` with a compact summary including derived facts, an AI plan extract (when available), and a derived motion summary.
- Populate `interfaces.extra.motion_summary` with a structured summary of available animation channels (drivers/clip kinds/counts).
- Populate `interfaces.extra.motion_rig_v1` when the model declares explicit rig edges for runtime motion algorithms (walk/wheels, etc.).
- Populate `extra.facts` with a structured set of derived facts (size, mobility/attack presence, grounding, etc.).
- Populate `text.short` and `tags` via a best-effort AI call (when OpenAI config is available). Tools should treat these as suggestions and preserve human edits.
