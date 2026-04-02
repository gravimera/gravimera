#!/usr/bin/env python3
"""Build and package Gravimera release artifacts."""

from __future__ import annotations

import argparse
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import zipfile
from dataclasses import dataclass
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
ASSETS_DIR = ROOT / "assets"
CARGO_TOML = ROOT / "Cargo.toml"
DIST_DIR = ROOT / "dist"
TOOLCHAIN_DIR_NAME = "toolchain"
TOOLCHAIN_RUST_DIR_NAME = "rust"

APP_NAME = "Gravimera"
BUNDLE_ID = "com.flowbehappy.gravimera"

DESCRIPTION = """Build and package Gravimera release artifacts.

The script builds `gravimera` in release mode and writes packaged artifacts
under `dist/<platform>/`.

Packaging format by target platform:
  macOS: app bundle (`Gravimera.app`) plus a zip
  Windows: zip containing `gravimera.exe` and `assets/`
  Linux: tar.gz containing `gravimera` and `assets/`

Without `--target`, the script packages the host-default build.
With one or more `--target` flags, the script packages each explicit target
triple and suffixes artifact names with that target to avoid overwriting.

Runtime data (config/save/cache) is not bundled. Gravimera stores runtime data
under `~/.gravimera/` by default (override with `root_dir` in config or the
`GRAVIMERA_HOME` environment variable).

By default, the script also bundles a Rust toolchain (with the
`wasm32-unknown-unknown` standard library) under `toolchain/rust/` so players
can compile `rust_source` Intelligence WASM brain modules locally without
installing Rust.
"""

EXAMPLES = """Examples:
  Build and package the host-default target:
    python3 tools/publish.py

  Build and package both Apple Silicon and Intel macOS artifacts from one run:
    rustup target add aarch64-apple-darwin x86_64-apple-darwin
    python3 tools/publish.py --target aarch64-apple-darwin --target x86_64-apple-darwin

  Re-package prebuilt macOS targets without rebuilding:
    python3 tools/publish.py --no-build --target aarch64-apple-darwin --target x86_64-apple-darwin

  Build and package an explicit Linux target:
    python3 tools/publish.py --target x86_64-unknown-linux-gnu
"""


@dataclass(frozen=True)
class BuildSpec:
    target: str | None
    platform: str
    exe_name: str
    artifact_suffix: str | None
    bundle_name: str


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


def _check_output(cmd: list[str], *, cwd: Path = ROOT) -> str:
    return subprocess.check_output(cmd, cwd=str(cwd), text=True)


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


def _host_platform() -> str:
    if sys.platform == "win32":
        return "windows"
    if sys.platform == "darwin":
        return "macos"
    return "linux"


def _platform_from_target(target: str) -> str:
    lowered = target.lower()
    if "windows" in lowered:
        return "windows"
    if "apple-darwin" in lowered or lowered.endswith("-darwin"):
        return "macos"
    if "linux" in lowered:
        return "linux"
    raise SystemExit(
        f"Unsupported target triple for packaging: {target}\n"
        "Expected a Windows, macOS, or Linux target triple."
    )


def _exe_name_for_platform(platform: str) -> str:
    return "gravimera.exe" if platform == "windows" else "gravimera"


def _package_name(*, version: str, platform: str, artifact_suffix: str | None) -> str:
    name = f"gravimera-{version}-{platform}"
    if artifact_suffix:
        name += f"-{artifact_suffix}"
    return name


def _normalize_build_specs(targets: list[str] | None) -> list[BuildSpec]:
    if not targets:
        platform = _host_platform()
        return [
            BuildSpec(
                target=None,
                platform=platform,
                exe_name=_exe_name_for_platform(platform),
                artifact_suffix=None,
                bundle_name=APP_NAME,
            )
        ]

    specs: list[BuildSpec] = []
    seen: set[str] = set()
    for target in targets:
        if target in seen:
            continue
        seen.add(target)
        platform = _platform_from_target(target)
        specs.append(
            BuildSpec(
                target=target,
                platform=platform,
                exe_name=_exe_name_for_platform(platform),
                artifact_suffix=target,
                bundle_name=f"{APP_NAME}-{target}" if platform == "macos" else APP_NAME,
            )
        )
    return specs


def _installed_rust_targets() -> set[str]:
    try:
        output = _check_output(["rustup", "target", "list", "--installed"])
    except (OSError, subprocess.CalledProcessError):
        return set()
    return {line.strip() for line in output.splitlines() if line.strip()}


def _ensure_explicit_targets_installed(specs: list[BuildSpec]) -> None:
    explicit_targets = [spec.target for spec in specs if spec.target]
    if not explicit_targets:
        return
    installed = _installed_rust_targets()
    if not installed:
        return
    missing = [target for target in explicit_targets if target not in installed]
    if missing:
        joined = ", ".join(missing)
        lines = [f"Rust target(s) not installed: {joined}"]
        lines.extend(f"Run: `rustup target add {target}`" for target in missing)
        raise SystemExit("\n".join(lines))


def _rustc_host_triple() -> str:
    output = _check_output(["rustc", "-vV"])
    for raw in output.splitlines():
        line = raw.strip()
        if line.startswith("host: "):
            return line.split(":", 1)[1].strip()
    raise SystemExit("Failed to parse `rustc -vV` output (missing `host:` line).")


def _rustup_active_toolchain_name() -> str | None:
    try:
        output = _check_output(["rustup", "show", "active-toolchain"])
    except (OSError, subprocess.CalledProcessError):
        return None
    # Format: "<toolchain> (default)".
    return output.strip().split()[0] if output.strip() else None


def _derive_toolchain_for_target(
    *, active_toolchain: str | None, host_triple: str, desired_host_triple: str
) -> str | None:
    if active_toolchain is None:
        return None
    suffix = f"-{host_triple}"
    if desired_host_triple == host_triple:
        return active_toolchain
    if active_toolchain.endswith(suffix):
        channel = active_toolchain[: -len(suffix)]
        if channel:
            return f"{channel}-{desired_host_triple}"
    return None


def _toolchain_sysroot_for_toolchain(toolchain: str) -> Path:
    try:
        rustc_path = _check_output(["rustup", "which", "rustc", "--toolchain", toolchain]).strip()
    except (OSError, subprocess.CalledProcessError):
        raise SystemExit(
            "Failed to locate rustc for toolchain "
            f"{toolchain!r}. Install it via:\n"
            f"  rustup toolchain install {toolchain}"
        )
    rustc = Path(rustc_path)
    sysroot = rustc.parent.parent
    if not sysroot.is_dir():
        raise SystemExit(f"Invalid sysroot for toolchain {toolchain!r}: {sysroot}")
    return sysroot


def _toolchain_sysroot_for_spec(spec: BuildSpec) -> tuple[Path, str | None]:
    host_triple = _rustc_host_triple()
    active = _rustup_active_toolchain_name()
    desired_host_triple = spec.target or host_triple

    toolchain = _derive_toolchain_for_target(
        active_toolchain=active,
        host_triple=host_triple,
        desired_host_triple=desired_host_triple,
    )

    if toolchain is None:
        if spec.target and spec.target != host_triple:
            raise SystemExit(
                "Cannot auto-bundle a Rust toolchain for non-host target "
                f"{spec.target!r} without rustup. Install rustup and the matching toolchain."
            )
        sysroot_text = _check_output(["rustc", "--print", "sysroot"]).strip()
        sysroot = Path(sysroot_text)
        if not sysroot.is_dir():
            raise SystemExit(f"Invalid rustc sysroot: {sysroot}")
        return (sysroot, None)

    sysroot = _toolchain_sysroot_for_toolchain(toolchain)
    return (sysroot, toolchain)


def _ensure_wasm_target_installed(*, sysroot: Path, toolchain: str | None) -> None:
    wasm_std = sysroot / "lib" / "rustlib" / "wasm32-unknown-unknown"
    if wasm_std.is_dir():
        return
    if toolchain:
        raise SystemExit(
            "Rust toolchain sysroot is missing wasm32-unknown-unknown stdlib:\n"
            f"  {wasm_std}\n"
            "Install it via:\n"
            f"  rustup target add wasm32-unknown-unknown --toolchain {toolchain}"
        )
    raise SystemExit(
        "Rust toolchain sysroot is missing wasm32-unknown-unknown stdlib:\n"
        f"  {wasm_std}\n"
        "Install it via:\n"
        "  rustup target add wasm32-unknown-unknown"
    )


def _bundle_rust_toolchain(*, sysroot: Path, dst_root: Path) -> None:
    dst = dst_root / TOOLCHAIN_DIR_NAME / TOOLCHAIN_RUST_DIR_NAME
    print(f"Bundling Rust toolchain sysroot: {sysroot} -> {dst}")
    _copy_tree(sysroot, dst)


def _build_release(*, target: str | None) -> None:
    cmd = ["cargo", "build", "--release", "--bin", "gravimera"]
    if target:
        cmd += ["--target", target]
    _run(cmd)


def _release_bin_path(*, target: str | None, exe_name: str) -> Path:
    if target:
        base = ROOT / "target" / target / "release"
    else:
        base = ROOT / "target" / "release"
    return base / exe_name


def _package_windows(
    *,
    version: str,
    bin_path: Path,
    out_dir: Path,
    artifact_suffix: str | None,
    rust_sysroot: Path | None,
) -> None:
    pkg_name = _package_name(version=version, platform="windows", artifact_suffix=artifact_suffix)
    pkg_dir = out_dir / pkg_name
    _clean_dir(pkg_dir)

    shutil.copy2(bin_path, pkg_dir / bin_path.name)
    _copy_tree(ASSETS_DIR, pkg_dir / "assets")
    shutil.copy2(ROOT / "README.md", pkg_dir / "README.md")
    shutil.copy2(ROOT / "config.example.toml", pkg_dir / "config.example.toml")
    if rust_sysroot is not None:
        _bundle_rust_toolchain(sysroot=rust_sysroot, dst_root=pkg_dir)

    zip_path = out_dir / f"{pkg_name}.zip"
    _make_zip(zip_path, pkg_dir)
    print(f"Wrote {zip_path}")


def _package_linux(
    *,
    version: str,
    bin_path: Path,
    out_dir: Path,
    artifact_suffix: str | None,
    rust_sysroot: Path | None,
) -> None:
    pkg_name = _package_name(version=version, platform="linux", artifact_suffix=artifact_suffix)
    pkg_dir = out_dir / pkg_name
    _clean_dir(pkg_dir)

    dst_bin = pkg_dir / "gravimera"
    shutil.copy2(bin_path, dst_bin)
    _chmod_x(dst_bin)
    _copy_tree(ASSETS_DIR, pkg_dir / "assets")
    shutil.copy2(ROOT / "README.md", pkg_dir / "README.md")
    shutil.copy2(ROOT / "config.example.toml", pkg_dir / "config.example.toml")
    if rust_sysroot is not None:
        _bundle_rust_toolchain(sysroot=rust_sysroot, dst_root=pkg_dir)

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


def _package_macos(
    *,
    version: str,
    bin_path: Path,
    out_dir: Path,
    artifact_suffix: str | None,
    bundle_name: str,
    rust_sysroot: Path | None,
) -> None:
    pkg_name = _package_name(version=version, platform="macos", artifact_suffix=artifact_suffix)
    app_dir = out_dir / f"{bundle_name}.app"
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
    if rust_sysroot is not None:
        _bundle_rust_toolchain(sysroot=rust_sysroot, dst_root=resources)

    _write_info_plist(contents / "Info.plist", version=version)

    zip_path = out_dir / f"{pkg_name}.zip"
    _make_zip(zip_path, app_dir)
    print(f"Wrote {zip_path}")


def main() -> int:
    parser = argparse.ArgumentParser(
        description=DESCRIPTION,
        epilog=EXAMPLES,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("--no-build", action="store_true", help="Skip `cargo build --release`")
    parser.add_argument(
        "--target",
        action="append",
        dest="targets",
        default=None,
        metavar="TARGET",
        help="Cargo target triple (repeatable)",
    )
    parser.add_argument(
        "--no-bundle-rust-toolchain",
        action="store_true",
        help="Do not bundle the Rust toolchain under toolchain/rust/ (WASM brain compilation will require Rust installed).",
    )
    args = parser.parse_args()

    _ensure_icons()
    version = _read_version()
    specs = _normalize_build_specs(args.targets)
    if not args.no_build:
        _ensure_explicit_targets_installed(specs)

    for spec in specs:
        out_dir = DIST_DIR / spec.platform
        out_dir.mkdir(parents=True, exist_ok=True)

        if not args.no_build:
            _build_release(target=spec.target)

        bin_path = _release_bin_path(target=spec.target, exe_name=spec.exe_name)
        if not bin_path.is_file():
            raise SystemExit(f"Missing release binary: {bin_path}")

        rust_sysroot = None
        if not args.no_bundle_rust_toolchain:
            host_platform = _host_platform()
            if spec.platform != host_platform:
                raise SystemExit(
                    "Bundled toolchains must be packaged on the target platform.\n"
                    f"Host platform: {host_platform}\n"
                    f"Requested package platform: {spec.platform}\n"
                    "Re-run `tools/publish.py` on that platform, or pass `--no-bundle-rust-toolchain`."
                )
            rust_sysroot, toolchain = _toolchain_sysroot_for_spec(spec)
            _ensure_wasm_target_installed(sysroot=rust_sysroot, toolchain=toolchain)

        if spec.platform == "windows":
            _package_windows(
                version=version,
                bin_path=bin_path,
                out_dir=out_dir,
                artifact_suffix=spec.artifact_suffix,
                rust_sysroot=rust_sysroot,
            )
        elif spec.platform == "macos":
            _package_macos(
                version=version,
                bin_path=bin_path,
                out_dir=out_dir,
                artifact_suffix=spec.artifact_suffix,
                bundle_name=spec.bundle_name,
                rust_sysroot=rust_sysroot,
            )
        else:
            _package_linux(
                version=version,
                bin_path=bin_path,
                out_dir=out_dir,
                artifact_suffix=spec.artifact_suffix,
                rust_sysroot=rust_sysroot,
            )

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
