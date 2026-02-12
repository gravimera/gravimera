# User Experience (Human Creators and Players)

This document describes the intended **human-facing experience** for Gravimera as a realm-creation and story engine.

Agents can do everything via API, but a complete game also needs excellent human tools so people can:

- explore realms,
- create and edit scenes,
- author NPCs/quests/dialogue,
- inspect and debug “living world” behavior.

## Entry Experience

### Realm Browser

Users can:

- create a new realm from a template (empty, town, dungeon, narrative, sandbox),
- open an existing realm,
- import a realm package,
- (hosted universe) browse/search realms published by a host.

Each realm card shows:

- title and description,
- creator(s),
- enabled modules (combat/economy/etc),
- thumbnail(s) for featured scenes,
- last updated time and activity indicators (for always-on realms).

## In-Realm Experience

### Primary Modes (UI)

The UI exposes the same conceptual modes as the engine:

- **Play**: explore, talk, travel, interact.
- **Build**: place/edit objects, portals, markers, and triggers.
- **Story**: author quests and dialogue; inspect story variables.
- **Brains**: attach/configure brains; inspect blackboards and budgets.
- **Assets**: prefab browser; Gen3D workshop; import/export.

These are “tabs” or overlays, not separate games. Users can switch quickly and see changes immediately.

## Scene Authoring UX

### Terrain and Layout

Creators can:

- place primitives and prefab-based terrain pieces,
- paint “supports standing” / “movement blocker” tags onto objects (without guessing),
- place nav markers (paths, points of interest),
- set scene metadata (biome, lighting, ambient audio).

### Portals

Portal creation is a guided tool:

- place portal gate object,
- choose destination scene (or create new scene),
- choose destination spawn (marker or exact point),
- optionally lock behind story variables (“requires_key”).

The UI shows the portal graph (scene connectivity) as a map.

## NPC and Story UX

### NPC Inspector

Selecting an NPC shows:

- identity (stable id, name, role tags),
- current brain state (active node, target, cooldowns),
- relationships/reputation (if enabled),
- dialogue link and quest hooks.

Creators can edit these fields and see immediate effects.

### Dialogue Editor

The dialogue editor is graph-based:

- nodes: lines + speaker + optional emotes,
- choices: conditions + actions,
- preview mode to run a dialogue with simulated variables.

### Quest Editor

The quest editor is state-machine based:

- states + transitions,
- triggers (events + variable predicates),
- actions (spawn, set var, open portal, reward).

Creators can run a deterministic “quest test” that executes triggers against a simulated event stream.

## Agent Observability UX

Since agents are central, creators need tooling to understand agent-driven worlds:

- event stream viewer (filter by kind/player_id/scene)
- audit log viewer for authoring actions
- budget dashboard (entities, brains, events/sec)
- “replay” mode: load a save + event log and step deterministically

This is essential for debugging living worlds and for safe hosting.

