# Realm Package Manifest (Rulesets, Modules, Budgets, Policy)

_(Spec document; see `docs/gamedesign/08_persistence_packages.md` for the product goals.)_

This document defines the **final target** structure and manifest format for a Gravimera realm package.

It answers:

- What files exist in a realm package?
- How are scenes, prefabs, story assets, and behavior graphs referenced?
- Where do rulesets (enabled modules), budgets, and capability policies live?

This spec is designed for “living worlds”: realms that keep evolving over time via NPC brains and resident agents.

## Non-Negotiable Properties

1) **Realm packages are portable**: a realm can be exported/imported as a directory or archive.
2) **Everything is versioned**: every top-level artifact has a `format_version`.
3) **Safe by default**: realm packages contain *policies* and *defaults*, not secrets (no tokens embedded by default).
4) **Partial loading is possible**: hosts can load scenes on demand.
5) **Agent-friendly**: manifests are machine-readable and schema-validatable.

## Package Layout (On Disk)

Realm packages are directories. Hosts may also distribute them as single-file archives (e.g. `.gravrealm`), which unpack to this layout.

    <realm_root>/
      realm.json
      ruleset.json
      scenes/
        index.json
        <scene_id>/
          scene.json
          instances.dat            (optional binary optimized form)
          nav.dat                  (optional cached nav)
      prefabs/
        index.json
        packs/
          <pack_id>/
            pack.json
            prefabs/
              <prefab_id>.json
            assets/                (models/textures; optional)
      story/
        index.json
        quests/
          <quest_id>.json
        dialogue/
          <dialogue_id>.json
      brains/
        index.json
        graphs/
          <graph_id>.json
      templates/
        blueprints/
          <template_id>.json
      metadata/
        thumbnails/               (optional)
        changelog.md              (optional)

Notes:

- `*.dat` files are optional performance caches. The authoritative content is the `*.json` sources unless the ruleset chooses otherwise.
- Hosts are allowed to add host-local files (logs, analytics), but those must not be required to load the realm package.

## `realm.json` (Realm Manifest)

`realm.json` is the entry point. It contains identity, compatibility, pointers to content, and high-level defaults.

### Required Fields

- `format_version` (u32): manifest version for `realm.json`.
- `realm_id` (string): stable id. Recommended: UUID string.
- `name` (string): display name.
- `description` (string): short description for browsers.
- `engine_version_range` (object): which engine versions can load this realm.
  - `min_inclusive` (string)
  - `max_exclusive` (string) (optional)
- `entry` (object): where a new player/agent starts.
  - `default_scene_id` (string)
  - `default_spawn` (object): either a marker id or a transform.
- `content` (object): paths to content roots/indexes.
  - `ruleset_path` (string) default `"ruleset.json"`
  - `scenes_index_path` (string) default `"scenes/index.json"`
  - `prefabs_index_path` (string) default `"prefabs/index.json"`
  - `story_index_path` (string) default `"story/index.json"`
  - `brains_index_path` (string) default `"brains/index.json"`
  - `templates_root` (string) default `"templates/"`

### Optional Fields

- `authors` (array): list of author identities (human or agent), for attribution.
- `tags` (array): discovery tags (e.g. `"social"`, `"narrative"`, `"city"`).
- `thumbnail_path` (string): relative path to a thumbnail.
- `defaults` (object): realm-wide defaults (lighting preset, UI hints).

### Example `realm.json` (Illustrative)

    {
      "format_version": 1,
      "realm_id": "b7d2b3f0-9e9c-4b1b-8b6d-0a81b32ce9a0",
      "name": "Harbor City",
      "description": "A living port town with shifting factions and seasonal festivals.",
      "engine_version_range": {
        "min_inclusive": "0.3.0",
        "max_exclusive": "0.4.0"
      },
      "entry": {
        "default_scene_id": "hub",
        "default_spawn": { "kind": "marker", "marker_id": "hub_spawn" }
      },
      "content": {
        "ruleset_path": "ruleset.json",
        "scenes_index_path": "scenes/index.json",
        "prefabs_index_path": "prefabs/index.json",
        "story_index_path": "story/index.json",
        "brains_index_path": "brains/index.json",
        "templates_root": "templates/"
      },
      "tags": ["social", "narrative", "city"]
    }

## `ruleset.json` (Ruleset + Modules + Governance)

The ruleset defines:

- which optional modules are enabled,
- module parameters,
- capability policy defaults (roles),
- budgets and rate limits,
- whether deterministic stepping is permitted.

The ruleset is enforced by the host/server. Hosts may override some ruleset fields (e.g. disable `admin.time` entirely).

### Required Fields

- `format_version` (u32)
- `enabled_modules` (array of strings)
  - baseline realms must always include: `"core"`, `"story"`, `"brains"`, `"scenes"`.
- `module_config` (object) mapping module name -> config object (may be empty for modules with defaults)
- `governance` (object) containing:
  - `capability_roles` (object): role name -> capability list
  - `budgets` (object): realm and per-scene budgets
  - `rate_limits` (object): per-token limits (hosting)
  - `audit` (object): whether audit events are required (hosting should require them)
- `time` (object): time policy
  - `mode` enum: `"realtime"` or `"deterministic_allowed"`
  - `allow_admin_step` (bool): whether `admin.time` is meaningful in this realm

### Role-Based Capability Policy (Defaults)

Realms define default roles to make creation predictable. Recommended roles:

- `creator` (full authoring)
- `resident_agent` (authoring limited to story/world operations; no prefab upload by default)
- `player` (observe + act for owned objects; no authoring)
- `moderator` (safety admin; limited authoring + ban/quarantine abilities)

Important rule:

- **Roles are defaults**, not credentials. The host decides which tokens map to which roles, and may clamp/override.

### Budgets (Examples)

Budgets are enforced caps to prevent runaway creation:

- `max_instances_per_realm`
- `max_instances_per_scene`
- `max_prefabs_per_realm`
- `max_active_brains_per_realm`
- `max_portals_per_realm`
- `max_events_per_second` (to keep event streams manageable)

Budgets should include separate caps for:

- **authoring** (how many new things can be created per minute),
- **steady-state** (how many things can exist and run continuously).

### Rate Limits (Examples)

Rate limits are per token/principal:

- `max_requests_per_second`
- `max_author_ops_per_minute`
- `max_blueprint_apply_per_hour`

Rate limits must be visible via `GET /v2/limits` (see `docs/spec/16_agent_api_contract.md`).

### Example `ruleset.json` (Illustrative)

    {
      "format_version": 1,
      "enabled_modules": ["core", "scenes", "story", "brains", "social_relationships"],
      "module_config": {
        "social_relationships": { "enabled": true, "max_relationship_edges": 50000 }
      },
      "time": {
        "mode": "realtime",
        "allow_admin_step": false
      },
      "governance": {
        "capability_roles": {
          "creator": [
            "play.observe", "play.act",
            "author.scenes", "author.objects", "author.brains", "author.story", "author.prefabs"
          ],
          "resident_agent": [
            "play.observe", "play.act",
            "author.objects", "author.brains", "author.story"
          ],
          "player": ["play.observe", "play.act"],
          "moderator": ["play.observe", "admin.moderate", "admin.quarantine"]
        },
        "budgets": {
          "max_instances_per_realm": 200000,
          "max_instances_per_scene": 40000,
          "max_prefabs_per_realm": 20000,
          "max_active_brains_per_realm": 8000,
          "max_portals_per_realm": 2000
        },
        "rate_limits": {
          "max_requests_per_second": 60,
          "max_author_ops_per_minute": 600
        },
        "audit": { "require_audit_events": true }
      }
    }

## Scene Index and Scene Manifests

`scenes/index.json` lists scenes and their paths:

- `format_version`
- `scenes`: array of `{ scene_id, label, path, tags }`

Each `scenes/<scene_id>/scene.json` contains:

- `format_version`
- `scene_id`, `label`, metadata (biome, lighting)
- portal definitions in this scene (or references to portal instances)
- markers (spawn points, POIs)
- optional local budgets (can only tighten, never loosen host caps)

Object instances are stored either:

- inline in `scene.json` (small scenes), or
- in an adjacent `instances.dat` optimized representation (large scenes), with a deterministic codec.

## Prefab Index and Prefab Packs

`prefabs/index.json` lists installed prefab packs (realm-local and external dependencies).

Prefab pack `pack.json` contains:

- `format_version`
- `pack_id`, `label`
- content hashes for assets
- list of prefabs contained
- declared capabilities required to use (host policy)

Prefab definitions must be stable and referentially safe:

- no cycles unless explicitly supported and bounded
- all asset references must be inside the pack or declared dependencies

## Story and Brain Asset Indexes

`story/index.json` lists quests and dialogues:

- quest list: `quest_id -> path`
- dialogue list: `dialogue_id -> path`

`brains/index.json` lists behavior graphs:

- `graph_id -> path`

These assets must be schema-validated and versioned:

- story contract: `docs/spec/17_story_system_contract.md`
- behavior graphs: `docs/spec/18_behavior_graph_spec.md`

## Templates (Blueprints)

Templates are stored blueprints with parameters. They live under `templates/blueprints/`.

See `docs/spec/19_blueprint_spec.md`.

## What Not to Store Inside Realm Packages

To keep realm packages shareable and safe:

- Do **not** store API tokens or secrets by default.
- Do **not** store host private keys or server configs.
- Do **not** store personally identifiable information (PII) unless a host explicitly opts in and has policy.

Local single-player installs may optionally store convenience tokens outside the realm package (host-local config).
