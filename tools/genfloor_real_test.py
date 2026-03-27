#!/usr/bin/env python3
"""
Run a real (rendered) GenFloor build via the local Automation HTTP API.

This is a semantic driver that:
- Starts the game with a given config.toml (uses its AI base_url + token)
- Enables the local Automation API via CLI flags (no config edits needed)
- Enters GenFloor (Floor Preview)
- Resets to a fresh GenFloor session
- Sets a prompt + builds
- Runs one edit-overwrite build to validate the edit loop

Artifacts (logs + screenshots) are written under:
  test/run_1/genfloor_real_test_*/...

Note: this script does NOT print or persist secrets.
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


def _now_ms() -> int:
    return int(time.time() * 1000)


def _http_json(
    method: str,
    url: str,
    body: dict[str, Any] | None = None,
    timeout: float = 30.0,
) -> dict[str, Any]:
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            if not raw.strip():
                raise RuntimeError(f"Empty response body from {url}")
            return json.loads(raw)
    except urllib.error.HTTPError as err:
        raw = err.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"HTTP {err.code} {url}: {raw.strip()}") from None
    except urllib.error.URLError as err:
        raise RuntimeError(f"Request failed {url}: {err}") from None


class GameProcess:
    def __init__(self, *, repo_root: Path, config_path: Path, home_dir: Path, stdout_path: Path):
        self._repo_root = repo_root
        self._config_path = config_path
        self._home_dir = home_dir
        self._stdout_path = stdout_path
        self._proc: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self._stdout_path.parent.mkdir(parents=True, exist_ok=True)
        out = open(self._stdout_path, "wb")
        env = os.environ.copy()
        env["GRAVIMERA_HOME"] = str(self._home_dir)
        self._proc = subprocess.Popen(
            [
                "cargo",
                "run",
                "--quiet",
                "--",
                "--config",
                str(self._config_path),
                "--automation",
                "--automation-bind",
                "127.0.0.1:0",
                "--automation-disable-local-input",
                "--automation-pause-on-start",
            ],
            cwd=str(self._repo_root),
            env=env,
            stdout=out,
            stderr=subprocess.STDOUT,
        )

    def terminate(self) -> None:
        if self._proc is None:
            return
        if self._proc.poll() is not None:
            return
        try:
            self._proc.terminate()
        except Exception:
            pass

    def kill(self) -> None:
        if self._proc is None:
            return
        if self._proc.poll() is not None:
            return
        try:
            self._proc.kill()
        except Exception:
            pass

    def ensure_stopped(self) -> None:
        if self._proc is None:
            return
        self.terminate()
        time.sleep(0.5)
        self.kill()


def _discover_base_url_from_log(log_path: Path, timeout_secs: float = 60.0) -> str:
    deadline = time.time() + timeout_secs
    last_size = 0
    prefix = "Automation API listening on "

    while time.time() < deadline:
        if log_path.exists():
            raw = log_path.read_text(encoding="utf-8", errors="replace")
            if "Rendered mode exited with an error. Falling back to headless mode." in raw:
                raise RuntimeError(
                    "gravimera fell back to headless mode; expected rendered mode"
                )
            for line in raw.splitlines():
                if prefix in line:
                    idx = line.find(prefix)
                    url = line[idx + len(prefix) :].strip()
                    if url.startswith("http://"):
                        return url
            size = log_path.stat().st_size
            if size != last_size:
                last_size = size
        time.sleep(0.2)

    tail = ""
    if log_path.exists():
        text = log_path.read_text(encoding="utf-8", errors="replace")
        tail = text[-4000:]
    raise RuntimeError(
        "Timed out waiting for Automation API listen address. " f"Log tail:\n{tail}"
    )


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--config",
        default=str(Path("~/.gravimera/config.toml").expanduser()),
        help="Path to config.toml (AI provider base_url/token live here). Default: ~/.gravimera/config.toml",
    )
    ap.add_argument(
        "--prompt",
        default="A subtle stone floor with gentle variation and no bumps.",
        help="GenFloor prompt text.",
    )
    ap.add_argument(
        "--edit-prompt",
        default="Make it darker and add a subtle checker pattern.",
        help="Second prompt to exercise Edit-overwrite.",
    )
    ap.add_argument("--timeout-secs", type=float, default=600.0)
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0)
    args = ap.parse_args()

    repo_root = Path(__file__).resolve().parents[1]
    config_path = Path(args.config).expanduser().resolve()
    if not config_path.exists():
        raise RuntimeError(f"Config not found: {config_path}")

    run_root = repo_root / "test" / "run_1" / f"genfloor_real_test_{_now_ms()}"
    run_root.mkdir(parents=True, exist_ok=True)
    home_dir = run_root / ".gravimera"
    home_dir.mkdir(parents=True, exist_ok=True)
    stdout_path = run_root / "gravimera_stdout.log"

    game = GameProcess(
        repo_root=repo_root,
        config_path=config_path,
        home_dir=home_dir,
        stdout_path=stdout_path,
    )
    game.start()

    ok = False
    try:
        api_base = _discover_base_url_from_log(stdout_path, timeout_secs=90.0)

        def post(path: str, body: dict[str, Any]) -> dict[str, Any]:
            timeout = 300.0 if path in ("/v1/step", "/v1/screenshot", "/v1/shutdown") else 60.0
            return _http_json("POST", f"{api_base}{path}", body, timeout=timeout)

        def get(path: str) -> dict[str, Any]:
            return _http_json("GET", f"{api_base}{path}", None, timeout=30.0)

        # Wait for health.
        t0 = time.monotonic()
        while True:
            try:
                health = get("/v1/health")
                if health.get("ok") is True:
                    break
            except Exception:
                pass
            if time.monotonic() - t0 > 30.0:
                raise RuntimeError("Timed out waiting for /v1/health")
            time.sleep(0.2)

        # Enter GenFloor.
        post("/v1/mode", {"mode": "genfloor"})
        post("/v1/step", {"frames": 3, "dt_secs": args.dt_secs})

        post("/v1/genfloor/new", {})
        post("/v1/genfloor/prompt", {"prompt": args.prompt})
        post("/v1/genfloor/build", {})

        def wait_done(timeout_secs: float) -> dict[str, Any]:
            deadline = time.monotonic() + timeout_secs
            last = None
            while time.monotonic() < deadline:
                post("/v1/step", {"frames": 10, "dt_secs": args.dt_secs})
                st = get("/v1/genfloor/status")
                last = st
                if not st.get("running") and st.get("draft_ready"):
                    return st
            raise RuntimeError(f"Timed out waiting for genfloor. Last status: {last}")

        done = wait_done(args.timeout_secs)
        floor_id = done.get("edit_base_floor_id_uuid")
        if not floor_id:
            raise RuntimeError(f"Missing edit_base_floor_id_uuid: {done}")

        post("/v1/screenshot", {"path": str(run_root / "after_build.png")})

        # Edit overwrite.
        post("/v1/genfloor/prompt", {"prompt": args.edit_prompt})
        post("/v1/genfloor/build", {})
        done2 = wait_done(args.timeout_secs)
        floor_id2 = done2.get("edit_base_floor_id_uuid")
        if floor_id2 != floor_id:
            raise RuntimeError(f"Expected edit overwrite {floor_id}, got {floor_id2}. status={done2}")

        post("/v1/screenshot", {"path": str(run_root / "after_edit.png")})

        post("/v1/shutdown", {})
        ok = True
        print(f"OK: artifacts at {run_root}")
        return 0
    finally:
        if not ok:
            print(f"FAILED. See log: {stdout_path}", file=os.sys.stderr)
        game.ensure_stopped()


if __name__ == "__main__":
    raise SystemExit(main())

