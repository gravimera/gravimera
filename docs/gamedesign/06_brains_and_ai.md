# Brains and AI (Unit Autonomy)

Units can act autonomously through an attached **brain**. Brains are part of the authored content of a realm.

This document describes the concepts and goals. The detailed embedded behavior programming model is specified in:

- `docs/gamedesign/18_behavior_graph_spec.md`

## Why Brains Exist

Gravimera supports two ways to make units “intelligent”:

1) **External intelligence**: an agent observes and sends actions for units every tick.
2) **Embedded intelligence**: a brain runs inside the simulation using data-driven rules/graphs.

Both are first-class. Embedded brains enable worlds to feel alive even when the authoring agent is offline.

In “metaverse-like” realms, embedded brains are essential: NPCs should keep living even when no human is present.

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

## NPC Memory and Knowledge

NPCs need persistent state to feel alive:

- **short-term state**: blackboard variables in the behavior graph (current goal, current conversation partner).
- **long-term state**: story variables attached to an NPC identity (relationships, promises made, quest progress).

The design goal is that NPC “memory” is observable and editable by creators/agents (with permissions), so AI-created worlds remain debuggable and reproducible.

## Determinism Rules

Brains must be reproducible in deterministic mode:

- tick evaluation uses fixed dt,
- randomness (if any) is derived from `(realm_seed, unit_instance_id, tick_index)`,
- any “sensing” (enemy in range, line of sight) uses deterministic queries.

## Budgets and Guardrails (Required for Hosting)

Brains must be safe to run at scale:

- per-brain step budget per tick
- per-scene and per-realm max active brains
- bounded sensing queries (radius/limit)
- clear failure behavior (emit `brain_error` event; optionally disable brain)

These guardrails let hosts run “living worlds” without risking runaway CPU usage.

## Extensibility (Optional Advanced)

For creators who need more complex logic than behavior graphs:

- support managed sandboxed brain runtimes (standalone intelligence service processes) with strict resource limits and a constrained API surface.
- brains do not mutate simulation state directly; they only receive bounded observations/events and emit semantic action requests.
- deployments may be local sidecars, sandboxed containers/VMs, or remote services (host policy).

This is optional and can be disabled by realm policy.

For the proposed standalone intelligence service interface and host requirements, see:

- `docs/gamedesign/38_intelligence_service_spec.md`
