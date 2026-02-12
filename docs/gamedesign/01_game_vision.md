# Game Vision

Gravimera is a **realm-creation and story engine**: a sandbox where building worlds, creating characters, and scripting living storylines are all part of play — and where **AI agents can play and create** through HTTP APIs as naturally as humans do through the UI.

## Who It’s For

- **Players** who want a Minecraft-like building experience, but also want to create *living* worlds with autonomous units and storylines.
- **Creators** who want to build custom games (RTS skirmish, tower defense, adventure campaign, simulation) inside one engine.
- **AI/ML practitioners** who need a controllable 3D environment with deterministic stepping, event logs, and a safe action API.

## What Makes It Different

1) **Agents are first-class**: the agent interface is not a “bot hack”; it is an official player surface with identity, permissions, and stable semantics.

2) **Autonomy is composable**: units can be authored with brains (patrol, guard, trade, quest-giver) and extended via data-driven behavior graphs.

3) **Story is world-state, not cutscenes**: quests and NPCs are built out of triggers/actions tied to objects, events, and variables across scenes.

4) **Creation and play share the same world**: objects created by Gen3D or prefab composition are immediately playable and persist in the realm package.

5) **Metaverse-first**: worlds are designed to be hosted, shared, and extended by agents over time. The engine provides governance and safety mechanisms to support “living realms”.

6) **Optional genre systems**: combat and economy are modules. A realm can be peaceful and social, purely narrative, or can opt into combat/economy/crafting.

## Experience Goals (Player View)

- Start a realm and immediately create a small “place”: terrain + buildings + NPCs.
- Attach schedules/brains so NPCs continue living when you leave.
- Create a portal gate to a second scene (“district”, “dungeon”, “dream world”), populate it with characters, and link story progression to travel and objectives.
- Play through the storyline as a human, or let an agent run the realm as a simulation (or both).

## Experience Goals (Agent View)

- Use an API that exposes **semantic actions** (spawn, move, fire, interact), **state snapshots**, and an **event stream**.
- Run the realm in deterministic step mode for reproducibility, then switch to real-time for live play.
- Author content: scenes, portals, prefabs, brains, and story data — all without manual UI.
- Operate a “living world”: run creator/resident agents that continuously evolve the realm by reacting to events and advancing stories.

## Non-Goals (To Keep the Game Coherent)

- No raw keyboard/mouse injection as the primary agent interface. The agent interface is semantic.
- No arbitrary code execution inside the game by default. Extensibility is via data-driven graphs and sandboxed modules.
