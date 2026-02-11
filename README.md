# Gravimera

Gravimera is an **AI-driven game** that helps you build your own 3D game by generating 3D objects with AI and making them **directly playable** in the world.

It’s built with [Bevy](https://bevyengine.org/) and currently focuses on the Gen3D workflow: prompt (and optional reference images) → generate a prefab → drop it into the game as a unit/building.

Gen3D also tries to avoid duplicated LLM work by **reusing generated geometry** for repeated parts (for example: wheels, mirrored parts, or radial legs) via plan-level `reuse_groups` + deterministic copy tools (including subtree copy that can auto-expand missing target limb-chain descendants). Attachment animations support `time_offset_units` for deterministic phase offsets (staggered legs) without duplicating keyframes. The engine can auto-apply reuse after batch generation, and saves prefabs with bounds that account for animation keyframes to reduce ground-clipping surprises.

When supported by your OpenAI-compatible endpoint, Gen3D requests API-level **Structured Outputs** (strict JSON Schema) for plan / component / review JSON so runs spend less time in schema-repair loops. If the provider rejects structured outputs, Gravimera automatically disables it for the current session and falls back to the legacy “free-form JSON + local repair” behavior.

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

WSL (WSLg):

- Gravimera prefers the X11 backend on WSLg (XWayland) because Wayland connections can be flaky under WSL.
  - It auto-sets `WINIT_UNIX_BACKEND=x11` and unsets `WAYLAND_DISPLAY` when `DISPLAY` is available.
- Clipboard (Gen3D prompt paste + Tool Feedback copy) prefers the Windows clipboard via WSL interop (`powershell.exe` / `clip.exe`) when Windows `.exe` execution is available.
  - If Windows interop isn’t available, Gravimera falls back to an internal X11 clipboard backend when `DISPLAY` is set (no `wl-clipboard`/`xclip` required).
  - For Wayland-only sessions, install a Linux clipboard backend like `wl-clipboard` or `xclip`/`xsel`.
- If you see a crash mentioning missing `libxkbcommon-x11.so.0` / `libxcb-xkb.so.1`:
  - Install system packages: `sudo apt-get update && sudo apt-get install -y libxkbcommon-x11-0 libxcb-xkb1`
  - Or (no sudo) provide those `.so` files under `~/.local/gravimera-sysroot/usr/lib/<multiarch>/` (e.g. `x86_64-linux-gnu/`); Gravimera will re-exec with an updated `LD_LIBRARY_PATH`.
- If you force Wayland (e.g. `WINIT_UNIX_BACKEND=wayland`) and hit `WaylandError(Connection(NoCompositor))`, set `XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir`.

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

- Game design (long-term target): `docs/gamedesign/README.md`
- Gen3D Workshop + schemas + cache layout: `gen_3d.md`
- Local Automation HTTP API (tooling/tests): `docs/automation_http_api.md`
- Controls: `docs/controls.md`
- Publishing builds: `docs/publishing.md`
- Developer notes / code layout: `docs/development.md`
