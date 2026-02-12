# Story System Contract (Quests, Dialogue, Triggers, Actions)

This document defines the **final target** story system for Gravimera: how quests, dialogue, and living-world events are authored (by humans or agents) and executed deterministically in the simulation.

This is designed for “metaverse-like” realms where story content can evolve over time and can be authored continuously by AI agents.

## Design Goals

1) **Story is world state**: story logic changes the world by spawning/editing objects, unlocking portals, and mutating variables.
2) **Deterministic by construction**: story execution is reproducible in deterministic stepping mode.
3) **Safe to host**: bounded evaluation, validated assets, budgeted actions, audit events.
4) **Debuggable**: creators can inspect quest states, active triggers, and why an action fired.
5) **Composable with agents**: resident agents can author or extend story; embedded story logic can run without external agents.

## Core Concepts

### Story Variables

Story variables are persistent key/value pairs used to represent narrative state.

- Scope:
  - **realm-global** variables (shared across scenes)
  - **scene-local** variables (only for a scene)
  - **identity-scoped** variables (attached to a stable NPC identity)
- Types: bool, int, float, string, and small structured objects (host policy; JSON-like).
- Versioning: variable sets have a revision number for conflict-safe editing by multiple agents.

Any change to a story variable emits `story_var_changed` with:

- key, old value (optional), new value, scope
- by_principal_id (if changed by an agent/human authoring action)
- tick and event_id

### Triggers

A trigger is a condition that can fire when:

- an event occurs (event-pattern trigger),
- a time condition becomes true (time trigger),
- an entity enters/leaves an area (area trigger),
- a variable changes (var trigger),
- a quest changes state (quest trigger).

Triggers are always evaluated in a bounded way:

- event-pattern triggers evaluate only against new events, not by scanning history,
- time triggers evaluate on tick boundaries,
- area triggers evaluate based on collision/interaction volumes.

### Actions

Actions are the effects story logic can produce. Actions are the only way story changes the world.

Actions must be:

- validated before running (schema + permission + budgets),
- auditable (emit events with request_id / trigger_id),
- deterministic (same input state yields same outcome).

### Quests

A quest is a state machine (finite states + transitions) whose transitions are driven by triggers.

Quests can:

- start automatically when a trigger fires,
- be offered by NPCs via dialogue,
- span multiple scenes via portal travel.

### Dialogue

Dialogue is a directed graph with:

- nodes containing lines (speaker + text + optional emote),
- choices with conditions and actions,
- optional branching based on story variables and relationship state.

Dialogue choices are player/agent actions and are auditable.

## Stable Identities (NPCs and Story Objects)

To make story assets durable:

- Story references should prefer **stable identities** over ephemeral instance ids.

The contract defines an optional identity layer:

- `identity_id`: stable id for “the character” (NPC), “the artifact”, or “the portal”.
- Instances may be replaced over time (respawn, scene migration) while keeping the same identity_id.

This is critical for living worlds where objects evolve.

Hosts can choose to implement identity ids as:

- explicit ids stored in instance metadata, or
- separate identity registry mapping identity_id -> current instance_id(s).

## Execution Semantics

### Evaluation Order

On each tick:

1) The simulation produces zero or more events.
2) Story triggers are evaluated against:
   - newly produced events, and
   - time/area/variable conditions at this tick.
3) Trigger firings enqueue actions.
4) Actions execute in a deterministic order and may produce additional events.

Deterministic action ordering rule:

- actions are ordered by `(tick, trigger_priority, trigger_id, stable_tie_breaker)`.

The stable tie-breaker is required to avoid nondeterminism from hash iteration order or multithread scheduling.

### Budget and Guardrails

To avoid runaway loops:

- Maximum trigger firings per tick (realm-configurable).
- Maximum actions executed per tick (realm-configurable).
- Maximum number of objects that can be spawned by story per minute (budget).
- Action execution depth limit (prevents “trigger causes action causes event causes trigger…” infinite chains in one tick).

If budgets are exceeded:

- story execution stops for the tick,
- `story_error` event is emitted with details,
- the realm may optionally disable the offending trigger until a creator repairs it.

## Trigger Types (Contract)

### 1) Event Pattern Trigger

Fires when an event matches:

- `kind` (required)
- filters on payload fields (scene_id, instance_id, identity_id, quest_id, etc)
- optional additional conditions (variable predicates)

Example: “when the player enters portal X, set quest state”.

### 2) Time Trigger

Fires when world clock reaches a condition:

- at exact time (e.g. 18:00),
- after duration since quest start,
- repeating interval (every N minutes).

Time triggers must be expressed in world clock terms so they work in deterministic mode.

### 3) Area Trigger

Fires when an entity enters/leaves a volume:

- volume can be attached to an object instance (e.g. “town square”) or a marker region.
- subject entity filter (player, NPC role, faction).

### 4) Variable Trigger

Fires when:

- a variable key changes, or
- a variable predicate becomes true.

Variable triggers are realm-global, scene-local, or identity-scoped.

### 5) Quest Trigger

Fires when a quest enters a state or when a transition occurs.

## Action Types (Contract)

Actions are grouped into safe families. Realm policy and capabilities determine which are allowed.

### World Mutation Actions

- `spawn_object` (prefab id + transform + overrides)
- `destroy_object` (by instance_id or identity_id)
- `edit_object` (transform/overrides/tags)
- `create_portal` / `edit_portal` / `unlock_portal`
- `set_owner` / `set_faction` (if the realm supports factions)

### Story State Actions

- `set_var` / `delete_var` / `patch_vars`
- `start_quest` / `complete_quest` / `set_quest_state`
- `start_dialogue` / `end_dialogue`

### Brain/Autonomy Actions

- `attach_brain` / `edit_brain` / `detach_brain`
- `set_schedule` (if schedule is represented separately)

### Messaging and Presentation Actions

These affect user experience but should not be “authority-breaking”:

- `show_notification` (local UI)
- `play_sound` / `play_vfx` (if enabled)
- `emit_story_event` (custom event for agents and resident scripts)

Hosts may restrict presentation actions on public servers.

## Quest Model (State Machine)

Quest asset fields (conceptual):

- `quest_id`
- `label` and description
- `states`: list of state ids
- `initial_state`
- `transitions`: each transition has:
  - `from_state`
  - `trigger` (one of the trigger types, with parameters)
  - `conditions` (variable predicates)
  - `actions` (action list)
  - `to_state` (optional; transitions can also be “side effects”)

Quest runtime state per player/party (realm policy):

- quest state can be per-realm, per-player, per-party, or per-faction.
- the quest asset declares its intended scope, but the realm ruleset can override.

## Dialogue Model (Graph)

Dialogue asset fields (conceptual):

- `dialogue_id`
- `nodes`: each node:
  - `node_id`
  - `speaker_identity_id` (optional)
  - `lines`: text + optional emote tags
  - `choices`: each choice:
    - label text
    - conditions (vars/relationships/inventory)
    - actions
    - next_node_id (or end)

Dialogue runtime must emit:

- `dialogue_started`, `dialogue_choice`, `dialogue_ended`

These events are crucial for resident agents that “narrate” or extend worlds.

## Testing and Debugging (Required)

Creators must be able to:

- list all quests and their current states,
- view active triggers and when they last fired,
- inspect why a trigger fired (matched event + condition evaluation),
- run an offline/deterministic test.

The Agent API should support:

- “story test” endpoints (author/admin only),
- an event trace export for replay,
- a “dry run” mode for applying story asset changes to validate schema and budgets.

