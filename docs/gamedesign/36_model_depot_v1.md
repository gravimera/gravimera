# Local Model Depot (v1) — Removed

Status: **removed as of 2026-03-08**.

Gravimera no longer uses a global, realm-independent model depot under `~/.gravimera/depot/models/`.

Gen3D-saved prefabs are stored **scene-locally** under the active scene directory (see `docs/gamedesign/39_scene_local_prefab_packages_v1.md`).

This document is retained for **historical reference only**. New code must not read or write the depot.

## Historical Spec (v1)

This spec defines the **local model depot** format: how Gen3D-generated (or otherwise local) 3D models are stored on disk so they can be reused across realms.

The model depot is **realm-independent** and is intended as a creator’s personal library. Realm packages should use realm prefab packs for portable, shareable prefab definitions.

## Goals

- **Realm-independent library:** models generated in one realm can be reused in another without copying files manually.
- **Simple browsing:** list models by directory name (UUID).
- **Text-friendly:** prefab definitions and descriptors are JSON, stable, and diffable.

## Directory Layout

Depot root:

- `~/.gravimera/depot/models/`

Each depot model is stored in a folder named by its model UUID:

- `~/.gravimera/depot/models/<model_uuid>/`
  - `prefabs/`
    - `<prefab_uuid>.json` (prefab definition; `PrefabFileV1`, see `docs/gamedesign/34_realm_prefabs_v1.md`)
    - `<prefab_uuid>.desc.json` (optional descriptor; `PrefabDescriptorFileV1`, see `docs/gamedesign/35_prefab_descriptors_v1.md`)

Notes:

- `<model_uuid>` is the model’s identifier and is currently the **root prefab id** (the root prefab’s UUID).
- `prefabs/` may contain multiple prefab JSON files: the root prefab plus any internal component prefabs referenced by `object_ref`.
- Writers should pretty-print JSON with stable key ordering (see the “Canonical JSON” notes in the prefab specs).

## Enumeration Rules

To list available depot models:

1. List direct children of `~/.gravimera/depot/models/`.
2. Keep only directory entries whose names parse as UUIDs.
3. Treat each UUID as a model id.

Non-UUID directories are ignored.

## Engine Load Behavior (Informative)

On scene load, the engine may:

- load all depot prefab defs first, then realm prefab defs, so realm packs can override/extend as needed, and
- load any `.desc.json` descriptor files found alongside depot prefabs.
