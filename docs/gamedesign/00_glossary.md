# Glossary

This glossary defines terms used across the game design documents.

## World Structure

**Realm**  
A self-contained “game world” owned by one creator (human or agent). A realm contains scenes, prefabs, story state, and configuration (rules). Realms are the unit of saving, sharing, and hosting.

**Scene**  
A region of gameplay space with its own terrain and object instances. A realm contains multiple scenes connected by portal gates. At runtime, a server may load one or many scenes (see `docs/gamedesign/03_world_model.md`).

**Portal Gate (Portal)**  
An object instance that connects one scene to another. Entering a portal triggers travel (and usually a scene load/swap).

**Terrain**  
The physical play surface and obstacles in a scene. In Gravimera, terrain is built from basic objects/primitives plus optional procedural layers, not a separate special-case system.

## Objects and Simulation

**Prefab (Object Definition)**  
A reusable definition identified by a stable UUID. Prefabs define size, collision, interaction flags, parts, anchors, mobility, attack profiles, and optional “brain” defaults.

**Instance (Object Instance)**  
A spawned object in a scene that references a prefab id plus per-instance transform and overrides. Instances have stable UUIDs for networking/persistence.

**Part / Composition**  
Prefabs can contain multiple parts (primitives, models, or references to other prefabs). Parts enable building complex units/buildings from smaller components.

**Anchor / Attachment**  
Named coordinate frames used to attach parts deterministically (avoids ambiguous transforms and makes AI generation/editing stable).

**Unit**  
A movable, commandable object instance (RTS-style). Units can be controlled directly by a player/agent, or indirectly via an attached brain.

**Building**  
A mostly-static object instance used as structure, production, defense, portals, or story set-pieces.

**NPC**  
A unit or building with story-facing metadata (name, role, dialogue, quest hooks). NPCs are still ordinary objects; “NPC” is a role, not a separate engine type.

**Brain (Unit Brain)**  
An attached autonomy controller that issues actions for a unit (move, target selection, fire, interact). Brains can be built-in, behavior-graph driven, or external (agent-controlled).

**Behavior Graph**  
A data-defined behavior program that runs inside the simulation sandbox to drive a brain. It is not arbitrary code.

## Players and Agents

**Player**  
An actor that can observe and act in a realm. Players can be humans (UI client) or AI agents (HTTP client).

**Agent**  
An external program that connects via HTTP APIs, reads state/events, and sends actions. Agents can also author content (scenes, units, brains, story).

**Ownership**  
A mapping from object instance to a player/agent id (or faction). Ownership controls which actions are permitted by default.

**Capability**  
A permission granted to an API token (e.g., “spawn objects”, “edit prefabs”, “load scenes”, “advance time”). Capabilities gate high-impact operations.

## Time and Determinism

**Tick**  
A discrete simulation step. Brains and story triggers evaluate on ticks.

**Deterministic Mode**  
A realm/server mode where time advances only via explicit stepping (fixed dt). Used for reproducible agent training, testing, and debugging.

**Real-time Mode**  
Normal gameplay where time advances continuously.

## Story

**Story Variables**  
Persistent key/value state used by quests and dialogue (e.g., `quest.bandits_defeated = true`).

**Trigger**  
A condition that detects when something happens (event match, area entry, variable change, time reached).

**Action**  
An effect produced by story logic (spawn NPC, open portal, set variable, start quest, play dialogue).

**Quest**  
A structured set of goals and state transitions expressed as triggers + actions across one or more scenes.

