# Rulesets and Optional Modules

Gravimera’s baseline product is a **realm-creation and story engine**. Combat, economy, crafting, and other “game genre” systems are **optional modules** that a realm can enable.

This keeps the platform flexible: a realm can be a peaceful social world, an interactive narrative, a city builder, an RTS skirmish, or a hybrid.

## Ruleset

A **ruleset** is the configuration that defines:

- which optional modules are enabled,
- module parameters (difficulty, resource rates, allowed actions),
- permissions and safety policy (what agents/players can author),
- simulation settings (deterministic stepping allowed or not).

The ruleset is part of the realm package and enforced by the host.

## Module Design Principles

1) **Data-driven**: modules operate on prefab profiles and object tags, not hard-coded prefab ids.

2) **Composable**: modules can be enabled together. Each module must declare what it needs (events, variables, new components).

3) **Agent-compatible**: any module-specific actions must be exposed as semantic API endpoints with clear permissions.

4) **Graceful absence**: if a module is disabled, related UI and API surfaces are hidden or return a consistent “module disabled” error.

## Core (Always-On) Systems

These are not optional because they define the platform:

- scenes, portals, and travel
- object prefab + instance model
- interaction volumes and triggers
- story variables + triggers + actions
- brains (autonomy) for NPCs/units
- agent API surface (observe + act + author, capability-gated)
- persistence and packaging

## Optional Modules (Initial Set)

### Combat Module

Adds:

- health/damage/death rules
- attack profiles (melee/ranged/abilities)
- hostility/factions (combat semantics)
- combat events (`object_damaged`, `enemy_killed`, etc)

When disabled:
- objects may still have “hit” interactions for puzzles, but no damage/health loop.

### Economy Module

Adds:

- resources (types + amounts)
- production buildings (inputs/outputs)
- storage, transport, and trade
- economic NPC roles (worker, trader)

Economy must be authorable:
- resources can be defined by the realm,
- production recipes are data in the realm package,
- agents can spawn/modify economic infrastructure if permitted.

### Crafting Module

Adds:

- inventory and item definitions
- crafting recipes and crafting stations
- loot tables (if the realm wants)

### Social/Relationships Module (Living World)

Adds:

- NPC relationships (friendship, reputation, factions as social groups)
- schedules and jobs
- dialogue conditions based on relationships

This module is strongly recommended for “metaverse-like” realms even when combat/economy are off.

### Physics/Destruction Module (Optional)

Adds:

- breakable objects, structural integrity
- physics-driven debris and hazards

This module is optional because it can complicate determinism and server performance.

## Module Configuration and Versioning

Each module config must be:

- explicit in the realm manifest,
- versioned (so realm packages can be migrated),
- validated by the host (reject malformed configs).

