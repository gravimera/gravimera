#!/usr/bin/env python3

from __future__ import annotations

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


def load_automation_token(home_dir: Path) -> str | None:
    config_path = home_dir / "config.toml"
    if not config_path.exists():
        return None

    import tomllib

    cfg = tomllib.loads(config_path.read_text(encoding="utf-8"))
    token = cfg.get("automation_token")
    if isinstance(token, str) and token.strip():
        return token.strip()

    for section_name, key in (("automation", "token"), ("app", "automation_token")):
        section = cfg.get(section_name) or {}
        value = section.get(key)
        if isinstance(value, str) and value.strip():
            return value.strip()
    return None


def discover_base_url_from_output(
    proc: subprocess.Popen, log_fp, timeout_secs: float = 120.0
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
        "Timed out waiting for Automation API listen address. Last output:\n"
        + buf[-4000:].decode("utf-8", errors="replace")
    )


def http_json(
    method: str,
    url: str,
    data=None,
    *,
    token: str | None = None,
    timeout_secs: float = 20.0,
):
    body = None
    headers = {}
    if data is not None:
        body = json.dumps(data).encode("utf-8")
        headers["Content-Type"] = "application/json"
    if token:
        headers["Authorization"] = f"Bearer {token}"

    req = urllib.request.Request(url, method=method, data=body, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=timeout_secs) as resp:
            raw = resp.read().decode("utf-8")
            payload = json.loads(raw) if raw else {}
            return resp.status, payload
    except urllib.error.HTTPError as err:
        raw = err.read().decode("utf-8")
        payload = json.loads(raw) if raw else {}
        return err.code, payload


def wait_for_health(base_url: str, token: str | None, timeout_secs: float = 30.0):
    deadline = time.time() + timeout_secs
    last_err = None
    while time.time() < deadline:
        try:
            status, payload = http_json(
                "GET",
                f"{base_url}/v1/health",
                None,
                token=token,
                timeout_secs=3.0,
            )
            if status == 200 and payload.get("ok") is True:
                return payload
            last_err = RuntimeError(f"health not ok: status={status} payload={payload}")
        except Exception as e:  # pragma: no cover - real HTTP retries
            last_err = e
        time.sleep(0.2)
    raise RuntimeError(f"Timed out waiting for {base_url}/v1/health: {last_err}")


def step(base_url: str, token: str | None, frames: int = 5):
    http_json(
        "POST",
        f"{base_url}/v1/step",
        {"frames": frames},
        token=token,
        timeout_secs=30.0,
    )


def list_nested_gen3d_prefabs(home_dir: Path) -> list[dict]:
    prefabs_root = home_dir / "realm" / "default" / "prefabs"
    if not prefabs_root.exists():
        raise RuntimeError(f"missing prefabs root: {prefabs_root}")

    candidates = []
    for prefab_dir in prefabs_root.iterdir():
        if not prefab_dir.is_dir():
            continue
        if not (prefab_dir / "gen3d_edit_bundle_v1.json").exists():
            continue

        prefabs_dir = prefab_dir / "prefabs"
        root_json = prefabs_dir / f"{prefab_dir.name}.json"
        if not root_json.exists():
            continue

        try:
            by_id = {}
            for path in prefabs_dir.glob("*.json"):
                if path.name.endswith(".desc.json"):
                    continue
                obj = json.loads(path.read_text(encoding="utf-8"))
                prefab_id = obj.get("prefab_id")
                if isinstance(prefab_id, str) and prefab_id:
                    by_id[prefab_id] = obj
        except Exception:
            continue

        def depth(prefab_id: str, seen: tuple[str, ...] = ()) -> int:
            if prefab_id in seen:
                return 0
            obj = by_id.get(prefab_id)
            if not isinstance(obj, dict):
                return 0
            best_depth = 1
            for part in obj.get("parts") or []:
                kind = (part or {}).get("kind") or {}
                child_id = kind.get("object_id") if kind.get("kind") == "object_ref" else None
                if isinstance(child_id, str) and child_id:
                    best_depth = max(best_depth, 1 + depth(child_id, seen + (prefab_id,)))
            return best_depth

        root_depth = depth(prefab_dir.name)
        if root_depth <= 2:
            continue

        entry = {
            "prefab_id_uuid": prefab_dir.name,
            "depth": root_depth,
            "objects": len(by_id),
            "label": (by_id.get(prefab_dir.name) or {}).get("label") or prefab_dir.name,
        }
        candidates.append(entry)

    candidates.sort(
        key=lambda entry: (entry["depth"], entry["objects"], entry["prefab_id_uuid"]),
        reverse=True,
    )
    if not candidates:
        raise RuntimeError(
            f"no nested Gen3D prefab with depth > 2 found under {prefabs_root}"
        )
    return candidates


def wait_for_preview_components(
    base_url: str,
    token: str | None,
    timeout_secs: float = 45.0,
):
    deadline = time.time() + timeout_secs
    last_payload = None
    while time.time() < deadline:
        step(base_url, token, frames=3)
        try:
            status, payload = http_json(
                "GET",
                f"{base_url}/v1/gen3d/preview/components",
                None,
                token=token,
                timeout_secs=10.0,
            )
        except Exception:
            time.sleep(0.2)
            continue
        last_payload = payload
        components = payload.get("components") or []
        if status == 200 and payload.get("ok") is True and components:
            if any(isinstance(comp.get("projected"), dict) for comp in components):
                return payload
        time.sleep(0.1)

    raise RuntimeError(f"timed out waiting for preview components: last={last_payload}")


def get_preview_state(base_url: str, token: str | None) -> dict:
    status, payload = http_json(
        "GET",
        f"{base_url}/v1/gen3d/preview",
        None,
        token=token,
        timeout_secs=10.0,
    )
    if status != 200 or payload.get("ok") is not True:
        raise RuntimeError(f"preview state failed: status={status} payload={payload}")
    preview_state = payload.get("preview_state")
    if not isinstance(preview_state, dict):
        raise RuntimeError(f"preview state payload missing preview_state: {payload}")
    return preview_state


def vec3(value) -> tuple[float, float, float]:
    if not isinstance(value, list) or len(value) != 3:
        raise RuntimeError(f"expected vec3 list, got {value!r}")
    return (float(value[0]), float(value[1]), float(value[2]))


def vec3_add(
    a: tuple[float, float, float], b: tuple[float, float, float]
) -> tuple[float, float, float]:
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def vec3_distance(
    a: tuple[float, float, float], b: tuple[float, float, float]
) -> float:
    dx = a[0] - b[0]
    dy = a[1] - b[1]
    dz = a[2] - b[2]
    return float(dx * dx + dy * dy + dz * dz) ** 0.5


def frame_center(component: dict) -> tuple[float, float] | None:
    projected = component.get("projected")
    if not isinstance(projected, dict):
        return None
    frame = projected.get("frame_panel_logical") or {}
    minimum = frame.get("min") or []
    maximum = frame.get("max") or []
    if len(minimum) != 2 or len(maximum) != 2:
        return None
    return ((minimum[0] + maximum[0]) * 0.5, (minimum[1] + maximum[1]) * 0.5)


def frame_area(component: dict) -> float:
    projected = component.get("projected")
    if not isinstance(projected, dict):
        return float("inf")
    frame = projected.get("frame_panel_logical") or {}
    minimum = frame.get("min") or []
    maximum = frame.get("max") or []
    if len(minimum) != 2 or len(maximum) != 2:
        return float("inf")
    return max(0.0, (maximum[0] - minimum[0]) * (maximum[1] - minimum[1]))


def probe_components(base_url: str, token: str | None, components: list[dict]) -> list[dict]:
    hits = []
    seen = set()
    ordered = sorted(
        components,
        key=lambda comp: (-int(comp.get("depth") or 0), frame_area(comp), comp.get("entity_bits", 0)),
    )
    for component in ordered:
        center = frame_center(component)
        if center is None:
            continue
        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/preview/probe",
            {"x": center[0], "y": center[1]},
            token=token,
            timeout_secs=10.0,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"probe failed: status={status} payload={payload}")
        hovered = payload.get("hovered")
        if not isinstance(hovered, dict):
            continue
        entity_bits = hovered.get("entity_bits")
        if entity_bits in seen:
            continue
        seen.add(entity_bits)
        hits.append(
            {
                "probe": {"x": center[0], "y": center[1]},
                "expected_entity_bits": component.get("entity_bits"),
                "expected_label": component.get("label"),
                "hovered": hovered,
            }
        )
        if len(hits) >= 3:
            break
    return hits


def explode_offset_norm(component: dict) -> float:
    offset = component.get("explode_offset_local") or []
    if len(offset) != 3:
        return 0.0
    return float(offset[0] ** 2 + offset[1] ** 2 + offset[2] ** 2) ** 0.5


def label_anchor(component: dict) -> tuple[float, float] | None:
    projected = component.get("projected")
    if not isinstance(projected, dict):
        return None
    anchor = projected.get("label_anchor_panel_logical") or []
    if len(anchor) != 2:
        return None
    return (anchor[0], anchor[1])


def anchor_motion_stats(before_components: list[dict], after_components: list[dict]) -> dict:
    before_by_entity = {
        component["entity_bits"]: component
        for component in before_components
        if "entity_bits" in component
    }
    count = 0
    max_distance = 0.0
    over_25px = 0
    for component in after_components:
        entity_bits = component.get("entity_bits")
        if entity_bits not in before_by_entity:
            continue
        before_anchor = label_anchor(before_by_entity[entity_bits])
        after_anchor = label_anchor(component)
        if before_anchor is None or after_anchor is None:
            continue
        dx = after_anchor[0] - before_anchor[0]
        dy = after_anchor[1] - before_anchor[1]
        distance = (dx * dx + dy * dy) ** 0.5
        count += 1
        max_distance = max(max_distance, distance)
        if distance > 25.0:
            over_25px += 1
    return {
        "count": count,
        "max_distance": max_distance,
        "over_25px": over_25px,
    }


def main() -> int:
    script_dir = Path(__file__).resolve().parent
    repo_root = script_dir.parents[2]

    home_dir = Path("~/.gravimera").expanduser()
    token = load_automation_token(home_dir)
    prefab_candidates = list_nested_gen3d_prefabs(home_dir)

    tmp_root = script_dir / "tmp"
    tmp_root.mkdir(parents=True, exist_ok=True)
    run_root = Path(tempfile.mkdtemp(prefix="run_", dir=str(tmp_root)))
    log_path = run_root / "gravimera_stdout.log"

    print("Gen3D preview component inspection")
    print(f"- home: {home_dir}")
    print(f"- run_root: {run_root}")
    print(f"- nested_prefab_candidates: {len(prefab_candidates)}")
    (run_root / "prefab_candidates.json").write_text(
        json.dumps(prefab_candidates[:20], indent=2), encoding="utf-8"
    )

    env = os.environ.copy()
    env["GRAVIMERA_HOME"] = str(home_dir)

    cmd = [
        "cargo",
        "run",
        "--quiet",
        "--",
        "--automation",
        "--automation-bind",
        "127.0.0.1:0",
        "--automation-disable-local-input",
        "--automation-pause-on-start",
    ]

    proc = None
    log_fp = None
    drain_thread = None
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

        wait_for_health(base_url, token, timeout_secs=45.0)

        status, payload = http_json(
            "POST",
            f"{base_url}/v1/mode",
            {"mode": "build_preview"},
            token=token,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"/v1/mode failed: status={status} payload={payload}")
        step(base_url, token, frames=3)

        status, payload = http_json(
            "GET",
            f"{base_url}/v1/prefabs",
            None,
            token=token,
            timeout_secs=10.0,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"/v1/prefabs failed: status={status} payload={payload}")
        available_prefabs = {
            row.get("prefab_id_uuid")
            for row in payload.get("prefabs", [])
            if isinstance(row, dict) and row.get("prefab_id_uuid")
        }

        prefab = next(
            (
                candidate
                for candidate in prefab_candidates
                if candidate["prefab_id_uuid"] in available_prefabs
            ),
            None,
        )
        if prefab is None:
            raise RuntimeError(
                "no nested prefab candidate is available through /v1/prefabs"
            )

        print(f"- selected_prefab_id_uuid: {prefab['prefab_id_uuid']}")
        print(f"- selected_prefab_depth: {prefab['depth']}")
        print(f"- selected_prefab_objects: {prefab['objects']}")
        (run_root / "selected_prefab.json").write_text(
            json.dumps(prefab, indent=2), encoding="utf-8"
        )

        edit_payload = None
        for candidate in prefab_candidates:
            if candidate["prefab_id_uuid"] not in available_prefabs:
                continue
            status, payload = http_json(
                "POST",
                f"{base_url}/v1/gen3d/edit_from_prefab",
                {"prefab_id_uuid": candidate["prefab_id_uuid"]},
                token=token,
            )
            if status == 200 and payload.get("ok") is True:
                prefab = candidate
                edit_payload = payload
                break
            (run_root / f"edit_from_prefab_failed_{candidate['prefab_id_uuid']}.json").write_text(
                json.dumps({"status": status, "payload": payload}, indent=2),
                encoding="utf-8",
            )

        if edit_payload is None:
            raise RuntimeError("failed to seed any nested prefab via /v1/gen3d/edit_from_prefab")

        before = wait_for_preview_components(base_url, token)
        before_components = before.get("components") or []
        (run_root / "preview_before.json").write_text(
            json.dumps(before, indent=2), encoding="utf-8"
        )
        preview_state_before = get_preview_state(base_url, token)
        (run_root / "preview_state_before.json").write_text(
            json.dumps(preview_state_before, indent=2), encoding="utf-8"
        )

        camera_focus_before = vec3(preview_state_before.get("camera_focus"))
        draft_focus_before = vec3(preview_state_before.get("draft_focus"))
        view_pan_before = vec3(preview_state_before.get("view_pan"))
        if vec3_distance(view_pan_before, (0.0, 0.0, 0.0)) > 0.05:
            raise RuntimeError(f"expected preview pan to reset on draft load, got {preview_state_before}")
        if vec3_distance(camera_focus_before, draft_focus_before) > 0.15:
            raise RuntimeError(
                f"expected assembled preview camera focus to match draft focus, got {preview_state_before}"
            )

        nested_components = [c for c in before_components if int(c.get("depth") or 0) > 1]
        if not nested_components:
            raise RuntimeError("preview did not expose any nested components")

        projected_before = [c for c in before_components if isinstance(c.get("projected"), dict)]
        if len(projected_before) < 3:
            raise RuntimeError(
                f"expected at least 3 projected preview components, got {len(projected_before)}"
            )

        probe_hits_before = probe_components(base_url, token, projected_before)
        (run_root / "probe_hits_before.json").write_text(
            json.dumps(probe_hits_before, indent=2), encoding="utf-8"
        )
        distinct_before = {hit["hovered"]["entity_bits"] for hit in probe_hits_before}
        if len(distinct_before) < 2:
            raise RuntimeError(
                f"expected probing to resolve multiple components before explode, got {probe_hits_before}"
            )

        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/preview/explode",
            {"enabled": True},
            token=token,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"explode toggle failed: status={status} payload={payload}")

        after = wait_for_preview_components(base_url, token)
        after_components = after.get("components") or []
        (run_root / "preview_after.json").write_text(
            json.dumps(after, indent=2), encoding="utf-8"
        )
        preview_state_after_explode = get_preview_state(base_url, token)
        (run_root / "preview_state_after_explode.json").write_text(
            json.dumps(preview_state_after_explode, indent=2), encoding="utf-8"
        )

        camera_focus_after_explode = vec3(preview_state_after_explode.get("camera_focus"))
        exploded_center_after = preview_state_after_explode.get("exploded_component_center")
        if not isinstance(exploded_center_after, list) or len(exploded_center_after) != 3:
            raise RuntimeError(
                f"expected explode mode to report an exploded component center, got {preview_state_after_explode}"
            )
        exploded_center_after_vec = vec3(exploded_center_after)
        view_pan_after_explode = vec3(preview_state_after_explode.get("view_pan"))
        expected_focus_after_explode = vec3_add(
            exploded_center_after_vec, view_pan_after_explode
        )
        if vec3_distance(camera_focus_after_explode, expected_focus_after_explode) > 0.15:
            raise RuntimeError(
                "explode-mode camera focus is not anchored to exploded component center + pan: "
                f"{preview_state_after_explode}"
            )

        before_by_entity = {c["entity_bits"]: c for c in before_components if "entity_bits" in c}
        moved = 0
        non_zero_offsets = 0
        for component in after_components:
            entity_bits = component.get("entity_bits")
            if entity_bits not in before_by_entity:
                continue
            if explode_offset_norm(component) > 0.01:
                non_zero_offsets += 1
            before_anchor = label_anchor(before_by_entity[entity_bits])
            after_anchor = label_anchor(component)
            if before_anchor is None or after_anchor is None:
                continue
            dx = after_anchor[0] - before_anchor[0]
            dy = after_anchor[1] - before_anchor[1]
            if (dx * dx + dy * dy) ** 0.5 > 4.0:
                moved += 1

        if non_zero_offsets < 2:
            raise RuntimeError(
                f"expected at least 2 components with non-zero explode offsets, got {non_zero_offsets}"
            )
        if moved < 2:
            raise RuntimeError(
                f"expected at least 2 components to move visibly after explode, got {moved}"
            )

        projected_after = [c for c in after_components if isinstance(c.get("projected"), dict)]
        probe_hits_after = probe_components(base_url, token, projected_after)
        (run_root / "probe_hits_after.json").write_text(
            json.dumps(probe_hits_after, indent=2), encoding="utf-8"
        )
        distinct_after = {hit["hovered"]["entity_bits"] for hit in probe_hits_after}
        if len(distinct_after) < 2:
            raise RuntimeError(
                f"expected probing to resolve multiple components after explode, got {probe_hits_after}"
            )

        status, payload = http_json(
            "POST",
            f"{base_url}/v1/gen3d/preview/pan",
            {"dx": 2.0, "dy": -1.5},
            token=token,
            timeout_secs=10.0,
        )
        if status != 200 or payload.get("ok") is not True:
            raise RuntimeError(f"preview pan failed: status={status} payload={payload}")

        step(base_url, token, frames=2)
        preview_state_after_pan = get_preview_state(base_url, token)
        (run_root / "preview_state_after_pan.json").write_text(
            json.dumps(preview_state_after_pan, indent=2), encoding="utf-8"
        )
        camera_focus_after_pan = vec3(preview_state_after_pan.get("camera_focus"))
        exploded_center_after_pan = preview_state_after_pan.get("exploded_component_center")
        if not isinstance(exploded_center_after_pan, list) or len(exploded_center_after_pan) != 3:
            raise RuntimeError(
                f"expected exploded component center after pan, got {preview_state_after_pan}"
            )
        exploded_center_after_pan_vec = vec3(exploded_center_after_pan)
        view_pan_after_pan = vec3(preview_state_after_pan.get("view_pan"))
        if vec3_distance(view_pan_after_pan, (0.0, 0.0, 0.0)) < 0.2:
            raise RuntimeError(f"expected preview pan to move the camera focus, got {preview_state_after_pan}")
        expected_focus_after_pan = vec3_add(
            exploded_center_after_pan_vec, view_pan_after_pan
        )
        if vec3_distance(camera_focus_after_pan, expected_focus_after_pan) > 0.15:
            raise RuntimeError(
                "panned explode-mode camera focus is not anchored to exploded center + pan: "
                f"{preview_state_after_pan}"
            )
        if vec3_distance(camera_focus_after_pan, camera_focus_after_explode) < 0.2:
            raise RuntimeError(
                f"expected preview pan to change camera focus, got before={preview_state_after_explode} after={preview_state_after_pan}"
            )

        after_pan = wait_for_preview_components(base_url, token, timeout_secs=10.0)
        after_pan_components = after_pan.get("components") or []
        (run_root / "preview_after_pan.json").write_text(
            json.dumps(after_pan, indent=2), encoding="utf-8"
        )

        step(base_url, token, frames=1)
        after_next = wait_for_preview_components(base_url, token, timeout_secs=10.0)
        after_next_components = after_next.get("components") or []
        (run_root / "preview_after_next.json").write_text(
            json.dumps(after_next, indent=2), encoding="utf-8"
        )
        motion_stats = anchor_motion_stats(after_pan_components, after_next_components)
        if motion_stats["count"] >= 4 and motion_stats["over_25px"] > 2:
            raise RuntimeError(
                "explode layout is unstable across adjacent frames: "
                f"{motion_stats}"
            )

        summary = {
            "prefab": prefab,
            "projected_before": len(projected_before),
            "nested_before": len(nested_components),
            "distinct_probe_hits_before": len(distinct_before),
            "distinct_probe_hits_after": len(distinct_after),
            "components_moved_after_explode": moved,
            "components_with_non_zero_offsets": non_zero_offsets,
            "camera_focus_after_explode": camera_focus_after_explode,
            "camera_focus_after_pan": camera_focus_after_pan,
            "view_pan_after_pan": view_pan_after_pan,
            "explode_frame_to_frame_motion": motion_stats,
            "artifacts_dir": str(run_root),
        }
        (run_root / "summary.json").write_text(
            json.dumps(summary, indent=2), encoding="utf-8"
        )
        print(json.dumps(summary, indent=2))
        return 0
    finally:
        if proc is not None and proc.poll() is None:
            proc.send_signal(signal.SIGINT)
            try:
                proc.wait(timeout=20)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=10)
        if drain_thread is not None:
            drain_thread.join(timeout=2.0)
        if log_fp is not None:
            log_fp.close()


if __name__ == "__main__":
    raise SystemExit(main())
