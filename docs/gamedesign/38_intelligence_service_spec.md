# Intelligence Service Spec (Standalone Brains)

_(Spec document; product intent lives in `docs/gamedesign/06_brains_and_ai.md`.)_

This document defines a **final target** architecture for “AI plugins” that fully control a unit by running in a **standalone intelligence service process** (instead of embedded Wasm).

For developer/player convenience, the host may also support an **embedded** mode where the same intelligence-service implementation runs **in-process**. Embedded mode does **not** provide isolation and should only be used for **trusted** brain code (e.g. built-in/demo modules).

Implementation note (2026-04-02): the current game build ships the intelligence service **embedded-only**. Sidecar/remote deployment models are deferred.

A standalone brain is an optional advanced brain type alongside:

- external agents driving units via the HTTP API, and
- embedded behavior graphs (`docs/gamedesign/18_behavior_graph_spec.md`).

The core idea remains: **sense → decide → act**, where the host simulation is always authoritative.

## Design Goals

1) **Full unit control**: movement, abilities/actions, interaction, and speech.
2) **Host-authoritative**: the simulation validates and executes all actions.
3) **Deployable isolation**: the brain runtime can run as:
   - a local sidecar process,
   - inside a sandbox boundary (container/VM), or
   - on a remote server.
4) **Safe to host**: budgets, capability gating, and explicit trust boundaries.
5) **Scales to many units**: bounded sensing, AI LOD, batching, and parallel execution.
6) **Debuggable and replayable**: command outcomes, traceability, and deterministic replay via action logs.

## Non-Goals

- Let brain code mutate simulation state directly (no “write access to ECS”).
- Require a round-trip brain decision at 30–60Hz over a network.
- Provide unbounded queries (“give me the entire world state”).

## High-Level Architecture

### Components

- **Simulation Host** (the game server / single-player runtime)
  - produces authoritative state, events, and rule outcomes
  - generates perception edges (`SeenEnter/SeenExit`) and other gameplay events
  - validates and executes commands
- **Intelligence Service** (standalone process)
  - runs brain code and maintains per-unit brain state
  - receives bounded observations and events
  - outputs action requests (intents)

### Trust Boundary

The simulation host must assume the intelligence service is **untrusted** unless explicitly configured otherwise. All requests are validated, rate-limited, and capability-gated.

## Deployment Models

0) **Embedded (dev convenience / single-player)**  
   Intelligence service runs **inside** the host process. This is ergonomic, but it is **not** a trust boundary.

1) **Local sidecar (dev / low latency)**  
   Intelligence service runs on the same machine; transport is IPC (Unix domain socket / named pipe).

2) **Sandboxed local runtime (hosting)**  
   Intelligence service runs inside a container or VM with explicit CPU/memory/network restrictions. The host communicates over a virtual network or forwarded socket.

3) **Remote service (scale / central hosting)**  
   Intelligence service runs on another machine. Transport must be authenticated and encrypted; latency/jitter must be expected.

The **same logical protocol** should work across all three, even if transports differ.

## Protocol and Versioning

This spec is transport-agnostic. It can be implemented over gRPC/Protobuf, WebSocket, or HTTP/2. For IPC, use the same message schema over a framed stream.

### Versioning

- Protocol MUST declare a `protocol_version`.
- Messages MUST include stable ids (`realm_id`, `scene_id`, `unit_instance_id`, etc).
- Hosts may reject incompatible protocol versions.

### Authentication and Capabilities

The host grants the service a **capability set** (per realm, per unit, or per brain instance). Examples:

- `brain.move`, `brain.combat`, `brain.interact`, `brain.talk`
- `brain.query.nearby`, `brain.query.raycast`, `brain.query.path` (if exposed)

Capabilities must be enforced by the host command executor and (if applicable) by the host-side query endpoints.

## Execution Model

Brains run as instances managed by the intelligence service.

### Lifecycle

- `load_brain_module(module_descriptor) -> module_id` (optional; host policy gated)
- `spawn_brain_instance { module_id, unit_id, config } -> brain_instance_id`
- `tick_brain_instance { brain_instance_id, tick_input } -> tick_output`
- `despawn_brain_instance { brain_instance_id }`

Hosts may support hot-reload by despawn/spawn with preserved state (policy gated).

### Tick Scheduling

The host decides when the brain is expected to produce outputs:

- **Baseline**: once per simulation step (`FixedUpdate`), e.g. 30–60Hz (only viable for local low-latency deployments).
- **Event-driven**: the host schedules ticks when new events arrive or when a timer expires.
- **AI LOD**: the host reduces tick and sensor rates for less important units (e.g. 2–10Hz), and may fully sleep idle units.

Brains can request `sleep_for_ticks(n)` as a hint; the host may still wake early on events.

### Batching

For performance (and to reduce network overhead), the host should support batched ticks:

- `tick_many([{ brain_instance_id, tick_input }, ...]) -> [{ brain_instance_id, tick_output }, ...]`

The host must define a stable ordering for batched processing (e.g. by unit id) for determinism and debugging.

## Contract Boundary: Requests, Not Authority

Brains emit **intents/commands**, but the host remains authoritative. All commands are validated against:

- rules (cooldowns, costs, permissions),
- physics/collision constraints,
- module enablement (combat/economy),
- realm policy (speech limits, aggression policy).

Outcomes must be reported back to the brain via deterministic command-result events.

## Data Interfaces (Sense → Decide → Act)

### Input: `TickInput`

`TickInput` is a bounded, deterministic snapshot:

- **Self state**: transform/pose, velocity, health/stamina, statuses, inventory summary, cooldowns, current target, team/species/faction tags.
- **Perception snapshot** (bounded, stable ordering):
  - nearby entities: `entity_id`, kind/type, tags (species/faction), relative pose/vel, relationship hints.
  - hazards/projectiles/areas of effect (optional module data).
  - navigation hints (optional): local walkability sample, last path id, etc.
- **Events since last delivery** (see “Event Model”).
- **Time**: fixed `dt_ms`, `tick_index`, realm time (if enabled).
- **Deterministic randomness**: RNG seed derived from `(realm_seed, unit_id, brain_id, tick_index)`.
- **Capabilities**: effective capability set granted to this instance (read-only).

All lists must have explicit caps (`max_nearby_entities`, `max_events_per_delivery`, etc). Truncation must be explicit.

### Output: `TickOutput`

`TickOutput` is a bounded set of requested actions:

- **Motor control** (recomputed; not authoritative):
  - `set_move(vec2)` / `set_throttle(f32)` / `set_look(yaw,pitch)`, or
  - `move_to(position)` if pathfinding is host-side.
- **Discrete commands**:
  - `use_ability(ability_id, target_id|position)`
  - `interact(entity_id)`
  - `equip(item_id)` / `drop(item_id)`
  - `set_target(entity_id)` / `clear_target`
- **Speech/social**:
  - `say(channel, text, target_id?)`
  - `emote(kind)`
- **Scheduling hint**:
  - `sleep_for_ticks(n)`
- **Optional action horizon**:
  - commands may include `valid_until_tick` so the host can keep applying them if the brain is late.

## Latency and Reliability

### Latency Model

When the intelligence service is remote (or slow), per-tick synchronous decision-making is impractical. The host should support:

- **durable goals** (e.g. `move_to`, `attack_target_until_cancelled`) rather than “per-frame steering only”
- **command horizons** (`valid_until_tick`) so actions persist across missed ticks
- **asynchronous delivery**: if a tick response is late, apply it on the next sim step (or discard if expired)

### Deadlines and Timeouts

For each scheduled tick delivery, the host sets a deadline. On missed deadlines:

- keep last valid intents (until horizon expires),
- downgrade LOD (tick less often),
- or disable the brain and fall back to a safe built-in brain (realm policy).

### Deterministic Mode

For deterministic stepping and replay:

- The simulation is deterministic given an action stream.
- The host should record `(tick_index, tick_input_hash, tick_output, command_results)` for each unit.
- Replays should use the recorded action stream instead of depending on live brain execution.

## Event Model

Events are generated by the **host simulation** and delivered to the intelligence service.

### Event Delivery Rules

- Each unit has a bounded event inbox (ring buffer) on the host.
- Deliveries are deterministic (stable ordering).
- On overflow, emit a synthetic event:
  - `EventsDropped { count }`, and/or
  - `PerceptionReset { snapshot }`.

### SeenEnter / SeenExit (Perception Edges)

`SeenEnter`/`SeenExit` are **edge events** emitted only when a target transitions:

- not-visible → visible (`SeenEnter`)
- visible → not-visible (`SeenExit`)

They are detected at the rate the **perception system** runs (often slower than the simulation tick).

## Perception / Visibility System (Gameplay Sensing)

Bevy’s built-in `Visibility` is for rendering; unit perception is a **gameplay system** that must be implemented explicitly.

Baseline deterministic, bounded algorithm:

1) broadphase query from a spatial index (uniform grid / spatial hash),
2) filter by deterministic rules (range, FOV, tags),
3) optional LOS checks (raycast) with strict budgets,
4) compute `visible_set`,
5) diff against previous set to emit `SeenEnter/SeenExit`.

## Performance and Scaling Requirements

- Avoid `O(n²)` using a spatial index.
- Run perception at a sensor rate (5–20Hz) with staggered updates.
- Apply AI LOD: reduce tick/sensor rates for far/idle/offscreen units; sleep when possible.
- Cap and coalesce events; rate-limit speech.
- Prefer batched tick RPCs to reduce overhead and allow the service to optimize internally.

## Safety, Governance, and Fairness

### Budgets

Host and service must enforce:

- CPU budgets per brain (and per realm total),
- memory limits,
- max events per delivery,
- max commands per output,
- max speech bytes per window.

### Sandbox Expectations

When running untrusted brain code, isolation should be provided by the service deployment:

- container/VM boundary,
- restricted filesystem access,
- restricted network egress (or disabled),
- explicit resource quotas.

### Provenance and Trust

Hosts may require:

- signed brain modules (content hash + signature),
- a module registry with approvals,
- audit logs linking each command to `(principal_id, brain_id, unit_id)`.

## Tooling and Observability (Required)

- local brain runner harness and fixture scenes,
- structured, rate-limited logs correlated by unit id and brain id,
- metrics: tick latency, missed deadlines, event drops, command rejects.
