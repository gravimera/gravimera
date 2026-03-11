# Development notes

## Useful commands

- Tests: `cargo test`
- Format: `cargo fmt`
- Headless smoke test (isolated data dir):

```bash
tmpdir=$(mktemp -d)
GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --headless --headless-seconds 1
```

## Network proxy (OpenAI/Gen3D)

Gravimera calls the AI provider by spawning `curl`. `curl` uses proxy environment variables
(`http_proxy`/`https_proxy`/`all_proxy`) but does not automatically inherit macOS/Windows “system proxy”
settings.

To make `cargo run` work in proxied/VPN networks, Gravimera auto-detects the system proxy on:

- macOS: via `scutil --proxy`
- Windows: via Internet Settings (`HKCU\\...\\Internet Settings`) with a WinHTTP fallback

Auto-detection is used only when no proxy env vars are already set. You can override/force behavior:

- Set `http_proxy`/`https_proxy`/`all_proxy` before running.
- Disable system proxy auto-detection with `GRAVIMERA_DISABLE_SYSTEM_PROXY=1`.

Note: PAC/WPAD auto-config proxies are not evaluated; in those setups, set the proxy env vars manually.

## Code layout (high level)

- `src/main.rs`: entrypoint
- `src/app.rs`: app wiring + headless/render fallback
- `src/config.rs`: config loading (`GRAVIMERA_CONFIG`, `~/.gravimera/config.toml` default)
- `src/paths.rs`: default paths (`GRAVIMERA_HOME`, `~/.gravimera/`)
- `src/setup.rs`: scene + initial entities
- `src/ui.rs`: window title, UI overlays, minimap, health change popups
- `src/automation/*`: local Automation HTTP API (for tooling/tests)
- `src/gen3d/*`: Gen3D workshop UI + rendered preview world
- `src/gen3d/ai/*`: Gen3D AI orchestration (OpenAI calls, schemas, cache artifacts)
- `src/object/*`: object system (prefabs + composition + visuals)
- `src/scene_store.rs`: load/save persisted scenes (`scene.dat` for Object Preview, `scene.build.dat` for Scene Build). `scene.dat` v9 stores per-instance forms plus a Player Character flag.
