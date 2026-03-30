# Intelligence Service (Standalone Brains)

Gravimera can run unit “brains” via an **intelligence service**. The host simulation stays authoritative: the service only *requests* actions.

Design/spec reference: `docs/gamedesign/38_intelligence_service_spec.md`.

## Run modes

When enabled, Gravimera can run the intelligence service in two ways:

- **Embedded** (default): the service runs inside the Gravimera game process.
- **Sidecar**: the service runs as a separate local/remote process (`gravimera_intelligence_service`).

## Configure the game (host)

Copy the template config (or edit your existing config):

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

Enable and configure in `config.toml`:

```toml
[intelligence_service]
enabled = true

# "embedded" (default) | "sidecar" | "disabled"
mode = "embedded"

# addr meaning depends on mode:
# - embedded: bind addr (default: "127.0.0.1:0" for a random port)
# - sidecar: service addr to connect (default: "127.0.0.1:8792")
# addr = "127.0.0.1:0"

# Optional: require `Authorization: Bearer <token>` on every request.
# token = "secret"

# Optional: host-side standalone-brain tick rate.
# Default: 6 ticks/sec.
# ticks_per_sec = 6

# Dev-only: spawn a demo commandable unit with a demo standalone brain.
# debug_spawn_unit = true
```

Config location:

- Default: `<root_dir>/config.toml` (default `<root_dir>` is `~/.gravimera/`)
- Override base dir:
  - Config: set `root_dir = "/path/to/dir"` (top-level or under `[app]` in `config.toml`)
  - Env: `GRAVIMERA_HOME=/path/to/dir` (highest precedence; default config becomes `$GRAVIMERA_HOME/config.toml`)
- Override config path: `GRAVIMERA_CONFIG=/path/to/config.toml`

## Embedded mode (default)

If `mode = "embedded"`, Gravimera starts the intelligence service at startup.

- Default bind: `127.0.0.1:0` (random free port).
- The actual listen address is logged on startup.

Quick checks (HTTP/JSON):

```bash
# Replace with the logged address.
curl -s http://127.0.0.1:<port>/v1/health
curl -s http://127.0.0.1:<port>/v1/modules
```

## Sidecar mode (standalone process)

If `mode = "sidecar"`, run the service in one terminal:

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

Then run the game in another terminal:

```bash
cargo run
```

## Demo brain modules (built-in)

The service currently ships a few **demo** modules for development and testing:

- `demo.orbit.v1`: circles around a center point.
  - config: `{ "center": [x,z], "radius": f32, "rads_per_tick": f32 }`
- `demo.coward.v1`: wanders/rests/“looks around”, but flees from nearby units of different `kind`.
  - on taking damage (health drop), it guesses the attacker as the nearest hostile unit in its snapshot.
  - if the attacker seems more powerful (based on health/max health), it panic-flees (may try to hide behind nearby buildings) and remembers that attacker as “dangerous” for ~60 seconds.
- `demo.opportunist.v1`: mostly rests (≈3/4) and sometimes wanders (≈1/4).
  - it may attack nearby moving units of different `kind`, and may fight back when attacked.
  - it only engages if it estimates it can win and still remain above 1/4 health.
  - if the attacker seems more powerful (based on health/max health), it disengages and flees.
  - ranged units (tagged `attack.ranged` by the host) prefer to attack from distance instead of closing in.
  - requires host-granted capabilities: `brain.move` and `brain.combat`.
- `demo.belligerent.v1`: aggressive brain that attacks nearby units of different `kind`.
  - when attacked, the attacker gets the most attention: it focuses that unit and chases it until it gets away (lost for a while), then resumes normal targeting.
  - ranged units (tagged `attack.ranged`) attack from distance.
  - requires host-granted capabilities: `brain.move` and `brain.combat`.

Notes:

- `TickInput.self_state.kind` and `TickInput.nearby_entities[*].kind` are currently the unit/building prefab UUID string.
- The host tags units with `attack.melee` / `attack.ranged` based on their attack definition.
- The host includes both units and build objects in `nearby_entities` (bounded and distance-sorted).

## Host defaults (when enabled)

When `intelligence_service.enabled = true` (and `mode != "disabled"`), the host automatically attaches a standalone brain to **non-player units** (entities with `Commandable` and without `Player`) when they enter **Play** mode:

- Units that can attack (have an attack definition in the object library):
  - Melee: 50% `demo.belligerent.v1`, 50% `demo.opportunist.v1`
  - Ranged projectile: `demo.opportunist.v1`
- Units that cannot attack: `demo.coward.v1`

Brains only tick/act in **Play** mode. When switching back to **Build**, the host clears brain-issued move/attack orders so units stay still for review. The host tick rate is configurable via `intelligence_service.ticks_per_sec` and defaults to 6 ticks per second (10x slower than the original 60 Hz synthetic tick).

If the intelligence service disconnects (or is temporarily unavailable), the host will automatically retry connecting. If the service restarts and forgets existing brain instance ids, the host respawns those brain instances.

You can override per-unit in the UI via the Meta panel’s Brain section.

On Windows PowerShell, use `curl.exe` instead of `curl` if you hit the `Invoke-WebRequest` alias:

```powershell
curl.exe -s http://127.0.0.1:<port>/v1/health
```

## Use from the UI (Meta panel)

- Double-click a unit’s selection circle to open the **Meta** panel.
- In the **Brain** section:
  - `Fallback (default)` detaches the standalone brain.
  - Brain modules are fetched asynchronously from the service after the panel opens (via `GET /v1/modules`).

## Tests

`cargo test` includes an end-to-end smoke test that spawns `gravimera_intelligence_service` and exercises the `/v1/*` API.

## Troubleshooting

### Windows: build/test fails with “Access is denied” removing `gravimera_intelligence_service.exe`

This usually means the service process is still running and has the `.exe` open. Stop it and retry:

```powershell
Get-Process gravimera_intelligence_service -ErrorAction SilentlyContinue | Stop-Process -Force
```
