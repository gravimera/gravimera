# SKILL: Using Gravimera as an external “agent monitor” (HTTP Automation API)

This skill describes a generic pattern for any external tool/agent to use a **local Gravimera process** as a human-visible monitor: persistent scenes, spawned “agent units”, toast popups, and built-in TTS speech.

The API surface is the **Automation HTTP API (v1)** documented at `docs/automation_http_api.md`.

## 0) Start Gravimera with Automation enabled

Minimal CLI (local-only):

    cargo run -- \
      --automation \
      --automation-bind 127.0.0.1:8791 \
      --automation-disable-local-input \
      --automation-pause-on-start

Or use a config file with:

    [automation]
    enabled = true
    bind = "127.0.0.1:8791"
    disable_local_input = true
    pause_on_start = true

## 1) Discover APIs (don’t hardcode)

Use discovery to confirm the server is up and to get a “starter index” of endpoints:

    curl -s http://127.0.0.1:8791/v1/discovery

Also use:

    curl -s http://127.0.0.1:8791/v1/health

## 2) Create a dedicated scene for your run (reviewable later)

Create a scene scaffold (id must match `[A-Za-z0-9._-]`):

    curl -s -X POST http://127.0.0.1:8791/v1/realm_scene/create \
      -H 'Content-Type: application/json' \
      -d '{"scene_id":"OpenClaw","label":"OpenClaw","description":"External agent monitor scene","switch_to":true}'

Scene switching is deferred; step a few frames after scheduling:

    curl -s -X POST http://127.0.0.1:8791/v1/step -H 'Content-Type: application/json' -d '{"frames":3}'

At any time, check which scene is active:

    curl -s http://127.0.0.1:8791/v1/realm_scene/active

## 3) Spawn “agent units” and props (visualize what you’re doing)

List prefabs to find something by label (builtins + saved):

    curl -s http://127.0.0.1:8791/v1/prefabs

Spawn a prefab instance:

    curl -s -X POST http://127.0.0.1:8791/v1/spawn \
      -H 'Content-Type: application/json' \
      -d '{"prefab_id_uuid":"<from /v1/prefabs>","x":2.0,"z":2.0,"yaw":0.0}'

Clean up a prop/unit:

    curl -s -X POST http://127.0.0.1:8791/v1/despawn \
      -H 'Content-Type: application/json' \
      -d '{"instance_id_uuid":"<from /v1/state>"}'

Replication is just spawning the same prefab multiple times (or copying a spawned unit by reusing its `prefab_id_uuid`).

## 4) Show “what I’m doing” (toast popups + voice + bubble)

Popup toast (rendered mode only):

    curl -s -X POST http://127.0.0.1:8791/v1/ui/toast \
      -H 'Content-Type: application/json' \
      -d '{"text":"Searching… 🔍","kind":"info","ttl_secs":3.5}'

Speak text via built-in TTS (async). If you pass `instance_id_uuid` and `bubble=true`, a speech bubble appears above that object:

    curl -s -X POST http://127.0.0.1:8791/v1/speak \
      -H 'Content-Type: application/json' \
      -d '{"content":"Collecting materials.","voice":"dog","volume":1.0,"instance_id_uuid":"<unit id>","bubble":true}'

Notes:

- `voice` is one of `dog|cow|dragon`.
- `bubble=true` requires rendered mode and `instance_id_uuid`.

## 5) (Optional) Generate a custom avatar via Gen3D, then spawn it

Gen3D runs in Build Preview:

    curl -s -X POST http://127.0.0.1:8791/v1/mode -H 'Content-Type: application/json' -d '{"mode":"gen3d"}'
    curl -s -X POST http://127.0.0.1:8791/v1/step -H 'Content-Type: application/json' -d '{"frames":3}'

Prompt + build, stepping frames while polling status:

    curl -s -X POST http://127.0.0.1:8791/v1/gen3d/prompt -H 'Content-Type: application/json' -d '{"prompt":"A small cute robotic claw assistant mascot, stylized"}'
    curl -s -X POST http://127.0.0.1:8791/v1/gen3d/build -H 'Content-Type: application/json' -d '{}'

When `GET /v1/gen3d/status` reports `draft_ready=true`, save:

    curl -s -X POST http://127.0.0.1:8791/v1/gen3d/save -H 'Content-Type: application/json' -d '{}'

Use the returned `prefab_id_uuid` to spawn the avatar back in your monitor scene (switch back to `mode=build`, then `spawn`).

## 6) Persist the scene so it’s reviewable later

Force-save `scene.dat`:

    curl -s -X POST http://127.0.0.1:8791/v1/scene/save -H 'Content-Type: application/json' -d '{}'

The scene lives under `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/` and can be revisited by switching back to it later.

## Shutdown (clean exit)

    curl -s -X POST http://127.0.0.1:8791/v1/shutdown -H 'Content-Type: application/json' -d '{}'

