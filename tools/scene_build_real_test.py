#!/usr/bin/env python3
"""
Run a real (rendered) Scene Build via the local Automation HTTP API.

This script:
- Starts the game with a given config.toml (Automation enabled)
- Switches to Build mode
- Starts Scene Build from a description
- Steps frames while polling build status until completion
- Captures a final screenshot into the run_dir

Artifacts are written under the scene build run directory:
  ~/.gravimera/realm/<realm_id>/scenes/<scene_id>/runs/<run_id>/
"""

from __future__ import annotations

import argparse
import json
import re
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _parse_automation_bind(config_text: str) -> str:
    # Best-effort parse (no external TOML dependency).
    in_automation = False
    for raw in config_text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_automation = line == "[automation]"
            continue
        if not in_automation:
            continue
        m = re.match(r'bind\s*=\s*"([^"]+)"\s*$', line)
        if m:
            return m.group(1)
    raise ValueError("config.toml: missing [automation].bind (example: bind = \"127.0.0.1:8791\")")


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


@dataclass
class RunResult:
    run_id: str
    run_dir: Path


class GameProcess:
    def __init__(self, *, bin_path: Path, config_path: Path, workdir: Path, stdout_path: Path):
        self._bin_path = bin_path
        self._config_path = config_path
        self._workdir = workdir
        self._stdout_path = stdout_path
        self._proc: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self._stdout_path.parent.mkdir(parents=True, exist_ok=True)
        out = open(self._stdout_path, "wb")
        self._proc = subprocess.Popen(
            [str(self._bin_path), "--config", str(self._config_path)],
            cwd=str(self._workdir),
            stdout=out,
            stderr=subprocess.STDOUT,
        )

    def is_running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

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

    def wait(self, timeout: float) -> None:
        if self._proc is None:
            return
        try:
            self._proc.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            pass

    def ensure_stopped(self) -> None:
        if self._proc is None:
            return
        self.terminate()
        self.wait(1.0)
        self.kill()
        self.wait(1.0)


def run_scene_build(
    *,
    api_base: str,
    description: str,
    timeout_secs: float,
    dt_secs: float,
) -> RunResult:
    def post(path: str, body: dict[str, Any]) -> dict[str, Any]:
        if path in ("/v1/step", "/v1/screenshot", "/v1/shutdown"):
            timeout = 300.0
        else:
            timeout = 30.0
        return _http_json("POST", f"{api_base}{path}", body, timeout=timeout)

    def get(path: str) -> dict[str, Any]:
        return _http_json("GET", f"{api_base}{path}", None, timeout=30.0)

    # Ensure Build mode (Scene Build requires mode != gen3d).
    post("/v1/mode", {"mode": "build"})
    post("/v1/step", {"frames": 3, "dt_secs": dt_secs})

    start = post("/v1/scene_build/start", {"description": description})
    run_id = str(start.get("run_id") or "").strip()
    if not run_id:
        raise RuntimeError(f"scene_build/start returned no run_id: {start}")

    t0 = time.monotonic()
    last_message = None
    last_phase = None
    run_dir: Path | None = None

    while True:
        post("/v1/step", {"frames": 10, "dt_secs": dt_secs})
        status_resp = get("/v1/scene_build/status")
        status = status_resp.get("status") or {}
        running = bool(status.get("running"))
        cur_run_id = str(status.get("run_id") or "").strip()
        message = status.get("message")
        phase = status.get("phase")
        run_dir_raw = status.get("run_dir") or ""
        if run_dir_raw:
            run_dir = Path(str(run_dir_raw))

        if message != last_message or phase != last_phase:
            msg = str(message or "").replace("\n", " ").strip()
            if len(msg) > 160:
                msg = msg[:157] + "…"
            print(f"running={running} phase={phase} message={msg}")
            last_message = message
            last_phase = phase

        if cur_run_id and cur_run_id != run_id:
            raise RuntimeError(f"scene_build/status run_id mismatch: expected={run_id} got={cur_run_id}")

        if not running:
            break

        if time.monotonic() - t0 > timeout_secs:
            raise RuntimeError(f"Timed out waiting for Scene Build after {timeout_secs:.1f}s. Last status: {status_resp}")

        time.sleep(0.05)

    if run_dir is None:
        raise RuntimeError("Missing run_dir from scene_build/status")

    # Final screenshot into the run dir for convenience.
    out_dir = run_dir / "external_screenshots_world"
    out_dir.mkdir(parents=True, exist_ok=True)
    out_path = out_dir / "final.png"
    post("/v1/screenshot", {"path": str(out_path)})
    # Screenshot capture is applied on the next frame; step a few frames and wait briefly.
    t0 = time.monotonic()
    while time.monotonic() - t0 < 10.0:
        post("/v1/step", {"frames": 2, "dt_secs": dt_secs})
        if out_path.exists():
            break
        time.sleep(0.05)
    if not out_path.exists():
        print(f"warn: final screenshot not written: {out_path}")

    return RunResult(run_id=run_id, run_dir=run_dir)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True, help="Path to config.toml (automation must be enabled)")
    ap.add_argument("--bin", default=None, help="Path to gravimera binary (default: target/debug/gravimera.exe)")
    ap.add_argument("--workdir", default=None, help="Working directory for the game process (default: config dir)")
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0, help="Fixed dt for /v1/step")
    ap.add_argument("--timeout-secs", type=float, default=3600.0, help="Timeout for Scene Build")
    ap.add_argument(
        "--description",
        default=None,
        help="Scene description. If omitted, read from stdin.",
    )
    args = ap.parse_args()

    config_path = Path(args.config).expanduser().resolve()
    config_text = _read_text(config_path)
    bind = _parse_automation_bind(config_text)
    api_base = f"http://{bind}"

    bin_path = Path(args.bin) if args.bin else Path("target/debug/gravimera.exe")
    if not bin_path.is_absolute():
        bin_path = (Path.cwd() / bin_path).resolve()

    workdir = Path(args.workdir).expanduser().resolve() if args.workdir else config_path.parent
    stdout_path = workdir / "gravimera_stdout.log"

    description = (args.description or "").strip()
    if not description:
        description = sys.stdin.read().strip()
    if not description:
        raise SystemExit("No description provided. Use --description or pipe via stdin.")

    interrupted = False

    def _sigint(_signum: int, _frame: Any) -> None:
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, _sigint)

    def wait_health(game: GameProcess) -> None:
        t0 = time.monotonic()
        while True:
            if not game.is_running():
                raise RuntimeError(f"Game exited early. See {stdout_path}")
            try:
                health = _http_json("GET", f"{api_base}/v1/health", None, timeout=0.5)
                if health.get("ok"):
                    break
            except Exception:
                pass
            if time.monotonic() - t0 > 30.0:
                raise RuntimeError(
                    f"Timed out waiting for Automation API on {api_base}. See {stdout_path}"
                )
            time.sleep(0.1)

    def shutdown_game(game: GameProcess) -> None:
        try:
            _http_json("POST", f"{api_base}/v1/shutdown", {}, timeout=2.0)
        except Exception:
            pass
        game.ensure_stopped()

    game = GameProcess(bin_path=bin_path, config_path=config_path, workdir=workdir, stdout_path=stdout_path)
    game.start()
    try:
        wait_health(game)
        if interrupted:
            return 130

        result = run_scene_build(
            api_base=api_base,
            description=description,
            timeout_secs=args.timeout_secs,
            dt_secs=args.dt_secs,
        )
        print(f"OK: run_id={result.run_id} run_dir={result.run_dir}")
        return 0
    finally:
        shutdown_game(game)


if __name__ == "__main__":
    raise SystemExit(main())
