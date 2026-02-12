# Multiplayer and Hosting

Gravimera supports multiple “hosting styles” that all share the same simulation rules.

## Hosting Styles

### Local Single-Player (Default)

- one process runs simulation + UI,
- agent API is local-only by default,
- good for creators iterating quickly.

### Local Dedicated Server (Headless)

- one process runs simulation headlessly,
- humans connect via a UI client (future) and agents connect via HTTP,
- deterministic stepping is typically enabled for tests/training.

### Remote Hosted Realms

- a hosted server runs the simulation and exposes a secured API,
- creators publish realm packages,
- players and agents connect with scoped tokens.

### Hosted Universes (Metaverse)

A “universe” is a curated directory of multiple realms hosted together:

- realm discovery (browse/search)
- cross-realm identity (player accounts or stable ids)
- optional cross-realm travel (portal destinations can reference other realms)
- shared safety and governance policy

Universes are optional: the core game works with a single realm.

## Authority Model

The server is authoritative:

- clients propose actions,
- server validates permissions and applies actions,
- server emits events and snapshots.

This avoids divergence between human and agent clients and makes deterministic replay possible.

## Security Model (Non-Negotiable for Hosting)

- HTTPS termination (at or in front of the server),
- per-token capabilities,
- rate limiting and budgets,
- strict validation of all authoring payloads,
- optional disabling of high-risk surfaces (prefab upload, sandboxed modules).

See `docs/gamedesign/11_safety_and_governance.md` for the full safety requirements.

## Replay and Auditing

Hosted realms should support:

- exporting an event log for debugging and training,
- replaying from a save + event log in deterministic mode,
- auditing which principal performed which authoring actions.

## Resident Agents (Always-On Worlds)

Hosted realms often run resident agents:

- world event agents (daily cycles, festivals)
- narrator agents (ongoing story creation)
- moderation agents (safety enforcement; abuse response)

Resident agents must use the same capability-gated APIs as external clients, so hosts can audit and control them.
