#!/usr/bin/env python3

import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request
from pathlib import Path


def discover_base_url_from_output(proc: subprocess.Popen, log_fp, timeout_secs: float = 30.0) -> str:
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
        "Timed out waiting for Automation API listen address. Last output:\n"
        + buf[-4000:].decode("utf-8", errors="replace")
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


def wait_for_health(base_url: str, timeout_secs: float = 30.0):
    deadline = time.time() + timeout_secs
    last_err = None
    while time.time() < deadline:
        try:
            status, payload = http_json("GET", f"{base_url}/v1/health", None, timeout_secs=2.0)
            if status == 200 and payload.get("ok") is True:
                return payload
            last_err = RuntimeError(f"health not ok: status={status} payload={payload}")
        except Exception as e:
            last_err = e
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {base_url}/v1/health: {last_err}")


def wait_for_preview_mode(base_url: str, timeout_secs: float = 20.0):
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        status, payload = http_json("GET", f"{base_url}/v1/state", None)
        if status == 200 and payload.get("ok") is True:
            if payload.get("mode") == "build" and payload.get("build_scene") == "preview":
                return payload
        http_json("POST", f"{base_url}/v1/step", {"frames": 5})
    raise RuntimeError("Timed out waiting for build preview mode")


def wait_for_gen3d_done(base_url: str, timeout_secs: float = 240.0):
    deadline = time.time() + timeout_secs
    last = None
    while time.time() < deadline:
        status, payload = http_json("GET", f"{base_url}/v1/gen3d/status", None)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"gen3d status failed: status={status} payload={payload}")
        last = payload

        if payload.get("error"):
            raise RuntimeError(f"gen3d run failed: {payload.get('error')}")

        if payload.get("running") is False and payload.get("build_complete") is True:
            return payload

        http_json("POST", f"{base_url}/v1/step", {"frames": 5})

    raise RuntimeError(f"Timed out waiting for Gen3D run to complete. Last status: {last}")


def main():
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parents[2]
    config_path = script_dir / "config.toml"
    bind = "127.0.0.1:0"

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

        # Switch to Build Preview scene for single-session Gen3D endpoints.
        status, payload = http_json("POST", f"{base_url}/v1/mode", {"mode": "gen3d"})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"failed to switch mode: status={status} payload={payload}")
        wait_for_preview_mode(base_url, timeout_secs=30.0)

        # Start a mock build run and save it so we have a Gen3D-saved prefab to edit.
        status, payload = http_json("POST", f"{base_url}/v1/gen3d/prompt", {"prompt": "A mock unit"})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"prompt failed: status={status} payload={payload}")

        status, payload = http_json("POST", f"{base_url}/v1/gen3d/build", {})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"build failed: status={status} payload={payload}")

        wait_for_gen3d_done(base_url, timeout_secs=240.0)

        status, payload = http_json("POST", f"{base_url}/v1/gen3d/save", {})
        if status != 200 or payload.get("ok") is not True or not payload.get("prefab_id_uuid"):
            raise RuntimeError(f"save failed: status={status} payload={payload}")
        prefab_id_uuid = payload["prefab_id_uuid"]

        # Seed an Edit session from that prefab.
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/edit_from_prefab",
            {"prefab_id_uuid": prefab_id_uuid},
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"edit_from_prefab failed: status={status} payload={payload}")

        status, payload = http_json("POST", f"{base_url}/v1/gen3d/prompt", {"prompt": "Add extra armor (mock)"})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"prompt failed: status={status} payload={payload}")

        # Run the edit once.
        status, payload = http_json("POST", f"{base_url}/v1/gen3d/resume", {})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"resume failed: status={status} payload={payload}")
        first_run_id = payload.get("run_id")

        first_done = wait_for_gen3d_done(base_url, timeout_secs=240.0)
        first_run_dir = first_done.get("run_dir")
        if not first_run_id or not first_run_dir:
            raise RuntimeError(f"missing run_id/run_dir after first edit run: {first_done}")

        # Run the edit again; should start a fresh run dir (new cache folder).
        status, payload = http_json("POST", f"{base_url}/v1/gen3d/resume", {})
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"resume failed: status={status} payload={payload}")
        second_run_id = payload.get("run_id")

        status, second_status = http_json("GET", f"{base_url}/v1/gen3d/status", None)
        if status != 200 or second_status.get("ok") is not True:
            raise RuntimeError(f"status failed: status={status} payload={second_status}")
        second_run_dir = second_status.get("run_dir")

        if not second_run_id or not second_run_dir:
            raise RuntimeError(f"missing run_id/run_dir after second resume: {second_status}")
        if second_run_id == first_run_id:
            raise RuntimeError(f"expected new run_id on rerun; got same: {second_run_id}")
        if second_run_dir == first_run_dir:
            raise RuntimeError(f"expected new run_dir on rerun; got same: {second_run_dir}")

        wait_for_gen3d_done(base_url, timeout_secs=240.0)

        # Graceful shutdown.
        try:
            http_json("POST", f"{base_url}/v1/shutdown", {})
        except Exception:
            pass

        try:
            proc.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            proc.terminate()

        print("OK: Gen3D Edit rerun starts new cache folder (mock://gen3d)")
        ok = True
        return 0
    finally:
        if proc is not None and proc.poll() is None:
            try:
                proc.send_signal(signal.SIGINT)
                proc.wait(timeout=5.0)
            except Exception:
                proc.kill()
        if drain_thread is not None:
            try:
                drain_thread.join(timeout=2.0)
            except Exception:
                pass
        if log_fp is not None:
            log_fp.close()
        if ok:
            try:
                import shutil

                shutil.rmtree(run_root, ignore_errors=True)
            except Exception:
                pass
        else:
            print(f"Artifacts kept at: {run_root}", file=sys.stderr)
            print(f"Log: {log_path}", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())

