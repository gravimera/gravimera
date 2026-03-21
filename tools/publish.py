#!/usr/bin/env python3
"""
Build and package Gravimera for distribution.

Outputs:
  dist/<platform>/...

Platform notes:
  - macOS: creates `Gravimera.app` and a zipped app bundle.
  - Windows: creates a zip containing `gravimera.exe` + `assets/`.
  - Linux: creates a tar.gz containing `gravimera` + `assets/`.

Runtime data is stored under `<root_dir>/` (default: `~/.gravimera/`; override via `root_dir` in config or env `GRAVIMERA_HOME`).
"""

from __future__ import annotations

import argparse
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASSETS_DIR = ROOT / "assets"
CARGO_TOML = ROOT / "Cargo.toml"
DIST_DIR = ROOT / "dist"

APP_NAME = "Gravimera"
BUNDLE_ID = "com.flowbehappy.gravimera"


def _read_version() -> str:
    text = CARGO_TOML.read_text(encoding="utf-8")
    in_package = False
    for raw in text.splitlines():
        line = raw.strip()
        if line.startswith("[") and line.endswith("]"):
            in_package = line == "[package]"
            continue
        if not in_package:
            continue
        m = re.match(r'version\s*=\s*"([^"]+)"\s*$', line)
        if m:
            return m.group(1)
    raise RuntimeError("Cargo.toml: missing [package].version")


def _run(cmd: list[str], *, cwd: Path = ROOT) -> None:
    print("+", " ".join(cmd))
    subprocess.check_call(cmd, cwd=str(cwd))


def _ensure_icons() -> None:
    required = ["icon.png", "icon_64.png", "icon.ico", "icon.icns"]
    missing = [name for name in required if not (ASSETS_DIR / name).is_file()]
    if missing:
        joined = ", ".join(missing)
        raise SystemExit(
            f"Missing icon files in `assets/`: {joined}\n"
            f"Run: `python3 tools/gen_app_icon.py`"
        )


def _clean_dir(path: Path) -> None:
    if path.exists():
        shutil.rmtree(path)
    path.mkdir(parents=True, exist_ok=True)


def _copy_tree(src: Path, dst: Path) -> None:
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)


def _chmod_x(path: Path) -> None:
    try:
        mode = path.stat().st_mode
        path.chmod(mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)
    except Exception:
        pass


def _make_zip(zip_path: Path, folder: Path) -> None:
    if zip_path.exists():
        zip_path.unlink()
    with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
        for file in sorted(folder.rglob("*")):
            rel = file.relative_to(folder.parent)
            if file.is_dir():
                continue
            zi = zipfile.ZipInfo.from_file(file, arcname=str(rel))
            # Preserve executable bit on Unix-like systems.
            try:
                st_mode = file.stat().st_mode
                zi.external_attr = (st_mode & 0xFFFF) << 16
            except Exception:
                pass
            with open(file, "rb") as f:
                zf.writestr(zi, f.read())


def _make_targz(tar_path: Path, folder: Path) -> None:
    if tar_path.exists():
        tar_path.unlink()
    with tarfile.open(tar_path, "w:gz") as tf:
        tf.add(folder, arcname=folder.name)


def _build_release(*, target: str | None) -> None:
    cmd = ["cargo", "build", "--release", "--bin", "gravimera"]
    if target:
        cmd += ["--target", target]
    _run(cmd)


def _release_bin_path(*, target: str | None) -> Path:
    if target:
        base = ROOT / "target" / target / "release"
    else:
        base = ROOT / "target" / "release"
    exe = "gravimera.exe" if sys.platform == "win32" else "gravimera"
    return base / exe


def _package_windows(*, version: str, bin_path: Path, out_dir: Path) -> None:
    pkg_name = f"gravimera-{version}-windows"
    pkg_dir = out_dir / pkg_name
    _clean_dir(pkg_dir)

    shutil.copy2(bin_path, pkg_dir / bin_path.name)
    _copy_tree(ASSETS_DIR, pkg_dir / "assets")
    shutil.copy2(ROOT / "README.md", pkg_dir / "README.md")
    shutil.copy2(ROOT / "config.example.toml", pkg_dir / "config.example.toml")

    zip_path = out_dir / f"{pkg_name}.zip"
    _make_zip(zip_path, pkg_dir)
    print(f"Wrote {zip_path}")


def _package_linux(*, version: str, bin_path: Path, out_dir: Path) -> None:
    pkg_name = f"gravimera-{version}-linux"
    pkg_dir = out_dir / pkg_name
    _clean_dir(pkg_dir)

    dst_bin = pkg_dir / "gravimera"
    shutil.copy2(bin_path, dst_bin)
    _chmod_x(dst_bin)
    _copy_tree(ASSETS_DIR, pkg_dir / "assets")
    shutil.copy2(ROOT / "README.md", pkg_dir / "README.md")
    shutil.copy2(ROOT / "config.example.toml", pkg_dir / "config.example.toml")

    tar_path = out_dir / f"{pkg_name}.tar.gz"
    _make_targz(tar_path, pkg_dir)
    print(f"Wrote {tar_path}")


def _write_info_plist(path: Path, *, version: str) -> None:
    plist = f"""<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key>
  <string>en</string>
  <key>CFBundleExecutable</key>
  <string>{APP_NAME}</string>
  <key>CFBundleIdentifier</key>
  <string>{BUNDLE_ID}</string>
  <key>CFBundleInfoDictionaryVersion</key>
  <string>6.0</string>
  <key>CFBundleName</key>
  <string>{APP_NAME}</string>
  <key>CFBundleDisplayName</key>
  <string>{APP_NAME}</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>{version}</string>
  <key>CFBundleVersion</key>
  <string>{version}</string>
  <key>CFBundleIconFile</key>
  <string>icon.icns</string>
  <key>NSHighResolutionCapable</key>
  <true/>
</dict>
</plist>
"""
    path.write_text(plist, encoding="utf-8")


def _package_macos(*, version: str, bin_path: Path, out_dir: Path) -> None:
    pkg_name = f"gravimera-{version}-macos"
    app_dir = out_dir / f"{APP_NAME}.app"
    if app_dir.exists():
        shutil.rmtree(app_dir)

    contents = app_dir / "Contents"
    macos_dir = contents / "MacOS"
    resources = contents / "Resources"
    macos_dir.mkdir(parents=True, exist_ok=True)
    resources.mkdir(parents=True, exist_ok=True)

    dst_bin = macos_dir / APP_NAME
    shutil.copy2(bin_path, dst_bin)
    _chmod_x(dst_bin)

    _copy_tree(ASSETS_DIR, resources / "assets")
    shutil.copy2(ASSETS_DIR / "icon.icns", resources / "icon.icns")

    _write_info_plist(contents / "Info.plist", version=version)

    zip_path = out_dir / f"{pkg_name}.zip"
    _make_zip(zip_path, app_dir)
    print(f"Wrote {zip_path}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--no-build", action="store_true", help="Skip `cargo build --release`")
    parser.add_argument("--target", default=None, help="Cargo target triple (optional)")
    args = parser.parse_args()

    _ensure_icons()
    version = _read_version()

    platform = "windows" if sys.platform == "win32" else "macos" if sys.platform == "darwin" else "linux"
    out_dir = DIST_DIR / platform
    out_dir.mkdir(parents=True, exist_ok=True)

    if not args.no_build:
        _build_release(target=args.target)

    bin_path = _release_bin_path(target=args.target)
    if not bin_path.is_file():
        raise SystemExit(f"Missing release binary: {bin_path}")

    if platform == "windows":
        _package_windows(version=version, bin_path=bin_path, out_dir=out_dir)
    elif platform == "macos":
        _package_macos(version=version, bin_path=bin_path, out_dir=out_dir)
    else:
        _package_linux(version=version, bin_path=bin_path, out_dir=out_dir)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
