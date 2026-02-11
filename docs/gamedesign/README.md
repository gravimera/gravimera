# Gravimera — Game Design (Final Target)

This folder is a **living game design** for Gravimera: a Minecraft-like sandbox + “game creation engine” where **AI agents are first‑class players** through HTTP APIs.

The intent of these documents is to describe the *final* product: gameplay, world model, content creation, AI/NPC/story systems, and the platform surface that agents use to play and create.

If a design choice conflicts with existing implementation docs, treat this folder as the **target** and the rest of the repo as “current state”. Keep the design concrete enough that an implementer can translate it into an execution plan.

## Design Pillars

1) **Agents are players**: everything a human can do (observe, act, author content) is available to agents through a stable HTTP API + event stream.

2) **Everything is an object**: the world is made of prefabs (definitions) and instances (spawned objects). Terrain is also composed from basic objects/primitives, not special-cased.

3) **Authoring is gameplay**: building scenes, spawning units, and attaching “brains” is a first-class loop, not a dev-only feature.

4) **Deterministic simulation is a feature**: a realm can run in real-time or in step/paused mode for reproducible agent training and testing.

5) **Safe extensibility**: “intelligence” and “story logic” are data-driven and sandboxed. Agents can always run logic externally if they want full freedom.

## Document Map

- Concepts and vocabulary: `docs/gamedesign/00_glossary.md`
- Vision, audience, experience goals: `docs/gamedesign/01_game_vision.md`
- Game modes and core loops: `docs/gamedesign/02_game_modes_and_loops.md`
- World model (realms/scenes/portals/terrain): `docs/gamedesign/03_world_model.md`
- Entities and simulation (units/buildings/combat/etc): `docs/gamedesign/04_entities_and_simulation.md`
- Agent platform + HTTP API surface: `docs/gamedesign/05_agents_and_api.md`
- Unit brains / AI / behavior graphs: `docs/gamedesign/06_brains_and_ai.md`
- Story system + NPCs + dialogue: `docs/gamedesign/07_story_and_npcs.md`
- Persistence + packaging + modding: `docs/gamedesign/08_persistence_packages.md`
- Multiplayer + hosting model: `docs/gamedesign/09_multiplayer_and_hosting.md`

## Scope Notes (What “Complete Game” Means Here)

This design treats “complete” as:

- A player can create a realm with multiple scenes, travel via portals, and share it as a package.
- A player/agent can create units/buildings/props; attach autonomous behaviors; and build storylines with NPCs and quests across scenes.
- The game is fun to play without writing code, and powerful to author via APIs.
- The platform is safe enough to run untrusted agents against local or hosted realms.

