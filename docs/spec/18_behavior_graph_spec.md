# Behavior Graph Spec (Embedded Brains)

_(Spec document; product intent lives in `docs/gamedesign/06_brains_and_ai.md`.)_

This document defines the **final target** embedded behavior system used by NPCs and autonomous units. It is a safe, data-driven way to author “intelligence” inside the simulation without running arbitrary code.

It complements:

- `docs/gamedesign/06_brains_and_ai.md` (concepts)
- `docs/spec/16_agent_api_contract.md` (API surfaces)
- `docs/spec/17_story_system_contract.md` (story actions and variables)

## Design Goals

1) **Expressive enough for living worlds**: schedules, patrols, social behaviors, quest-giver NPCs, vendors.
2) **Deterministic**: same state + same tick stream => same decisions.
3) **Safe and bounded**: budgets prevent runaway CPU; sensing is limited.
4) **Debuggable**: creators can inspect current node and blackboard; failures produce events.
5) **Composable with external agents**: behavior graphs can call the same semantic actions as the HTTP API.

## Core Model

A behavior graph is a directed graph evaluated by a runtime as a **tree of execution** with explicit status:

- `Running`: the node is still working (continue next tick)
- `Success`: node completed successfully
- `Failure`: node failed

The runtime evaluates at most **N node steps per tick** (budget).

### Blackboard

Each brain has a blackboard (key/value store) with typed values:

- `bool`, `i64`, `f64`, `string`
- `vec2`, `vec3`
- `entity_ref` (instance id) and `identity_ref` (identity id)

Hosts may allow small structured objects, but the baseline should keep types simple for determinism and validation.

### Deterministic Randomness

Some behaviors need randomness (idle variety). Randomness is allowed only if deterministic:

- RNG seed = `(realm_seed, unit_identity_or_instance_id, graph_id, tick)`

The graph runtime exposes a `rand_f64()` primitive that is deterministic under this seed.

## Node Families (Contract)

The graph supports a core set of node types. Hosts may extend with additional node libraries, but the core set must remain stable.

### Control Flow Nodes

- `Sequence`: run children in order; fails on first failure; succeeds if all succeed.
- `Selector`: run children in order; succeeds on first success; fails if all fail.
- `ParallelAll`: run children “concurrently” (cooperatively); succeeds if all succeed; fails if any fail; budget applies.
- `Repeat`: repeats a child until count or condition.

### Decorators

- `Invert`: swaps success/failure.
- `Cooldown`: prevents child from running more than once per duration (world time).
- `Timeout`: fails child if it has been running longer than duration.
- `Guard`: evaluates a condition; runs child only if true.

### Conditions (Pure)

Conditions must be side-effect free.

- `VarEquals`, `VarGreater`, `VarExists` (story variables or blackboard variables)
- `HasTargetInRange` (hostile/any; deterministic query with limit)
- `IsTimeInRange` (world clock)
- `IsInScene` (scene id match)

### Actions (Side Effects)

Action nodes map to semantic actions:

- `MoveTo`: pathfind to a marker or position
- `Follow`: follow an entity_ref
- `Interact`: talk/use/pickup
- `SetBlackboard`: set local memory
- `SetStoryVar`: modify story variables (capability and realm policy gated)
- `EmitEvent`: custom events for resident agents (“npc_thought”, “job_completed”)

If the combat module is enabled:

- `AttackTarget` / `FireAtPoint`

### Sensing and Queries (Bounded)

Query nodes read the world and write to the blackboard:

- `FindNearby`: writes a list of entity_refs (bounded length)
- `PickClosest`: selects one target deterministically
- `RaycastLOS`: line-of-sight check (bounded)

All sensing must be bounded and deterministic:

- queries have a max radius and max results,
- ties are broken by stable ordering (entity id / instance id).

## Execution Semantics

### Tick-Based Evaluation

On each tick:

1) The brain resumes from its current running node.
2) The runtime executes up to `budget_steps` node steps.
3) Node statuses propagate to parents.
4) The brain may emit actions/events.

Long-running actions (MoveTo, Follow) remain `Running` until completion or timeout.

### Failure Handling

When an action fails (no path, invalid target):

- the node returns `Failure`,
- the brain emits a `brain_error` event with:
  - node id, reason, tick, and relevant ids.

Realms can choose policy:

- keep running (selector may pick alternative),
- disable the brain if repeated failures exceed a threshold.

## Schedules as Graphs

NPC schedules are a common case:

- schedules can be built as behavior graphs with time guards, or
- represented as a higher-level schedule asset that compiles into a graph.

The design requirement is:

- schedule behavior is inspectable and editable by creators and agents.

## Debuggability (Required)

The engine must expose:

- current active node path (stack),
- blackboard state (with sensitive keys optionally hidden by host policy),
- last N transitions (node enter/exit with status),
- last error (if any).

This is essential for AI-authored worlds where creators need to understand why NPCs behave a certain way.

## Safety Budgets (Required)

Per brain:

- `budget_steps_per_tick`
- `max_query_radius`
- `max_query_results`

Per realm/scene:

- max active brains
- max total brain steps per tick across the realm (global cap)

When budgets are exceeded:

- the brain yields early,
- a `brain_budget_exceeded` event may be emitted (rate-limited).
