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


def wait_for_health(base_url: str, timeout_secs: float = 20.0):
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


def list_prefab_def_json_files(prefabs_dir: Path):
    out = []
    for p in prefabs_dir.rglob("*.json"):
        if p.name.endswith(".desc.json"):
            continue
        out.append(p)
    return sorted(out)


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

        base_url = discover_base_url_from_output(proc, log_fp, timeout_secs=30.0)

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

        status, active = http_json("GET", f"{base_url}/v1/realm_scene/active", None)
        if status != 200 or active.get("ok") is not True:
            raise RuntimeError(f"realm_scene/active failed: status={status} payload={active}")
        realm_id = active.get("realm_id")
        if not realm_id:
            raise RuntimeError(f"realm_scene/active missing realm_id: payload={active}")

        # 1) Generate a prefab package (mock://gen3d).
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/tasks/enqueue",
            {"kind": "build", "prompt": "A snake enemy unit (mock)"},
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"enqueue failed: status={status} payload={payload}")
        task_id = payload.get("task_id")
        if not task_id:
            raise RuntimeError(f"enqueue missing task_id: payload={payload}")

        src_prefab_id_uuid = None
        deadline = time.time() + 120.0
        while time.time() < deadline:
            status, tasks_payload = http_json("GET", f"{base_url}/v1/gen3d/tasks/{task_id}", None)
            if status != 200 or tasks_payload.get("ok") is not True:
                raise RuntimeError(f"task status failed: status={status} payload={tasks_payload}")
            task = tasks_payload.get("task", {})
            state = task.get("state")
            if state in ("failed", "canceled"):
                raise RuntimeError(f"task did not finish cleanly: {task}")
            if state == "done":
                src_prefab_id_uuid = task.get("result_prefab_id_uuid")
                if not src_prefab_id_uuid:
                    raise RuntimeError(f"done task missing result_prefab_id_uuid: {task}")
                break
            http_json("POST", f"{base_url}/v1/step", {"frames": 5})
        else:
            raise RuntimeError("timed out waiting for Gen3D build to complete")

        # 2) Duplicate the prefab package.
        status, dup = http_json(
            "POST",
            f"{base_url}/v1/prefabs/duplicate",
            {"prefab_id_uuid": src_prefab_id_uuid},
        )
        if status != 200 or dup.get("ok") is not True:
            raise RuntimeError(f"prefab duplicate failed: status={status} payload={dup}")
        new_prefab_id_uuid = dup.get("new_prefab_id_uuid")
        if not new_prefab_id_uuid:
            raise RuntimeError(f"duplicate missing new_prefab_id_uuid: payload={dup}")
        if new_prefab_id_uuid == src_prefab_id_uuid:
            raise RuntimeError("duplicate returned same prefab id")

        # 3) Verify on-disk package structure.
        realm_prefabs_root = home_dir / "realm" / realm_id / "prefabs"
        src_pkg = realm_prefabs_root / src_prefab_id_uuid
        dst_pkg = realm_prefabs_root / new_prefab_id_uuid
        if not src_pkg.exists():
            raise RuntimeError(f"missing src package dir: {src_pkg}")
        if not dst_pkg.exists():
            raise RuntimeError(f"missing dst package dir: {dst_pkg}")

        src_prefabs_dir = src_pkg / "prefabs"
        dst_prefabs_dir = dst_pkg / "prefabs"
        if not src_prefabs_dir.is_dir():
            raise RuntimeError(f"missing src prefabs dir: {src_prefabs_dir}")
        if not dst_prefabs_dir.is_dir():
            raise RuntimeError(f"missing dst prefabs dir: {dst_prefabs_dir}")

        src_defs = list_prefab_def_json_files(src_prefabs_dir)
        dst_defs = list_prefab_def_json_files(dst_prefabs_dir)
        if len(src_defs) == 0:
            raise RuntimeError(f"src package has no def json files: {src_prefabs_dir}")
        if len(dst_defs) != len(src_defs):
            raise RuntimeError(f"def count mismatch: src={len(src_defs)} dst={len(dst_defs)}")

        if not (dst_prefabs_dir / f"{new_prefab_id_uuid}.json").exists():
            raise RuntimeError("dst package missing root def json file")
        if not (dst_prefabs_dir / f"{new_prefab_id_uuid}.desc.json").exists():
            raise RuntimeError("dst package missing root descriptor json file")

        src_gen3d_source = src_pkg / "gen3d_source_v1"
        dst_gen3d_source = dst_pkg / "gen3d_source_v1"
        if src_gen3d_source.exists():
            if not dst_gen3d_source.exists():
                raise RuntimeError("dst package missing gen3d_source_v1 directory")

        src_edit_bundle = src_pkg / "gen3d_edit_bundle_v1.json"
        dst_edit_bundle = dst_pkg / "gen3d_edit_bundle_v1.json"
        if src_edit_bundle.exists():
            if not dst_edit_bundle.exists():
                raise RuntimeError("dst package missing gen3d_edit_bundle_v1.json")
            with open(dst_edit_bundle, "rb") as f:
                bundle = json.loads(f.read().decode("utf-8"))
            root_field = bundle.get("root_prefab_id_uuid")
            if root_field != new_prefab_id_uuid:
                raise RuntimeError(
                    f"dst edit bundle root_prefab_id_uuid mismatch: got={root_field} want={new_prefab_id_uuid}"
                )

        # 4) Verify it shows up in prefab list and can spawn.
        status, prefabs_payload = http_json("GET", f"{base_url}/v1/prefabs", None)
        if status != 200 or prefabs_payload.get("ok") is not True:
            raise RuntimeError(f"prefabs list failed: status={status} payload={prefabs_payload}")
        prefab_ids = {p.get("prefab_id_uuid") for p in prefabs_payload.get("prefabs", [])}
        if src_prefab_id_uuid not in prefab_ids:
            raise RuntimeError("src prefab id not present in /v1/prefabs")
        if new_prefab_id_uuid not in prefab_ids:
            raise RuntimeError("new prefab id not present in /v1/prefabs")

        status, spawn = http_json(
            "POST",
            f"{base_url}/v1/spawn",
            {"prefab_id_uuid": new_prefab_id_uuid, "x": 6.0, "z": 6.0},
        )
        if status != 200 or spawn.get("ok") is not True:
            raise RuntimeError(f"spawn duplicated prefab failed: status={status} payload={spawn}")

        # 5) Ensure the historical regression log string isn't present.
        log_fp.flush()
        with open(log_path, "rb") as f:
            text = f.read().decode("utf-8", errors="replace")
        if "Missing prefab def" in text:
            raise RuntimeError("found unexpected 'Missing prefab def' in gravimera stdout log")

        # Graceful shutdown.
        try:
            http_json("POST", f"{base_url}/v1/shutdown", {})
        except Exception:
            pass

        try:
            proc.wait(timeout=10.0)
        except subprocess.TimeoutExpired:
            proc.terminate()

        print("OK: Prefab duplication API (mock://gen3d)")
        ok = True
        return 0
    except Exception as e:
        print(
            f"ERROR: {e}\nArtifacts kept at: {run_root}\nLog: {log_path}",
            file=sys.stderr,
        )
        raise
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


if __name__ == "__main__":
    raise SystemExit(main())

