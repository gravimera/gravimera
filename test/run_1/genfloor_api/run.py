#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import signal
import subprocess
import tempfile
import threading
import time
import urllib.request
from pathlib import Path


def discover_base_url_from_output(
    proc: subprocess.Popen, log_fp, timeout_secs: float = 30.0
) -> str:
    deadline = time.time() + timeout_secs
    buf = b""
    while time.time() < deadline:
        if proc.stdout is None:
            raise RuntimeError("gravimera stdout pipe is missing")

        import select

        r, _, _ = select.select([proc.stdout], [], [], 0.2)
        if not r:
            if proc.poll() is not None:
                raise RuntimeError(f"gravimera exited early (code={proc.returncode})")
            continue

        chunk = proc.stdout.readline()
        if not chunk:
            if proc.poll() is not None:
                raise RuntimeError(f"gravimera exited early (code={proc.returncode})")
            continue

        log_fp.write(chunk)
        log_fp.flush()
        buf += chunk

        line = chunk.decode("utf-8", errors="replace").strip()
        if "Rendered mode exited with an error. Falling back to headless mode." in line:
            raise RuntimeError("gravimera fell back to headless mode; expected rendered mode")
        prefix = "Automation API listening on "
        if prefix in line:
            idx = line.find(prefix)
            url = line[idx + len(prefix) :].strip()
            if url.startswith("http://"):
                return url
    raise RuntimeError(
        "Timed out waiting for Automation API listen address. "
        f"Last output:\n{buf[-4000:].decode('utf-8', errors='replace')}"
    )


def http_json(method: str, url: str, data=None, timeout_secs: float = 10.0):
    body = None
    headers = {}
    if data is not None:
        body = json.dumps(data).encode("utf-8")
        headers["Content-Type"] = "application/json"

    req = urllib.request.Request(url, method=method, data=body, headers=headers)
    with urllib.request.urlopen(req, timeout=timeout_secs) as resp:
        raw = resp.read().decode("utf-8")
        payload = json.loads(raw) if raw else {}
        return resp.status, payload


def wait_for_health(base_url: str, timeout_secs: float = 20.0):
    deadline = time.time() + timeout_secs
    last_err = None
    while time.time() < deadline:
        try:
            status, payload = http_json(
                "GET", f"{base_url}/v1/health", None, timeout_secs=2.0
            )
            if status == 200 and payload.get("ok") is True:
                return payload
            last_err = RuntimeError(
                f"health not ok: status={status} payload={payload}"
            )
        except Exception as e:
            last_err = e
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {base_url}/v1/health: {last_err}")


def ensure_ok(status: int, payload: dict, ctx: str):
    if status != 200 or payload.get("ok") is not True:
        raise RuntimeError(f"{ctx}: status={status} payload={payload}")


def step(base_url: str, frames: int, dt_secs: float):
    status, payload = http_json(
        "POST", f"{base_url}/v1/step", {"frames": frames, "dt_secs": dt_secs}, timeout_secs=300.0
    )
    ensure_ok(status, payload, "step")


def poll_genfloor_done(base_url: str, *, timeout_secs: float, dt_secs: float):
    deadline = time.time() + timeout_secs
    last = None
    while time.time() < deadline:
        step(base_url, 10, dt_secs)
        status, payload = http_json("GET", f"{base_url}/v1/genfloor/status", None, timeout_secs=10.0)
        ensure_ok(status, payload, "genfloor/status")
        last = payload
        running = bool(payload.get("running"))
        draft_ready = bool(payload.get("draft_ready"))
        if (not running) and draft_ready:
            return payload
    raise RuntimeError(f"Timed out waiting for GenFloor completion. Last status: {last}")


def main():
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parents[2]
    config_path = script_dir / "config.toml"
    bind = "127.0.0.1:0"
    dt_secs = 1.0 / 60.0

    tmp_root = script_dir / "tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix="run_", dir=str(tmp_root)))
    home_dir = run_root / ".gravimera"
    home_dir.mkdir(parents=True, exist_ok=True)

    log_path = run_root / "gravimera_stdout.log"
    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(home_dir)

    cmd = [
        "cargo",
        "run",
        "--quiet",
        "--",
        "--config",
        str(config_path),
        "--automation",
        "--automation-bind",
        bind,
        "--automation-disable-local-input",
        "--automation-pause-on-start",
    ]

    proc = None
    log_fp = None
    drain_thread = None
    ok = False
    try:
        log_fp = open(log_path, "wb")
        proc = subprocess.Popen(
            cmd,
            cwd=str(repo_root),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )

        base_url = discover_base_url_from_output(proc, log_fp, timeout_secs=120.0)

        def drain_stdout():
            if proc is None or proc.stdout is None:
                return
            while True:
                chunk = proc.stdout.readline()
                if not chunk:
                    break
                try:
                    log_fp.write(chunk)
                    log_fp.flush()
                except Exception:
                    break

        drain_thread = threading.Thread(target=drain_stdout, name="gravimera_stdout_drain")
        drain_thread.daemon = True
        drain_thread.start()

        wait_for_health(base_url, timeout_secs=30.0)

        # Enter GenFloor (Build Floor Preview scene).
        status, payload = http_json("POST", f"{base_url}/v1/mode", {"mode": "genfloor"})
        ensure_ok(status, payload, "mode genfloor")
        step(base_url, 3, dt_secs)

        # Fresh build.
        status, payload = http_json("POST", f"{base_url}/v1/genfloor/new", {})
        ensure_ok(status, payload, "genfloor/new")
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/genfloor/prompt",
            {"prompt": "A checkerboard stone floor (mock)"},
        )
        ensure_ok(status, payload, "genfloor/prompt")
        status, payload = http_json("POST", f"{base_url}/v1/genfloor/build", {})
        ensure_ok(status, payload, "genfloor/build")
        done = poll_genfloor_done(base_url, timeout_secs=30.0, dt_secs=dt_secs)

        floor_id = done.get("edit_base_floor_id_uuid")
        last_saved = done.get("last_saved_floor_id_uuid")
        if not floor_id or not last_saved or floor_id != last_saved:
            raise RuntimeError(f"Expected saved floor id. status={done}")

        # Edit run should overwrite the same floor id.
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/genfloor/prompt",
            {"prompt": "A wavy water floor (mock)"},
        )
        ensure_ok(status, payload, "genfloor/prompt edit")
        status, payload = http_json("POST", f"{base_url}/v1/genfloor/build", {})
        ensure_ok(status, payload, "genfloor/build edit")
        done2 = poll_genfloor_done(base_url, timeout_secs=30.0, dt_secs=dt_secs)

        floor_id2 = done2.get("edit_base_floor_id_uuid")
        last_saved2 = done2.get("last_saved_floor_id_uuid")
        if floor_id2 != floor_id or last_saved2 != last_saved:
            raise RuntimeError(f"Expected edit overwrite. before={done} after={done2}")

        # Back to Realm and ensure /genfloor/new clears the session.
        status, payload = http_json("POST", f"{base_url}/v1/mode", {"mode": "build"})
        ensure_ok(status, payload, "mode build")
        step(base_url, 3, dt_secs)

        status, payload = http_json("POST", f"{base_url}/v1/mode", {"mode": "genfloor"})
        ensure_ok(status, payload, "mode genfloor 2")
        step(base_url, 3, dt_secs)

        status, payload = http_json("POST", f"{base_url}/v1/genfloor/new", {})
        ensure_ok(status, payload, "genfloor/new 2")
        status, payload = http_json("GET", f"{base_url}/v1/genfloor/status", None)
        ensure_ok(status, payload, "genfloor/status after new")
        if payload.get("edit_base_floor_id_uuid") is not None:
            raise RuntimeError(f"Expected fresh session. status={payload}")
        if payload.get("draft_ready") is not False:
            raise RuntimeError(f"Expected draft_ready=false. status={payload}")
        if (payload.get("prompt") or "").strip() != "":
            raise RuntimeError(f"Expected empty prompt. status={payload}")

        # Shutdown.
        http_json("POST", f"{base_url}/v1/shutdown", {}, timeout_secs=30.0)

        ok = True
        print(f"OK: artifacts at {run_root}")
    finally:
        if proc is not None and proc.poll() is None:
            try:
                proc.send_signal(signal.SIGINT)
            except Exception:
                pass
            time.sleep(0.5)
            try:
                proc.terminate()
            except Exception:
                pass
            time.sleep(0.5)
            try:
                proc.kill()
            except Exception:
                pass
        if drain_thread is not None:
            drain_thread.join(timeout=0.5)
        if log_fp is not None:
            try:
                log_fp.close()
            except Exception:
                pass
        if not ok:
            print(f"FAILED. See log: {log_path}", file=os.sys.stderr)
            raise SystemExit(1)


if __name__ == "__main__":
    main()

