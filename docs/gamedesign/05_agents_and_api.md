# Agents and HTTP API (First-Class Players)

Agents interact with Gravimera via HTTP APIs. The API is a **player interface**, not a debug-only harness.

## Core Principles

1) **Semantic actions, not raw input**  
Agents send commands like “move unit X to (x,z)” or “spawn prefab P at T”. They do not inject keyboard/mouse events.

2) **Stable ids everywhere**  
Objects, prefabs, scenes, and players use stable ids so agents can reason across saves and over time.

3) **Event-first design**  
Agents consume an ordered event stream (plus state snapshots) to build robust control loops and story logic.

4) **Capability-based security**  
Tokens grant explicit capabilities (play-only vs authoring vs admin/time-control).

5) **Determinism as a mode**  
Realms can run in real-time or deterministic stepping. The API must support both.

## Player/Agent Identity

Every API request is attributed to a principal:

- `player_id`: stable id for the actor (human or agent),
- `display_name`: optional,
- `capabilities`: what the token allows.

Ownership and permissions are evaluated against this identity.

## API Surfaces

### Observation

- state snapshot endpoints (objects, prefabs, scenes, story variables),
- an ordered event stream endpoint (SSE or long-poll),
- optional: spatial queries (objects in radius, raycasts) to keep agent bandwidth manageable.

### Actions (Play)

- move/stop commands for units,
- target selection / firing / ability activation,
- interact with objects (talk, use, open, pick up),
- party control (select group, follow, formations) if the realm rules enable it.

### Authoring (Build)

- create scenes and define metadata,
- place terrain primitives and buildings,
- spawn units and attach brains,
- create and link portals,
- upload/modify prefab data (with safety constraints),
- author story assets (quests, dialogue, triggers).

### Admin / Time Control

High-impact endpoints:

- pause/resume/step deterministic time,
- force save/load,
- shutdown server.

These require explicit admin capabilities and are typically disabled in public hosting.

## Event Model

Events are append-only and ordered per realm:

- each event has `event_id` (monotonic), `timestamp` (simulation time), `kind`, and `payload`.
- clients store a cursor and can resume from a known `event_id`.

Event kinds include (non-exhaustive):

- object lifecycle: `object_spawned`, `object_destroyed`, `object_damaged`, `object_owner_changed`
- movement: `unit_move_started`, `unit_move_completed`
- combat: `attack_started`, `attack_hit`, `enemy_killed`
- scene: `scene_loaded`, `portal_entered`
- story: `story_var_changed`, `quest_started`, `quest_completed`, `dialogue_started`, `dialogue_choice`
- errors: `api_error`, `story_error`, `brain_error`

## Multi-Agent Concurrency

When multiple agents act in one realm:

- actions are processed in tick order; within a tick, actions are ordered by receive-time then by a stable tie-breaker (player id).
- conflicting edits (two agents edit same object) are resolved by realm rules (owner wins, admin wins, or last-write with audit events).

## Safety and Limits

To keep servers stable:

- per-realm budgets (max objects, max brains, max portals),
- per-token rate limits (actions/sec, authoring ops/min),
- validation on all authoring payloads (bounds, schema, disallowed references),
- optional “authoring sandbox” scenes where heavy generation occurs.

