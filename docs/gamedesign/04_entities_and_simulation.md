# Entities and Simulation

This document describes the “game object” model and the simulation rules that make authored content playable.

## Object Model (Prefab + Instance)

Everything in the world is an object instance that references a prefab. Prefabs define:

- size and collision profile,
- interaction flags (movement blocking, projectile blocking, supports standing),
- mobility profile (static / ground / air),
- optional attack profile (melee or ranged),
- anchors and attachments (for deterministic assembly),
- optional default brain profile.

Instances define:

- stable instance id,
- transform (pos/rot/scale),
- overrides (tint, health, owner/faction, brain config, story tags).

## Ownership, Factions, and Permissions

Objects can be:

- **owned** by a player/agent (commandable by default),
- **neutral** (world props, story objects),
- **hostile** (enemy factions),
- **shared** (co-op owned objects or realm-owned infrastructure).

Ownership controls default command permissions. Realm rules can override (e.g. allow allies to command each other’s units).

## Units

Units are movable, commandable objects with:

- mobility (ground/air) and navigation constraints,
- optional attack profile,
- optional brain for autonomy,
- optional inventory and equipment slots (if the realm rules enable it).

Unit behaviors must remain predictable under deterministic stepping:

- movement uses the realm’s navigation data,
- firing uses explicit targets or brain-selected targets with deterministic rules,
- any randomness must be seeded from stable ids + deterministic clocks.

## Buildings

Buildings are mostly static objects used for:

- defense (turrets, walls),
- production (workshops, spawners),
- story (quest hubs, portals, triggers),
- infrastructure (resource processors, storage).

Buildings can also have brains (e.g. “auto-repair nearby structures”, “spawn guards when attacked”).

## Combat (Generic, Data-Driven)

Combat is expressed through data:

- melee: range + arc + damage + cooldown,
- ranged: projectile prefab + muzzle anchor + projectile physics profile,
- defenses: health, resistances (optional), shields (optional).

The engine must not hard-code behavior to specific prefab ids. Prefabs carry profiles and systems operate on profiles.

## Interactions and Triggers

The simulation supports “interaction volumes”:

- portal entry zones,
- quest area entry/exit,
- dialogue interaction radius,
- pickups and switches.

Interactions produce events and can mutate story variables via story actions.

