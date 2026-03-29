# Publishing builds

Gravimera ships as:

- the game binary
- an `assets/` folder next to the binary (Windows/Linux), or inside the app bundle (macOS)

Runtime data (config/save/cache) lives under `~/.gravimera/` and is not bundled.

## Build + package (current platform)

```bash
python3 tools/publish.py
```

Without `--target`, the script builds the host-default release binary and writes outputs under `dist/<platform>/`:

- macOS: `Gravimera.app` + a zip
- Windows: a zip containing `gravimera.exe` + `assets/`
- Linux: a tar.gz containing `gravimera` + `assets/`

## Build + package explicit targets

`--target` is repeatable. Each explicit target is built and packaged according to the target triple instead of the host OS, and the artifact names include the target triple so a single run can emit multiple packages without overwriting earlier ones.

Example: build both Apple Silicon and Intel macOS packages from one macOS machine:

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
python3 tools/publish.py \
  --target aarch64-apple-darwin \
  --target x86_64-apple-darwin
```

Example outputs under `dist/macos/`:

- `Gravimera-aarch64-apple-darwin.app`
- `gravimera-0.1.0-macos-aarch64-apple-darwin.zip`
- `Gravimera-x86_64-apple-darwin.app`
- `gravimera-0.1.0-macos-x86_64-apple-darwin.zip`

Notes:

- Use `--no-build` if the target binaries are already present under `target/<triple>/release/`.
- `tools/publish.py` does not install compilers or SDKs for you. If a Rust target is missing, it prints the `rustup target add ...` command to run.
- Whether a non-host target can compile still depends on the toolchain/linker support available on your machine.

## Icons

Icon assets live in `assets/`:

- `assets/icon.png`
- `assets/icon_64.png` (window icon)
- `assets/icon.ico` (Windows)
- `assets/icon.icns` (macOS)

Regenerate:

```bash
python3 tools/gen_app_icon.py
```
