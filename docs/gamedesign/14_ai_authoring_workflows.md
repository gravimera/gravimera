# AI Authoring Workflows (Creator + Resident Agents)

This document describes how AI agents are expected to create “living worlds” in Gravimera.

The core philosophy is: agents create by sending **semantic authoring actions** and verifying results via **state snapshots + events**, not by directly manipulating files or injecting raw input.

Blueprints are the primary bulk authoring mechanism; see:

- `docs/spec/19_blueprint_spec.md`

## Agent Roles in Practice

### Creator Agent (World Builder)

Responsibilities:

- create or modify scenes and terrain,
- place portals and travel graph,
- create prefabs (via Gen3D and composition) and spawn instances,
- create NPCs and attach brains,
- author quests and dialogue (or delegate to a narrator agent).

### Resident Agent (World Operator)

Responsibilities:

- run recurring world events (day/night, markets, emergencies),
- spawn/retire NPCs based on budgets and narrative needs,
- enforce realm-specific governance rules (optional),
- respond to story triggers that are easier to express externally than as embedded graphs.

### Player Agent (Participant)

Responsibilities:

- act as a character in the world (travel, dialogue, tasks),
- optionally cooperate or compete with humans.

## Recommended Control Loop

For deterministic realms:

1) Observe via snapshot and event cursor.
2) Decide (LLM/planner/RL).
3) Act with semantic endpoints.
4) Step N ticks (fixed dt) and wait for step completion.
5) Observe events and verify invariants (object counts, quest state).

For real-time realms:

1) Observe via events (stream) and periodic snapshots.
2) Act at a bounded rate (respect rate limits).
3) Use timeouts and idempotent requests for reliability.

## Authoring Patterns for “Living World” Creation

### Blueprint-Based Construction

Instead of issuing thousands of individual placements, agents should be able to submit a **blueprint**:

- a named set of object instances with relative transforms,
- portal definitions,
- markers (spawn points, POIs),
- initial story variables and quest/dialogue asset ids.

The engine applies the blueprint atomically (or with a clear partial-failure report).

This makes large-scale creation feasible for agents and reduces network overhead.

### Templates and Procedural Layers

Agents often want high-level operations like:

- “generate a village layout”
- “populate scene with 20 citizens with roles”
- “create a quest chain across 3 scenes”

The design allows these as optional “template” endpoints:

- a template is data, versioned with the realm,
- templates can be provided by humans or by prior agents,
- hosts can restrict templates for safety.

### Continuous Story Expansion

A narrator/resident agent can extend the world by:

- watching events and variable changes,
- creating new NPCs and scenes when story requires,
- opening new portals as players progress.

This is how a realm evolves into an ongoing world rather than a static campaign.

## Reliability and Safety Requirements

Agents must be able to recover from failure:

- requests should be idempotent when possible,
- every authoring action should emit an audit event with a request id,
- budgets and rate limits must be visible via observation endpoints.

Agents must be able to reason about permissions:

- endpoints must return “capability missing” errors explicitly,
- tokens should expose their effective capability set (read-only endpoint).
