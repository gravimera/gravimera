# Intelligence Service (Standalone Brains)

Gravimera can run unit “brains” via an **intelligence service**. The host simulation stays authoritative: the service only *requests* actions.

Design/spec reference: `docs/gamedesign/38_intelligence_service_spec.md`.

## Run modes

When enabled, Gravimera runs the intelligence service in one way:

- **Embedded** (default): the service runs inside the Gravimera game process.

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

# "embedded" (default) | "disabled"
mode = "embedded"

# Embedded bind addr (default: "127.0.0.1:0" for a random port)
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

## WASM brain modules (on disk)

In addition to the built-in demo modules, the service can load **WASM brain modules** from disk.

Module store root:

```
<root_dir>/intelligence/wasm_modules/
```

Each module lives in:

```
<root_dir>/intelligence/wasm_modules/<module_id>/
```

With:

- `module.json`:
  - `module_id` (string, must match folder name)
  - `abi_version` (currently `1`)
  - `source_kind` (`"wasm_only"` or `"rust_source"`)
- `brain.wasm` if `source_kind = "wasm_only"`
- `brain_user.rs` if `source_kind = "rust_source"` (compiled on demand into `build/brain.wasm`)

Notes:

- `GET /v1/modules` lists both built-in demo modules and on-disk WASM modules.
- The host calls `POST /v1/load_module` automatically for modules in use; for `rust_source` modules, this triggers compilation.
- If compilation fails, `load_module` returns an error. You can override the compiler via `GRAVIMERA_RUSTC=/path/to/rustc` (useful for development).
- On startup, the game syncs built-in demo modules from `assets/intelligence/wasm_modules/` into the module store (replacing existing folders with the same `module_id`).
- The desktop distribution bundles a Rust toolchain under `toolchain/rust/`, and the service auto-detects it for `rust_source` compilation (`GRAVIMERA_RUSTC` remains an override).

WASM guest ABI + encoding details: `docs/intelligence_wasm_brains.md`.

## Demo brain modules (built-in)

The service currently ships a few **demo** modules for development and testing:

- `demo.orbit.v1`
- `demo.coward.v1`
- `demo.opportunist.v1`
- `demo.belligerent.v1`

Notes:

- In normal desktop builds, these demo module ids are shipped as on-disk **WASM modules** and seeded into `<root_dir>/intelligence/wasm_modules/` from `assets/intelligence/wasm_modules/`.
- The seeded WASM demo modules are intentionally **simplified v1** and currently do **not** read the per-brain `config` JSON (obs-only guest ABI v1 does not carry config).
- If the on-disk WASM module for a demo id is missing, the service falls back to the built-in Rust demo implementations in `src/intelligence/service.rs` (these may be more featureful and may use `config`).

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

Performance note:

- The host submits `tick_many` to the service on a dedicated worker thread and applies the results on the main thread, to avoid blocking rendering frames on service execution time.

On Windows PowerShell, use `curl.exe` instead of `curl` if you hit the `Invoke-WebRequest` alias:

```powershell
curl.exe -s http://127.0.0.1:<port>/v1/health
```

## Use from the UI (Meta panel)

- Double-click a unit’s selection circle to open the **Meta** panel.
- In the **Brain** section:
  - `Fallback (default)` detaches the standalone brain and prevents the host from auto-attaching a default brain to that unit.
  - Brain modules are fetched asynchronously from the service after the panel opens (via `GET /v1/modules`).

## Troubleshooting
