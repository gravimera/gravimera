# Safety and Governance (Metaverse-Grade)

Because Gravimera is designed for **AI freedom creation**, safety cannot be an afterthought. This document defines the constraints that make agent-driven creation safe and hostable.

## Threat Model (What We Must Defend Against)

- Untrusted agents spamming actions (DoS) or allocating infinite content.
- Agents trying to gain unauthorized control (edit someone else’s objects, steal ownership).
- Malformed authoring payloads that crash the simulation or corrupt saves.
- Story/brain logic creating infinite loops or runaway entity spawning.
- Unsafe content creation in public realms (policy decisions vary by host).

## Capability-Based Access (Non-Negotiable)

Every API token grants a **capability set**. Examples:

- `play.observe` (read-only state snapshots and events)
- `play.act` (issue unit actions for owned units)
- `author.spawn` (spawn objects)
- `author.edit` (edit object transforms/overrides)
- `author.prefabs` (create/edit prefabs)
- `author.story` (create/edit quests/dialogue/triggers)
- `author.scenes` (create/load scenes; create portals)
- `admin.time` (pause/step deterministic time)
- `admin.server` (shutdown, force save/load)

Capabilities are evaluated per request and enforced server-side.

## Ownership and Auditability

- Instances have an owner principal id (or realm-owned).
- Authoring actions must emit audit events (`object_spawned` includes `by_player_id`).
- Hosts can export audit logs for moderation and debugging.

## Budgets and Rate Limits

To prevent runaway creation:

- per-realm budgets (max instances, max prefabs, max brains, max portal gates)
- per-scene budgets (max nav obstacles, max dynamic units)
- per-token rate limits (actions/sec and authoring ops/min)
- per-brain instruction budgets (max graph steps per tick)

Budget overruns produce clear errors and events; they never crash the server.

## Sandboxed “Intelligence”

Two supported patterns:

1) **External AI (recommended)**: agents run models externally and send semantic actions. The server never executes untrusted code.

2) **Embedded AI (optional)**: behavior graphs and optional managed brain runtimes run under a sandbox boundary:
   - behavior graphs run in-process with strict step budgets
   - managed brain runtimes run out-of-process (sidecar/container/VM/remote) with CPU/memory limits and restricted network egress
   - only a narrow API: bounded observations/events + semantic action requests

Realms/hosts can disable embedded brains entirely.

## Safe Authoring Validation

All authoring endpoints must validate:

- schema and required fields
- coordinate bounds and scene constraints
- referenced ids exist and are allowed
- payload size limits
- forbidden cycles (e.g. prefab references that create infinite recursion)

Validation errors must be deterministic and descriptive so agents can repair.

## Content Policy (Host Choice)

Gravimera provides mechanisms, not global policy:

- local single-player: permissive by default
- hosted realms: allow hosts to define policies (allowed assets, max violence, etc)

The engine must support:

- realm-level policy configuration
- moderation tools (ban token, revoke capabilities, quarantine a scene)
