# Persistence, Packaging, and Modding

The final product must make authored realms portable and durable over time.

## Realm Save Layout (On Disk)

A realm is stored as a directory containing:

- a realm manifest (name, version, author, dependencies),
- a ruleset (enabled modules + module configuration),
- scenes (each with terrain + object instances),
- prefab defs (built-ins referenced + generated/imported prefabs),
- story assets (quests, dialogue) and story variables,
- optional agent metadata (allowed tokens/capabilities) when self-hosted.

The exact file formats are implementation details, but the layout must support:

- versioned migrations,
- partial loading (load one scene),
- packaging as a single archive for sharing.

See `docs/gamedesign/12_content_formats.md` for the responsibilities and versioning expectations of these artifacts.

## Packaging

A “realm package” is a distributable artifact:

- includes all required prefabs and assets (or declares dependencies),
- includes scenes and story content,
- includes a manifest describing required engine version and optional mods.

## Modding (Safe Extensions)

Modding is a policy choice per realm/server. Supported extension mechanisms:

- new prefab packs (data + assets),
- new behavior graph node libraries (data-defined),
- optional sandboxed brain runtimes (standalone intelligence services) under strict permissions.

No mod is allowed to silently expand its capabilities; every capability is explicit in the manifest and must be approved by the host.
