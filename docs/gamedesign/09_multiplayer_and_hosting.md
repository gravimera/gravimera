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

## Replay and Auditing

Hosted realms should support:

- exporting an event log for debugging and training,
- replaying from a save + event log in deterministic mode,
- auditing which principal performed which authoring actions.

