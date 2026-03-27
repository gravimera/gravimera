# Gravimera

Gravimera is an **AI-driven Metaverse Editor**. You can use natural language to:

- Generate any 3D model with motion animations and make it directly playable in the world.
- Generate game scenes with highly interactive units and buildings.
- Generate a whole story. **[TODO]**

<img src="assets/icon.png" width="128" height="128" alt="Gravimera app icon" />

## Quickstart

**Supported OS:** macOS, Linux, Windows (MSVC).

**Toolchain:** Rust via `rustup` (toolchain pinned in `rust-toolchain.toml`).

**Minimal config (AI):**

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

Edit `~/.gravimera/config.toml` and set:

```toml
[openai]
base_url = "https://api.openai.com/v1" # or your OpenAI-compatible gateway
token = "YOUR_OPENAI_API_KEY"          # or set env `OPENAI_API_KEY`
```

Tip: start from `config.example.toml` so sensible defaults (including `openai.model`) are already present.

**Run:**

```bash
cargo run
```

## Screenshots
<img src="assets/screenshot/ss1.png" width="640" height="374" alt="ss1.png" />
<img src="assets/screenshot/ss2.png" width="640" height="374" alt="ss2.png" />

## Docs

- Developer onboarding: [docs/developer_onboarding.md](docs/developer_onboarding.md)
- Game design (final target): [docs/gamedesign/README.md](docs/gamedesign/README.md)
- Specs (contracts/formats): [docs/gamedesign/specs.md](docs/gamedesign/specs.md)
- Gen3D workflow + schemas: [docs/gen3d/README.md](docs/gen3d/README.md)
- GenFloor workflow + schemas: [docs/genfloor/README.md](docs/genfloor/README.md)
- Controls (rendered UI): [docs/controls.md](docs/controls.md)
- Local Automation HTTP API: [docs/automation_http_api.md](docs/automation_http_api.md)
- External agent monitor skill (scene + units + toast + TTS): [docs/agent_skills/SKILL_agent.md](docs/agent_skills/SKILL_agent.md)
- Intelligence service: [docs/intelligence_service.md](docs/intelligence_service.md)
- Publishing builds: [docs/publishing.md](docs/publishing.md)
