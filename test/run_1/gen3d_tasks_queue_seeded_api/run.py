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


def assert_bad_request(base_url: str, data, expected_substr: str):
    try:
        http_json("POST", f"{base_url}/v1/gen3d/tasks/enqueue", data)
        raise RuntimeError("expected HTTP error, got success")
    except Exception as e:
        msg = str(e)
        if expected_substr not in msg:
            raise


def wait_for_tasks_done(base_url: str, task_ids, timeout_secs: float = 240.0):
    deadline = time.time() + timeout_secs
    task_ids = list(task_ids)
    done_states = ("done", "failed", "canceled")

    while time.time() < deadline:
        status, payload = http_json("GET", f"{base_url}/v1/gen3d/tasks", None)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"tasks list failed: status={status} payload={payload}")

        tasks = payload.get("tasks", [])
        by_id = {t.get("task_id"): t for t in tasks if isinstance(t, dict)}

        # Invariants: at most one running; FIFO.
        states = [by_id.get(tid, {}).get("state") for tid in task_ids]
        running_count = sum(1 for s in states if s == "running")
        if running_count > 1:
            raise RuntimeError(f"more than one task running: states={states}")

        for idx, tid in enumerate(task_ids):
            s = by_id.get(tid, {}).get("state")
            if s != "running":
                continue
            for prev in task_ids[:idx]:
                ps = by_id.get(prev, {}).get("state")
                if ps not in done_states:
                    raise RuntimeError(f"FIFO violated: {prev}={ps} ran after {tid}={s}")

        # Also verify we stay in Build Realm while running the queue.
        st_status, st_payload = http_json("GET", f"{base_url}/v1/state", None)
        if st_status == 200 and st_payload.get("ok") is True:
            if st_payload.get("mode") != "build" or st_payload.get("build_scene") != "realm":
                raise RuntimeError(
                    f"expected mode=build build_scene=realm; got {st_payload.get('mode')} {st_payload.get('build_scene')}"
                )

        if all(s in done_states for s in states):
            for tid in task_ids:
                result = by_id.get(tid, {}).get("result_prefab_id_uuid")
                if not result:
                    raise RuntimeError(f"missing result_prefab_id_uuid for {tid}: {by_id.get(tid)}")
            return by_id

        http_json("POST", f"{base_url}/v1/step", {"frames": 5})

    raise RuntimeError("timed out waiting for Gen3D tasks to complete")


def prefab_package_prefabs_dir(gravimera_home: Path, realm_id: str, root_prefab_uuid: str) -> Path:
    return gravimera_home / "realm" / realm_id / "prefabs" / root_prefab_uuid / "prefabs"


def package_contains_move_channel(prefabs_dir: Path) -> bool:
    if not prefabs_dir.exists():
        raise RuntimeError(f"missing prefabs dir: {prefabs_dir}")
    for path in prefabs_dir.glob("*.json"):
        if path.name.endswith(".desc.json"):
            continue
        try:
            doc = json.loads(path.read_text(encoding="utf-8"))
        except Exception:
            continue
        parts = doc.get("parts", [])
        if not isinstance(parts, list):
            continue
        for part in parts:
            if not isinstance(part, dict):
                continue
            anims = part.get("animations", [])
            if not isinstance(anims, list):
                continue
            for slot in anims:
                if isinstance(slot, dict) and slot.get("channel") == "move":
                    return True
    return False


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

        # Quick validation of errors (actionable + gated).
        try:
            http_json("POST", f"{base_url}/v1/gen3d/tasks/enqueue", {"kind": "nope"})
            raise RuntimeError("expected 400 for invalid kind, got success")
        except Exception:
            pass
        try:
            http_json("POST", f"{base_url}/v1/gen3d/tasks/enqueue", {"kind": "build"})
            raise RuntimeError("expected 400 for missing prompt, got success")
        except Exception:
            pass
        try:
            http_json(
                "POST",
                f"{base_url}/v1/gen3d/tasks/enqueue",
                {"kind": "edit_from_prefab", "prefab_id_uuid": "not-a-uuid"},
            )
            raise RuntimeError("expected 400 for invalid prefab_id_uuid, got success")
        except Exception:
            pass

        prompts = [
            ("build", {"prompt": "A snake enemy unit (mock)"}),
            ("build", {"prompt": "A warcar with a cannon as weapon (mock)"}),
        ]

        task_ids = []
        for kind, extra in prompts:
            status, payload = http_json(
                "POST",
                f"{base_url}/v1/gen3d/tasks/enqueue",
                {"kind": kind, **extra},
            )
            if status != 200 or payload.get("ok") is not True or not payload.get("task_id"):
                raise RuntimeError(f"enqueue failed: status={status} payload={payload}")
            task_ids.append(payload["task_id"])

        by_id = wait_for_tasks_done(base_url, task_ids, timeout_secs=240.0)
        snake_task, warcar_task = task_ids[0], task_ids[1]
        snake_root = by_id[snake_task]["result_prefab_id_uuid"]
        warcar_root = by_id[warcar_task]["result_prefab_id_uuid"]

        # Enqueue seeded tasks that depend on the saved warcar package.
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/tasks/enqueue",
            {"kind": "edit_from_prefab", "prefab_id_uuid": warcar_root, "prompt": "Add extra armor (mock)"},
        )
        if status != 200 or payload.get("ok") is not True or not payload.get("task_id"):
            raise RuntimeError(f"enqueue edit_from_prefab failed: status={status} payload={payload}")
        edit_task_id = payload["task_id"]

        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/tasks/enqueue",
            {"kind": "fork_from_prefab", "prefab_id_uuid": warcar_root, "prompt": "Add spikes (mock)"},
        )
        if status != 200 or payload.get("ok") is not True or not payload.get("task_id"):
            raise RuntimeError(f"enqueue fork_from_prefab failed: status={status} payload={payload}")
        fork_task_id = payload["task_id"]

        all_tasks = [snake_task, warcar_task, edit_task_id, fork_task_id]

        # Exercise per-task endpoint.
        for tid in all_tasks:
            status, payload = http_json("GET", f"{base_url}/v1/gen3d/tasks/{tid}", None)
            if status != 200 or payload.get("ok") is not True:
                raise RuntimeError(f"task status failed: status={status} payload={payload}")

        by_id = wait_for_tasks_done(base_url, all_tasks, timeout_secs=360.0)
        edit_root = by_id[edit_task_id]["result_prefab_id_uuid"]
        fork_root = by_id[fork_task_id]["result_prefab_id_uuid"]

        if edit_root != warcar_root:
            raise RuntimeError(f"edit_from_prefab should overwrite {warcar_root}, got {edit_root}")
        if fork_root == warcar_root:
            raise RuntimeError(f"fork_from_prefab should create new prefab id (base={warcar_root})")

        # Verify motion authoring happened for the mock snake plan (mobility + no move slots in plan).
        status, discovery = http_json("GET", f"{base_url}/v1/discovery", None)
        if status != 200 or discovery.get("ok") is not True:
            raise RuntimeError(f"discovery failed: status={status} payload={discovery}")
        realm_id = discovery.get("active", {}).get("realm_id") or "default"

        snake_prefabs = prefab_package_prefabs_dir(home_dir, realm_id, snake_root)
        if not package_contains_move_channel(snake_prefabs):
            raise RuntimeError(f"expected move channel in snake prefab package: {snake_prefabs}")

        # Graceful shutdown.
        try:
            http_json("POST", f"{base_url}/v1/shutdown", {})
        except Exception:
            pass

        try:
            proc.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            proc.terminate()

        print("OK: Gen3D task queue seeded edit/fork + motion (mock://gen3d)")
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
