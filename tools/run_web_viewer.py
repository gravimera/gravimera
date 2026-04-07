#!/usr/bin/env python3
"""Start the Gravimera web viewer dev server.

Defaults:
  - host: 0.0.0.0 (listen on all addresses)
  - port: 5173

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
        default=int(os.environ.get("PORT", "5173")),
        help="Server port (default: 5173). Can also be set via env PORT.",
    )
    parser.add_argument(
        "--host",
        default=os.environ.get("HOST", "0.0.0.0"),
        help="Server host bind (default: 0.0.0.0). Can also be set via env HOST.",
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

    # Best-effort validation: the server can still start without these, but autoload will fail.
    for name in ["scene.grav", "terrain.grav"]:
        path = WASTELAND_DIR / name
        if not path.is_file():
            print(f"warning: missing demo asset: {path}", file=sys.stderr)

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
        ],
        cwd=VIEWER_DIR,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))

