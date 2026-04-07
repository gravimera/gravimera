#!/usr/bin/env python3
"""Start the Gravimera web viewer dev server.

Defaults:
  - host: 0.0.0.0 (listen on all addresses)
  - port: 233

The viewer auto-loads the demo build outputs from:
  assets/scene_wasteland/scene.grav
  assets/scene_wasteland/terrain.grav
"""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
VIEWER_DIR = ROOT / "web" / "viewer"
WASTELAND_DIR = ROOT / "assets" / "scene_wasteland"


def _run(cmd: list[str], *, cwd: Path) -> None:
    print("+", " ".join(cmd))
    subprocess.check_call(cmd, cwd=str(cwd))


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--port",
        type=int,
        default=int(os.environ.get("PORT", "233")),
        help="Server port (default: 233). Can also be set via env PORT.",
    )
    parser.add_argument(
        "--host",
        default=os.environ.get("HOST", "0.0.0.0"),
        help="Server host bind (default: 0.0.0.0). Can also be set via env HOST.",
    )
    parser.add_argument(
        "--mode",
        choices=["preview", "dev"],
        default=os.environ.get("MODE", "preview"),
        help="Server mode (default: preview). 'preview' serves the built bundle; 'dev' runs Vite dev server.",
    )
    parser.add_argument(
        "--no-build",
        action="store_true",
        help="Skip `npm run build` before previewing (only applies to --mode preview).",
    )
    parser.add_argument(
        "--install",
        action="store_true",
        help="Run `npm install` in web/viewer before starting (useful on first run).",
    )
    args = parser.parse_args(argv)

    if not VIEWER_DIR.is_dir():
        print(f"web viewer dir not found: {VIEWER_DIR}", file=sys.stderr)
        return 2

    if args.install or not (VIEWER_DIR / "node_modules").exists():
        _run(["npm", "install"], cwd=VIEWER_DIR)

    try:
        # Ports below 1024 typically require root/capabilities on Unix-like systems.
        if args.port < 1024 and hasattr(os, "geteuid") and os.geteuid() != 0:
            print(
                f"warning: port {args.port} may require root/cap_net_bind_service on Linux/macOS.",
                file=sys.stderr,
            )
            print("         if it fails to bind, re-run with sudo or use a higher --port.", file=sys.stderr)
    except Exception:
        pass

    # Best-effort validation: the server can still start without these, but autoload will fail.
    for name in ["scene.grav", "terrain.grav"]:
        path = WASTELAND_DIR / name
        if not path.is_file():
            print(f"warning: missing demo asset: {path}", file=sys.stderr)

    if args.mode == "preview":
        if not args.no_build:
            _run(["npm", "run", "build"], cwd=VIEWER_DIR)
        _run(
            [
                "npm",
                "run",
                "preview",
                "--",
                "--host",
                str(args.host),
                "--port",
                str(args.port),
                "--strictPort",
            ],
            cwd=VIEWER_DIR,
        )
    else:
        _run(
            [
                "npm",
                "run",
                "dev",
                "--",
                "--host",
                str(args.host),
                "--port",
                str(args.port),
                "--strictPort",
            ],
            cwd=VIEWER_DIR,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
