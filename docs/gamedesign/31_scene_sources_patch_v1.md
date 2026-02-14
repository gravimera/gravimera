# Scene Sources Patch (v1) — BlueprintPatch Subset

_(Spec document; see `docs/gamedesign/19_blueprint_spec.md` for the final target blueprint format.)_

This spec defines a minimal, **scene-scoped patch format** used by the engine tooling loop:

validate → apply → recompile → validate.

The patch format is intentionally generic and deterministic. It encodes no domain heuristics.

## Purpose

Scene sources are the authoritative, git-friendly representation of a scene (see
`docs/gamedesign/30_scene_sources_and_build_artifacts.md`). A patch expresses a small, auditable set
of mutations to those sources:

- upsert/remove pinned instances,
- upsert/remove procedural layer files,
- upsert/remove portal files.

Patches are intended to be retry-safe and idempotent under `request_id`.

## Patch Document (v1)

A patch is a JSON object:

- `format_version`: integer (`1`)
- `request_id`: string (stable id for this patch application attempt)
- `ops`: array of operations

### Operation Kinds (v1)

All operations are JSON objects with a required `kind` string.

#### `kind = "upsert_pinned_instance"`

Upserts a pinned instance file under `src/pinned_instances/`.

Fields:

- exactly one of:
  - `instance_id`: UUID string, or
  - `local_ref`: string (used to derive a deterministic UUID; see below)
- `prefab_id`: UUID string
- `transform`: object (same shape as pinned instance schema)
- `tint_rgba`: optional object (same shape as pinned instance schema)

#### `kind = "delete_pinned_instance"`

Deletes `src/pinned_instances/<instance_id>.json` if present.

Fields:

- `instance_id`: UUID string

#### `kind = "upsert_layer"`

Upserts `src/layers/<layer_id>.json`.

Fields:

- `layer_id`: string
- `doc`: JSON object (layer document). The engine/tooling may enforce:
  - `format_version = 1`
  - `layer_id` matches the operation’s `layer_id`

#### `kind = "delete_layer"`

Deletes `src/layers/<layer_id>.json` if present.

Fields:

- `layer_id`: string

#### `kind = "upsert_portal"`

Upserts `src/portals/<portal_id>.json`.

Fields:

- `portal_id`: string
- `destination_scene_id`: string
- `from_marker_id`: optional string

#### `kind = "delete_portal"`

Deletes `src/portals/<portal_id>.json` if present.

Fields:

- `portal_id`: string

## Deterministic Id Derivation (Pinned Instances via `local_ref`)

If `upsert_pinned_instance` provides `local_ref` (and no `instance_id`), the engine derives a stable
UUID via UUID v5 (namespace `uuid::Uuid::NAMESPACE_URL`) with name/key:

    gravimera/scene_sources_patch/v1/scene/<scene_id>/request/<request_id>/local/<local_ref>

This makes patch application idempotent: re-applying the same patch produces the same instance id.

## Patch Summary (What Changed)

Patch application should return a summary containing:

- `changed_paths`: relative `src/` paths that changed (added/updated/removed)
- `derived_instance_ids`: mapping from `local_ref` → derived UUID (if used)

This summary is intended for audit logs and auto-repair loops.

