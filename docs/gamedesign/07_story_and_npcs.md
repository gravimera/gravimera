# Story and NPCs

Gravimera’s story system is designed to be **authored** (by humans or agents) and to be **playable** across multiple scenes.

This document describes the concepts and player-facing goals. The detailed contract for quests/dialogue/triggers/actions is specified in:

- `docs/gamedesign/17_story_system_contract.md`

## Story as State + Events

At the foundation:

- an event stream records what happened in the realm,
- story variables record persistent narrative state.

Story content reacts to events by:

- checking triggers,
- executing actions,
- updating variables,
- spawning/transforming objects and portals,
- starting dialogues and quests.

This design supports “living worlds”: story logic can keep running continuously in Realm Ops, not only during a scripted campaign.

## Quests

A quest is a state machine defined by:

- named states (e.g. `NotStarted`, `InProgress`, `Completed`),
- triggers (event patterns + variable predicates),
- actions (set vars, spawn NPC, unlock portal, reward item).

Quests can span scenes by binding triggers to scene ids or to NPC identities.

## Dialogue

Dialogue is authored as a graph:

- nodes contain text, speaker, and optional animation/emote cues,
- choices can be conditional on story variables or inventory,
- choices run actions (set vars, give item, start quest, open portal).

Dialogue is designed to be driven via UI for humans and via API for agents (for automated playthroughs and testing).

## NPCs

NPCs are ordinary units/buildings with additional story metadata:

- display name, role tags,
- dialogue graph id,
- quest hooks,
- default brain profile (wander/idle/schedule).

NPC identity is stable across scenes and saves, so quests can refer to an NPC even if it travels.

## Living World Features (Metaverse Feel)

To feel like a persistent world rather than a static level, realms can enable:

- **schedules**: NPCs move between places across a day/night cycle.
- **relationships**: NPCs remember the player/agent and change dialogue accordingly.
- **world events**: timed or triggered events (festivals, emergencies, migrations, weather shifts).
- **ambient goals**: NPCs pursue roles (shopkeeper opens store; guard patrols).

These features can be implemented via embedded brains + story triggers, or via resident agents.

## Multi-Scene Storylines

Portals are the primary “chapter” mechanism:

- story variables can lock/unlock portals,
- quests can require travel to specific scenes,
- scenes can be authored as “episodes” with their own NPCs and objectives.

## Authoring Workflows

Humans:
- place NPCs and portals in Build mode,
- attach dialogue and quest ids,
- test by playing.

Agents:
- create quests and dialogue via API,
- run deterministic playthroughs to validate.

In hosted realms, creators often also deploy **resident agents**:

- schedule and event agents (“it’s market day in the hub scene”),
- moderation agents (rate-limit griefing; quarantine unsafe content),
- narrator agents that extend story content over time.
