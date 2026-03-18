# Developer onboarding

This doc covers building and running Gravimera from source, plus platform notes and common dev workflows.

## Prerequisites

Supported OS:

- macOS
- Linux
- Windows (MSVC toolchain)

Toolchain:

- Rust via `rustup` (toolchain pinned in `rust-toolchain.toml`).
- Optional: Python 3 (tools under `tools/`).

Windows:

- Install Visual Studio 2022 Build Tools (or Visual Studio) with **Desktop development with C++**.
- If you see `can't find crate for 'std'` / `core`:

```bash
rustup component add rust-std-x86_64-pc-windows-msvc --toolchain 1.93.0-x86_64-pc-windows-msvc
```

## Config

Gravimera reads config from `~/.gravimera/config.toml` by default.

Create a default config:

```bash
mkdir -p ~/.gravimera
cp config.example.toml ~/.gravimera/config.toml
```

Override config loading:

- CLI: `cargo run -- --config /path/to/config.toml`
- Env: `GRAVIMERA_CONFIG=/path/to/config.toml cargo run`

### Minimal AI config (OpenAI-compatible)

Gen3D uses OpenAI-compatible endpoints by default (`[gen3d].ai_service = "openai"`).

- Optional: `[openai].model` (defaults to `gpt-5.4`)
- Optional: `[openai].base_url` (defaults to `https://api.openai.com/v1`)
- Required: `[openai].token` (or env `OPENAI_API_KEY`)

Example:

```toml
[openai]
base_url = "https://api.openai.com/v1" # or your gateway
model = "gpt-5.4"
token = "YOUR_OPENAI_API_KEY"
```

Other providers:

- Gemini: set `[gen3d].ai_service = "gemini"` and configure `[gemini]` (token env: `X_GOOG_API_KEY` / `GEMINI_API_KEY`).
- Claude: set `[gen3d].ai_service = "claude"` and configure `[claude]` (token env: `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY`).

See `config.example.toml` for the full config surface.

## Build & run

Rendered (normal UI):

```bash
cargo run
```

Headless (CI / no GPU):

```bash
cargo run -- --headless --headless-seconds 10
```

### Smoke test (rendered, isolated data dir)

After changes, prefer a short rendered smoke test:

```bash
tmpdir=$(mktemp -d); GRAVIMERA_HOME="$tmpdir/.gravimera" cargo run -- --rendered-seconds 2
```

## Data directory

By default Gravimera stores runtime data under `~/.gravimera/`:

- `~/.gravimera/config.toml` (settings, AI providers)
- `~/.gravimera/openai_capabilities_cache.json` (cached OpenAI-compatible endpoint capabilities, keyed by `base_url` + `model`)
- `~/.gravimera/realm/` (realms + scenes)
- `~/.gravimera/realm/active.json` (active realm/scene selection)
- `~/.gravimera/realm/<realm_id>/scenes/<scene_id>/build/scene.dat` (saved scene)
- `~/.gravimera/realm/<realm_id>/prefabs/` (realm prefab packages; Gen3D saves prefabs here; layout spec `docs/gamedesign/39_realm_prefab_packages_v1.md`)
- `~/.gravimera/cache/` (Gen3D artifacts, logs, screenshots)

Override the base directory with `GRAVIMERA_HOME=/path/to/dir`.

## WSL (WSLg)

Gravimera can run under WSLg. Notes:

- Gravimera prefers the X11 backend on WSLg (XWayland) because Wayland connections can be flaky under WSL.
  - It auto-sets `WINIT_UNIX_BACKEND=x11` and unsets `WAYLAND_DISPLAY` when `DISPLAY` is available.
- Clipboard (paste actions, e.g. Gen3D prompt paste) prefers the Windows clipboard via WSL interop (`powershell.exe` / `clip.exe`) when Windows `.exe` execution is available.
  - If Windows interop isn’t available, Gravimera falls back to an internal X11 clipboard backend when `DISPLAY` is set (no `wl-clipboard`/`xclip` required).
  - For Wayland-only sessions, install a Linux clipboard backend like `wl-clipboard` or `xclip`/`xsel`.
- If you see a crash mentioning missing `libxkbcommon-x11.so.0` / `libxcb-xkb.so.1`:
  - Install system packages: `sudo apt-get update && sudo apt-get install -y libxkbcommon-x11-0 libxcb-xkb1`
  - Or (no sudo) provide those `.so` files under `~/.local/gravimera-sysroot/usr/lib/<multiarch>/` (e.g. `x86_64-linux-gnu/`); Gravimera will re-exec with an updated `LD_LIBRARY_PATH`.
- If you force Wayland (e.g. `WINIT_UNIX_BACKEND=wayland`) and hit `WaylandError(Connection(NoCompositor))`, set `XDG_RUNTIME_DIR=/mnt/wslg/runtime-dir`.

## Units / scale

- World space uses meters: `1.0` world unit = `1 meter`.
- Build mode snapping uses a small grid: `0.05m` (5 cm).
- Scene persistence (`scene.dat`) quantizes positions to centimeters (1 cm).

## Optional services

Intelligence service (standalone brains):

- Default mode (when enabled): **embedded** (runs inside the Gravimera process).
- Sidecar mode: run `cargo run --bin gravimera_intelligence_service` and set `[intelligence_service].mode = "sidecar"` in `config.toml`.

Docs: `docs/intelligence_service.md` (spec: `docs/gamedesign/38_intelligence_service_spec.md`).

Automation HTTP API:

- Local-only control surface used by tooling/tests (select/move/fire/mode/screenshot/shutdown).

Docs: `docs/automation_http_api.md`.

## Dev workflow

- Tests: `cargo test`
- Format: `cargo fmt`
- Lints (optional): `cargo clippy`

Network proxy notes (OpenAI/Gen3D):

- Gravimera calls the AI provider by spawning `curl`, so proxy env vars (`http_proxy`/`https_proxy`/`all_proxy`) apply.
- If you rely on macOS/Windows “system proxy” settings, see `docs/development.md` for the auto-detection rules and overrides.

Code layout (high level): see `docs/development.md`.
