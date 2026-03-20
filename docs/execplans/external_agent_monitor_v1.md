# External agent monitor via Automation HTTP API (scene + units + UI toast + TTS)

This ExecPlan is a living document. The sections `Progress`, `Surprises & Discoveries`, `Decision Log`, and `Outcomes & Retrospective` must be kept up to date as work proceeds.

This repository includes `PLANS.md` at the repo root. This document must be maintained in accordance with `PLANS.md`.


## Purpose / Big Picture

Gravimera already has a local “Automation HTTP API” (`docs/automation_http_api.md`) that can drive gameplay and authoring. What it lacks is a small set of **monitoring-oriented** primitives that let an external tool/agent use Gravimera as a “live status monitor”:

- Create/switch to a dedicated scene for a run (so the result is reviewable later).
- Spawn/replicate “agent units” and simple props to visualize work-in-progress.
- Show **popup message boxes** (non-modal toasts) and speak text via **built-in TTS** (with a speech bubble).
- Discover APIs and find prefabs by listing, without hardcoding UUIDs.

After this change, an external tool can:

1) Start a local Gravimera process with `--automation`.
2) Create (or reuse) a scene like `OpenClaw` and switch to it.
3) (Optionally) use Gen3D (`/v1/gen3d/*`) to generate an avatar prefab, then spawn multiple units of that prefab.
4) While working, repeatedly post toast notifications and TTS speech tied to a unit instance id.
5) Spawn/despawn props as needed, and force-save the scene so it’s reviewable later.

How to see it working (after implementation):

- Run the new real test script in `test/monitor_real_test.py` (it starts the game rendered, exercises new endpoints, then shuts down cleanly).
- Manually: start the game with `--automation`, then use `curl` to call `/v1/realm_scene/create`, `/v1/realm_scene/switch`, `/v1/prefabs`, `/v1/spawn`, `/v1/ui/toast`, `/v1/speak`, `/v1/scene/save`.


## Progress

- [x] (2026-03-21 20:55 CST) Drafted this ExecPlan.
- [x] (2026-03-21) Add monitor endpoints to the Automation HTTP API (discovery, scene create/switch/list, prefab list, despawn, force-save).
- [x] (2026-03-21) Add rendered UI “toast” popup system + HTTP endpoint to trigger it.
- [x] (2026-03-21) Add HTTP endpoint for built-in TTS speech + speech-bubble rendering, with non-blocking worker thread and completion cleanup.
- [x] (2026-03-21) Update docs (`docs/automation_http_api.md`) and add `docs/agent_skills/SKILL_agent.md`.
- [x] (2026-03-21) Add a real rendered API smoke script under `test/` (no secrets).
- [x] (2026-03-21) Run rendered smoke tests (`python3 test/monitor_real_test.py` and `cargo run -- --rendered-seconds 2`).
- [x] (2026-03-21) Commit changes.


## Surprises & Discoveries

- Observation: Gravimera already has built-in TTS plumbing via `soundtest` (Meta Speak), including a trigger-agnostic speech bubble command channel (`ModelSpeechBubbleCommand`) with a `Network` source.
  Evidence: `docs/meta_speak.md`, `src/meta_speak.rs`, `src/types.rs` (`ModelSpeechBubbleCommand`, `ModelSpeechSource::Network`), `src/ui.rs` bubble renderer.

- Observation: Scene switching already exists as a deferred “pending switch” (`PendingRealmSceneSwitch`) applied in `scene_store::apply_pending_realm_scene_switch`, which also scaffolds directories and persists the active selection.
  Evidence: `src/realm.rs`, `src/scene_store.rs::apply_pending_realm_scene_switch`.


## Decision Log

- Decision: Implement this as additions to the existing `/v1` Automation HTTP API rather than shipping a full `/v2` Agent API.
  Rationale: The monitor feature must work now for local tooling. `/v2` is a larger contract with identity/capabilities/events; we can layer that later while keeping `/v1` useful.
  Date/Author: 2026-03-21 / assistant + user

- Decision: Use the existing Meta Speak backend (`MetaSpeakRuntime` + `soundtest`) for built-in TTS and reuse `ModelSpeechBubbleCommand` for on-screen speech bubbles.
  Rationale: This avoids duplicating audio backend selection and keeps the “speech bubble” UI stable across triggers (Meta UI vs HTTP).
  Date/Author: 2026-03-21 / assistant + user

- Decision: Implement “popup message box” as a non-modal toast overlay (stacked, timed) rather than a blocking modal dialog.
  Rationale: Modal OS dialogs are brittle for automation and can block the render loop. Toasts satisfy “human-visible status” and are easy to spam safely with TTL + max-count.
  Date/Author: 2026-03-21 / assistant


## Outcomes & Retrospective

- Implemented an “external agent monitor” layer on the existing Automation HTTP API:
  - Discovery + listing: `GET /v1/discovery`, `GET /v1/prefabs`, `GET /v1/realm_scene/active`, `GET /v1/realm_scene/list`
  - Scene management: `POST /v1/realm_scene/create`, `POST /v1/realm_scene/switch` (deferred)
  - Monitoring UI: `POST /v1/ui/toast` (rendered-only)
  - Built-in voice: `POST /v1/speak` (async TTS; optional speech bubble over an instance)
  - Persistence + cleanup: `POST /v1/scene/save`, `POST /v1/despawn`
- Added a small toast overlay UI (stacked, TTL + fade) and a speak runtime that reaps async TTS jobs and stops bubbles when they finish.
- Updated `docs/automation_http_api.md` and added `docs/agent_skills/SKILL_agent.md` with a concise “monitor loop”.
- Added a rendered real test driver (`test/monitor_real_test.py`) and minimal config (`test/monitor_test_config.toml`) to exercise the new endpoints end-to-end.


## Context and Orientation

Key files/modules:

- Automation HTTP server and routes: `src/automation/mod.rs`
- Scene/realm directories + scaffolding: `src/paths.rs`, `src/realm.rs`
- Scene switching: `src/scene_store.rs::apply_pending_realm_scene_switch`
- Built-in TTS adapter: `src/meta_speak.rs` (uses vendored `third_party/soundtest`)
- Speech bubble commands + rendering: `src/types.rs` (`ModelSpeechBubbleCommand`), `src/ui.rs`
- Rendered UI spawn: `src/setup.rs` (creates UI roots), `src/app_plugins.rs` (wires UI systems)
- Automation API docs: `docs/automation_http_api.md`

Important repo rule:

- After any code change, run the rendered smoke test (no headless): `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`
- After validation, commit with a clear message.


## Plan of Work

1) Add **API discovery** and “monitor primitives” endpoints to the Automation API:

   - `GET /v1/discovery`: a machine-readable list of routes/features and the active realm/scene.
   - `GET /v1/prefabs`: list known prefabs (id + label + mobility + optional descriptor metadata).
   - `GET /v1/realm_scene/active`: return active realm/scene ids and their on-disk directories.
   - `GET /v1/realm_scene/list`: list realms and scenes on disk.
   - `POST /v1/realm_scene/create`: create a new scene scaffold (optionally set label/description; optionally switch to it).
   - `POST /v1/realm_scene/switch`: schedule a realm/scene switch (deferred; applied after a few stepped frames).
   - `POST /v1/scene/save`: force a scene.dat save via `SceneSaveRequest`.
   - `POST /v1/despawn`: despawn an instance by `instance_id_uuid` (for cleanup of props/units).

2) Add a generic **UI toast** popup system:

   - Define `UiToastCommand` message type in `src/types.rs`.
   - Implement `apply_ui_toast_commands` + `update_ui_toasts` in `src/ui.rs` (rich text + emoji, TTL fade, max stack).
   - Add `POST /v1/ui/toast` that enqueues a toast command and (optionally) also writes to `ActionLogState`.

3) Add a **TTS speak** endpoint that is non-blocking and renders a speech bubble:

   - `POST /v1/speak` accepts `{ content, voice, volume, instance_id_uuid?, bubble? }`.
   - It spawns a worker thread calling `MetaSpeakRuntime.adapter().speak(...)`.
   - It emits `ModelSpeechBubbleCommand::Start` immediately (when `bubble=true` and `instance_id_uuid` resolves), and later emits `Stop` when the job completes (success or failure).
   - Add a small `AutomationSpeakRuntime` resource + a polling system to reap completed jobs and stop bubbles.

4) Update docs and add a short agent guide:

   - Update `docs/automation_http_api.md` with new endpoints (inputs/outputs/errors and notes about deferred scene switch and toast/speech availability).
   - Add `docs/agent_skills/SKILL_agent.md` describing the “monitor loop” and how an agent discovers APIs and prefabs.

5) Add a real rendered smoke script under `test/`:

   - `test/monitor_test_config.toml`: enables automation and pauses on start (no secrets).
   - `test/monitor_real_test.py`: starts the game with that config, exercises the new endpoints, and calls `/v1/shutdown`.

6) Validate:

   - Run `python3 test/monitor_real_test.py` and confirm it exits successfully.
   - Run the required rendered smoke test (`--rendered-seconds 2`).


## Concrete Steps

All commands run from repo root.

1) Implement endpoints + UI:

   - Edit `src/automation/mod.rs`, `src/types.rs`, `src/ui.rs` (and any small glue files).
   - Update `docs/automation_http_api.md`.
   - Add `docs/agent_skills/SKILL_agent.md`.
   - Add `test/monitor_real_test.py` and `test/monitor_test_config.toml`.

2) Validate:

   - `python3 test/monitor_real_test.py`
   - `tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2`


## Validation and Acceptance

Acceptance is met when:

- `GET /v1/discovery` returns `ok=true` and includes the new monitor routes.
- A fresh run can:
  - `POST /v1/realm_scene/create {"scene_id":"OpenClaw"}` (idempotent if it exists),
  - `POST /v1/realm_scene/switch {"scene_id":"OpenClaw"}` then step a few frames,
  - `GET /v1/prefabs` and find a prefab by label (e.g. `Human`),
  - `POST /v1/spawn` to create units and `POST /v1/despawn` to clean up a prop,
  - `POST /v1/ui/toast` to show a popup notification (visually, in rendered mode),
  - `POST /v1/speak` to speak text via built-in TTS and show a speech bubble above a unit,
  - `POST /v1/scene/save` to force persistence,
  - `POST /v1/shutdown` to exit cleanly.
- `python3 test/monitor_real_test.py` passes.
- Rendered smoke test (`--rendered-seconds 2`) starts and exits without crash.


## Idempotence and Recovery

- `/v1/realm_scene/create` must be safe to retry; if the scene already exists, it returns `ok=true` and does not destroy existing content.
- `/v1/realm_scene/switch` is safe to retry; switching to the already-active realm/scene is a no-op.
- `/v1/speak` and `/v1/ui/toast` are non-idempotent by nature; external agents should include their own coalescing if needed.


## Artifacts and Notes

- “Speech bubble UI” is already designed to be trigger-agnostic; HTTP speech should use `ModelSpeechSource::Network`.
- When UI is unavailable (headless mode), toast and speech bubble endpoints should return `501` with an actionable message.
- Scene switching is deferred until after the next `PostUpdate`; automation drivers should step a few frames after calling switch.
