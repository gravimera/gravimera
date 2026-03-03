# Intelligence Service (Standalone Brains)

Gravimera can optionally run unit “brains” in a **separate process** (a local or remote *intelligence service*). The host simulation stays authoritative: the service only *requests* actions.

Design/spec reference: `docs/gamedesign/38_intelligence_service_spec.md`.

## Start the local service

In one terminal:

```bash
cargo run --bin gravimera_intelligence_service
```

Options:

- Bind address: `--bind 127.0.0.1:8792`
- Require auth token: `--token <token>` (expects `Authorization: Bearer <token>`)

Example:

```bash
cargo run --bin gravimera_intelligence_service -- --bind 127.0.0.1:8792 --token secret
```

## Configure the game (host)

Copy the template config (or edit your existing config):

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

Enable the service in `config.toml`:

```toml
[intelligence_service]
enabled = true
addr = "127.0.0.1:8792"
# token = "secret" # if the service was started with --token

# Dev-only: spawn a demo commandable unit with a demo standalone brain.
# debug_spawn_unit = true
```

Config location:

- Default: `~/.gravimera/config.toml`
- Override base dir: `GRAVIMERA_HOME=/path/to/dir` (config becomes `$GRAVIMERA_HOME/config.toml`)
- Override config path: `GRAVIMERA_CONFIG=/path/to/config.toml`

## Run + verify

In another terminal:

```bash
cargo run
```

Quick checks (the protocol is HTTP/JSON, not gRPC):

- `GET /v1/health`
- `GET /v1/modules` (list “brain modules” the service reports)

Example:

```bash
curl -s http://127.0.0.1:8792/v1/health
curl -s http://127.0.0.1:8792/v1/modules
```

## Demo brain modules (built-in)

The service currently ships a few **demo** modules for development and testing:

- `demo.orbit.v1`: circles around a center point.
  - config: `{ "center": [x,z], "radius": f32, "rads_per_tick": f32 }`
- `demo.coward.v1`: wanders/rests/“looks around”, but flees from nearby units of different `kind`.
  - on taking damage (health drop), it tries to hide behind nearby buildings and remembers the attacker as “dangerous” for ~60 seconds.
- `demo.opportunist.v1`: mostly rests (≈3/4) and sometimes wanders (≈1/4).
  - it may attack nearby moving units of different `kind`, and may fight back when attacked.
  - it only engages if it estimates it can win and still remain above 1/4 health.
  - requires host-granted capabilities: `brain.move` and `brain.combat`.

Notes:

- `TickInput.self_state.kind` and `TickInput.nearby_entities[*].kind` are currently the unit/building prefab UUID string.
- The host includes both units and build objects in `nearby_entities` (bounded and distance-sorted).

On Windows PowerShell, use `curl.exe` instead of `curl` if you hit the `Invoke-WebRequest` alias:

```powershell
curl.exe -s http://127.0.0.1:8792/v1/health
```

## Use from the UI (Meta panel)

- Double-click a unit’s selection circle to open the **Meta** panel.
- In the **Brain** section:
  - `Fallback (default)` detaches the standalone brain.
  - Remote brains are fetched asynchronously from the service after the panel opens (via `GET /v1/modules`).

## Tests

`cargo test` includes an end-to-end smoke test that spawns `gravimera_intelligence_service` and exercises the `/v1/*` API.

## Troubleshooting

### Windows: build/test fails with “Access is denied” removing `gravimera_intelligence_service.exe`

This usually means the service process is still running and has the `.exe` open. Stop it and retry:

```powershell
Get-Process gravimera_intelligence_service -ErrorAction SilentlyContinue | Stop-Process -Force
```
