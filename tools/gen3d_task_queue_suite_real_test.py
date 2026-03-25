#!/usr/bin/env python3
"""
Real-provider Gen3D task-queue suite via the local Automation HTTP API.

This is a regression runner for:
- Background Gen3D task queue (`/v1/gen3d/tasks*`)
- Fresh builds (build)
- Seeded overwrite edits (edit_from_prefab)
- Seeded forks (fork_from_prefab)
- Seeded edit routing (`llm_select_edit_strategy_v1`)

Secrets:
- Reads provider config from `~/.gravimera/config.toml` but DOES NOT write tokens to disk.
- Writes a temporary `config.toml` under the run dir with an empty token and passes the API key via env.

Artifacts:
- Written under `test/run_1/gen3d_task_queue_suite_real_test/tmp/run_*/`
  - config.toml
  - gravimera_stdout.log
  - suite_report.json
  - .gravimera/ (isolated GRAVIMERA_HOME; contains cache/gen3d runs)
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


@dataclass
class ProviderConfig:
    service: str
    base_url: str
    model: str
    reasoning_effort: str | None
    api_key: str


def load_provider_from_home_config(path: Path) -> ProviderConfig:
    if not path.exists():
        raise RuntimeError(f"missing {path}")

    import tomllib

    cfg = tomllib.loads(path.read_text(encoding="utf-8"))
    service = (cfg.get("gen3d") or {}).get("ai_service") or "openai"
    service = str(service).strip().lower()

    providers: dict[str, tuple[str, list[str], list[str]]] = {
        "openai": (
            "openai",
            ["token", "api_key", "OPENAI_API_KEY"],
            ["base_url", "model", "reasoning_effort"],
        ),
        "mimo": ("mimo", ["token", "api_key", "MIMO_API_KEY"], ["base_url", "model"]),
        "gemini": (
            "gemini",
            ["token", "api_key", "X_GOOG_API_KEY", "GEMINI_API_KEY"],
            ["base_url", "model"],
        ),
        "claude": (
            "claude",
            ["token", "api_key", "ANTHROPIC_API_KEY", "CLAUDE_API_KEY"],
            ["base_url", "model"],
        ),
    }

    section, token_keys, _info_keys = providers.get(service, providers["openai"])
    sec = cfg.get(section) or {}

    api_key = ""
    for k in token_keys:
        v = sec.get(k)
        if isinstance(v, str) and v.strip():
            api_key = v.strip()
            break

    base_url = sec.get("base_url") if isinstance(sec.get("base_url"), str) else ""
    model = sec.get("model") if isinstance(sec.get("model"), str) else ""
    reasoning_effort = (
        sec.get("reasoning_effort") if isinstance(sec.get("reasoning_effort"), str) else None
    )

    if not base_url:
        base_url = "https://api.openai.com/v1" if service == "openai" else ""
    if not model:
        model = "gpt-5.4" if service == "openai" else ""

    if not api_key:
        raise RuntimeError(
            f"{path}: missing {section}.token/api_key (or {section}.*API_KEY). Set it in config or via env."
        )

    return ProviderConfig(
        service=service,
        base_url=str(base_url),
        model=str(model),
        reasoning_effort=str(reasoning_effort).strip() if reasoning_effort else None,
        api_key=api_key,
    )


def env_key_name_for_service(service: str) -> str:
    if service == "openai":
        return "OPENAI_API_KEY"
    if service == "mimo":
        return "MIMO_API_KEY"
    if service == "gemini":
        return "X_GOOG_API_KEY"
    if service == "claude":
        return "ANTHROPIC_API_KEY"
    return "OPENAI_API_KEY"


def write_test_config(config_path: Path, provider: ProviderConfig) -> None:
    service = provider.service
    lines: list[str] = []
    lines.append("[gen3d]\n")
    lines.append(f'ai_service = "{service}"\n')
    lines.append("require_structured_outputs = true\n")
    # Hard budgets: keep spend bounded for real-provider regression.
    lines.append("max_seconds = 1800\n")
    lines.append("max_tokens = 500000\n")
    lines.append("\n")

    if service == "openai":
        lines.append("[openai]\n")
        lines.append(f'base_url = "{provider.base_url}"\n')
        lines.append(f'model = "{provider.model}"\n')
        if provider.reasoning_effort:
            lines.append(f'reasoning_effort = "{provider.reasoning_effort}"\n')
        lines.append('token = ""\n')
    elif service == "mimo":
        lines.append("[mimo]\n")
        lines.append(f'base_url = "{provider.base_url}"\n')
        lines.append(f'model = "{provider.model}"\n')
        lines.append('token = ""\n')
    elif service == "gemini":
        lines.append("[gemini]\n")
        lines.append(f'base_url = "{provider.base_url}"\n')
        lines.append(f'model = "{provider.model}"\n')
        lines.append('token = ""\n')
    elif service == "claude":
        lines.append("[claude]\n")
        lines.append(f'base_url = "{provider.base_url}"\n')
        lines.append(f'model = "{provider.model}"\n')
        lines.append('token = ""\n')
    else:
        raise RuntimeError(f"unsupported gen3d.ai_service={service!r}")

    config_path.write_text("".join(lines), encoding="utf-8")


def discover_base_url_from_output(
    proc: subprocess.Popen[bytes], log_fp, timeout_secs: float
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
                return url.rstrip("/")
    raise RuntimeError(
        "Timed out waiting for Automation API listen address. Last output:\n"
        + buf[-4000:].decode("utf-8", errors="replace")
    )


class LocalHttp:
    def __init__(self, base_url: str):
        self.base_url = base_url.rstrip("/")
        # Some host environments may set HTTP(S)_PROXY env vars; urllib will honor them by default,
        # which can break loopback requests. Use an explicit "no proxy" opener for local calls.
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def json(
        self, method: str, path: str, body: dict[str, Any] | None = None, timeout_secs: float = 30.0
    ) -> tuple[int, dict[str, Any]]:
        url = f"{self.base_url}{path}"
        headers = {"Content-Type": "application/json"}
        data = json.dumps(body).encode("utf-8") if body is not None else None
        req = urllib.request.Request(url, method=method, data=data, headers=headers)
        try:
            with self._opener.open(req, timeout=timeout_secs) as resp:
                raw = resp.read().decode("utf-8", errors="replace")
                payload = json.loads(raw) if raw.strip() else {}
                if not isinstance(payload, dict):
                    return resp.status, {"ok": False, "error": f"Non-JSON response: {raw[:200]}"}
                return resp.status, payload
        except urllib.error.HTTPError as err:
            raw = err.read().decode("utf-8", errors="replace")
            try:
                payload = json.loads(raw) if raw.strip() else {}
                if not isinstance(payload, dict):
                    payload = {"ok": False, "error": raw.strip()}
            except Exception:
                payload = {"ok": False, "error": raw.strip()}
            return err.code, payload
        except (TimeoutError, OSError) as err:
            raise RuntimeError(f"Request failed {url}: {err}") from None
        except urllib.error.URLError as err:
            raise RuntimeError(f"Request failed {url}: {err}") from None


def wait_for_health(api: LocalHttp, timeout_secs: float) -> dict[str, Any]:
    deadline = time.time() + timeout_secs
    last_err: str | None = None
    while time.time() < deadline:
        try:
            status, payload = api.json("GET", "/v1/health", None, timeout_secs=2.0)
            if status == 200 and payload.get("ok") is True:
                return payload
            last_err = f"health not ok: status={status} payload={payload}"
        except Exception as e:
            last_err = str(e)
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {api.base_url}/v1/health: {last_err}")


def wait_for_tasks_done(
    api: LocalHttp,
    task_ids: list[str],
    timeout_secs: float,
    print_every_secs: float = 5.0,
) -> dict[str, dict[str, Any]]:
    deadline = time.time() + timeout_secs
    done_states = ("done", "failed", "canceled")
    last_print = 0.0
    last_poll_err = 0.0

    while time.time() < deadline:
        try:
            status, payload = api.json("GET", "/v1/gen3d/tasks", None, timeout_secs=30.0)
        except Exception as err:
            now = time.time()
            if now - last_poll_err >= 10.0:
                last_poll_err = now
                print(f"WARN: /v1/gen3d/tasks poll failed: {err}", flush=True)
            time.sleep(0.5)
            continue
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"tasks list failed: status={status} payload={payload}")

        tasks = payload.get("tasks", [])
        by_id: dict[str, dict[str, Any]] = {
            str(t.get("task_id")): t for t in tasks if isinstance(t, dict) and t.get("task_id")
        }

        # Fallback: some callers might not see the task in the list yet; ask the per-task endpoint.
        missing: list[str] = []
        states: list[str | None] = []
        for tid in task_ids:
            t = by_id.get(tid)
            if t is None:
                st, single = api.json("GET", f"/v1/gen3d/tasks/{tid}", None, timeout_secs=10.0)
                if st == 200 and single.get("ok") is True and isinstance(single.get("task"), dict):
                    by_id[tid] = single["task"]
                    t = by_id.get(tid)
            if t is None:
                missing.append(tid)
                states.append(None)
            else:
                states.append(t.get("state"))

        now = time.time()
        if now - last_print >= print_every_secs:
            last_print = now
            short = []
            for tid in task_ids:
                t = by_id.get(tid, {})
                st = t.get("state")
                msg = (t.get("status") or "").split("\n", 1)[0][:72]
                short.append(f"{tid[:8]}={st}:{msg}")
            if missing:
                short.append(f"missing={len(missing)}")
            print("tasks:", ", ".join(short), flush=True)

        if missing:
            time.sleep(0.2)
            continue

        if all(s in done_states for s in states if s is not None):
            for tid in task_ids:
                t = by_id.get(tid, {})
                if t.get("state") != "done":
                    raise RuntimeError(
                        f"task failed: {tid} state={t.get('state')} error={t.get('error')}"
                    )
                if not t.get("result_prefab_id_uuid"):
                    raise RuntimeError(f"task missing result_prefab_id_uuid: {tid} task={t}")
            return by_id

        time.sleep(0.2)

    raise RuntimeError(f"timed out waiting for tasks to complete: {task_ids}")


def read_jsonl(path: Path) -> list[dict[str, Any]]:
    out: list[dict[str, Any]] = []
    for line in path.read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            rec = json.loads(line)
        except Exception:
            continue
        if isinstance(rec, dict):
            out.append(rec)
    return out


def find_tool_results(run_dir: Path) -> list[Path]:
    return sorted(run_dir.glob("attempt_*/pass_*/tool_results.jsonl"))


def analyze_router_strategy(run_dir: Path) -> dict[str, Any]:
    files = find_tool_results(run_dir)
    if not files:
        return {"ok": False, "error": f"missing tool_results.jsonl under {run_dir}"}

    router: dict[str, Any] | None = None
    for path in files:
        for rec in read_jsonl(path):
            if rec.get("tool_id") != "llm_select_edit_strategy_v1":
                continue
            router = {
                "tool_results": str(path),
                "ok": rec.get("ok"),
                "result": rec.get("result"),
            }
    if router is None:
        return {"ok": False, "error": "missing llm_select_edit_strategy_v1 result"}

    result = router.get("result") if isinstance(router.get("result"), dict) else {}
    return {
        "ok": True,
        "tool_results": router.get("tool_results"),
        "strategy": result.get("strategy"),
        "snapshot_components": result.get("snapshot_components"),
        "reason": result.get("reason"),
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--run-dir",
        default="test/run_1/gen3d_task_queue_suite_real_test",
        help="Where to write artifacts (default: test/run_1/...)",
    )
    ap.add_argument(
        "--home-config",
        default="~/.gravimera/config.toml",
        help="Provider config source (default: ~/.gravimera/config.toml)",
    )
    ap.add_argument("--timeout-mins", type=float, default=60.0, help="Timeout per phase (build/seeded)")
    args = ap.parse_args()

    repo_root = Path(__file__).resolve().parent.parent
    run_base = Path(args.run_dir).expanduser().resolve()
    tmp_root = run_base / "tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix="run_", dir=str(tmp_root)))
    home_dir = run_root / ".gravimera"
    home_dir.mkdir(parents=True, exist_ok=True)

    provider_path = Path(args.home_config).expanduser()
    provider = load_provider_from_home_config(provider_path)
    env_key = env_key_name_for_service(provider.service)

    config_path = run_root / "config.toml"
    write_test_config(config_path, provider)

    log_path = run_root / "gravimera_stdout.log"

    print("Real Gen3D task-queue suite")
    print(f"- service: {provider.service}")
    print(f"- base_url: {provider.base_url}")
    print(f"- model: {provider.model}")
    print(f"- run_root: {run_root}")

    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(home_dir)
    env[env_key] = provider.api_key

    cmd = [
        "cargo",
        "run",
        "--quiet",
        "--",
        "--config",
        str(config_path),
        "--automation",
        "--automation-bind",
        "127.0.0.1:0",
        "--automation-disable-local-input",
        "--automation-pause-on-start",
    ]

    proc: subprocess.Popen[bytes] | None = None
    log_fp = None
    drain_thread: threading.Thread | None = None
    api: LocalHttp | None = None

    interrupted = False

    def _sigint(_signum: int, _frame: Any) -> None:
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, _sigint)

    try:
        log_fp = open(log_path, "wb")
        proc = subprocess.Popen(
            cmd,
            cwd=str(repo_root),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )

        base_url = discover_base_url_from_output(proc, log_fp, timeout_secs=300.0)
        api = LocalHttp(base_url)

        def drain_stdout() -> None:
            if proc is None or proc.stdout is None or log_fp is None:
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

        drain_thread = threading.Thread(target=drain_stdout, name="gravimera_stdout_drain", daemon=True)
        drain_thread.start()

        wait_for_health(api, timeout_secs=45.0)

        st, payload = api.json("POST", "/v1/mode", {"mode": "build_preview"}, timeout_secs=30.0)
        if st != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"/v1/mode failed: status={st} payload={payload}")

        # Unpause time so the suite doesn't rely on tight /v1/step loops for long-running AI calls.
        api.json("POST", "/v1/resume", {}, timeout_secs=10.0)
        time.sleep(0.25)

        # Enqueue builds.
        builds = [
            ("crate", "A simple low-poly wooden crate prop."),
            ("snake", "A simple low-poly snake enemy unit."),
            ("warcar", "A simple low-poly warcar with a cannon as weapon."),
        ]
        build_task_ids: list[str] = []
        for name, prompt in builds:
            st, payload = api.json(
                "POST",
                "/v1/gen3d/tasks/enqueue",
                {"kind": "build", "prompt": prompt},
                timeout_secs=30.0,
            )
            if st != 200 or payload.get("ok") is not True or not payload.get("task_id"):
                raise RuntimeError(f"enqueue build failed ({name}): status={st} payload={payload}")
            tid = str(payload["task_id"])
            build_task_ids.append(tid)
            print(f"enqueued build {name}: {tid}", flush=True)

        # While the first build is running, verify preview is hidden (fresh, empty workshop).
        hide_checked = False
        hide_deadline = time.time() + 120.0
        first_task = build_task_ids[0]
        while time.time() < hide_deadline and not interrupted:
            st, payload = api.json("GET", f"/v1/gen3d/tasks/{first_task}", None, timeout_secs=10.0)
            if st != 200 or payload.get("ok") is not True:
                raise RuntimeError(f"task status failed: status={st} payload={payload}")
            state = payload.get("task", {}).get("state")
            if state == "running":
                st2, preview = api.json("GET", "/v1/gen3d/preview", None, timeout_secs=10.0)
                if st2 != 200 or preview.get("ok") is not True:
                    raise RuntimeError(f"/v1/gen3d/preview failed: status={st2} payload={preview}")
                if preview.get("should_hide_running_preview") is not True:
                    raise RuntimeError(f"expected should_hide_running_preview=true, got {preview}")
                hide_checked = True
                break
            if state in ("done", "failed", "canceled"):
                break
            time.sleep(0.2)
        if not hide_checked:
            print("WARN: did not observe first build in running state fast enough for preview-hide check", flush=True)

        # Wait for builds.
        by_id = wait_for_tasks_done(api, build_task_ids, timeout_secs=args.timeout_mins * 60.0)
        build_results: dict[str, dict[str, Any]] = {}
        for (name, _), tid in zip(builds, build_task_ids):
            build_results[name] = {
                "task_id": tid,
                "run_id": by_id[tid].get("run_id"),
                "prefab_id_uuid": by_id[tid].get("result_prefab_id_uuid"),
            }

        warcar_root = str(build_results["warcar"]["prefab_id_uuid"] or "").strip()
        if not warcar_root:
            raise RuntimeError("missing warcar prefab id")

        # Seeded edits/fork (from warcar).
        seeded = [
            (
                "edit_draft_ops",
                {
                    "kind": "edit_from_prefab",
                    "prefab_id_uuid": warcar_root,
                    "prompt": "Make the cannon longer and darken it.",
                },
                "draft_ops_only",
            ),
            (
                "edit_plan_ops",
                {
                    "kind": "edit_from_prefab",
                    "prefab_id_uuid": warcar_root,
                    "prompt": "Add a NEW component named roof_armor_plate. Attach it to the hull/chassis. Do NOT modify existing component geometry.",
                },
                "plan_ops",
            ),
            (
                "fork_rebuild",
                {
                    "kind": "fork_from_prefab",
                    "prefab_id_uuid": warcar_root,
                    "prompt": "Rebuild from scratch into a simple heavy tank with tracks.",
                },
                "rebuild",
            ),
        ]
        seeded_task_ids: list[str] = []
        for name, req, _expect in seeded:
            st, payload = api.json("POST", "/v1/gen3d/tasks/enqueue", req, timeout_secs=30.0)
            if st != 200 or payload.get("ok") is not True or not payload.get("task_id"):
                raise RuntimeError(f"enqueue seeded failed ({name}): status={st} payload={payload}")
            tid = str(payload["task_id"])
            seeded_task_ids.append(tid)
            print(f"enqueued seeded {name}: {tid}", flush=True)

        seeded_by_id = wait_for_tasks_done(api, seeded_task_ids, timeout_secs=args.timeout_mins * 60.0)

        seeded_results: dict[str, dict[str, Any]] = {}
        for (name, _req, _expect), tid in zip(seeded, seeded_task_ids):
            seeded_results[name] = {
                "task_id": tid,
                "run_id": seeded_by_id[tid].get("run_id"),
                "prefab_id_uuid": seeded_by_id[tid].get("result_prefab_id_uuid"),
            }

        # Invariants: edit overwrites, fork creates new id.
        if seeded_results["edit_draft_ops"]["prefab_id_uuid"] != warcar_root:
            raise RuntimeError(
                f"edit_from_prefab should overwrite {warcar_root}, got {seeded_results['edit_draft_ops']['prefab_id_uuid']}"
            )
        if seeded_results["edit_plan_ops"]["prefab_id_uuid"] != warcar_root:
            raise RuntimeError(
                f"edit_from_prefab should overwrite {warcar_root}, got {seeded_results['edit_plan_ops']['prefab_id_uuid']}"
            )
        if seeded_results["fork_rebuild"]["prefab_id_uuid"] == warcar_root:
            raise RuntimeError(f"fork_from_prefab should create a new prefab id (base={warcar_root})")

        # Router coverage (seeded runs): validate strategy selection for each seeded run.
        router_analysis: dict[str, Any] = {}
        for name, _req, expect in seeded:
            run_id = str(seeded_results[name].get("run_id") or "").strip()
            if not run_id:
                continue
            run_dir = home_dir / "cache" / "gen3d" / run_id
            router_analysis[name] = analyze_router_strategy(run_dir)
            strat = router_analysis[name].get("strategy")
            if expect == "plan_ops":
                if strat not in ("plan_ops_only", "plan_ops_then_draft_ops"):
                    raise RuntimeError(f"{name}: expected plan_ops strategy, got {strat}. analysis={router_analysis[name]}")
            else:
                if strat != expect:
                    raise RuntimeError(f"{name}: expected strategy={expect}, got {strat}. analysis={router_analysis[name]}")

        report = {
            "provider": {"service": provider.service, "base_url": provider.base_url, "model": provider.model},
            "run_root": str(run_root),
            "build_results": build_results,
            "seeded_results": seeded_results,
            "router_analysis": router_analysis,
        }
        report_path = run_root / "suite_report.json"
        report_path.write_text(json.dumps(report, indent=2), encoding="utf-8")

        print("Suite OK", flush=True)
        print(f"- report: {report_path}", flush=True)
        return 0
    finally:
        if api is not None:
            try:
                api.json("POST", "/v1/shutdown", {}, timeout_secs=5.0)
            except Exception:
                pass

        if proc is not None and proc.poll() is None:
            try:
                proc.send_signal(signal.SIGINT)
                proc.wait(timeout=5.0)
            except Exception:
                try:
                    proc.kill()
                except Exception:
                    pass

        if drain_thread is not None:
            try:
                drain_thread.join(timeout=2.0)
            except Exception:
                pass
        if log_fp is not None:
            try:
                log_fp.close()
            except Exception:
                pass


if __name__ == "__main__":
    raise SystemExit(main())
