# Publishing builds

Gravimera ships as:

- the game binary
- an `assets/` folder next to the binary (Windows/Linux), or inside the app bundle (macOS)
- optionally, a bundled Rust toolchain under `toolchain/rust/` (enable with `python3 tools/publish.py --bundle-rust-toolchain`, used for local compilation of Intelligence WASM brain modules)

Runtime data (config/save/cache) lives under `~/.gravimera/` and is not bundled.

## Build + package (current platform)

```bash
python3 tools/publish.py
```

Without `--target`, the script builds the host-default release binary and writes outputs under `dist/<platform>/`:

- macOS: `Gravimera.app` + a zip
- Windows: a zip containing `gravimera.exe` + `assets/`
- Linux: a tar.gz containing `gravimera` + `assets/`

Example outputs:

- `dist/macos/Gravimera.app`
- `dist/macos/gravimera-0.1.0-macos.zip`
- `dist/linux/gravimera-0.1.0-linux.tar.gz`
- `dist/windows/gravimera-0.1.0-windows.zip`

## macOS signing + notarization (recommended for GitHub Releases)

If you upload macOS artifacts to GitHub Releases, users will download a quarantined zip and Gatekeeper will assess the app. To avoid the “app is damaged” / blocked warnings, ship a **Developer ID–signed** and **notarized** app.

On macOS:

```bash
python3 tools/publish.py --macos-notarize
```

Notes:

- `--macos-notarize` implies `--macos-sign` and uses hardened runtime.
- Provide the Developer ID identity via `--macos-codesign-identity` (or `GRAVIMERA_MACOS_CODESIGN_IDENTITY`).
  - List identities: `security find-identity -v -p codesigning`
- Notarization authentication (choose one):
  - Recommended: create a notarytool keychain profile and pass `--macos-notarytool-profile` (or `GRAVIMERA_MACOS_NOTARYTOOL_PROFILE`).
  - Or pass `--macos-notary-apple-id`, `--macos-notary-team-id`, `--macos-notary-password` (or the matching `GRAVIMERA_MACOS_NOTARY_*` env vars). Prefer `@keychain:<item>` for the password.

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

Example: package an explicit Linux target:

```bash
rustup target add x86_64-unknown-linux-gnu
python3 tools/publish.py --target x86_64-unknown-linux-gnu
```

Example outputs under `dist/linux/`:

- `gravimera-0.1.0-linux-x86_64-unknown-linux-gnu/`
- `gravimera-0.1.0-linux-x86_64-unknown-linux-gnu.tar.gz`

Example: re-package already-built targets without rebuilding:

```bash
python3 tools/publish.py --no-build \
  --target aarch64-apple-darwin \
  --target x86_64-apple-darwin
```

Notes:

- Use `--no-build` if the target binaries are already present under `target/<triple>/release/`.
- `tools/publish.py` does not install compilers or SDKs for you. If a Rust target is missing, it prints the `rustup target add ...` command to run.
- Whether a non-host target can compile still depends on the toolchain/linker support available on your machine.
- Toolchain bundling:
  - By default, `tools/publish.py` does not bundle a Rust toolchain (smaller artifacts; players will need Rust installed to compile `rust_source` brain modules).
  - Enable bundling with: `python3 tools/publish.py --bundle-rust-toolchain`
  - Bundling requires the `wasm32-unknown-unknown` standard library to be present for the bundled toolchain.
  - Bundled toolchains must be packaged on the target platform (e.g. package Windows toolchains on Windows).

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
