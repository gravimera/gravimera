# Gravimera

Gravimera is an **AI-driven game** that helps you build your own 3D game by generating 3D objects with AI and making them **directly playable** in the world.

It’s built with [Bevy](https://bevyengine.org/) and currently focuses on the Gen3D workflow: prompt (and optional reference images) → generate a prefab → drop it into the game as a unit/building.

<img src="assets/icon.png" width="128" height="128" alt="Gravimera app icon" />

## Prerequisites

- Rust via `rustup` (toolchain pinned in `rust-toolchain.toml`).
- A GPU for rendered mode (macOS uses Metal). If you don’t have one, use headless mode.
- Optional: Python 3 (packaging tools under `tools/`).

Windows:

- Install MSVC build tools (Visual Studio 2022 Build Tools: “Desktop development with C++”).
- If you see `can't find crate for 'std'` / `core`:
  `rustup component add rust-std-x86_64-pc-windows-msvc --toolchain 1.93.0-x86_64-pc-windows-msvc`

## Build & Run

Rendered:

```bash
cargo run
```

Headless (no GPU / CI):

```bash
cargo run -- --headless --headless-seconds 10
```

## Config & Data Directory

By default Gravimera stores runtime data under `~/.gravimera/`:

- `~/.gravimera/config.toml` (settings, OpenAI)
- `~/.gravimera/scene.dat` (saved world)
- `~/.gravimera/cache/` (Gen3D artifacts, logs, screenshots)

Override the base directory with `GRAVIMERA_HOME=/path/to/dir`.

Create a default config:

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

Override config loading:

- CLI: `cargo run -- --config /path/to/config.toml`
- Env: `GRAVIMERA_CONFIG=/path/to/config.toml cargo run`

Gen3D requires OpenAI settings in `config.toml` (or `OPENAI_API_KEY` via env).

## Docs

- Gen3D Workshop + schemas + cache layout: `gen_3d.md`
- Local Automation HTTP API (tooling/tests): `docs/automation_http_api.md`
- Controls: `docs/controls.md`
- Publishing builds: `docs/publishing.md`
- Developer notes / code layout: `docs/development.md`

