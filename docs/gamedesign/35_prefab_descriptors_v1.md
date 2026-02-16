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

Prefab descriptor files live **next to** their prefab definition JSON:

- `~/.gravimera/realm/<realm_id>/prefabs/packs/<pack_id>/prefabs/<prefab_uuid>.json`
- `~/.gravimera/realm/<realm_id>/prefabs/packs/<pack_id>/prefabs/<prefab_uuid>.desc.json`

Notes:

- `<prefab_uuid>` is the prefab id (UUID). The filename should match the document’s `prefab_id`.
- `generated` is the reserved pack id for engine/AI-generated prefabs (Gen3D/Object agent).

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

### `provenance`

`provenance` records where the prefab came from and how it evolved.

Fields:

- `source`: optional string (open vocabulary; recommended: `"gen3d"`, `"import"`, `"handmade"`, `"builtin"`).
- `created_at_ms`: optional integer (unix epoch milliseconds).
- `gen3d`: optional object (present when `source = "gen3d"`):
  - `prompt`: optional string (the user’s intent; may be multi-line).
  - `style_prompt`: optional string
  - `run_id`: optional string (links to Gen3D run artifacts)
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

