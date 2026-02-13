# Content Formats and Versioning

Gravimera realms must be shareable and durable. This requires stable, versioned content formats.

This document intentionally focuses on *format responsibilities* rather than choosing one exact encoding (JSON/protobuf/etc). The key is that everything is:

- versioned,
- validated,
- migratable,
- partially loadable (especially scenes).

## Realm Manifest

The realm manifest defines:

- realm id, name, author(s), creation time
- required engine version range
- enabled modules + module configuration (see `docs/gamedesign/10_rulesets_and_modules.md`)
- default scenes and spawn points
- capability policy defaults (what kinds of tokens can exist)
- dependencies on external asset packs (optional)

The concrete realm package layout and manifest/ruleset fields are specified in:

- `docs/gamedesign/20_realm_package_manifest.md`

## Scene Data

Each scene stores:

- terrain layer configuration (base surface + objects)
- object instances (prefab id + transform + overrides)
- portal gates located in this scene
- scene metadata (biome, lighting, nav settings)
- optional local story variables and triggers

Scenes must be independently loadable. Hosts may stream scenes in/out of memory.

## Prefab Packs

Prefabs can come from:

- built-in engine library
- realm-local prefabs (including Gen3D generated)
- external packs (dependencies)

A prefab pack includes:

- prefab definitions (with stable ids)
- referenced assets (models, textures) or pointers to known asset registries
- optional brain templates and story tags

## Behavior Graphs

A behavior graph format must define:

- node types and their parameters (versioned)
- a blackboard schema (typed variables)
- deterministic evaluation semantics

Graph execution must be bounded (step budget) and must produce events on errors.

The concrete behavior graph spec is defined in:

- `docs/gamedesign/18_behavior_graph_spec.md`

## Story Assets

Story assets include:

- quests (state machine + triggers/actions)
- dialogue graphs
- cutscene-like sequences (optional) expressed as actions over time

Story assets reference:

- scene ids
- NPC ids (stable identities)
- prefab ids (for spawn actions)
- variable keys

## Versioning and Migration

Every serialized artifact must include:

- a format version number
- enough information to migrate forward

Migration rules:

- the engine must migrate old realm packages in-place or to a new copy
- failures must be non-destructive (do not corrupt the source)
- migrations must be testable with fixture files under `tests/`

The concrete migration policy is defined in:

- `docs/gamedesign/21_versioning_and_migrations.md`
