# Gravimera

Gravimera is an **AI-driven game** that helps you build your own 3D game by generating 3D objects with AI and making them **directly playable** in the world.

It’s built with [Bevy](https://bevyengine.org/) and currently focuses on the Gen3D workflow: prompt (and optional reference images) → generate a prefab → drop it into the game as a unit/building.

Gen3D implementation details (agent loop, schemas, cache layout, validation) live in `gen_3d.md` and under `docs/`.

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

## Units / Scale

- World space uses meters: `1.0` world unit = `1 meter`.
- Build mode snapping uses a small grid: `0.05m` (5 cm).
- Scene persistence (`scene.dat`) quantizes positions to centimeters (1 cm).

## Config & Data Directory

By default Gravimera stores runtime data under `~/.gravimera/`:

- `~/.gravimera/config.toml` (settings, OpenAI/Gemini)
- `~/.gravimera/openai_capabilities_cache.json` (cached OpenAI-compatible endpoint capabilities, keyed by `base_url` + `model`)
- `~/.gravimera/realm/` (realms + scenes)
- `~/.gravimera/realm/active.json` (active realm/scene selection)
- `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/build/scene.dat` (saved scene)
- `~/.gravimera/realm/<realm_id>/prefabs/` (realm prefab packages; Gen3D saves prefabs here; layout spec `docs/gamedesign/39_realm_prefab_packages_v1.md`)
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

Gen3D requires AI settings in `config.toml` (`[openai]` by default; set `[gen3d].ai_service = "gemini"` to use `[gemini]`).

## Intelligence Service (Standalone Brains)

Optional: run unit brains via the intelligence service.

- Default mode (when enabled): **embedded** (runs inside the Gravimera process).
- Sidecar mode: run `cargo run --bin gravimera_intelligence_service` and set `[intelligence_service].mode = "sidecar"` in `config.toml`.

Docs: `docs/intelligence_service.md` (spec: `docs/gamedesign/38_intelligence_service_spec.md`).

## Docs

- Game design (long-term target): `docs/gamedesign/README.md`
- Specs (contracts/formats): `docs/gamedesign/specs.md`
- Scene Builder (rendered UI) + artifacts: `docs/controls.md` and `docs/gamedesign/30_scene_sources_and_build_artifacts.md`
- Gen3D Workshop + schemas + cache layout: `gen_3d.md`
- Local Automation HTTP API (tooling/tests): `docs/automation_http_api.md`
- Meta panel Speak (soundtest integration): `docs/meta_speak.md`
  - The `soundtest` crate is vendored in-repo under `third_party/soundtest` for out-of-box builds.
- Rendered Automation drivers (real-cycle): `python tools/gen3d_real_test.py` (Gen3D) and `python tools/scene_build_real_test.py` (Scene Builder)
- Controls: `docs/controls.md`
- Publishing builds: `docs/publishing.md`
- Developer notes / code layout: `docs/development.md`
