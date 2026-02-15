# Development notes

## Useful commands

- Tests: `cargo test`
- Format: `cargo fmt`
- Headless smoke test (isolated data dir):

```bash
tmpdir=$(mktemp -d)
GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --headless --headless-seconds 1
```

## Code layout (high level)

- `src/main.rs`: entrypoint
- `src/app.rs`: app wiring + headless/render fallback
- `src/config.rs`: config loading (`GRAVIMERA_CONFIG`, `~/.gravimera/config.toml` default)
- `src/paths.rs`: default paths (`GRAVIMERA_HOME`, `~/.gravimera/`)
- `src/setup.rs`: scene + initial entities
- `src/ui.rs`: window title, UI overlays, health bars, minimap
- `src/automation/*`: local Automation HTTP API (for tooling/tests)
- `src/gen3d/*`: Gen3D workshop UI + rendered preview world
- `src/gen3d/ai/*`: Gen3D AI orchestration (OpenAI calls, schemas, cache artifacts)
- `src/object/*`: object system (prefabs + composition + visuals)
- `src/scene_store.rs`: load/save persisted scenes (`scene.dat` per realm/scene)
