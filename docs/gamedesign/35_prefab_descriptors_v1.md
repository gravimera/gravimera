# Prefab Descriptors (v1)

This spec defines a **text-based prefab description format** used by:

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

In the current storage model, prefab defs + descriptors live inside **realm prefab packages** (see `docs/gamedesign/39_realm_prefab_packages_v1.md`):

- `~/.gravimera/realm/<realm_id>/prefabs/<root_prefab_uuid>/prefabs/<prefab_uuid>.json`
- `~/.gravimera/realm/<realm_id>/prefabs/<root_prefab_uuid>/prefabs/<prefab_uuid>.desc.json`

Notes:

- `<prefab_uuid>` is the prefab id (UUID). The filename should match the document’s `prefab_id`.
- A prefab package may contain multiple prefab defs + descriptors (root prefab + internal components).

## Descriptor Document: `PrefabDescriptorFileV1`

Descriptors are stored as JSON objects.

Top-level fields:

- `format_version`: integer. **Must be `1`** for this spec.
- `prefab_id`: UUID string. Prefab id this descriptor describes.
- `label`: optional string. Human-friendly name (may differ from prefab-def label). For Gen3D-saved prefabs, tools should keep this as a short name suitable for UI library listing (recommended: ≤3 words).
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
- `extra`: optional object. Best-effort structured extensions (string → JSON).

#### `interfaces.extra.motion_summary` (derived animation summary)

`interfaces.extra.motion_summary` is an optional, derived object written by the engine for
Gen3D-saved prefabs. It summarizes which animation channels exist across the prefab graph.

Notes:

- This data is best-effort and does not affect runtime behavior.
- Tools should ignore unknown fields and tolerate missing/null values.

Top-level fields (v1):

- `version`: integer. Must be `1`.
- `channels`: array of channel entries:
  - `channel`: string (e.g. `"idle"`, `"move"`, `"attack"`).
  - `slots`: integer (total slot count for this channel).
  - `animated_parts`: integer (parts that contain at least one slot for this channel).
  - `drivers`: array of strings (e.g. `"always"`, `"move_phase"`, `"move_distance"`, `"attack_time"`).
  - `clip_kinds`: array of strings (e.g. `"loop"`, `"once"`, `"ping_pong"`, `"spin"`).
  - `loop_duration_secs_min`: optional number or null.
  - `loop_duration_secs_max`: optional number or null.
  - `speed_scale_min`: optional number or null.
  - `speed_scale_max`: optional number or null.
  - `has_time_offsets`: bool.

### `provenance`

`provenance` records where the prefab came from and how it evolved.

Fields:

- `source`: optional string (open vocabulary; recommended: `"gen3d"`, `"import"`, `"handmade"`, `"builtin"`).
- `created_at_ms`: optional integer (unix epoch milliseconds).
- `created_duration_ms`: optional integer (milliseconds). Best-effort duration of the *initial* Gen3D creation run (the first saved `generated` revision).
- `modified_at_ms`: optional integer (unix epoch milliseconds). Last-modified timestamp (for sorting and recency).
- `total_tokens`: optional integer. Best-effort total tokens consumed by Gen3D across all saved revisions for this prefab (creation + modifications). When present, it should match the sum of per-revision `tokens_total`.
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
    - `descriptor_meta_policy`: string (`"suggest"` or `"preserve"`). Indicates how descriptor-meta was applied when saving.
    - `descriptor_meta_v1`: object. Raw Gen3D descriptor-meta output used to populate `label` + `text.short` + `tags`:
      - `version`: integer (schema version)
      - `name`: string (≤3 words recommended)
      - `short`: string (1–2 lines)
      - `tags`: array of strings
- `revisions`: optional array of revision entries:
  - `rev`: integer (monotonic)
  - `created_at_ms`: integer
  - `actor`: string (e.g. `"human"`, `"agent:object"`)
  - `summary`: string
  - Additional fields may be present. Recommended fields when available:
    - `prompt`: string. The Gen3D prompt used at the time this revision was saved.
    - `tokens_total`: integer. Best-effort token count consumed by Gen3D to produce this revision.
    - `duration_ms`: integer. Best-effort duration (milliseconds) of the Gen3D run that produced this revision.
    - `descriptor_meta_policy`: string (`"suggest"` or `"preserve"`). Indicates how descriptor-meta was applied.
    - `descriptor_meta_v1`: object. Raw Gen3D descriptor-meta output (see `gen3d.descriptor_meta_v1` above).

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
- Populate `extra.facts` with a structured set of derived facts (size, mobility/attack presence, grounding, etc.).
- Populate `label` (short name), `text.short`, and `tags` via a best-effort AI call (when AI config is available). To reduce end-of-run latency, Gen3D should start this request as soon as a plan is accepted (in parallel with geometry generation), cache it for the current `plan_hash`, and ensure it is available **before reporting the run complete** so Save can write a descriptor that already includes these fields. Falling back to blank/default values is acceptable. Tools should treat these as suggestions and preserve human edits.
- For seeded Edit/Fork sessions, Save should preserve the previous `label` + `text.short` + `tags` by default. If the user explicitly requests changes, the Gen3D agent may override them (via `set_descriptor_meta_v1` using args `name`→`label`, `short`→`text.short`, `tags`→`tags`).
