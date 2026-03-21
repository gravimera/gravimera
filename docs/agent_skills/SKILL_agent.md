# SKILL: Using Gravimera as an external “agent monitor” (HTTP Automation API)

This skill describes a generic pattern for any external tool/agent to use a **local Gravimera process** as a human-visible monitor: persistent scenes, spawned “agent units”, toast popups, and built-in TTS speech.

The API surface is the **Automation HTTP API (v1)** documented at `docs/automation_http_api.md`.

## 0) Onboard (plugin-style)

This flow treats Gravimera as a downloadable local “monitor plugin” you start per run.

### 0.1 Download the `gravimera` binary (example: v0.1.0)

```bash
mkdir -p ./gravimera_monitor/bin
curl -L -o ./gravimera_monitor/bin/gravimera \
  https://github.com/gravimera/gravimera/releases/download/0.1.0/gravimera
chmod +x ./gravimera_monitor/bin/gravimera
```

### 0.2 Create an isolated config + home (do NOT use `~/.gravimera`)

Pick a monitor root directory (keep it stable if you want to review scenes later):

```bash
MONITOR_ROOT=./gravimera_monitor/run
mkdir -p "$MONITOR_ROOT"
```

Create `"$MONITOR_ROOT/config.toml"`:

```toml
[automation]
enabled = true
bind = "127.0.0.1:0" # use ":0" for an ephemeral port, or a fixed port like "127.0.0.1:8791"
monitor_mode = true  # local UI is read-only (camera + browsing allowed; mutations via API)
# token = "CHANGE_ME" # optional; enables Authorization: Bearer <token>

# Optional: enable Gen3D later (requires an AI provider config).
# [gen3d]
# ai_service = "openai"  # or "gemini"
#
# If the external agent already uses GPT (gpt-5.2 / gpt-5.4+), reuse the same base_url + token here:
# [openai]
# base_url = "https://api.openai.com/v1" # or your OpenAI-compatible gateway
# token = ""                              # or set env OPENAI_API_KEY
# model = "gpt-5.4"
#
# If the external agent already uses Gemini (gemini 3.1+), reuse the same token here:
# [gemini]
# token = "" # or set env GEMINI_API_KEY / X_GOOG_API_KEY
# model = "gemini-3.1-pro-preview"
```

Config discovery tips (for agents):

- Default config path: `~/.gravimera/config.toml` (unless overridden by `--config` or env `GRAVIMERA_CONFIG`).
- In this plugin-style flow: config is `"$MONITOR_ROOT/config.toml"` and all persistent state is under `"$MONITOR_ROOT/home"` via `GRAVIMERA_HOME`.

Credential policy:

- If you already have GPT (gpt-5.2 / gpt-5.4+) credentials in the external agent, reuse them in `[openai]`.
- Else if you already have Gemini 3.1+ credentials, reuse them in `[gemini]`.
- Otherwise: run monitor-only (skip Gen3D endpoints) and ask the user for a key only when you need Gen3D.

### 0.3 Start Gravimera using that config

Run with an isolated `GRAVIMERA_HOME` so all state persists under the monitor root:

```bash
GRAVIMERA_HOME="$MONITOR_ROOT/home" \
  ./gravimera_monitor/bin/gravimera --config "$MONITOR_ROOT/config.toml"
```

If `bind = "127.0.0.1:0"`, parse stdout for:

    Automation API listening on http://127.0.0.1:<port>

Important: Gravimera **does not hot-reload** `config.toml`. If you change the config (ports, token, AI provider, etc.),
you must **stop and restart** the Gravimera process for changes to take effect.

Tip: if your HTTP client honors `HTTP(S)_PROXY`, ensure loopback is not proxied (set `NO_PROXY=127.0.0.1,localhost`).

### (Dev) Run from source instead (optional)

```bash
cargo run -- \
  --automation \
  --automation-bind 127.0.0.1:8791 \
  --automation-monitor-mode
```

## 1) Discover APIs (don’t hardcode)

Use discovery to confirm the server is up and to get a “starter index” of endpoints:

```bash
BASE_URL=http://127.0.0.1:8791 # or parse from "Automation API listening on ..."
curl -s "$BASE_URL/v1/discovery"
```

Also use:

```bash
curl -s "$BASE_URL/v1/health"
```

## 2) Create a dedicated scene for your run (reviewable later)

Create a scene scaffold (id must match `[A-Za-z0-9._-]`):

```bash
curl -s -X POST "$BASE_URL/v1/realm_scene/create" \
  -H 'Content-Type: application/json' \
  -d '{"scene_id":"AgentMonitor","label":"AgentMonitor","description":"External agent monitor scene","switch_to":true}'
```

Scene switching is deferred; step a few frames after scheduling:

```bash
curl -s -X POST "$BASE_URL/v1/step" -H 'Content-Type: application/json' -d '{"frames":3}'
```

At any time, check which scene is active:

```bash
curl -s "$BASE_URL/v1/realm_scene/active"
```

## 3) Spawn “agent units” and props (visualize what you’re doing)

List prefabs to find something by label (builtins + saved):

```bash
curl -s "$BASE_URL/v1/prefabs"
```

Spawn a prefab instance:

```bash
curl -s -X POST "$BASE_URL/v1/spawn" \
  -H 'Content-Type: application/json' \
  -d '{"prefab_id_uuid":"<from /v1/prefabs>","x":2.0,"z":2.0,"yaw":0.0}'
```

Clean up a prop/unit:

```bash
curl -s -X POST "$BASE_URL/v1/despawn" \
  -H 'Content-Type: application/json' \
  -d '{"instance_id_uuid":"<from /v1/state>"}'
```

Replication is just spawning the same prefab multiple times (or copying a spawned unit by reusing its `prefab_id_uuid`).

## 4) Show “what I’m doing” (toast popups + voice + bubble)

Popup toast (rendered mode only):

```bash
curl -s -X POST "$BASE_URL/v1/ui/toast" \
  -H 'Content-Type: application/json' \
  -d '{"text":"Searching… 🔍","kind":"info","ttl_secs":3.5}'
```

Speak text via built-in TTS (async). If you pass `instance_id_uuid` and `bubble=true`, a speech bubble appears above that object:

```bash
curl -s -X POST "$BASE_URL/v1/speak" \
  -H 'Content-Type: application/json' \
  -d '{"content":"Collecting materials.","voice":"dog","volume":1.0,"instance_id_uuid":"<unit id>","bubble":true}'
```

Notes:

- `voice` is one of `dog|cow|dragon`.
- `bubble=true` requires rendered mode and `instance_id_uuid`.

## 5) (Optional) Generate a custom avatar via Gen3D, then spawn it

Gen3D runs in Build Preview:

```bash
curl -s -X POST "$BASE_URL/v1/mode" -H 'Content-Type: application/json' -d '{"mode":"gen3d"}'
curl -s -X POST "$BASE_URL/v1/step" -H 'Content-Type: application/json' -d '{"frames":3}'
```

Prompt + build, stepping frames while polling status:

```bash
curl -s -X POST "$BASE_URL/v1/gen3d/prompt" -H 'Content-Type: application/json' -d '{"prompt":"A small cute assistant robot mascot, stylized"}'
curl -s -X POST "$BASE_URL/v1/gen3d/build" -H 'Content-Type: application/json' -d '{}'
```

When `GET /v1/gen3d/status` reports `draft_ready=true`, save:

```bash
curl -s -X POST "$BASE_URL/v1/gen3d/save" -H 'Content-Type: application/json' -d '{}'
```

Use the returned `prefab_id_uuid` to spawn the avatar back in your monitor scene (switch back to `mode=build`, then `spawn`).

## 6) Persist the scene so it’s reviewable later

Force-save `scene.dat`:

```bash
curl -s -X POST "$BASE_URL/v1/scene/save" -H 'Content-Type: application/json' -d '{}'
```

The scene lives under `"$GRAVIMERA_HOME/realm/<realm_id>/scenes/<scene_id>/"` and can be revisited by switching back to it later.

## Shutdown (clean exit)

```bash
curl -s -X POST "$BASE_URL/v1/shutdown" -H 'Content-Type: application/json' -d '{}'
```
