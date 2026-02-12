# Blueprint Spec (Bulk World Creation)

This document defines the **final target** “blueprint” format used by AI agents (and advanced human tools) to create and modify large parts of a realm efficiently.

Blueprints are essential for AI freedom creation because a living world often involves thousands of objects, multiple scenes, portal graphs, NPC populations, and story assets. These changes must be expressible as:

- a small number of API calls,
- validated and budgeted before application,
- auditable and reproducible.

This spec is referenced by:

- `docs/gamedesign/16_agent_api_contract.md` (blueprint validate/apply endpoints)
- `docs/gamedesign/12_content_formats.md` (realm durability/versioning)

## What a Blueprint Is (Conceptually)

A blueprint is a versioned document that describes a **set of mutations** to a realm:

- create or update scenes,
- place objects (terrain pieces, buildings, NPCs),
- connect scenes with portals,
- attach brains and schedules to NPCs,
- create or update story assets (quests/dialogue) and initial variables.

Blueprint application is the primary “bulk authoring” mechanism for agents.

## Blueprint Application Modes

### Validate (Dry Run)

Validation checks:

- schema correctness and version compatibility,
- referential integrity (ids exist or are created),
- budget estimates (objects/brains/portals/story size),
- safety policy (capability checks, disallowed actions),
- cycle detection (prefab references, trigger loops where detectable).

Validation does **not** mutate the realm.

### Apply (Mutation)

Apply performs the mutations and returns:

- created ids (scenes, prefabs, instances, portals, brains, story assets),
- a stable mapping from blueprint-local references to server ids,
- a summary of what changed and budget consumption,
- an audit event stream that links changes to `request_id`.

## Atomicity and Recovery

Preferred: blueprint apply is atomic at the realm level.

If full atomicity is not feasible (very large blueprints), the server must provide:

- a server-generated rollback token, or
- a transaction log that can be replayed/undone deterministically.

In all cases, partial failures must be explicit and must not leave silent corruption.

## Identity and References

Blueprints must support two kinds of identifiers:

1) **Stable ids** (preferred for long-lived world content):
   - explicit `scene_id`, `prefab_id_uuid`, `instance_id_uuid`, `identity_id`.
   - used when an agent wants durable references across iterations.

2) **Local references** (ergonomic for “generate a new thing”):
   - blueprint-local ids like `@scene.town`, `@npc.bob`, `@portal.to_dungeon`.
   - the server generates stable ids and returns a mapping.

Local references make it easy for agents to author a new world without inventing UUIDs.

## Versioning

Every blueprint includes:

- `blueprint_version`: a semver-like version of this spec (or a monotonic integer),
- `engine_version_range`: compatible Gravimera versions (host choice),
- optional `requires_modules`: list of modules that must be enabled (combat/economy/etc).

If version is unsupported, the server must return a structured validation error.

## Budget and Policy Fields

Blueprints may include optional constraints to help safety and determinism:

- `max_new_instances`
- `max_new_prefabs`
- `max_new_brains`
- `max_new_portals`

If the server cannot satisfy these constraints, it must fail validation rather than silently applying a smaller subset.

## Core Sections of a Blueprint (Recommended Shape)

This spec does not mandate exact JSON field names, but it mandates responsibilities. A practical blueprint structure includes:

1) `scenes`: create/update scene metadata, markers, and budgets.
2) `prefabs`: register or update prefabs (including Gen3D job outputs).
3) `instances`: spawn/edit/destroy instances in scenes, including terrain objects.
4) `portals`: create/edit portal gates connecting scenes (and optionally realms).
5) `brains`: attach/configure brains and schedules.
6) `story`: quests/dialogue assets + variable initialization.

## Determinism Requirements

Blueprint application must be deterministic given the same:

- starting realm state,
- blueprint document,
- server version and ruleset.

If a blueprint requests “server-generated ids”, the id generation must still be deterministic:

- derive ids from `(realm_id, request_id, blueprint_hash, local_ref)`.

This allows reproducible world generation in deterministic stepping.

## Templates (Parameterizable Blueprints)

To make creation scalable, realms can include reusable templates:

- a template is a stored blueprint with parameters.

Examples:

- “village layout” template with parameters: population size, style, biome.
- “quest chain” template with parameters: protagonist, number of steps, reward theme.

Template application uses:

- a parameter dictionary,
- deterministic expansion rules,
- full validation and budgeting like normal blueprints.

Hosts can restrict which templates are allowed in public realms.

## Example (Illustrative Only)

An example blueprint conceptually includes:

- create scene “hub”
- create scene “dungeon”
- place terrain blocks and buildings in hub
- spawn 12 citizens (NPC identities) with schedules and dialogue
- place a portal from hub to dungeon, locked until `quest.intro_completed = true`
- create an intro quest and set initial story vars

The key property is that all references are resolvable and auditable, and that validation can estimate the resulting world size before apply.

