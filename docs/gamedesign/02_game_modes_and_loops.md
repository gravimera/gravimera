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

Build mode has two scene views:

- **Realm scene**: the normal in-world camera view where instances live.
- **3D Preview scene**: an asset-creation view for Gen3D (workshop UI + model preview). Gen3D exists only in this view.

In Build mode, the top toolbar includes **Scenes**, **3D Models**, and **Terrain** toggles plus a Play/Build toggle. Selecting **3D Models** shows a left-side 3D Models panel listing **realm prefab packages** for the active realm. The panel includes a search box; each row shows a thumbnail + short name (fallback to prefab id for legacy prefabs). The list sorts by last-modified time by default, and query results are ranked by relevance. Clicking a row selects it (visually marked in the list) and opens a preview overlay (↑/↓ cycles through matching items and auto-scrolls the list to keep the selection visible); dragging a row spawns the prefab into the world (closing any open preview overlay). Long lists are scrollable via a vertical scrollbar with a draggable thumb, and while the cursor is over scrollable UI panels such as **Scenes**, **3D Models**, and **Terrain**, mouse-wheel scrolling stays captured by that UI instead of zooming the world camera. The preview overlay uses a fixed-size 3D viewport with orbit controls (rotate: LMB drag, zoom: mouse wheel) and a scrollable info box; while the cursor is over the overlay, world inputs (camera/selection/commands) are captured by the preview UI. Selecting **Scenes** shows a left-side Scenes panel listing all scenes in the current realm, with an **Add Scene** flow that creates a new scene workspace and switches to it (saving the current scene first) plus a manage mode for scene import/export/delete. Selecting **Terrain** shows a left-side Terrain panel for browsing, previewing, importing, exporting, and applying realm terrain packages.

### Play Mode (Gameplay)

Play mode is for moment-to-moment gameplay:

- direct control of an avatar (optional) and/or commanding units (optional),
- exploration, interaction, and quest progression,
- interacting with NPCs (dialogue, trades, quest handoffs),
- traveling across scenes via portals.

### Build: 3D Preview Scene (Gen3D Workshop)

Gen3D is the in-game workflow for generating new prefabs from prompts and optional reference images, then saving them into the **active realm’s prefab library** so they can be spawned into the world as units/buildings/props (see `docs/gamedesign/39_realm_prefab_packages_v1.md`).

Gen3D is an authoring tool; it does not define gameplay rules. Generated prefabs become ordinary objects that can be assigned mobility/interaction/attack/brains.

### Realm Ops (Always-On World Operation)

“Realm Ops” is not a UI screen; it is the **operating posture** of a hosted realm:

- the realm can run continuously (real-time) so NPCs and story triggers keep progressing,
- resident agents and embedded brains can keep the world “alive” (schedules, reactions, dynamic events),
- creators (human or agent) can still author content with capability-gated operations.

Realm Ops is what makes Gravimera feel like a “metaverse”: worlds evolve over time, not only when a single human is actively playing.

## Core Loops

### Creator Loop (Human or Agent)

1) Define the realm’s theme and rules (biomes, factions, progression).
2) Create scenes and terrain.
3) Create or import prefabs (composition + Gen3D).
4) Populate scenes with NPCs/enemies/objects and connect scenes with portals.
5) Attach brains and define story triggers/quests/dialogue.
6) Run the realm (Realm Ops): schedules, events, continuous story.
7) Test by playing or by running deterministic simulations via the agent API.
8) Package and share the realm.

### Player Loop (Human)

1) Explore a scene, gather resources, and secure safe areas.
2) Build structures and recruit/construct units.
3) Engage enemies and complete objectives.
4) Unlock new capabilities (new prefabs/brains/areas) through progression.
5) Travel via portals to new scenes and continue the storyline.

Note: this is an example loop. In many realms, combat/economy are disabled and the player loop is primarily exploration + social interaction + story.

### Agent Loop (AI Program)

1) Observe: poll state snapshots + receive events.
2) Decide: run planning/learning logic externally.
3) Act: send semantic actions (spawn/move/fire/interact) and authoring actions when permitted.
4) Advance time: in deterministic mode, step ticks explicitly; in real-time, pace actions to server time.

In “living world” realms, agent responsibilities often split into roles:

- a **creator agent** that authors scenes, NPCs, and quests,
- one or more **resident agents** that run ongoing world logic (events, schedules, moderation),
- optional **player agents** that participate as characters.
