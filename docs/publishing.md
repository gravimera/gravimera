# Publishing builds

Gravimera ships as:

- the game binary
- an `assets/` folder next to the binary (Windows/Linux), or inside the app bundle (macOS)

Runtime data (config/save/cache) lives under `~/.gravimera/` and is not bundled.

## Build + package (current platform)

```bash
python3 tools/publish.py
```

Outputs are written under `dist/<platform>/`:

- macOS: `Gravimera.app` + a zip
- Windows: a zip containing `gravimera.exe` + `assets/`
- Linux: a tar.gz containing `gravimera` + `assets/`

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

