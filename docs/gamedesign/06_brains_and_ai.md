# Brains and AI (Unit Autonomy)

Units can act autonomously through an attached **brain**. Brains are part of the authored content of a realm.

## Why Brains Exist

Gravimera supports two ways to make units “intelligent”:

1) **External intelligence**: an agent observes and sends actions for units every tick.
2) **Embedded intelligence**: a brain runs inside the simulation using data-driven rules/graphs.

Both are first-class. Embedded brains enable worlds to feel alive even when the authoring agent is offline.

## Brain Types

### Built-in Brains (Deterministic)

Built-in brains are configuration-driven and deterministic:

- Patrol: follow a loop of waypoints.
- Guard: hold a position, engage hostiles in radius.
- Follow: follow a target unit or player.
- Worker: gather/transport/build if the realm enables economy/crafting.
- Quest NPC: idle + dialogue + quest handoff behavior.

### Behavior Graph Brains (Data-Driven)

A behavior graph is a safe “program” expressed as data. It defines:

- **nodes** (conditions, actions, control flow),
- **edges** (transitions),
- a **blackboard** (typed variables),
- timeouts and cooldowns.

The runtime evaluates the graph on ticks. Actions are the same semantic actions agents can call (move, fire, interact, set story variable, etc).

This is how creators define new NPC behaviors without embedding arbitrary code.

## Determinism Rules

Brains must be reproducible in deterministic mode:

- tick evaluation uses fixed dt,
- randomness (if any) is derived from `(realm_seed, unit_instance_id, tick_index)`,
- any “sensing” (enemy in range, line of sight) uses deterministic queries.

## Extensibility (Optional Advanced)

For creators who need more complex logic than behavior graphs:

- support sandboxed modules (e.g. WebAssembly) with strict resource limits and a constrained API surface.
- modules cannot access filesystem/network directly; they only read observation data and emit actions.

This is optional and can be disabled by realm policy.

