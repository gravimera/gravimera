# Game Modes and Core Loops

Gravimera is built around **authoring + playing** the same world. Humans and agents use different interfaces, but the same underlying simulation.

## Modes

### Build Mode (Authoring in-world)

Build mode is for placing and editing objects in a scene:

- place terrain primitives and structural blocks,
- spawn units/buildings/props,
- edit transforms and object overrides,
- place portal gates and configure their destinations,
- attach brains to units and configure them,
- author story triggers on objects (optional UI; always possible via API).

Build mode is not “paused by definition”; a realm can be live while authoring, but certain high-impact operations require permissions (see API design).

### Play Mode (Gameplay)

Play mode is for moment-to-moment gameplay:

- direct control of the hero/avatar (optional) and/or RTS-style unit control,
- combat, exploration, resource gathering, and quest progression,
- interacting with NPCs (dialogue, trades, quest handoffs),
- traveling across scenes via portals.

### Gen3D Workshop Mode (Asset Creation)

Gen3D is the in-game workflow for generating new prefabs from prompts and optional reference images, then saving them into the realm so they are playable as units/buildings/props.

Gen3D is an authoring tool; it does not define gameplay rules. Generated prefabs become ordinary objects that can be assigned mobility/interaction/attack/brains.

## Core Loops

### Creator Loop (Human or Agent)

1) Define the realm’s theme and rules (biomes, factions, progression).
2) Create scenes and terrain.
3) Create or import prefabs (composition + Gen3D).
4) Populate scenes with NPCs/enemies/objects and connect scenes with portals.
5) Attach brains to units and define story triggers/quests.
6) Test by playing or by running deterministic simulations via the agent API.
7) Package and share the realm.

### Player Loop (Human)

1) Explore a scene, gather resources, and secure safe areas.
2) Build structures and recruit/construct units.
3) Engage enemies and complete objectives.
4) Unlock new capabilities (new prefabs/brains/areas) through progression.
5) Travel via portals to new scenes and continue the storyline.

### Agent Loop (AI Program)

1) Observe: poll state snapshots + receive events.
2) Decide: run planning/learning logic externally.
3) Act: send semantic actions (spawn/move/fire/interact) and authoring actions when permitted.
4) Advance time: in deterministic mode, step ticks explicitly; in real-time, pace actions to server time.

