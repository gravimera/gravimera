# Agent API Contract (vNext)

_(Spec document; product intent lives in `docs/gamedesign/05_agents_and_api.md`.)_

This document defines the **final target** HTTP API contract for AI agents as first-class players and creators in Gravimera.

It is intentionally *more specific* than `docs/gamedesign/05_agents_and_api.md`: it describes common types, error semantics, event streaming, and the endpoint families needed for “living world” creation.

## Relationship to the Current Repo

Today’s repository already contains a local Automation HTTP API (`docs/automation_http_api.md`). That API is a good foundation, but the long-term product needs:

- per-agent identity and capabilities,
- stateless commands (not “global selection”),
- multi-scene authoring and portal travel,
- event streaming suitable for resident agents,
- safe bulk/blueprint operations for large-scale world creation,
- consistent error + idempotency semantics.

This document describes the **target** API; an implementation may ship it as `/v2` while keeping `/v1` (automation) for compatibility.

## Goals (Non-Negotiable)

1) Agents can **observe** the world (state + events) efficiently and deterministically.
2) Agents can **act** with semantic actions (move/fire/interact), not raw input injection.
3) Agents can **author** realms (scenes, portals, NPCs, story assets) subject to capabilities and budgets.
4) The API is safe for hosting: capability-gated, rate-limited, validated, auditable.
5) Deterministic stepping is supported where enabled, and remains admin-only by policy.

## Transport and Versioning

- Base path: `/v2/` (recommended). `/v1/` may continue to exist as “Automation API”.
- All endpoints use JSON request/response bodies unless explicitly stated (event streaming).
- Servers must expose their supported API versions via discovery.

### Discovery Endpoints

- `GET /v2/health`  
  Returns server name, version, realm id (if single-realm server), and enabled features.

- `GET /v2/me`  
  Returns the caller’s principal identity and capability set.

- `GET /v2/limits`  
  Returns budgets and rate limits relevant to this caller and realm (object limits, brains, portals, events/sec).

## Authentication and Identity

Agents authenticate with `Authorization: Bearer <token>`.

The server maps tokens to a **principal**:

- `principal_id`: stable id string (or UUID)
- `display_name`: optional
- `capabilities`: explicit capability set
- `realm_scope`: which realm(s) the token can access (host policy)

Tokens can be:

- local-dev tokens stored in a config file (single-player),
- host-issued tokens in remote hosting (universe).

The API contract does not require a particular identity provider; it only requires stable principal ids and capabilities.

## Capabilities (Examples)

Capabilities are strings; hosts can extend them, but the core set includes:

Play:

- `play.observe` (read state/events)
- `play.act` (issue actions for owned objects)

Authoring:

- `author.scenes` (create/edit scenes; create portals)
- `author.objects` (spawn/destroy/edit object instances)
- `author.prefabs` (create/edit prefabs; upload asset packs, if supported)
- `author.brains` (attach/configure brains)
- `author.story` (create/edit quests/dialogue/triggers; set story vars)

Admin:

- `admin.time` (pause/resume/step deterministic time)
- `admin.save` (force save/load)
- `admin.server` (shutdown; realm resets)

All endpoints must return a structured “capability missing” error when required capabilities are absent.

## Common Types

### IDs

- `realm_id`: string or UUID; stable across saves and hosting.
- `scene_id`: string or UUID; stable within a realm.
- `prefab_id_uuid`: UUID string; stable object definition id.
- `instance_id_uuid`: UUID string; stable object instance id.
- `request_id`: client-provided string for idempotency/auditing.

### Vector and Transform

- `Vec3`: `{ "x": <f32>, "y": <f32>, "z": <f32> }`
- `Yaw`: radians or degrees must be declared by the API; the contract standardizes on **radians** for all angles.
- `Transform`:

  - `pos`: `Vec3`
  - `yaw`: `f32` (radians)
  - `scale`: `Vec3` (optional; defaults to 1,1,1)

### Time

- `tick`: `u64` monotonic simulation tick counter.
- `sim_time_secs`: `f64` simulation time.
- `paused`: bool.
- `dt_secs`: fixed step size in deterministic stepping mode.

### “Ok/Error” Envelope

All JSON responses use:

- success: `{ "ok": true, ... }`
- error: `{ "ok": false, "error": { ... } }`

Error object fields:

- `code`: stable machine-readable string (`capability_missing`, `not_found`, `conflict`, `validation_failed`, `budget_exceeded`, `rate_limited`, `module_disabled`)
- `message`: human-readable message
- `details`: optional object with structured data
- `request_id`: echoed back if provided

## Idempotency and Auditing

Authoring and high-impact endpoints must support idempotency.

### Request Id

Clients should send:

- `X-Request-Id: <string>` header, or
- include `"request_id": "<string>"` in the body (server must echo it).

For idempotent endpoints, the server must treat repeated identical requests with the same request id as “already applied” and return the same resulting ids where possible.

### Audit Events

Any authoring action must emit an audit event that includes:

- `by_principal_id`
- `request_id`
- affected ids (scene/object/prefab)

This is essential for moderation and debugging living worlds.

## Consistency Model: Snapshots + Events

Agents need a reliable way to:

- read a state snapshot,
- then continue from the event stream without missing or duplicating changes.

### Snapshot Contract

Every snapshot response must include:

- `tick`
- `event_id` (the latest event id included in the snapshot)

This lets clients resume events from `event_id + 1`.

### Events Contract

Events are append-only per realm:

- `event_id`: `u64` monotonic
- `tick`: `u64`
- `sim_time_secs`: `f64`
- `scene_id`: optional (some events are realm-global)
- `kind`: string
- `by_principal_id`: optional (system events may omit)
- `payload`: object

Events must be totally ordered within a realm. If the server is sharded, it must still present a single realm-wide ordering.

### Event Retrieval

Two supported patterns:

1) **Long poll** (works everywhere):  
   `GET /v2/events?after=<event_id>&timeout_secs=<n>&limit=<n>&kinds=<...>&scene_id=<...>`

2) **Server-Sent Events** (recommended for “resident agents”):  
   `GET /v2/events/stream?after=<event_id>&kinds=<...>&scene_id=<...>`  
   Response content-type: `text/event-stream`

Hosts may disable SSE; long poll must remain available if `play.observe` is granted.

## Endpoint Families

This section lists the minimum endpoint families required for “AI freedom creation” and living worlds.

### Realms and Universes (Optional)

If hosting a universe:

- `GET /v2/universe/realms` (browse/search)
- `POST /v2/universe/realms` (create realm; capability-gated)
- `GET /v2/realms/{realm_id}/...` (realm-scoped APIs)

If single-realm server, realm_id can be implicit and endpoints can omit the prefix.

### Scenes

Requires `author.scenes` for mutations, `play.observe` for reads.

- `GET /v2/scenes` -> list scenes (id, label, metadata, portal counts)
- `GET /v2/scenes/{scene_id}` -> scene metadata
- `POST /v2/scenes` -> create scene (optionally from template)
- `PATCH /v2/scenes/{scene_id}` -> edit metadata (biome, lighting preset, budgets)
- `POST /v2/scenes/{scene_id}:load` -> load scene (hosting policy; often admin-only)

### Portals

Requires `author.scenes`.

- `GET /v2/scenes/{scene_id}/portals`
- `POST /v2/scenes/{scene_id}/portals` -> create portal gate instance with destination `(scene_id)` or `(realm_id, scene_id)` if enabled
- `PATCH /v2/portals/{portal_id}` -> edit destination, lock conditions, travel rule
- `DELETE /v2/portals/{portal_id}` -> remove portal

Portals are **one-way** by default. For bidirectional travel, create two portals (A→B and B→A).

Portal entry emits `portal_entered` events; portal travel emits `scene_loaded` and `travel_completed` events.

### Objects (Instances)

Reads require `play.observe`; mutations require `author.objects` or `play.act` depending on operation.

- `GET /v2/objects?scene_id=...&kind=...&owner=...&bbox=...&limit=...`
- `GET /v2/objects/{instance_id}`
- `POST /v2/objects:spawn` (author) -> spawn one or many instances, optionally snapped to grid/markers
- `PATCH /v2/objects/{instance_id}` (author) -> edit transform/overrides/tags
- `POST /v2/objects:destroy` (author) -> destroy a list of instances

Ownership-sensitive actions (play):

- `POST /v2/units:move`
- `POST /v2/units:stop`
- `POST /v2/units:interact`
- `POST /v2/units:fire` (only if combat module enabled)

The contract prefers **stateless commands**: all actions identify the target units explicitly, not via global selection.

### Queries (Keeping Agent Bandwidth Reasonable)

Requires `play.observe`.

- `POST /v2/query:raycast` -> returns hit point and instance id if any
- `POST /v2/query:nearby` -> objects within radius and optional filters
- `POST /v2/query:navigation` -> optional: path query or reachability (hosting policy; may be expensive)

### Prefabs (Object Definitions)

Reads require `play.observe`; authoring requires `author.prefabs`.

- `GET /v2/prefabs` -> list prefabs available in the realm
- `GET /v2/prefabs/{prefab_id}` -> prefab definition (parts, anchors, profiles)
- `POST /v2/prefabs` -> create prefab (composition-based) or “register prefab from job result”
- `PATCH /v2/prefabs/{prefab_id}` -> edit safe fields (label, interaction flags, mobility) subject to validation

Hosts may forbid arbitrary prefab editing in public realms. The contract still defines it so local/offline creation can be fully programmable.

### Brains (Embedded Autonomy)

Reads require `play.observe`; mutations require `author.brains` (or realm policy).

- `GET /v2/brains/types` -> lists built-in brain kinds and their schemas
- `POST /v2/brains:attach` -> attach a brain to a unit (kind + config)
- `PATCH /v2/brains/{brain_id}` -> edit config
- `DELETE /v2/brains/{brain_id}` -> detach brain
- `GET /v2/brains/{brain_id}/state` -> current node/blackboard (debuggability is key for AI-authored worlds)

Brains must emit events on failure (`brain_error`) and must respect budgets.

### Story (Variables, Quests, Dialogue)

Reads require `play.observe`; mutations require `author.story`.

- `GET /v2/story/vars`
- `PATCH /v2/story/vars` -> set/delete multiple keys atomically
- `GET /v2/story/quests`
- `PUT /v2/story/quests/{quest_id}` -> create/update quest asset
- `GET /v2/story/dialogue`
- `PUT /v2/story/dialogue/{dialogue_id}` -> create/update dialogue asset

Story triggers should be inspectable and testable:

- `POST /v2/story:test` -> runs triggers against a provided simulated event sequence (admin/author only)

### Bulk Operations and Blueprints (Required for Agent Creation at Scale)

Agents need high-level operations to avoid thousands of API calls.

Requires `author.objects` and often `author.scenes` / `author.story` depending on content.

- `POST /v2/blueprints:validate`  
  Returns a budget estimate and detailed validation errors without mutating state.

- `POST /v2/blueprints:apply`  
  Applies a blueprint that may include:
  - scene creation,
  - portal graph updates,
  - prefab registrations,
  - object spawns/edits,
  - brain attachments,
  - story assets and variable initialization.

Blueprint apply must be either:

- atomic (preferred), or
- transactional with a clear partial-apply report and a server-generated rollback token.

### Jobs (Asynchronous Work)

Long-running operations (Gen3D builds, large imports, heavy validations) must be asynchronous.

Requires capability based on the job kind.

- `POST /v2/jobs` -> start job (`kind`, parameters)
- `GET /v2/jobs/{job_id}` -> status and results
- `POST /v2/jobs/{job_id}:cancel`

Jobs emit events (`job_started`, `job_progress`, `job_completed`, `job_failed`).

### Time Control (Admin)

Requires `admin.time`. Many hosted realms will disable it.

- `POST /v2/time:pause`
- `POST /v2/time:resume`
- `POST /v2/time:step` body: `{ frames, dt_secs }` (or `{ ticks, dt_secs }`)
- `GET /v2/time` -> `{ paused, tick, sim_time_secs, dt_secs }`

## Conflict Resolution and Concurrency

Authoring conflicts must be explicit:

- objects and story vars have a `revision` number,
- mutation endpoints accept `expected_revision` (optional),
- server returns `409 conflict` with current revision when mismatched.

This allows multiple agents (creator + resident + moderators) to safely edit in the same realm.

## “Module Disabled” Semantics

If a realm has disabled a module (combat, economy), any endpoint that depends on it must return:

- HTTP `409` or `400` (host choice), with:
  - `error.code = "module_disabled"`
  - `error.details.module = "combat"` (for example)

## Minimum Acceptance for “Metaverse-Like” Worlds

A hosted realm can be considered “living world capable” when:

- resident agents can stream events continuously (SSE or long-poll),
- story variables and quests can be edited safely and audited,
- NPC brains can run embedded with budgets and are debuggable,
- blueprint apply can build large scenes without thousands of calls,
- budgets and rate limits are visible to agents.
