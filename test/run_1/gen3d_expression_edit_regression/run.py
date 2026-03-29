#!/usr/bin/env python3

from __future__ import annotations

import json
import os
import shutil
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

    providers: dict[str, tuple[str, list[str]]] = {
        "openai": ("openai", ["token", "api_key", "OPENAI_API_KEY"]),
        "mimo": ("mimo", ["token", "api_key", "MIMO_API_KEY"]),
        "gemini": ("gemini", ["token", "api_key", "X_GOOG_API_KEY", "GEMINI_API_KEY"]),
        "claude": ("claude", ["token", "api_key", "ANTHROPIC_API_KEY", "CLAUDE_API_KEY"]),
    }

    section, token_keys = providers.get(service, providers["openai"])
    sec = cfg.get(section) or {}

    api_key = ""
    for key in token_keys:
        value = sec.get(key)
        if isinstance(value, str) and value.strip():
            api_key = value.strip()
            break

    base_url = sec.get("base_url") if isinstance(sec.get("base_url"), str) else ""
    model = sec.get("model") if isinstance(sec.get("model"), str) else ""
    reasoning_effort = None
    for key in ("reasoning_effort", "model_reasoning_effort"):
        value = sec.get(key)
        if isinstance(value, str) and value.strip():
            reasoning_effort = value.strip()
            break

    if not base_url:
        base_url = "https://api.openai.com/v1" if service == "openai" else ""
    if not model:
        model = "gpt-5.4" if service == "openai" else ""
    if not api_key:
        raise RuntimeError(f"{path}: missing provider token/api_key for [{section}]")

    return ProviderConfig(
        service=service,
        base_url=str(base_url),
        model=str(model),
        reasoning_effort=reasoning_effort,
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
    lines.append('orchestrator = "pipeline"\n')
    lines.append(f'ai_service = "{service}"\n')
    lines.append("require_structured_outputs = true\n")
    lines.append("max_seconds = 1800\n")
    lines.append("max_tokens = 500000\n\n")

    lines.append("[log]\n")
    lines.append('level = "debug"\n\n')

    lines.append(f"[{service}]\n")
    lines.append(f'base_url = "{provider.base_url}"\n')
    lines.append(f'model = "{provider.model}"\n')
    if service == "openai" and provider.reasoning_effort:
        lines.append(f'reasoning_effort = "{provider.reasoning_effort}"\n')
    lines.append('token = ""\n')

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

        ready, _, _ = select.select([proc.stdout], [], [], 0.2)
        if not ready:
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
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def json(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        timeout_secs: float = 30.0,
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
                    return resp.status, {"ok": False, "error": raw.strip()}
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


def wait_for_health(http: LocalHttp, timeout_secs: float) -> dict[str, Any]:
    deadline = time.time() + timeout_secs
    last: str | None = None
    while time.time() < deadline:
        try:
            status, payload = http.json("GET", "/v1/health", None, timeout_secs=3.0)
            if status == 200 and payload.get("ok") is True:
                return payload
            last = f"status={status} payload={payload}"
        except Exception as err:
            last = str(err)
        time.sleep(0.2)
    raise RuntimeError(f"timed out waiting for health: {last}")


def step(http: LocalHttp, frames: int = 5) -> None:
    status, payload = http.json("POST", "/v1/step", {"frames": frames}, timeout_secs=60.0)
    if status != 200 or payload.get("ok") is not True:
        raise RuntimeError(f"step failed: status={status} payload={payload}")


def wait_for_preview_mode(http: LocalHttp, timeout_secs: float) -> None:
    deadline = time.time() + timeout_secs
    while time.time() < deadline:
        status, payload = http.json("GET", "/v1/state", None, timeout_secs=10.0)
        if (
            status == 200
            and payload.get("ok") is True
            and payload.get("mode") == "build"
            and payload.get("build_scene") == "preview"
        ):
            return
        step(http, 3)
    raise RuntimeError("timed out waiting for build preview mode")


def wait_for_gen3d_done(http: LocalHttp, timeout_secs: float) -> dict[str, Any]:
    deadline = time.time() + timeout_secs
    last_status: dict[str, Any] | None = None
    last_line = ""

    while time.time() < deadline:
        status, payload = http.json("GET", "/v1/gen3d/status", None, timeout_secs=20.0)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"gen3d status failed: status={status} payload={payload}")
        last_status = payload

        error = payload.get("error")
        if isinstance(error, str) and error.strip():
            raise RuntimeError(f"gen3d run failed: {error.strip()}")

        line = (
            f"running={payload.get('running')} build_complete={payload.get('build_complete')} "
            f"draft_ready={payload.get('draft_ready')} status={str(payload.get('status') or '').splitlines()[0]}"
        )
        if line != last_line:
            print(line, flush=True)
            last_line = line

        if payload.get("running") is False and payload.get("build_complete") is True:
            if payload.get("draft_ready") is not True:
                raise RuntimeError(f"gen3d stopped before draft_ready=true: {payload}")
            return payload

        step(http, 5)
        time.sleep(0.4)

    raise RuntimeError(f"timed out waiting for gen3d run. last status={last_status}")


def load_bundle(bundle_path: Path) -> dict[str, Any]:
    if not bundle_path.exists():
        raise RuntimeError(f"missing bundle {bundle_path}")
    return json.loads(bundle_path.read_text(encoding="utf-8"))


def normalize_json_value(value: Any) -> Any:
    if isinstance(value, float):
        return round(value, 6)
    if isinstance(value, list):
        return [normalize_json_value(item) for item in value]
    if isinstance(value, dict):
        return {key: normalize_json_value(item) for key, item in value.items()}
    return value


def collect_edit_bundle_channel_fingerprints(bundle: dict[str, Any]) -> dict[str, str]:
    by_channel: dict[str, list[dict[str, Any]]] = {}
    for comp in bundle.get("planned_components", []):
        component_name = str(comp.get("name") or "")
        for location, slots in (
            ("root", comp.get("root_animations", [])),
            ("attach", (comp.get("attach_to") or {}).get("animations", [])),
        ):
            if not isinstance(slots, list):
                continue
            for slot in slots:
                if not isinstance(slot, dict):
                    continue
                channel = str(slot.get("channel") or "").strip().lower()
                if not channel:
                    continue
                normalized_slot = normalize_json_value(slot)
                by_channel.setdefault(channel, []).append(
                    {
                        "component": component_name,
                        "location": location,
                        "slot": normalized_slot,
                    }
                )

    out: dict[str, str] = {}
    for channel, entries in by_channel.items():
        entries.sort(
            key=lambda item: (
                item["component"],
                item["location"],
                json.dumps(item["slot"], sort_keys=True, separators=(",", ":")),
            )
        )
        out[channel] = json.dumps(entries, sort_keys=True, separators=(",", ":"))
    return out


def collect_package_channel_names(prefab_package_dir: Path) -> set[str]:
    prefabs_dir = prefab_package_dir / "prefabs"
    if not prefabs_dir.is_dir():
        raise RuntimeError(f"missing prefabs dir {prefabs_dir}")

    channels: set[str] = set()
    for prefab_path in sorted(prefabs_dir.glob("*.json")):
        prefab = json.loads(prefab_path.read_text(encoding="utf-8"))
        for part in prefab.get("parts", []):
            if not isinstance(part, dict):
                continue
            animations = part.get("animations", [])
            if not isinstance(animations, list):
                continue
            for slot in animations:
                if not isinstance(slot, dict):
                    continue
                channel = str(slot.get("channel") or "").strip().lower()
                if not channel:
                    continue
                channels.add(channel)
    return channels


def parse_motion_call_channels(run_dir: Path) -> list[list[str]]:
    trace_path = run_dir / "agent_trace.jsonl"
    if not trace_path.exists():
        raise RuntimeError(f"missing {trace_path}")

    calls: list[list[str]] = []
    for raw_line in trace_path.read_text(encoding="utf-8").splitlines():
        raw_line = raw_line.strip()
        if not raw_line:
            continue
        event = json.loads(raw_line).get("event") or {}
        if event.get("kind") != "tool_call":
            continue
        if event.get("tool_id") != "llm_generate_motions_v1":
            continue
        args = event.get("args") or {}
        channels = args.get("channels")
        if not isinstance(channels, list):
            continue
        calls.append([str(ch).strip().lower() for ch in channels if str(ch).strip()])
    return calls


def ensure_prefab_copied(real_home: Path, isolated_home: Path, prefab_id: str) -> Path:
    src = real_home / "realm" / "default" / "prefabs" / prefab_id
    if not src.is_dir():
        raise RuntimeError(f"missing source prefab package {src}")

    dst = isolated_home / "realm" / "default" / "prefabs" / prefab_id
    dst.parent.mkdir(parents=True, exist_ok=True)
    shutil.copytree(src, dst)
    return dst


def main() -> int:
    source_prefab_id = os.environ.get("SOURCE_PREFAB_ID", "").strip()
    if not source_prefab_id:
        print("SOURCE_PREFAB_ID is required", file=sys.stderr)
        return 2

    target_channel = os.environ.get("TARGET_CHANNEL", "shy_smile").strip().lower()
    if not target_channel:
        print("TARGET_CHANNEL must be non-empty", file=sys.stderr)
        return 2

    prompt = os.environ.get(
        "PROMPT",
        f"Add a new expression animation channel named {target_channel}. Keep the existing geometry, look, and existing motion channels unchanged.",
    ).strip()
    if not prompt:
        print("PROMPT must be non-empty", file=sys.stderr)
        return 2

    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parents[2]
    real_home = Path.home() / ".gravimera"
    provider = load_provider_from_home_config(real_home / "config.toml")

    tmp_root = script_dir / "tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix="run_", dir=str(tmp_root)))
    home_dir = run_root / ".gravimera"
    home_dir.mkdir(parents=True, exist_ok=True)
    config_path = run_root / "config.toml"
    log_path = run_root / "gravimera_stdout.log"
    report_path = run_root / "suite_report.json"

    prefab_dir = ensure_prefab_copied(real_home, home_dir, source_prefab_id)
    bundle_path = prefab_dir / "gen3d_edit_bundle_v1.json"
    before_bundle = load_bundle(bundle_path)
    before_bundle_channels = collect_edit_bundle_channel_fingerprints(before_bundle)
    before_package_channels = collect_package_channel_names(prefab_dir)
    if target_channel in before_package_channels:
        raise RuntimeError(
            f"target channel `{target_channel}` already exists in source prefab; choose a new TARGET_CHANNEL"
        )

    write_test_config(config_path, provider)

    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(home_dir)
    env[env_key_name_for_service(provider.service)] = provider.api_key

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
    http: LocalHttp | None = None

    try:
        log_fp = open(log_path, "wb")
        proc = subprocess.Popen(
            cmd,
            cwd=str(repo_root),
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        base_url = discover_base_url_from_output(proc, log_fp, timeout_secs=180.0)
        http = LocalHttp(base_url)

        def drain_stdout() -> None:
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

        wait_for_health(http, timeout_secs=30.0)

        status, payload = http.json("POST", "/v1/mode", {"mode": "build_preview"}, timeout_secs=20.0)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"failed to switch mode: status={status} payload={payload}")
        wait_for_preview_mode(http, timeout_secs=30.0)

        status, payload = http.json(
            "POST",
            "/v1/gen3d/edit_from_prefab",
            {"prefab_id_uuid": source_prefab_id},
            timeout_secs=20.0,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"edit_from_prefab failed: status={status} payload={payload}")

        status, payload = http.json(
            "POST",
            "/v1/gen3d/prompt",
            {"prompt": prompt},
            timeout_secs=20.0,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"prompt failed: status={status} payload={payload}")

        status, payload = http.json("POST", "/v1/gen3d/resume", {}, timeout_secs=20.0)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"resume failed: status={status} payload={payload}")

        done = wait_for_gen3d_done(http, timeout_secs=1200.0)
        run_dir = Path(str(done.get("run_dir") or "")).expanduser()
        if not run_dir.is_dir():
            raise RuntimeError(f"missing run_dir from status: {done}")

        status, payload = http.json("POST", "/v1/gen3d/save", {}, timeout_secs=20.0)
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"save failed: status={status} payload={payload}")

        if not bundle_path.exists():
            raise RuntimeError(f"missing saved edit bundle {bundle_path}")
        after_bundle = load_bundle(bundle_path)
        after_bundle_channels = collect_edit_bundle_channel_fingerprints(after_bundle)
        after_package_channels = collect_package_channel_names(prefab_dir)
        if target_channel not in after_package_channels:
            raise RuntimeError(
                f"saved prefab package is missing requested target channel `{target_channel}`"
            )

        changed_unrelated_channels = sorted(
            channel
            for channel in set(before_bundle_channels) | set(after_bundle_channels)
            if channel != target_channel
            and before_bundle_channels.get(channel) != after_bundle_channels.get(channel)
        )

        motion_call_channels = parse_motion_call_channels(run_dir)
        if not motion_call_channels:
            raise RuntimeError("run did not contain any llm_generate_motions_v1 calls")

        unexpected_motion_calls = [
            channels
            for channels in motion_call_channels
            if any(channel != target_channel for channel in channels)
        ]
        if unexpected_motion_calls:
            raise RuntimeError(
                "motion authoring requested unrelated channels: "
                + json.dumps(unexpected_motion_calls, ensure_ascii=False)
            )

        report = {
            "ok": True,
            "source_prefab_id": source_prefab_id,
            "target_channel": target_channel,
            "prompt": prompt,
            "run_dir": str(run_dir),
            "bundle_path": str(bundle_path),
            "motion_call_channels": motion_call_channels,
            "changed_unrelated_channels_in_bundle": changed_unrelated_channels,
        }
        report_path.write_text(
            json.dumps(report, indent=2, ensure_ascii=False) + "\n",
            encoding="utf-8",
        )
        print(json.dumps(report, indent=2, ensure_ascii=False))
        return 0
    finally:
        if http is not None:
            try:
                http.json("POST", "/v1/shutdown", {}, timeout_secs=10.0)
            except Exception:
                pass
        if proc is not None:
            try:
                proc.wait(timeout=15.0)
            except subprocess.TimeoutExpired:
                try:
                    proc.send_signal(signal.SIGTERM)
                except Exception:
                    pass
                try:
                    proc.wait(timeout=10.0)
                except Exception:
                    try:
                        proc.kill()
                    except Exception:
                        pass
        if drain_thread is not None and drain_thread.is_alive():
            drain_thread.join(timeout=5.0)
        if log_fp is not None:
            log_fp.close()


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        raise SystemExit(130)
