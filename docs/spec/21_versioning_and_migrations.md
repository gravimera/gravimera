# Versioning and Migrations

_(Spec document; see `docs/gamedesign/12_content_formats.md` for the product goals.)_

This document defines how Gravimera keeps realm packages durable over time: versioning rules, compatibility expectations, and migration behavior.

This is required for a “metaverse-like” engine where realms live for a long time and are continuously modified by AI agents.

## Principles

1) **Never corrupt user content**: migrations must be non-destructive.
2) **Explicit versions everywhere**: every artifact declares its `format_version`.
3) **Forward progress**: the engine can upgrade old formats to the newest supported format.
4) **Clear failure modes**: if a migration cannot be performed, the engine must explain why and leave the source intact.
5) **Testable migrations**: every supported historical format version has fixture files under `tests/`.

## Versioning Model

There are multiple independent version axes:

1) **Engine version**: the Gravimera binary version.
2) **API version**: `/v1`, `/v2`, etc (see `docs/spec/16_agent_api_contract.md`).
3) **Realm package versions**:
   - manifest version (`realm.json.format_version`)
   - ruleset version (`ruleset.json.format_version`)
   - scene manifest version (`scene.json.format_version`)
   - prefab definitions version (`prefab.json.format_version`)
   - story assets versions (quest/dialogue)
   - behavior graph version

These formats must evolve independently; a change in story assets should not force a change in prefab format.

## Compatibility Contract

### Engine Loading Rules

On load:

1) Parse `realm.json`.
2) Verify `engine_version_range` compatibility.
3) Load `ruleset.json`.
4) Load indexes (`scenes/index.json`, `prefabs/index.json`, `story/index.json`, `brains/index.json`).
5) Load the entry scene (or host-selected scene).

If any artifact is too new (format version unsupported):

- the engine must refuse to load that artifact,
- report which file/version caused the issue,
- and offer a safe fallback if possible (e.g. open the realm browser but not the realm).

### Host Overrides

Hosted servers may override:

- rate limits and budgets (tighten),
- disallow deterministic stepping,
- disable risky authoring capabilities,
- disable prefab uploads/imports.

Hosts must not silently *expand* a realm’s permissions; only restrict or require explicit admin approval.

## Migration Behavior

### Migration Modes

The engine provides two modes:

1) **In-place upgrade with backup** (default for local use):
   - create a backup copy of the realm directory (`<realm_root>.bak.<timestamp>`)
   - apply migrations in place

2) **Upgrade to new directory** (recommended for hosting):
   - migrate from `<old_root>` to `<new_root>`
   - only swap directories atomically after full validation

### Migration Order

Migrations must run in a deterministic order:

1) manifest
2) ruleset
3) indexes
4) scenes
5) prefabs and packs
6) story assets
7) behavior graphs
8) templates/blueprints

Each step may emit a migration report entry.

### Idempotence

Migrations must be idempotent:

- running migration twice yields the same result
- partial failures do not leave the realm in a “half upgraded” state without a recovery path

### Migration Reports

After migration, the engine writes a report (host-local or realm-local depending on policy) that includes:

- from/to versions for each artifact kind
- file paths changed
- warnings (fields dropped, defaults applied)
- errors (if any; with safe rollback instructions)

## Determinism and Replay Compatibility

To support agent training and debugging:

- event logs and deterministic stepping must remain replayable across minor engine upgrades when possible.

Realm packages should optionally be exportable with:

- a snapshot (save)
- an event log segment
- the engine version used to produce it

If deterministic semantics change, the engine must:

- bump a “determinism compatibility” version,
- and clearly signal that replays may diverge.

## Testing Requirements

For every artifact format version supported for migration:

- include a fixture under `tests/realm_fixtures/<version>/...`
- include a test that:
  - loads the old fixture
  - migrates it to current
  - asserts key invariants (ids preserved, scenes accessible, story assets parse)

Hosted servers should run these tests in CI to prevent shipping a build that cannot load existing realms.

## Deprecation Policy

To keep the engine maintainable:

- the engine supports migrating from the last **N** major realm-format versions (host policy; recommended N=2).
- older formats must be migrated via intermediate versions or a dedicated offline migrator tool.

Deprecation must be documented prominently in release notes and in tooling.
