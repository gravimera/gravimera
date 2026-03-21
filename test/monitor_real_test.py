#!/usr/bin/env python3
"""
Real (rendered) monitor smoke test via the local Automation HTTP API.

This script:
- Starts Gravimera with a test config (Automation enabled)
- Isolates all disk writes under a temporary GRAVIMERA_HOME
- Creates/switches to a new scene
- Lists prefabs, spawns a unit, posts a toast + TTS speak with a bubble
- Forces a scene save, despawns the unit, and shuts down cleanly
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
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
    raise ValueError(
        'config.toml: missing [automation].bind (example: bind = "127.0.0.1:18791")'
    )


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
    def __init__(
        self, *, bin_path: Path, config_path: Path, workdir: Path, stdout_path: Path, env: dict[str, str]
    ):
        self._bin_path = bin_path
        self._config_path = config_path
        self._workdir = workdir
        self._stdout_path = stdout_path
        self._env = env
        self._proc: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self._stdout_path.parent.mkdir(parents=True, exist_ok=True)
        out = open(self._stdout_path, "wb")
        self._proc = subprocess.Popen(
            [str(self._bin_path), "--config", str(self._config_path)],
            cwd=str(self._workdir),
            stdout=out,
            stderr=subprocess.STDOUT,
            env=self._env,
        )

    def is_running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def terminate(self) -> None:
        if self._proc is None or self._proc.poll() is not None:
            return
        try:
            self._proc.terminate()
        except Exception:
            pass

    def kill(self) -> None:
        if self._proc is None or self._proc.poll() is not None:
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


def _default_bin_path() -> Path:
    exe = "gravimera.exe" if sys.platform.startswith("win") else "gravimera"
    return Path("target/debug") / exe


def wait_health(api_base: str, game: GameProcess, *, stdout_path: Path, timeout_secs: float = 30.0) -> None:
    t0 = time.monotonic()
    while True:
        if not game.is_running():
            raise RuntimeError(f"Game exited early. See {stdout_path}")
        try:
            health = _http_json("GET", f"{api_base}/v1/health", None, timeout=0.5)
            if health.get("ok"):
                return
        except Exception:
            pass
        if time.monotonic() - t0 > timeout_secs:
            raise RuntimeError(f"Timed out waiting for Automation API on {api_base}. See {stdout_path}")
        time.sleep(0.1)


def wait_listen_addr(
    game: GameProcess,
    *,
    stdout_path: Path,
    timeout_secs: float = 30.0,
) -> str:
    pat = re.compile(r"Automation API listening on (http://\S+)")
    bind_err_pat = re.compile(r"Automation API: failed to bind (.+?): (.+)$")
    t0 = time.monotonic()

    last_size = 0
    while True:
        if not game.is_running():
            raise RuntimeError(f"Game exited early. See {stdout_path}")

        try:
            if stdout_path.exists():
                data = stdout_path.read_text(encoding="utf-8", errors="replace")
                m = pat.search(data)
                if m:
                    return m.group(1).strip()

                # Helpful early failure if bind failed.
                m2 = bind_err_pat.search(data)
                if m2:
                    raise RuntimeError(f"Automation bind failed: {m2.group(1)}: {m2.group(2)}")

                last_size = len(data)
        except Exception:
            pass

        if time.monotonic() - t0 > timeout_secs:
            raise RuntimeError(
                f"Timed out waiting for Automation listen addr. See {stdout_path} (size={last_size})"
            )
        time.sleep(0.1)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", default="test/monitor_test_config.toml", help="Path to config.toml")
    ap.add_argument("--bin", default=None, help="Path to gravimera binary (default: target/debug/gravimera)")
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0, help="Fixed dt for /v1/step")
    args = ap.parse_args()

    config_path = Path(args.config).expanduser().resolve()

    bin_path = Path(args.bin).expanduser().resolve() if args.bin else (Path.cwd() / _default_bin_path()).resolve()
    if not bin_path.exists():
        raise SystemExit(f"Missing gravimera binary at {bin_path}. Run `cargo build` first or pass --bin.")

    tmp_root = Path(tempfile.mkdtemp(prefix="gravimera_monitor_real_test_")).resolve()
    gravimera_home = tmp_root / ".gravimera"
    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(gravimera_home)

    stdout_path = tmp_root / "gravimera_stdout.log"
    workdir = config_path.parent

    interrupted = False

    def _sigint(_signum: int, _frame: Any) -> None:
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, _sigint)

    game = GameProcess(
        bin_path=bin_path,
        config_path=config_path,
        workdir=workdir,
        stdout_path=stdout_path,
        env=env,
    )
    game.start()

    api_base: str | None = None

    def shutdown_game() -> None:
        if api_base:
            try:
                _http_json("POST", f"{api_base}/v1/shutdown", {}, timeout=2.0)
            except Exception:
                pass
        game.ensure_stopped()

    try:
        api_base = wait_listen_addr(game, stdout_path=stdout_path)
        wait_health(api_base, game, stdout_path=stdout_path)
        if interrupted:
            return 130

        def post(path: str, body: dict[str, Any]) -> dict[str, Any]:
            timeout = 300.0 if path in ("/v1/step", "/v1/shutdown") else 30.0
            return _http_json("POST", f"{api_base}{path}", body, timeout=timeout)

        def get(path: str) -> dict[str, Any]:
            return _http_json("GET", f"{api_base}{path}", None, timeout=30.0)

        disc = get("/v1/discovery")
        if not disc.get("ok"):
            raise RuntimeError(f"discovery not ok: {disc}")
        if not bool(((disc.get("features") or {}).get("monitor_mode"))):
            raise RuntimeError(f"discovery did not report monitor_mode=true: {disc}")

        health = get("/v1/health")
        automation = health.get("automation") or {}
        if not bool(automation.get("monitor_mode")):
            raise RuntimeError(f"health did not report automation.monitor_mode=true: {health}")

        scene_id = "MonitorTest"
        post(
            "/v1/realm_scene/create",
            {
                "scene_id": scene_id,
                "label": scene_id,
                "description": "Automation monitor real test scene",
                "switch_to": True,
            },
        )
        post("/v1/step", {"frames": 5, "dt_secs": args.dt_secs})

        # Wait for the deferred switch to apply.
        for _ in range(20):
            active = get("/v1/realm_scene/active")
            if active.get("scene_id") == scene_id:
                break
            post("/v1/step", {"frames": 2, "dt_secs": args.dt_secs})
        else:
            raise RuntimeError(f"Timed out waiting for scene switch to {scene_id}. Active: {active}")

        initial_state = get("/v1/state")
        if initial_state.get("objects"):
            raise RuntimeError(
                f"Newly created monitor scene should start empty, got objects={initial_state.get('objects')}"
            )

        prefabs = get("/v1/prefabs")
        items = prefabs.get("prefabs") or []
        human_id = None
        for p in items:
            if str(p.get("label") or "").strip().lower() == "human":
                human_id = str(p.get("prefab_id_uuid") or "").strip()
                break
        if not human_id:
            raise RuntimeError("Failed to find prefab with label 'Human' in /v1/prefabs")

        # Also validate build-object move via /v1/move (teleport).
        build_prefab_id = None
        for p in items:
            if p.get("mobility") is False and str(p.get("prefab_id_uuid") or "").strip():
                build_prefab_id = str(p.get("prefab_id_uuid") or "").strip()
                break
        if not build_prefab_id:
            raise RuntimeError("Failed to find a non-mobility prefab for build-object move test.")

        spawned = post("/v1/spawn", {"prefab_id_uuid": human_id, "x": 0.0, "z": 0.0, "yaw": 0.0})
        instance_id = str(spawned.get("instance_id_uuid") or "").strip()
        if not instance_id:
            raise RuntimeError(f"spawn returned no instance_id_uuid: {spawned}")

        build_spawned = post(
            "/v1/spawn", {"prefab_id_uuid": build_prefab_id, "x": 2.0, "z": 0.0, "yaw": 0.0}
        )
        build_instance_id = str(build_spawned.get("instance_id_uuid") or "").strip()
        if not build_instance_id:
            raise RuntimeError(f"build spawn returned no instance_id_uuid: {build_spawned}")
        post("/v1/step", {"frames": 2, "dt_secs": args.dt_secs})

        post("/v1/select", {"instance_ids": [build_instance_id]})
        post("/v1/move", {"x": -3.0, "z": 1.0})
        post("/v1/step", {"frames": 2, "dt_secs": args.dt_secs})
        state = get("/v1/state")
        moved = next(
            (o for o in (state.get("objects") or []) if o.get("instance_id_uuid") == build_instance_id),
            None,
        )
        if not moved:
            raise RuntimeError("Moved build object not found in /v1/state.")
        pos = moved.get("pos") or []
        if len(pos) != 3 or abs(float(pos[0]) - (-3.0)) > 0.08 or abs(float(pos[2]) - 1.0) > 0.08:
            raise RuntimeError(f"Build object did not move as expected. pos={pos}")

        post("/v1/ui/toast", {"text": "Monitor test toast ✅", "kind": "info", "ttl_secs": 1.2})
        post(
            "/v1/speak",
            {
                "content": "Monitor test speaking.",
                "voice": "dog",
                "volume": 1.0,
                "instance_id_uuid": instance_id,
                "bubble": True,
            },
        )

        post("/v1/step", {"frames": 4, "dt_secs": args.dt_secs})
        post("/v1/scene/save", {})
        post("/v1/despawn", {"instance_id_uuid": instance_id})
        post("/v1/step", {"frames": 2, "dt_secs": args.dt_secs})

        return 0
    finally:
        shutdown_game()


if __name__ == "__main__":
    raise SystemExit(main())
