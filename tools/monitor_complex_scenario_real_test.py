#!/usr/bin/env python3
"""
Complex rendered monitor scenario test via the local Gravimera Automation HTTP API.

This script is a "stand-in external agent" that exercises the monitor-oriented
API surface:

- /v1/discovery
- /v1/realm_scene/create + /v1/realm_scene/switch (deferred) + /v1/realm_scene/active
- /v1/prefabs + /v1/spawn + /v1/despawn
- /v1/ui/toast (rendered-only)
- /v1/speak (built-in TTS from text; optional speech bubble)
- /v1/scene/save

It also validates a few important error cases (id sanitization, speak limits,
double-despawn behavior) and captures screenshots for later review.

All artifacts are written under --run-dir (default: test/run_1/...):
  - config.toml
  - gravimera_stdout.log
  - shots/*.png
  - .gravimera/ (GRAVIMERA_HOME; contains the persisted realm/scene)
"""

from __future__ import annotations

import argparse
import json
import os
import re
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Any


def _now_tag() -> str:
    return time.strftime("%Y%m%d_%H%M%S", time.localtime())


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _http_json_status(
    method: str,
    url: str,
    body: dict[str, Any] | None = None,
    timeout: float = 30.0,
) -> tuple[int, dict[str, Any] | str]:
    # Some host environments (including agent runners) may set HTTP(S)_PROXY
    # env vars; urllib will honor them by default, which can break loopback
    # requests. Use an explicit "no proxy" opener for local automation calls.
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    data = None
    headers = {"Content-Type": "application/json"}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with opener.open(req, timeout=timeout) as resp:
            raw = resp.read().decode("utf-8", errors="replace")
            if not raw.strip():
                return resp.status, {}
            return resp.status, json.loads(raw)
    except urllib.error.HTTPError as err:
        raw = err.read().decode("utf-8", errors="replace")
        try:
            return err.code, json.loads(raw)
        except Exception:
            return err.code, raw.strip()
    except urllib.error.URLError as err:
        raise RuntimeError(f"Request failed {url}: {err}") from None


def _http_json(
    method: str,
    url: str,
    body: dict[str, Any] | None = None,
    timeout: float = 30.0,
) -> dict[str, Any]:
    status, payload = _http_json_status(method, url, body, timeout)
    if status < 200 or status >= 300:
        raise RuntimeError(f"HTTP {status} {url}: {payload}")
    if not isinstance(payload, dict):
        raise RuntimeError(f"Non-JSON response from {url}: {payload}")
    return payload


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


def _default_bin_path(repo_root: Path) -> Path:
    exe = "gravimera.exe" if sys.platform.startswith("win") else "gravimera"
    return repo_root / "target" / "debug" / exe


def wait_listen_addr(
    game: GameProcess,
    *,
    stdout_path: Path,
    timeout_secs: float = 45.0,
) -> str:
    pat = re.compile(r"Automation API listening on (http://\S+)")
    bind_err_pat = re.compile(r"Automation API: failed to bind (.+?): (.+)$")
    t0 = time.monotonic()
    last_size = 0

    while True:
        if not game.is_running():
            raise RuntimeError(f"Game exited early. See {stdout_path}")

        if stdout_path.exists():
            data = stdout_path.read_text(encoding="utf-8", errors="replace")
            m = pat.search(data)
            if m:
                return m.group(1).strip()
            m2 = bind_err_pat.search(data)
            if m2:
                raise RuntimeError(f"Automation bind failed: {m2.group(1)}: {m2.group(2)}")
            last_size = len(data)

        if time.monotonic() - t0 > timeout_secs:
            raise RuntimeError(
                f"Timed out waiting for Automation listen addr. See {stdout_path} (size={last_size})"
            )
        time.sleep(0.1)


def wait_health(api_base: str, game: GameProcess, *, stdout_path: Path, timeout_secs: float = 45.0) -> None:
    t0 = time.monotonic()
    last_err: str | None = None
    while True:
        if not game.is_running():
            raise RuntimeError(f"Game exited early. See {stdout_path}")
        try:
            health = _http_json("GET", f"{api_base}/v1/health", None, timeout=0.75)
            if health.get("ok"):
                return
        except Exception as err:
            last_err = str(err)
        if time.monotonic() - t0 > timeout_secs:
            detail = f" last_err={last_err!r}" if last_err else ""
            raise RuntimeError(
                f"Timed out waiting for Automation API on {api_base}. See {stdout_path}.{detail}"
            )
        time.sleep(0.1)


def _pick_prefab_id(
    prefabs: list[dict[str, Any]],
    *,
    want_mobility: bool,
    prefer_labels: list[str],
) -> str | None:
    def _norm(s: str) -> str:
        return s.strip().lower()

    prefer = {_norm(s) for s in prefer_labels}
    for p in prefabs:
        if bool(p.get("mobility")) != want_mobility:
            continue
        label = _norm(str(p.get("label") or ""))
        if label and label in prefer:
            return str(p.get("prefab_id_uuid") or "").strip() or None
    for p in prefabs:
        if bool(p.get("mobility")) != want_mobility:
            continue
        pid = str(p.get("prefab_id_uuid") or "").strip()
        if pid:
            return pid
    return None


def _find_object(state: dict[str, Any], instance_id_uuid: str) -> dict[str, Any] | None:
    for obj in state.get("objects") or []:
        if str(obj.get("instance_id_uuid") or "").strip() == instance_id_uuid:
            return obj
    return None


def main() -> int:
    repo_root = Path(__file__).resolve().parents[1]

    ap = argparse.ArgumentParser()
    ap.add_argument("--bin", default=None, help="Path to gravimera binary (default: target/debug/gravimera)")
    ap.add_argument(
        "--run-dir",
        default=None,
        help="Run artifacts directory (default: test/run_1/monitor_complex_<timestamp>)",
    )
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0, help="Fixed dt for /v1/step")
    args = ap.parse_args()

    run_dir = (
        Path(args.run_dir).expanduser().resolve()
        if args.run_dir
        else (repo_root / "test" / "run_1" / f"monitor_complex_{_now_tag()}").resolve()
    )
    run_dir.mkdir(parents=True, exist_ok=True)
    print(f"RUN_DIR={run_dir}")

    config_path = run_dir / "config.toml"
    config_path.write_text(
        "\n".join(
            [
                "[automation]",
                "enabled = true",
                'bind = "127.0.0.1:0"',
                "disable_local_input = true",
                "pause_on_start = true",
                "",
            ]
        ),
        encoding="utf-8",
    )

    bin_path = (
        Path(args.bin).expanduser().resolve() if args.bin else _default_bin_path(repo_root).resolve()
    )
    if not bin_path.exists():
        raise SystemExit(f"Missing gravimera binary at {bin_path}. Run `cargo build` first or pass --bin.")

    gravimera_home = run_dir / ".gravimera"
    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(gravimera_home)

    stdout_path = run_dir / "gravimera_stdout.log"
    workdir = repo_root

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
                _http_json("POST", f"{api_base}/v1/shutdown", {}, timeout=3.0)
            except Exception:
                pass
        game.ensure_stopped()

    def post(path: str, body: dict[str, Any], *, timeout: float = 30.0) -> dict[str, Any]:
        # /v1/step and /v1/shutdown can take longer.
        if path in ("/v1/step", "/v1/shutdown"):
            timeout = max(timeout, 300.0)
        return _http_json("POST", f"{api_base}{path}", body, timeout=timeout)

    def post_expect_status(
        path: str, body: dict[str, Any], *, want_status: int, timeout: float = 30.0
    ) -> dict[str, Any] | str:
        if path in ("/v1/step", "/v1/shutdown"):
            timeout = max(timeout, 300.0)
        status, payload = _http_json_status("POST", f"{api_base}{path}", body, timeout)
        if status != want_status:
            raise RuntimeError(f"Expected HTTP {want_status} for {path}, got {status}: {payload}")
        return payload

    def get(path: str, *, timeout: float = 30.0) -> dict[str, Any]:
        return _http_json("GET", f"{api_base}{path}", None, timeout=timeout)

    def step(frames: int) -> None:
        post("/v1/step", {"frames": int(frames), "dt_secs": float(args.dt_secs)})

    def screenshot(rel_path: str) -> Path:
        out = (run_dir / rel_path).resolve()
        post("/v1/screenshot", {"path": str(out)})
        # Saving is async: step a few frames so the file is written.
        step(3)
        if not out.exists():
            raise RuntimeError(f"Screenshot not written yet: {out} (see {stdout_path})")
        return out

    try:
        api_base = wait_listen_addr(game, stdout_path=stdout_path)
        wait_health(api_base, game, stdout_path=stdout_path)
        if interrupted:
            return 130

        # 0) Discovery
        disc = get("/v1/discovery")
        if not disc.get("ok"):
            raise RuntimeError(f"/v1/discovery not ok: {disc}")
        features = disc.get("features") or {}
        if not (features.get("ui_toast") and features.get("tts") and features.get("realm_scene_switch")):
            raise RuntimeError(f"Missing required features in /v1/discovery: {features}")

        # 1) Sanity: reject invalid scene id.
        post_expect_status(
            "/v1/realm_scene/create",
            {"scene_id": "bad/id", "label": "bad", "description": "bad", "switch_to": False},
            want_status=400,
        )

        # 2) Create + switch to monitor scene (deferred)
        scene_id = f"AgentMonitor_{_now_tag()}"
        post(
            "/v1/realm_scene/create",
            {
                "scene_id": scene_id,
                "label": scene_id,
                "description": "Complex monitor scenario (automation real test)",
                "switch_to": True,
            },
        )
        step(6)

        # Wait for deferred switch.
        for _ in range(30):
            active = get("/v1/realm_scene/active")
            if active.get("scene_id") == scene_id:
                break
            step(2)
        else:
            raise RuntimeError(f"Timed out waiting for scene switch to {scene_id}. Active: {active}")

        # 3) Prefab discovery
        prefabs = get("/v1/prefabs")
        items = list(prefabs.get("prefabs") or [])
        if not items:
            raise RuntimeError("No prefabs returned by /v1/prefabs")

        unit_prefab_id = _pick_prefab_id(items, want_mobility=True, prefer_labels=["human", "bot", "robot"])
        if not unit_prefab_id:
            raise RuntimeError("Failed to find a mobility=true unit prefab in /v1/prefabs")

        prop_prefab_id = _pick_prefab_id(
            items,
            want_mobility=False,
            prefer_labels=["crate", "box", "barrel", "paper", "rock", "stone"],
        )
        if not prop_prefab_id:
            prop_prefab_id = unit_prefab_id

        # 4) Spawn a coordinator + multiple workers (replication)
        coordinator = post("/v1/spawn", {"prefab_id_uuid": unit_prefab_id, "x": 0.0, "z": 0.0, "yaw": 0.0})
        coordinator_id = str(coordinator.get("instance_id_uuid") or "").strip()
        if not coordinator_id:
            raise RuntimeError(f"spawn(coordinator) returned no instance_id_uuid: {coordinator}")

        workers: list[str] = []
        for i, (x, z) in enumerate([(3.0, 0.0), (0.0, 3.0), (-3.0, 0.0), (0.0, -3.0)]):
            w = post("/v1/spawn", {"prefab_id_uuid": unit_prefab_id, "x": x, "z": z, "yaw": 0.0})
            wid = str(w.get("instance_id_uuid") or "").strip()
            if not wid:
                raise RuntimeError(f"spawn(worker {i}) returned no instance_id_uuid: {w}")
            workers.append(wid)

        step(4)
        screenshot("shots/00_boot.png")

        # 5) Plan stage (papers on the floor + coordinator speaks)
        post("/v1/ui/toast", {"text": "Monitor connected. Planning work… 📝", "kind": "info", "ttl_secs": 2.5})
        post(
            "/v1/speak",
            {
                "content": "Initializing monitor scene and splitting into parallel tasks.",
                "voice": "dog",
                "volume": 1.0,
                "instance_id_uuid": coordinator_id,
                "bubble": True,
            },
        )
        papers: list[str] = []
        for x, z in [(-0.8, 0.6), (0.0, 0.8), (0.8, 0.6)]:
            p = post("/v1/spawn", {"prefab_id_uuid": prop_prefab_id, "x": x, "z": z, "yaw": 0.0})
            pid = str(p.get("instance_id_uuid") or "").strip()
            if pid:
                papers.append(pid)
        step(8)
        screenshot("shots/01_planning.png")

        # 6) Parallel tasks: Search / Collect / Analyze / Build
        task_labels = ["Search 🔍", "Collect 📦", "Analyze 🧠", "Build 🧰"]
        task_props: list[str] = []
        for wid, label, (dx, dz) in zip(workers, task_labels, [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)]):
            post("/v1/ui/toast", {"text": f"{label}: starting", "kind": "info", "ttl_secs": 2.0})
            post(
                "/v1/speak",
                {
                    "content": f"{label}: working now.",
                    "voice": "cow" if "Collect" in label else "dog",
                    "volume": 1.0,
                    "instance_id_uuid": wid,
                    "bubble": True,
                },
            )
            # Visualize work with a couple of props near the worker.
            wstate = get("/v1/state")
            obj = _find_object(wstate, wid)
            if obj and isinstance(obj.get("pos"), list) and len(obj["pos"]) >= 3:
                wx, wz = float(obj["pos"][0]), float(obj["pos"][2])
            else:
                wx, wz = 0.0, 0.0
            for j, (ox, oz) in enumerate([(dx, dz), (dx * 0.5, dz * 0.5)]):
                t = post(
                    "/v1/spawn",
                    {"prefab_id_uuid": prop_prefab_id, "x": wx + ox, "z": wz + oz, "yaw": 0.0},
                )
                tid = str(t.get("instance_id_uuid") or "").strip()
                if not tid:
                    raise RuntimeError(f"spawn(task prop {label} #{j}) returned no instance_id_uuid: {t}")
                task_props.append(tid)
            step(3)

        step(10)
        screenshot("shots/02_parallel_tasks.png")

        # 7) Movement orders (prove selection + move works; accept "moved some distance", not exact arrival)
        move_targets = [(6.0, 0.0), (0.0, 6.0), (-6.0, 0.0), (0.0, -6.0)]
        before_state = get("/v1/state")
        before_pos: dict[str, tuple[float, float]] = {}
        for wid in workers:
            obj = _find_object(before_state, wid)
            if obj and isinstance(obj.get("pos"), list) and len(obj["pos"]) >= 3:
                before_pos[wid] = (float(obj["pos"][0]), float(obj["pos"][2]))

        for wid, (tx, tz) in zip(workers, move_targets):
            post("/v1/select", {"instance_ids": [wid]})
            post("/v1/move", {"x": float(tx), "z": float(tz)})
            step(20)

        step(30)
        after_state = get("/v1/state")
        for wid in workers:
            if wid not in before_pos:
                continue
            obj = _find_object(after_state, wid)
            if not (obj and isinstance(obj.get("pos"), list) and len(obj["pos"]) >= 3):
                raise RuntimeError(f"Worker missing after move: {wid}")
            ax, az = float(obj["pos"][0]), float(obj["pos"][2])
            bx, bz = before_pos[wid]
            moved = abs(ax - bx) + abs(az - bz)
            if moved < 0.15:
                raise RuntimeError(f"Worker did not appear to move: {wid} before=({bx:.2f},{bz:.2f}) after=({ax:.2f},{az:.2f})")

        screenshot("shots/03_after_moves.png")

        # 8) Speak validation: reject too-long content.
        too_long = "a" * 801
        post_expect_status(
            "/v1/speak",
            {"content": too_long, "voice": "dog", "volume": 1.0, "bubble": False},
            want_status=400,
        )

        # 9) Save + persistence check via scene switch away/back.
        post("/v1/scene/save", {})
        step(5)

        def _count_prefab(state: dict[str, Any], prefab_id_uuid: str) -> int:
            n = 0
            for obj in state.get("objects") or []:
                if str(obj.get("prefab_id_uuid") or "").strip() == prefab_id_uuid:
                    n += 1
            return n

        snap_before = get("/v1/state")
        units_before = _count_prefab(snap_before, unit_prefab_id)
        props_before = _count_prefab(snap_before, prop_prefab_id)

        tmp_scene_id = f"{scene_id}_Tmp"
        post("/v1/realm_scene/create", {"scene_id": tmp_scene_id, "label": tmp_scene_id, "switch_to": True})
        step(6)
        for _ in range(30):
            active = get("/v1/realm_scene/active")
            if active.get("scene_id") == tmp_scene_id:
                break
            step(2)
        else:
            raise RuntimeError(f"Timed out waiting for scene switch to {tmp_scene_id}")

        # Switch back.
        post("/v1/realm_scene/switch", {"scene_id": scene_id})
        step(6)
        for _ in range(30):
            active = get("/v1/realm_scene/active")
            if active.get("scene_id") == scene_id:
                break
            step(2)
        else:
            raise RuntimeError(f"Timed out waiting for scene switch back to {scene_id}")

        snap_after = get("/v1/state")
        units_after = _count_prefab(snap_after, unit_prefab_id)
        props_after = _count_prefab(snap_after, prop_prefab_id)
        if units_after < max(1, units_before):
            raise RuntimeError(f"Scene persistence failed for units: before={units_before} after={units_after}")
        if props_after < max(0, props_before - 1):
            raise RuntimeError(f"Scene persistence suspicious for props: before={props_before} after={props_after}")

        screenshot("shots/04_after_scene_roundtrip.png")

        # 10) Cleanup: despawn one prop and one unit; validate double-despawn => 404.
        if task_props:
            victim_prop = task_props[0]
            post("/v1/despawn", {"instance_id_uuid": victim_prop})
            step(2)
            post_expect_status("/v1/despawn", {"instance_id_uuid": victim_prop}, want_status=404)

        victim_unit = workers[-1]
        post("/v1/despawn", {"instance_id_uuid": victim_unit})
        step(2)
        post_expect_status("/v1/despawn", {"instance_id_uuid": victim_unit}, want_status=404)

        post("/v1/scene/save", {})
        step(6)
        screenshot("shots/05_final.png")

        return 0
    finally:
        shutdown_game()


if __name__ == "__main__":
    raise SystemExit(main())
