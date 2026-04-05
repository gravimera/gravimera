#!/usr/bin/env python3
"""
Build showcase scenes in the user's default Gravimera home (~/.gravimera)
by driving the rendered game via the local Automation HTTP API.

This tool is designed to be resumable across interruptions:

- Repo-local artifacts live under --run-dir (default: test/run_1/showcase_scene_<timestamp>/):
  - gravimera_stdout.log
  - manifest.json
  - shots/*.png
- Durable scene build steps are stored by the engine under:
  ~/.gravimera/realm/<realm_id>/scenes/<scene_id>/runs/<run_id>/steps/...

Notes:
- This tool never prints or persists AI tokens; it only reads config for optional automation auth.
- It starts the game with `cargo run --release` as requested for smoother generation.
"""

from __future__ import annotations

import argparse
import json
import os
import random
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


MANIFEST_VERSION = 1


@dataclass(frozen=True)
class SceneProfile:
    profile_id: str
    scene_prefix: str
    run_dir_prefix: str
    label_prefix: str
    description: str
    floor_prompt: str
    min_terrain_size_m: float
    layout_kind: str
    asset_plan: list[tuple[str, str]]


def _now_tag() -> str:
    return time.strftime("%Y%m%d_%H%M%S", time.localtime())


def _today_yyyymmdd() -> str:
    return time.strftime("%Y%m%d", time.localtime())


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8")


def _write_json(path: Path, doc: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(doc, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def _read_json(path: Path) -> dict[str, Any]:
    return json.loads(path.read_text(encoding="utf-8"))


def gravimera_default_home_dir() -> Path:
    # This builder intentionally targets the user's default Gravimera home so the generated
    # scene is usable later without copying any repo-local artifacts.
    return Path("~/.gravimera").expanduser()


def read_terrain_size_m(
    *,
    gravimera_home: Path,
    realm_id: str,
    terrain_id_uuid: str,
) -> tuple[float, float] | None:
    """
    Best-effort read of terrain size from the realm terrain store on disk.
    Returns (size_x_m, size_z_m) or None if unknown.
    """
    path = (
        gravimera_home
        / "realm"
        / str(realm_id)
        / "terrain"
        / str(terrain_id_uuid)
        / "terrain_def_v1.json"
    )
    if not path.exists():
        return None
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
        mesh = doc.get("mesh") or {}
        size = mesh.get("size_m") or []
        if not (isinstance(size, list) and len(size) == 2):
            return None
        sx = float(size[0])
        sz = float(size[1])
        if not (sx > 0.0 and sz > 0.0):
            return None
        return (sx, sz)
    except Exception:
        return None


def _clamp(v: float, lo: float, hi: float) -> float:
    if v < lo:
        return lo
    if v > hi:
        return hi
    return v


def _yaw_deg_to_quat(yaw_deg: float) -> dict[str, float]:
    # Rotation around Y axis.
    r = float(yaw_deg) * 3.141592653589793 / 180.0
    half = 0.5 * r
    return {"x": 0.0, "y": float(math_sin(half)), "z": 0.0, "w": float(math_cos(half))}


def math_sin(v: float) -> float:
    import math

    return math.sin(v)


def math_cos(v: float) -> float:
    import math

    return math.cos(v)


class LocalHttp:
    def __init__(self, base_url: str, *, token: str | None = None):
        self.base_url = base_url.rstrip("/")
        self.token = token.strip() if token and token.strip() else None
        # Avoid environment proxies for loopback calls.
        self._opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def _headers(self) -> dict[str, str]:
        headers = {"Content-Type": "application/json"}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        return headers

    def json(
        self,
        method: str,
        path: str,
        body: dict[str, Any] | None = None,
        *,
        timeout_secs: float = 30.0,
        retries: int = 5,
        retry_sleep_secs: float = 0.35,
    ) -> dict[str, Any]:
        url = f"{self.base_url}{path}"
        data = json.dumps(body).encode("utf-8") if body is not None else None
        req = urllib.request.Request(url, data=data, headers=self._headers(), method=method)

        last_err: str | None = None
        for attempt in range(retries):
            try:
                with self._opener.open(req, timeout=timeout_secs) as resp:
                    raw = resp.read().decode("utf-8", errors="replace")
                    if not raw.strip():
                        raise RuntimeError(f"Empty response body from {url}")
                    return json.loads(raw)
            except urllib.error.HTTPError as err:
                raw = err.read().decode("utf-8", errors="replace")
                raise RuntimeError(f"HTTP {err.code} {url}: {raw.strip()}") from None
            except (urllib.error.URLError, TimeoutError, OSError) as err:
                last_err = str(err)
                if attempt + 1 >= retries:
                    break
                time.sleep(retry_sleep_secs * (1.0 + attempt * 0.35))
        raise RuntimeError(f"Request failed {url}: {last_err}")


class GameProcess:
    def __init__(
        self,
        *,
        repo_root: Path,
        config_path: Path,
        stdout_path: Path,
        extra_env: dict[str, str] | None = None,
        use_release: bool = True,
    ):
        self._repo_root = repo_root
        self._config_path = config_path
        self._stdout_path = stdout_path
        self._use_release = use_release
        self._proc: subprocess.Popen[bytes] | None = None
        self._env = os.environ.copy()
        if extra_env:
            self._env.update(extra_env)

    def start(self) -> None:
        self._stdout_path.parent.mkdir(parents=True, exist_ok=True)
        out = open(self._stdout_path, "wb")

        cmd: list[str] = [
            "cargo",
            "run",
            "--release" if self._use_release else "",
            "--quiet",
            "--",
            "--config",
            str(self._config_path),
            "--automation",
            "--automation-bind",
            "127.0.0.1:0",
            "--automation-disable-local-input",
            "--automation-pause-on-start",
        ]
        cmd = [c for c in cmd if c]

        self._proc = subprocess.Popen(
            cmd,
            cwd=str(self._repo_root),
            env=self._env,
            stdout=out,
            stderr=subprocess.STDOUT,
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

    def ensure_stopped(self) -> None:
        self.terminate()
        time.sleep(0.8)
        self.kill()
        time.sleep(0.5)


def discover_base_url_from_log(log_path: Path, *, timeout_secs: float = 90.0) -> str:
    deadline = time.time() + timeout_secs
    prefix = "Automation API listening on "
    last_size = 0
    while time.time() < deadline:
        if log_path.exists():
            text = log_path.read_text(encoding="utf-8", errors="replace")
            if "Rendered mode exited with an error. Falling back to headless mode." in text:
                raise RuntimeError("gravimera fell back to headless mode; expected rendered mode")
            for line in text.splitlines():
                if prefix in line:
                    url = line.split(prefix, 1)[1].strip()
                    if url.startswith("http://"):
                        return url.rstrip("/")
            size = log_path.stat().st_size
            if size != last_size:
                last_size = size
        time.sleep(0.2)
    tail = ""
    if log_path.exists():
        tail = log_path.read_text(encoding="utf-8", errors="replace")[-4000:]
    raise RuntimeError(f"Timed out waiting for automation listen addr. Log tail:\n{tail}")


def wait_health(http: LocalHttp, *, timeout_secs: float = 45.0) -> None:
    t0 = time.monotonic()
    last_err: str | None = None
    while time.monotonic() - t0 < timeout_secs:
        try:
            health = http.json("GET", "/v1/health", None, timeout_secs=0.75)
            if health.get("ok") is True:
                return
        except Exception as err:
            last_err = str(err)
        time.sleep(0.15)
    raise RuntimeError(f"Timed out waiting for /v1/health. last_err={last_err!r}")


def step(http: LocalHttp, frames: int, *, dt_secs: float) -> None:
    http.json("POST", "/v1/step", {"frames": int(frames), "dt_secs": float(dt_secs)}, timeout_secs=300.0)


def set_mode(http: LocalHttp, mode: str, *, dt_secs: float) -> None:
    http.json("POST", "/v1/mode", {"mode": mode})
    step(http, 3, dt_secs=dt_secs)


def list_scenes(http: LocalHttp, *, realm_id: str) -> set[str]:
    payload = http.json("GET", "/v1/realm_scene/list")
    realms = payload.get("realms") or []
    for r in realms:
        if str(r.get("realm_id") or "") == realm_id:
            scenes = r.get("scenes") or []
            return {str(s) for s in scenes}
    return set()


def pick_versioned_scene_id(existing: set[str], *, prefix: str) -> str:
    date = _today_yyyymmdd()
    base = f"{prefix}_{date}"
    for v in range(1, 100):
        candidate = f"{base}_v{v}"
        if candidate not in existing:
            return candidate
    # Fallback: include timestamp.
    return f"{base}_{_now_tag()}"


def realm_scene_create_and_switch(
    http: LocalHttp,
    *,
    realm_id: str,
    scene_id: str,
    label: str,
    description: str,
    dt_secs: float,
) -> dict[str, Any]:
    resp = http.json(
        "POST",
        "/v1/realm_scene/create",
        {
            "realm_id": realm_id,
            "scene_id": scene_id,
            "label": label,
            "description": description,
            "switch_to": True,
        },
    )
    # Switching is deferred; step a few frames.
    step(http, 5, dt_secs=dt_secs)
    return resp


def get_active_scene_dirs(http: LocalHttp) -> dict[str, str]:
    resp = http.json("GET", "/v1/realm_scene/active")
    if not resp.get("ok"):
        raise RuntimeError(f"realm_scene/active failed: {resp}")
    return {
        "realm_id": str(resp.get("realm_id") or ""),
        "scene_id": str(resp.get("scene_id") or ""),
        "scene_dir": str(resp.get("scene_dir") or ""),
        "scene_src_dir": str(resp.get("scene_src_dir") or ""),
        "scene_build_dir": str(resp.get("scene_build_dir") or ""),
    }


def import_scene_sources(http: LocalHttp, src_dir: str, *, dt_secs: float) -> None:
    http.json("POST", "/v1/scene_sources/import", {"src_dir": src_dir})
    step(http, 2, dt_secs=dt_secs)


def ensure_run_id(manifest: dict[str, Any], *, profile_id: str) -> str:
    run_id = str(manifest.get("scene_run_id") or "").strip()
    if run_id:
        return run_id
    # Keep it stable for this run dir.
    run_id = f"{profile_id}_build_{_now_tag()}"
    manifest["scene_run_id"] = run_id
    return run_id


def scene_run_status(http: LocalHttp, run_id: str) -> dict[str, Any]:
    return http.json("POST", "/v1/scene_sources/run_status", {"run_id": run_id})


def scorecard_default() -> dict[str, Any]:
    return {
        "format_version": 1,
        "hard_gates": [
            {"kind": "schema"},
            {"kind": "budget", "max_instances": 40000, "max_portals": 2000},
        ],
    }


def apply_run_step(
    http: LocalHttp,
    *,
    run_id: str,
    step_no: int,
    patch_ops: list[dict[str, Any]],
) -> dict[str, Any]:
    patch = {"format_version": 1, "request_id": f"{run_id}_{step_no:04}", "ops": patch_ops}
    resp = http.json(
        "POST",
        "/v1/scene_sources/run_apply_patch",
        {"run_id": run_id, "step": int(step_no), "scorecard": scorecard_default(), "patch": patch},
        timeout_secs=300.0,
        retries=3,
    )
    return resp


def get_prefab_catalog(http: LocalHttp) -> dict[str, dict[str, Any]]:
    payload = http.json("GET", "/v1/prefabs")
    prefabs = payload.get("prefabs") or []
    out: dict[str, dict[str, Any]] = {}
    for p in prefabs:
        pid = str(p.get("prefab_id_uuid") or "").strip()
        if not pid:
            continue
        out[pid] = p
    return out


def reload_realm_prefabs(http: LocalHttp) -> dict[str, Any]:
    """
    Ensure realm-prefab packages saved on disk are loaded into the running world.

    This is important after restarting the game or switching realm/scene, because the in-memory
    prefab library can reset to builtins. Scene-sources patch validation requires prefabs to be
    present in the library.
    """
    resp = http.json("POST", "/v1/prefabs/reload_realm", {})
    if not resp.get("ok"):
        raise RuntimeError(f"prefabs/reload_realm failed: {resp}")
    return resp


def enqueue_gen3d_task(
    http: LocalHttp,
    *,
    kind: str,
    prompt: str | None = None,
    prefab_id_uuid: str | None = None,
) -> str:
    body: dict[str, Any] = {"kind": str(kind)}
    if prompt is not None:
        body["prompt"] = str(prompt)
    if prefab_id_uuid is not None:
        body["prefab_id_uuid"] = str(prefab_id_uuid)
    resp = http.json("POST", "/v1/gen3d/tasks/enqueue", body)
    task_id = str(resp.get("task_id") or "").strip()
    if not task_id:
        raise RuntimeError(f"gen3d/tasks/enqueue returned no task_id: {resp}")
    return task_id


def list_gen3d_tasks(http: LocalHttp) -> dict[str, dict[str, Any]]:
    resp = http.json("GET", "/v1/gen3d/tasks")
    tasks = resp.get("tasks") or []
    out: dict[str, dict[str, Any]] = {}
    for t in tasks:
        tid = str(t.get("task_id") or "").strip()
        if tid:
            out[tid] = t
    return out


def poll_gen3d_task(
    http: LocalHttp,
    *,
    task_id: str,
    dt_secs: float,
    timeout_secs: float = 3600.0,
) -> str:
    t0 = time.monotonic()
    last_state = None
    last_status = None
    while True:
        step(http, 10, dt_secs=dt_secs)
        resp = http.json("GET", f"/v1/gen3d/tasks/{task_id}")
        task = resp.get("task") or {}
        state = str(task.get("state") or "")
        status = str(task.get("status") or "")
        if state != last_state or status != last_status:
            msg = status.replace("\n", " ").strip()
            if len(msg) > 140:
                msg = msg[:137] + "…"
            print(f"gen3d task {task_id[:8]} state={state} status={msg}")
            last_state = state
            last_status = status

        if state in ("done", "failed", "canceled"):
            prefab_id = str(task.get("result_prefab_id_uuid") or "").strip()
            if state == "done" and prefab_id:
                return prefab_id
            err = task.get("error")
            raise RuntimeError(f"Gen3D task {task_id} ended state={state} error={err!r}")

        if time.monotonic() - t0 > timeout_secs:
            raise RuntimeError(f"Timed out waiting for gen3d task {task_id}")
        time.sleep(0.05)


def build_genfloor_flat(http: LocalHttp, *, dt_secs: float, prompt: str) -> str:
    set_mode(http, "floor_preview", dt_secs=dt_secs)
    http.json("POST", "/v1/genfloor/new", {})
    http.json("POST", "/v1/genfloor/prompt", {"prompt": prompt})
    http.json("POST", "/v1/genfloor/build", {})

    t0 = time.monotonic()
    last_msg = None
    while True:
        step(http, 10, dt_secs=dt_secs)
        status = http.json("GET", "/v1/genfloor/status")
        running = bool(status.get("running"))
        msg = str(status.get("status") or "").strip()
        if msg != last_msg:
            print(f"genfloor running={running} status={msg}")
            last_msg = msg
        if not running:
            floor_id = str(status.get("last_saved_floor_id_uuid") or "").strip()
            if not floor_id:
                raise RuntimeError(f"genfloor completed but no last_saved_floor_id_uuid: {status}")
            return floor_id
        if time.monotonic() - t0 > 1800.0:
            raise RuntimeError("Timed out waiting for genfloor build (30m)")
        time.sleep(0.05)


def select_scene_terrain(http: LocalHttp, floor_id_uuid: str, *, dt_secs: float) -> None:
    http.json("POST", "/v1/scene/terrain/select", {"floor_id_uuid": floor_id_uuid})
    step(http, 2, dt_secs=dt_secs)


def set_camera_and_shot(
    http: LocalHttp,
    *,
    x: float,
    y: float,
    z: float,
    yaw: float,
    pitch: float,
    zoom_t: float,
    out_path: Path,
    dt_secs: float,
) -> None:
    http.json(
        "POST",
        "/v1/camera",
        {
            "focus": [float(x), float(y), float(z)],
            "yaw": float(yaw),
            "pitch": float(pitch),
            "zoom_t": float(zoom_t),
        },
    )
    # Let camera state apply on the next frame.
    step(http, 2, dt_secs=dt_secs)
    http.json("POST", "/v1/screenshot", {"path": str(out_path)}, timeout_secs=300.0)
    # Screenshot write is async; step a little.
    t0 = time.monotonic()
    while time.monotonic() - t0 < 10.0:
        step(http, 2, dt_secs=dt_secs)
        if out_path.exists():
            return
        time.sleep(0.05)
    print(f"warn: screenshot not written yet: {out_path}")


def layer_doc_explicit(layer_id: str, instances: list[dict[str, Any]]) -> dict[str, Any]:
    return {
        "format_version": 1,
        "layer_id": layer_id,
        "kind": "explicit_instances",
        "instances": instances,
    }


def make_instance(
    *,
    local_id: str,
    prefab_id_uuid: str,
    x: float,
    y: float,
    z: float,
    yaw_deg: float,
    scale: float,
    tint_rgba: dict[str, float] | None = None,
) -> dict[str, Any]:
    quat = _yaw_deg_to_quat(float(yaw_deg))
    inst: dict[str, Any] = {
        "local_id": local_id,
        "prefab_id": prefab_id_uuid,
        "transform": {
            "translation": {"x": float(x), "y": float(y), "z": float(z)},
            "rotation": quat,
            "scale": {"x": float(scale), "y": float(scale), "z": float(scale)},
        },
    }
    if tint_rgba:
        inst["tint_rgba"] = tint_rgba
    return inst


def grounded_y(prefab_catalog: dict[str, dict[str, Any]], prefab_id_uuid: str, *, scale: float) -> float:
    info = prefab_catalog.get(prefab_id_uuid)
    if not info:
        # Best effort: assume centered 1m tall.
        return 0.5 * float(scale)
    base = float(info.get("ground_origin_y") or 0.0)
    return base * float(scale)


def build_layout_layers_chrome(
    *,
    prefab_catalog: dict[str, dict[str, Any]],
    assets: dict[str, str],
    layout_extent_m: float,
    plaza_extent_m: float,
) -> dict[str, dict[str, Any]]:
    """
    Returns {layer_id: layer_doc}.
    """
    def pid(key: str) -> str | None:
        v = assets.get(key)
        pid = str(v).strip() if v else ""
        if not pid:
            return None
        # Never reference unknown prefabs in patches; it will fail `scene_sources` validation.
        if pid not in prefab_catalog:
            return None
        return pid

    road = pid("road_tile")
    sidewalk = pid("sidewalk_tile")
    plaza = pid("plaza_tile") or sidewalk
    crosswalk = pid("crosswalk_tile")

    light_neon = pid("streetlight_neon")
    light_old = pid("streetlight_old")
    bench_modern = pid("bench_modern")
    bench_old = pid("bench_old")
    billboard = pid("billboard_holo")
    holo_pillar = pid("holo_sign_pillar")

    fountain = pid("fountain_chrome")
    statue = pid("statue_abstract")
    kiosk = pid("kiosk_info")
    vendor = pid("vendor_stall")
    planter_tree = pid("planter_tree")
    planter_flowers = pid("planter_flowers")
    trash_bin = pid("trash_bin")
    bollard = pid("bollard")

    skybridge = pid("skybridge_module")

    # Buildings: modern ring + old district + spaceport corner.
    modern_building_keys = [
        "tower_chrome_tall",
        "tower_chrome_mid",
        "tower_chrome_spire",
        "tower_chrome_twist",
        "tower_glass_arc",
        "residential_pods",
        "hotel_sleek",
        "lab_research",
        "mall_plaza",
    ]
    old_building_keys = [
        "building_old_brick",
        "building_old_artdeco",
        "building_old_clocktower",
        "building_old_market",
        "building_old_factory",
        "building_old_shrine",
        "building_old_townhouse",
    ]
    modern_buildings: list[str] = [v for k in modern_building_keys if (v := pid(k))]
    old_buildings: list[str] = [v for k in old_building_keys if (v := pid(k))]

    dome_terminal = pid("dome_terminal")
    hangar = pid("hangar_spaceport")
    ship = pid("ship_starship_lander")

    # Vehicles and units.
    ground_vehicle_keys = [
        "vehicle_hovercar",
        "vehicle_hovercar_taxi",
        "vehicle_hoverbike",
        "vehicle_cargo_truck",
        "vehicle_service_van",
        "vehicle_police_patrol",
    ]
    air_vehicle_keys = [
        "vehicle_skybus",
        "vehicle_aerial_taxi",
        "vehicle_drone_courier",
        "vehicle_shuttle",
    ]
    unit_keys = [
        "unit_robot_worker",
        "unit_robot_security",
        "unit_robot_medic",
        "unit_robot_vendor",
        "unit_alien_diplomat",
        "unit_alien_merchant",
        "unit_alien_scientist",
        "unit_alien_child",
        "unit_human_civilian",
        "unit_human_pilot",
        "unit_android_artist",
        "unit_alien_guardian",
    ]
    drone_keys = ["unit_drone_camera", "unit_drone_security"]
    ground_vehicles: list[str] = [v for k in ground_vehicle_keys if (v := pid(k))]
    air_vehicles: list[str] = [v for k in air_vehicle_keys if (v := pid(k))]
    units: list[str] = [v for k in unit_keys if (v := pid(k))]
    drones: list[str] = [v for k in drone_keys if (v := pid(k))]

    layers: dict[str, dict[str, Any]] = {}

    infra_roads: list[dict[str, Any]] = []
    infra_plaza: list[dict[str, Any]] = []
    deco: list[dict[str, Any]] = []
    buildings_modern: list[dict[str, Any]] = []
    buildings_old: list[dict[str, Any]] = []
    district_spaceport: list[dict[str, Any]] = []
    vehicles_ground: list[dict[str, Any]] = []
    vehicles_air: list[dict[str, Any]] = []
    population_walk: list[dict[str, Any]] = []
    population_fly: list[dict[str, Any]] = []

    # "City scale" placement unit for non-tile objects (benches, lights, buildings, vehicles).
    # Tile spacing for roads/plaza is derived from prefab sizes below.
    spacing = 10.0
    extent = max(spacing, float(layout_extent_m))
    plaza_extent = max(spacing, min(float(plaza_extent_m), extent - spacing))

    def snap(v: float) -> float:
        return round(v / spacing) * spacing

    road_step = spacing
    if road:
        info = prefab_catalog.get(road) or {}
        size = info.get("size") or []
        if isinstance(size, list) and len(size) == 3:
            road_step = _clamp(float(size[0]), 2.0, 12.0)

    plaza_step = spacing
    if plaza:
        info = prefab_catalog.get(plaza) or {}
        size = info.get("size") or []
        if isinstance(size, list) and len(size) == 3:
            plaza_step = _clamp(float(size[0]), 0.8, 6.0)

    # --- Color palette (pastel neon accents) ---
    #
    # The overall theme is "utopian chrome", but the user wants the scene to be colorful on first
    # glance. We keep the base materials clean and use deterministic tints to add readable color.
    palette: list[dict[str, float]] = [
        {"r": 0.32, "g": 0.82, "b": 1.00, "a": 1.0},  # cyan
        {"r": 1.00, "g": 0.36, "b": 0.86, "a": 1.0},  # magenta
        {"r": 1.00, "g": 0.86, "b": 0.30, "a": 1.0},  # gold
        {"r": 0.55, "g": 1.00, "b": 0.55, "a": 1.0},  # lime
        {"r": 0.78, "g": 0.58, "b": 1.00, "a": 1.0},  # lavender
    ]

    def palette_pick(seed: int) -> dict[str, float]:
        return dict(palette[seed % len(palette)])

    def palette_pick_muted(seed: int, *, mix: float = 0.35) -> dict[str, float]:
        # Mix the palette color toward white so chrome stays chrome, but picks up color accents.
        c = palette_pick(seed)
        mix = float(_clamp(float(mix), 0.0, 1.0))
        return {
            "r": (1.0 - mix) + mix * float(c["r"]),
            "g": (1.0 - mix) + mix * float(c["g"]),
            "b": (1.0 - mix) + mix * float(c["b"]),
            "a": 1.0,
        }

    def plaza_tile_tint(ix: int, iz: int, x: float, z: float) -> dict[str, float] | None:
        # Deterministic pattern: more vibrant in the plaza center, sparse accents in the ring.
        r = float((x * x + z * z) ** 0.5)
        if r > plaza_extent * 0.66:
            return None
        h = (ix * 1103515245 + iz * 12345) & 0xFFFFFFFF
        if r < plaza_extent * 0.38:
            return palette_pick(h)
        if (h % 19) == 0:
            return palette_pick(h // 19)
        return None

    # --- Streets: a grid of boulevards ---
    if road:
        if extent >= 120.0:
            fracs = [-0.45, -0.22, 0.0, 0.22, 0.45]
        elif extent >= 80.0:
            fracs = [-0.4, -0.2, 0.0, 0.2, 0.4]
        elif extent >= 55.0:
            fracs = [-0.3, 0.0, 0.3]
        else:
            fracs = [0.0]

        road_lines: list[float] = []
        for f in fracs:
            v = snap(float(f) * extent)
            if abs(v) <= extent - spacing + 1e-3:
                road_lines.append(v)
        if 0.0 not in road_lines:
            road_lines.append(0.0)
        road_lines = sorted(set(road_lines))
        i = 0
        for z_line in road_lines:
            x = -extent
            while x <= extent + 1e-3:
                y = grounded_y(prefab_catalog, road, scale=1.0)
                infra_roads.append(
                    make_instance(
                        local_id=f"road_x_{int(z_line):+03}_{i:04}",
                        prefab_id_uuid=road,
                        x=x,
                        y=y,
                        z=float(z_line),
                        yaw_deg=0.0,
                        scale=1.0,
                    )
                )
                x += road_step
                i += 1

        i = 0
        for x_line in road_lines:
            z = -extent
            while z <= extent + 1e-3:
                # Skip intersections (already covered by X roads) to reduce z-fighting.
                if any(abs(z - zl) < (road_step * 0.5) for zl in road_lines):
                    z += road_step
                    i += 1
                    continue
                y = grounded_y(prefab_catalog, road, scale=1.0)
                infra_roads.append(
                    make_instance(
                        local_id=f"road_z_{int(x_line):+03}_{i:04}",
                        prefab_id_uuid=road,
                        x=float(x_line),
                        y=y,
                        z=z,
                        yaw_deg=90.0,
                        scale=1.0,
                    )
                )
                z += road_step
                i += 1

    # --- Central plaza ---
    if plaza:
        ix = 0
        x = -plaza_extent
        while x <= plaza_extent + 1e-3:
            iz = 0
            z = -plaza_extent
            while z <= plaza_extent + 1e-3:
                py = grounded_y(prefab_catalog, plaza, scale=1.0)
                tint = plaza_tile_tint(ix, iz, float(x), float(z))
                infra_plaza.append(
                    make_instance(
                        local_id=f"plaza_{ix:02}_{iz:02}",
                        prefab_id_uuid=plaza,
                        x=x,
                        y=py,
                        z=z,
                        yaw_deg=0.0,
                        scale=1.0,
                        tint_rgba=tint,
                    )
                )
                z += plaza_step
                iz += 1
            x += plaza_step
            ix += 1

    # --- Crosswalks near the center ---
    if crosswalk:
        cross_line = snap(min(extent - spacing, max(spacing, extent * 0.22)))
        cross_x = spacing
        for idx, (x, z, yaw) in enumerate(
            [
                (-cross_x, -cross_line, 0.0),
                (cross_x, -cross_line, 0.0),
                (-cross_x, cross_line, 0.0),
                (cross_x, cross_line, 0.0),
                (-cross_line, -cross_x, 90.0),
                (-cross_line, cross_x, 90.0),
                (cross_line, -cross_x, 90.0),
                (cross_line, cross_x, 90.0),
            ]
        ):
            cy = grounded_y(prefab_catalog, crosswalk, scale=1.0)
            deco.append(
                make_instance(
                    local_id=f"crosswalk_{idx:02}",
                    prefab_id_uuid=crosswalk,
                    x=x,
                    y=cy,
                    z=z,
                    yaw_deg=yaw,
                    scale=1.0,
                )
            )

    # --- Plaza centerpiece ---
    if fountain:
        fy = grounded_y(prefab_catalog, fountain, scale=1.4)
        deco.append(
            make_instance(
                local_id="fountain_center",
                prefab_id_uuid=fountain,
                x=0.0,
                y=fy,
                z=0.0,
                yaw_deg=0.0,
                scale=1.4,
            )
        )
    if statue:
        sy = grounded_y(prefab_catalog, statue, scale=1.1)
        statue_dx = max(3.0, plaza_extent * 0.15)
        statue_dz = max(2.0, plaza_extent * 0.11)
        deco.append(
            make_instance(
                local_id="statue_center",
                prefab_id_uuid=statue,
                x=statue_dx,
                y=sy,
                z=-statue_dz,
                yaw_deg=35.0,
                scale=1.1,
            )
        )

    # --- Street furniture (modern core + old district) ---
    if light_neon:
        lane = spacing * 1.2
        step_m = spacing * 2.0
        span = max(spacing * 4.0, extent - spacing * 2.0)
        k = 0
        pos = -span
        while pos <= span + 1e-3:
            ly = grounded_y(prefab_catalog, light_neon, scale=1.0)
            deco.append(
                make_instance(local_id=f"light_neon_n_{k:03}", prefab_id_uuid=light_neon, x=float(pos), y=ly, z=-lane, yaw_deg=0.0, scale=1.0)
            )
            deco.append(
                make_instance(local_id=f"light_neon_p_{k:03}", prefab_id_uuid=light_neon, x=float(pos), y=ly, z=lane, yaw_deg=180.0, scale=1.0)
            )
            pos += step_m
            k += 1

    if light_old:
        # Old district lamps (SE quadrant)
        old_base = snap(max(plaza_extent + spacing * 2.0, extent * 0.58))
        old_end = snap(min(extent - spacing * 2.0, old_base + extent * 0.32))
        step_m = max(12.0, spacing * 1.5)
        k = 0
        pos = old_base
        while pos <= old_end + 1e-3:
            ly = grounded_y(prefab_catalog, light_old, scale=1.0)
            deco.append(
                make_instance(local_id=f"light_old_x_{k:03}", prefab_id_uuid=light_old, x=float(pos), y=ly, z=old_base, yaw_deg=0.0, scale=1.0)
            )
            deco.append(
                make_instance(local_id=f"light_old_z_{k:03}", prefab_id_uuid=light_old, x=old_base, y=ly, z=float(pos), yaw_deg=90.0, scale=1.0)
            )
            pos += step_m
            k += 1

    if bench_modern:
        by = grounded_y(prefab_catalog, bench_modern, scale=1.0)
        bench_span = snap(plaza_extent * 0.73)
        bench_z = plaza_extent - spacing * 0.7
        k = 0
        pos = -bench_span
        while pos <= bench_span + 1e-3:
            deco.append(
                make_instance(local_id=f"bench_m_{k:03}", prefab_id_uuid=bench_modern, x=float(pos), y=by, z=-bench_z, yaw_deg=0.0, scale=1.0)
            )
            deco.append(
                make_instance(local_id=f"bench_m2_{k:03}", prefab_id_uuid=bench_modern, x=float(pos), y=by, z=bench_z, yaw_deg=180.0, scale=1.0)
            )
            pos += spacing
            k += 1

    if bench_old:
        by = grounded_y(prefab_catalog, bench_old, scale=1.0)
        old_base = snap(max(plaza_extent + spacing * 2.0, extent * 0.58))
        old_end = snap(min(extent - spacing * 2.0, old_base + extent * 0.32))
        step_m = max(11.0, spacing * 1.3)
        old_bench_z = old_base + spacing * 1.6
        k = 0
        pos = old_base + spacing * 0.8
        while pos <= old_end + 1e-3:
            deco.append(
                make_instance(local_id=f"bench_o_{k:03}", prefab_id_uuid=bench_old, x=float(pos), y=by, z=old_bench_z, yaw_deg=90.0, scale=1.0)
            )
            pos += step_m
            k += 1

    if billboard:
        hy = grounded_y(prefab_catalog, billboard, scale=1.0)
        billboard_z = plaza_extent * 0.5 + spacing * 0.3
        step_m = spacing * 3.0
        span = max(step_m, extent - spacing * 2.0)
        k = 0
        pos = -span
        while pos <= span + 1e-3:
            deco.append(
                make_instance(
                    local_id=f"billboard_{k:03}",
                    prefab_id_uuid=billboard,
                    x=float(pos),
                    y=hy,
                    z=billboard_z,
                    yaw_deg=180.0,
                    scale=1.0,
                    tint_rgba=palette_pick(k + 2),
                )
            )
            pos += step_m
            k += 1

    if holo_pillar:
        hy = grounded_y(prefab_catalog, holo_pillar, scale=1.0)
        holo_r = plaza_extent + 3.0
        for k, (x, z, yaw) in enumerate(
            [
                (0.0, -holo_r, 0.0),
                (0.0, holo_r, 180.0),
                (-holo_r, 0.0, 90.0),
                (holo_r, 0.0, -90.0),
            ]
        ):
            deco.append(
                make_instance(
                    local_id=f"holo_{k:02}",
                    prefab_id_uuid=holo_pillar,
                    x=x,
                    y=hy,
                    z=z,
                    yaw_deg=yaw,
                    scale=1.0,
                    tint_rgba=palette_pick(k),
                )
            )

    if planter_tree:
        py = grounded_y(prefab_catalog, planter_tree, scale=1.0)
        tree_r = plaza_extent + 3.0
        for k in range(0, 18):
            ang = (k / 18.0) * 2.0 * 3.141592653589793
            x = tree_r * math_cos(ang)
            z = tree_r * math_sin(ang)
            deco.append(make_instance(local_id=f"tree_{k:03}", prefab_id_uuid=planter_tree, x=x, y=py, z=z, yaw_deg=(k * 20.0), scale=1.0))

    if planter_flowers:
        py = grounded_y(prefab_catalog, planter_flowers, scale=1.0)
        flowers_r = max(spacing * 2.0, plaza_extent * 0.8)
        for k in range(0, 24):
            ang = (k / 24.0) * 2.0 * 3.141592653589793
            x = flowers_r * math_cos(ang)
            z = flowers_r * math_sin(ang)
            deco.append(make_instance(local_id=f"flowers_{k:03}", prefab_id_uuid=planter_flowers, x=x, y=py, z=z, yaw_deg=(k * 15.0), scale=1.0))

    if kiosk:
        ky = grounded_y(prefab_catalog, kiosk, scale=1.0)
        edge = plaza_extent - 1.0
        inset = plaza_extent * 0.4
        for k, (x, z, yaw) in enumerate(
            [
                (-inset, -edge, 0.0),
                (inset, -edge, 0.0),
                (-edge, -inset, 90.0),
                (-edge, inset, 90.0),
                (edge, -inset, -90.0),
                (edge, inset, -90.0),
            ]
        ):
            deco.append(make_instance(local_id=f"kiosk_{k:02}", prefab_id_uuid=kiosk, x=x, y=ky, z=z, yaw_deg=yaw, scale=1.0))

    if vendor:
        vy = grounded_y(prefab_catalog, vendor, scale=1.0)
        vendor_span = snap(plaza_extent * 0.73)
        vendor_z = plaza_extent + spacing * 0.7
        k = 0
        x = -vendor_span
        while x <= vendor_span + 1e-3:
            deco.append(
                make_instance(
                    local_id=f"vendor_n_{k:02}",
                    prefab_id_uuid=vendor,
                    x=float(x),
                    y=vy,
                    z=-vendor_z,
                    yaw_deg=0.0,
                    scale=1.0,
                    tint_rgba=palette_pick(k),
                )
            )
            deco.append(
                make_instance(
                    local_id=f"vendor_s_{k:02}",
                    prefab_id_uuid=vendor,
                    x=float(x),
                    y=vy,
                    z=vendor_z,
                    yaw_deg=180.0,
                    scale=1.0,
                    tint_rgba=palette_pick(k + 1),
                )
            )
            x += spacing
            k += 1

    if trash_bin:
        ty = grounded_y(prefab_catalog, trash_bin, scale=1.0)
        tx = plaza_extent * 0.55
        tz = plaza_extent * 0.8
        old_base = snap(max(plaza_extent + spacing * 2.0, extent * 0.58))
        old_z = old_base + spacing * 1.2
        for k, (x, z) in enumerate(
            [
                (-tx, -tz),
                (tx, -tz),
                (-tx, tz),
                (tx, tz),
                (old_base + spacing * 1.2, old_z),
                (old_base + spacing * 2.6, old_z),
            ]
        ):
            deco.append(make_instance(local_id=f"trash_{k:02}", prefab_id_uuid=trash_bin, x=x, y=ty, z=z, yaw_deg=0.0, scale=1.0))

    if bollard:
        by = grounded_y(prefab_catalog, bollard, scale=1.0)
        bollard_r = plaza_extent + 11.0
        for k in range(0, 32):
            ang = (k / 32.0) * 2.0 * 3.141592653589793
            x = bollard_r * math_cos(ang)
            z = bollard_r * math_sin(ang)
            deco.append(make_instance(local_id=f"bollard_{k:03}", prefab_id_uuid=bollard, x=x, y=by, z=z, yaw_deg=(k * 11.25), scale=1.0))

    # --- Modern buildings: ring around plaza ---
    if modern_buildings:
        ring_base = max(plaza_extent + spacing * 1.5, min(extent - spacing * 2.0, extent * 0.66))
        ring2 = min(extent - spacing * 1.2, ring_base + spacing)
        ring3 = min(extent - spacing * 0.8, ring_base + spacing * 2.0)
        radii = [(ring_base, -0.045), (ring2, 0.0), (ring3, 0.045)]
        n = float(len(modern_buildings))
        for i, b in enumerate(modern_buildings):
            base = (i / n) * 2.0 * 3.141592653589793
            for j, (r, d_ang) in enumerate(radii):
                ang = base + d_ang
                x = r * math_cos(ang)
                z = r * math_sin(ang)
                yaw = (ang * 180.0 / 3.141592653589793) + 180.0
                scale = 1.25 if i % 3 == 0 else (1.05 if i % 3 == 1 else 0.95)
                y = grounded_y(prefab_catalog, b, scale=scale)
                tint = palette_pick_muted((i * 7) + (j * 3), mix=0.35) if (i + j) % 3 == 0 else None
                buildings_modern.append(
                    make_instance(
                        local_id=f"modern_{i:02}_{j:02}",
                        prefab_id_uuid=b,
                        x=x,
                        y=y,
                        z=z,
                        yaw_deg=yaw,
                        scale=scale,
                        tint_rgba=tint,
                    )
                )

    # Skybridges: a couple of floating connectors.
    if skybridge:
        sb_scale = 1.2
        sb_y = grounded_y(prefab_catalog, skybridge, scale=sb_scale) + 14.0
        sb_pos = max(plaza_extent + spacing * 2.0, min(extent - spacing * 2.0, extent * 0.68))
        for idx, (x, z, yaw) in enumerate(
            [
                (0.0, -sb_pos, 0.0),
                (-sb_pos, 0.0, 90.0),
                (0.0, sb_pos, 0.0),
                (sb_pos, 0.0, 90.0),
            ]
        ):
            buildings_modern.append(
                make_instance(
                    local_id=f"skybridge_{idx:02}",
                    prefab_id_uuid=skybridge,
                    x=x,
                    y=sb_y,
                    z=z,
                    yaw_deg=yaw,
                    scale=sb_scale,
                    tint_rgba=palette_pick_muted(idx + 2, mix=0.45),
                )
            )

    # --- Old district buildings: SE quadrant ---
    if old_buildings:
        rng = random.Random(777)
        old_base = snap(max(plaza_extent + spacing * 2.0, extent * 0.58))
        grid_step = spacing * 1.8
        for i, b in enumerate(old_buildings):
            for j in range(0, 6):
                cx = old_base + (j % 3) * grid_step + rng.uniform(-1.5, 1.5)
                cz = old_base + (j // 3) * grid_step + rng.uniform(-1.5, 1.5)
                yaw = 90.0 if (j % 2 == 0) else 0.0
                scale = 1.0 + rng.uniform(-0.08, 0.08)
                y = grounded_y(prefab_catalog, b, scale=scale)
                buildings_old.append(
                    make_instance(
                        local_id=f"old_{i:02}_{j:02}",
                        prefab_id_uuid=b,
                        x=cx + i * (spacing * 0.2),
                        y=y,
                        z=cz + i * (spacing * 0.1),
                        yaw_deg=yaw,
                        scale=scale,
                    )
                )

    # --- Spaceport corner (NW quadrant) ---
    sp_base = snap(-extent * 0.8)
    if dome_terminal:
        scale = 1.15
        y = grounded_y(prefab_catalog, dome_terminal, scale=scale)
        district_spaceport.append(
            make_instance(
                local_id="terminal_dome",
                prefab_id_uuid=dome_terminal,
                x=sp_base,
                y=y,
                z=sp_base + spacing * 1.8,
                yaw_deg=30.0,
                scale=scale,
            )
        )
    if hangar:
        scale = 1.2
        y = grounded_y(prefab_catalog, hangar, scale=scale)
        district_spaceport.append(
            make_instance(
                local_id="hangar_main",
                prefab_id_uuid=hangar,
                x=sp_base - spacing * 1.2,
                y=y,
                z=sp_base - spacing * 1.2,
                yaw_deg=45.0,
                scale=scale,
            )
        )
    if ship:
        scale = 1.05
        y = grounded_y(prefab_catalog, ship, scale=scale) + 0.2
        district_spaceport.append(
            make_instance(
                local_id="ship_lander",
                prefab_id_uuid=ship,
                x=sp_base + spacing * 1.4,
                y=y,
                z=sp_base - spacing * 0.6,
                yaw_deg=-10.0,
                scale=scale,
            )
        )

    # --- Vehicles ---
    if ground_vehicles:
        rng = random.Random(123)
        lane = spacing * 0.26
        x_start = -extent * 0.93
        x_end = extent * 0.93
        x_step = (x_end - x_start) / max(1, (44 - 1))
        old_base = snap(max(plaza_extent + spacing * 2.0, extent * 0.58))
        for idx in range(0, 44):
            pid_choice = ground_vehicles[idx % len(ground_vehicles)]
            lane_z = -lane if (idx % 2 == 0) else lane
            x = x_start + idx * x_step
            z = lane_z
            if idx % 11 == 0:
                # A few vehicles in the old district streets.
                x = old_base + rng.uniform(0.0, extent * 0.32)
                z = old_base + rng.uniform(0.0, extent * 0.32)
            y = grounded_y(prefab_catalog, pid_choice, scale=0.95)
            vehicles_ground.append(
                make_instance(
                    local_id=f"veh_g_{idx:03}",
                    prefab_id_uuid=pid_choice,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=0.0,
                    scale=0.95,
                    tint_rgba=palette_pick(idx + 1) if (idx % 3 == 0) else None,
                )
            )

    if air_vehicles:
        rng = random.Random(456)
        z_start = -extent * 0.85
        z_end = extent * 0.85
        z_step = (z_end - z_start) / max(1, (22 - 1))
        air_x = -(plaza_extent * 0.33)
        for idx in range(0, 22):
            pid_choice = air_vehicles[idx % len(air_vehicles)]
            z = z_start + idx * z_step
            x = air_x + rng.uniform(-2.0, 2.0)
            base = grounded_y(prefab_catalog, pid_choice, scale=1.0)
            vehicles_air.append(
                make_instance(
                    local_id=f"veh_a_{idx:03}",
                    prefab_id_uuid=pid_choice,
                    x=x,
                    y=base + 16.0,
                    z=z,
                    yaw_deg=90.0,
                    scale=1.0,
                    tint_rgba=palette_pick_muted(idx + 3, mix=0.55),
                )
            )

    # --- Population ---
    if units:
        rng = random.Random(42)
        pop_range = min(plaza_extent * 0.82, extent * 0.35)
        for idx in range(0, 160):
            pid_choice = units[idx % len(units)]
            px = rng.uniform(-pop_range, pop_range)
            pz = rng.uniform(-pop_range, pop_range)
            scale = 1.0 + rng.uniform(-0.07, 0.07)
            uy = grounded_y(prefab_catalog, pid_choice, scale=scale)
            tint = None
            if idx % 11 == 0:
                tint = {"r": 0.35, "g": 0.78, "b": 1.0, "a": 1.0}
            elif idx % 17 == 0:
                tint = {"r": 0.95, "g": 0.85, "b": 0.35, "a": 1.0}
            population_walk.append(
                make_instance(
                    local_id=f"pop_{idx:03}",
                    prefab_id_uuid=pid_choice,
                    x=px,
                    y=uy,
                    z=pz,
                    yaw_deg=rng.uniform(0.0, 360.0),
                    scale=scale,
                    tint_rgba=tint,
                )
            )

    if drones:
        rng = random.Random(9001)
        drone_range = plaza_extent * 1.0
        for idx in range(0, 36):
            pid_choice = drones[idx % len(drones)]
            px = rng.uniform(-drone_range, drone_range)
            pz = rng.uniform(-drone_range, drone_range)
            scale = 1.0
            uy = grounded_y(prefab_catalog, pid_choice, scale=scale) + rng.uniform(6.0, 14.0)
            population_fly.append(
                make_instance(
                    local_id=f"drone_{idx:03}",
                    prefab_id_uuid=pid_choice,
                    x=px,
                    y=uy,
                    z=pz,
                    yaw_deg=rng.uniform(0.0, 360.0),
                    scale=scale,
                )
            )

    layers["infra_roads"] = layer_doc_explicit("infra_roads", infra_roads)
    layers["infra_plaza"] = layer_doc_explicit("infra_plaza", infra_plaza)
    layers["decor"] = layer_doc_explicit("decor", deco)
    layers["buildings_modern"] = layer_doc_explicit("buildings_modern", buildings_modern)
    layers["buildings_old"] = layer_doc_explicit("buildings_old", buildings_old)
    layers["district_spaceport"] = layer_doc_explicit("district_spaceport", district_spaceport)
    layers["vehicles_ground"] = layer_doc_explicit("vehicles_ground", vehicles_ground)
    layers["vehicles_air"] = layer_doc_explicit("vehicles_air", vehicles_air)
    layers["population_walk"] = layer_doc_explicit("population_walk", population_walk)
    layers["population_fly"] = layer_doc_explicit("population_fly", population_fly)
    return layers


def build_layout_layers_wasteland(
    *,
    prefab_catalog: dict[str, dict[str, Any]],
    assets: dict[str, str],
    layout_extent_m: float,
    plaza_extent_m: float,
) -> dict[str, dict[str, Any]]:
    def pid(key: str) -> str | None:
        value = assets.get(key)
        prefab_id = str(value).strip() if value else ""
        if not prefab_id:
            return None
        if prefab_id not in prefab_catalog:
            return None
        return prefab_id

    road = pid("road_cracked_tile")
    shoulder = pid("shoulder_scrap_tile") or road
    crosswalk = pid("crosswalk_faded_tile")

    streetlamp = pid("streetlamp_patchwork")
    string_lights = pid("string_lights")
    bench = pid("bench_scrap")
    sign_neon = pid("shop_sign_neon")
    sign_totem = pid("shop_totem")
    market_stall = pid("market_stall")
    food_cart = pid("food_cart")
    awning = pid("awning_patchwork")
    barrel_cluster = pid("barrel_cluster")
    crate_stack = pid("crate_stack")
    scrap_heap = pid("scrap_heap")
    solar_canopy = pid("solar_canopy")
    water_tank_small = pid("water_tank_small")
    antenna_pole = pid("antenna_pole")
    generator_box = pid("generator_box")
    planter_drum = pid("planter_drum")
    junk_fence = pid("junk_fence")
    sat_dish = pid("sat_dish")

    garage_bay = pid("garage_bay")
    repair_shed = pid("repair_shed")
    container_shop = pid("container_shop")
    stacked_home = pid("stacked_home")
    market_hall = pid("market_hall")
    clinic_shop = pid("clinic_shop")
    diner_bar = pid("diner_bar")
    recycler_tower = pid("recycler_tower")
    water_tower = pid("water_tower")
    drone_pad = pid("drone_pad")
    watch_post = pid("watch_post")
    motel_block = pid("motel_block")

    ground_vehicle_keys = [
        "vehicle_buggy",
        "vehicle_cargo_trike",
        "vehicle_salvage_truck",
        "vehicle_hover_sled",
        "vehicle_market_van",
        "vehicle_scout_bike",
    ]
    air_vehicle_keys = [
        "vehicle_sky_skiff",
        "vehicle_courier_drone",
        "vehicle_patrol_drone",
        "vehicle_cargo_quad",
    ]
    citizen_keys = [
        "unit_robot_mechanic",
        "unit_robot_vendor",
        "unit_robot_sweeper",
        "unit_robot_guard",
        "unit_human_scavenger",
        "unit_human_shopkeeper",
        "unit_human_child",
        "unit_android_clerk",
        "unit_alien_trader",
        "unit_alien_farmer",
    ]
    animal_keys = [
        "unit_wasteland_dog",
        "unit_pack_lizard",
        "unit_desert_bird",
        "unit_moss_goat",
    ]
    drone_keys = ["unit_drone_helper", "unit_drone_observer"]

    ground_vehicles: list[str] = [value for key in ground_vehicle_keys if (value := pid(key))]
    air_vehicles: list[str] = [value for key in air_vehicle_keys if (value := pid(key))]
    citizens: list[str] = [value for key in citizen_keys if (value := pid(key))]
    animals: list[str] = [value for key in animal_keys if (value := pid(key))]
    drone_units: list[str] = [value for key in drone_keys if (value := pid(key))]

    layers: dict[str, dict[str, Any]] = {}

    infra_roads: list[dict[str, Any]] = []
    infra_shoulders: list[dict[str, Any]] = []
    district_market: list[dict[str, Any]] = []
    district_garage: list[dict[str, Any]] = []
    district_housing: list[dict[str, Any]] = []
    district_utility: list[dict[str, Any]] = []
    props: list[dict[str, Any]] = []
    vehicles_ground: list[dict[str, Any]] = []
    vehicles_air: list[dict[str, Any]] = []
    population_walk: list[dict[str, Any]] = []
    population_fly: list[dict[str, Any]] = []
    animal_life: list[dict[str, Any]] = []

    spacing = 8.0
    extent = max(spacing * 2.0, float(layout_extent_m))
    hub_extent = max(spacing, min(float(plaza_extent_m), extent - (spacing * 0.5)))

    def snap(v: float) -> float:
        return round(v / spacing) * spacing

    road_step = spacing * 0.7
    road_width_m = spacing * 0.5
    road_sink_m = 0.08
    if road:
        info = prefab_catalog.get(road) or {}
        size = info.get("size") or []
        if isinstance(size, list) and len(size) == 3:
            road_step = _clamp(max(float(size[0]), float(size[2])), 2.5, 7.6)
            road_width_m = _clamp(min(float(size[0]), float(size[2])), 2.0, 5.0)
            road_sink_m = _clamp(float(size[1]) - 0.08, 0.06, 0.14)

    shoulder_step = road_step
    shoulder_width_m = road_width_m * 0.9
    shoulder_sink_m = 0.2
    if shoulder:
        info = prefab_catalog.get(shoulder) or {}
        size = info.get("size") or []
        if isinstance(size, list) and len(size) == 3:
            shoulder_step = _clamp(max(float(size[0]), float(size[2])), 2.5, 7.5)
            shoulder_width_m = _clamp(min(float(size[0]), float(size[2])), 2.4, 4.2)
            shoulder_sink_m = _clamp(float(size[1]) - 0.1, 0.16, 0.32)

    crosswalk_sink_m = min(road_sink_m, 0.06)
    road_lines = [0.0]

    palette: list[dict[str, float]] = [
        {"r": 0.91, "g": 0.44, "b": 0.21, "a": 1.0},
        {"r": 0.16, "g": 0.83, "b": 0.79, "a": 1.0},
        {"r": 0.97, "g": 0.79, "b": 0.20, "a": 1.0},
        {"r": 0.89, "g": 0.30, "b": 0.61, "a": 1.0},
        {"r": 0.62, "g": 0.90, "b": 0.31, "a": 1.0},
        {"r": 0.35, "g": 0.63, "b": 0.98, "a": 1.0},
    ]

    def palette_pick(seed: int) -> dict[str, float]:
        return dict(palette[seed % len(palette)])

    def palette_pick_muted(seed: int, *, mix: float = 0.4) -> dict[str, float]:
        base = palette_pick(seed)
        return {
            "r": base["r"] * (1.0 - mix) + 0.42 * mix,
            "g": base["g"] * (1.0 - mix) + 0.37 * mix,
            "b": base["b"] * (1.0 - mix) + 0.32 * mix,
            "a": 1.0,
        }

    street_edge = max((road_width_m * 1.35), 4.75)
    market_line_z = snap(max(street_edge + spacing * 1.0, hub_extent + spacing * 0.25))
    housing_line_z = -snap(max(street_edge + spacing * 1.0, hub_extent + spacing * 0.25))
    garage_line_z = -snap(max(street_edge + spacing * 1.0, hub_extent + spacing * 0.3))
    utility_line_x = snap(max(street_edge + spacing * 1.0, hub_extent + spacing * 0.55))

    if road:
        for z_line in road_lines:
            x = -extent
            idx = 0
            while x <= extent + 1e-3:
                y = grounded_y(prefab_catalog, road, scale=1.0) - road_sink_m
                infra_roads.append(
                    make_instance(
                        local_id=f"road_x_{idx:03}_{int((z_line + 20.0) * 100):04}",
                        prefab_id_uuid=road,
                        x=x,
                        y=y,
                        z=z_line,
                        yaw_deg=0.0,
                        scale=1.0,
                    )
                )
                x += road_step
                idx += 1

        for x_line in road_lines:
            z = -extent
            idx = 0
            while z <= extent + 1e-3:
                if abs(z) <= (road_step * 1.02):
                    z += road_step
                    idx += 1
                    continue
                y = grounded_y(prefab_catalog, road, scale=1.0) - road_sink_m
                infra_roads.append(
                    make_instance(
                        local_id=f"road_z_{idx:03}_{int((x_line + 20.0) * 100):04}",
                        prefab_id_uuid=road,
                        x=x_line,
                        y=y,
                        z=z,
                        yaw_deg=90.0,
                        scale=1.0,
                    )
                )
                z += road_step
                idx += 1

    if shoulder:
        shoulder_pad_positions = [
            (snap(-extent * 0.72), market_line_z - spacing * 0.58, 0.0, 0.84),
            (snap(-extent * 0.22), market_line_z - spacing * 0.58, 0.0, 0.84),
            (snap(extent * 0.34), market_line_z - spacing * 0.58, 0.0, 0.84),
            (snap(-extent * 0.72), garage_line_z + spacing * 0.58, 0.0, 0.84),
            (snap(-extent * 0.32), garage_line_z + spacing * 0.58, 0.0, 0.84),
            (snap(extent * 0.26), housing_line_z + spacing * 0.58, 0.0, 0.84),
            (snap(extent * 0.7), housing_line_z + spacing * 0.58, 0.0, 0.84),
            (utility_line_x - shoulder_width_m * 0.58, snap(extent * 0.72), 90.0, 0.84),
        ]
        for idx, (x, z, yaw_deg, scale) in enumerate(shoulder_pad_positions):
            y = grounded_y(prefab_catalog, shoulder, scale=scale) - shoulder_sink_m
            infra_shoulders.append(
                make_instance(
                    local_id=f"shoulder_pad_{idx:02}",
                    prefab_id_uuid=shoulder,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 2, mix=0.74) if idx in (1, 3, 6, 8) else None,
                )
            )

    if crosswalk:
        cross_line = max(road_step * 1.6, hub_extent * 0.65)
        cross_span = max(road_step * 0.85, 3.0)
        cross_positions = [
            (0.0, cross_line, 0.0),
            (0.0, -cross_line, 0.0),
            (-cross_line, 0.0, 90.0),
            (cross_line, 0.0, 90.0),
            (-cross_span, cross_line, 0.0),
            (cross_span, cross_line, 0.0),
            (-cross_span, -cross_line, 0.0),
            (cross_span, -cross_line, 0.0),
        ]
        for idx, (x, z, yaw_deg) in enumerate(cross_positions):
            y = grounded_y(prefab_catalog, crosswalk, scale=1.0) - crosswalk_sink_m
            props.append(
                make_instance(
                    local_id=f"crosswalk_{idx:02}",
                    prefab_id_uuid=crosswalk,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=1.0,
                )
            )

    if streetlamp:
        lamp_positions: list[tuple[float, float, float]] = []
        axis_points = [
            snap(-extent * 0.78),
            0.0,
            snap(extent * 0.78),
        ]
        for x in axis_points:
            lamp_positions.append((x, street_edge, 180.0))
            lamp_positions.append((x, -street_edge, 0.0))
        side_points = [snap(-extent * 0.56), snap(extent * 0.56)]
        for z in side_points:
            if abs(z) <= (road_step * 1.05):
                continue
            lamp_positions.append((street_edge, z, -90.0))
            lamp_positions.append((-street_edge, z, 90.0))
        for idx, (x, z, yaw_deg) in enumerate(lamp_positions):
            y = grounded_y(prefab_catalog, streetlamp, scale=1.0)
            props.append(
                make_instance(
                    local_id=f"streetlamp_{idx:02}",
                    prefab_id_uuid=streetlamp,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=1.0,
                    tint_rgba=palette_pick_muted(idx + 1, mix=0.52) if idx % 3 == 0 else None,
                )
            )

    if string_lights:
        rig_xs = [snap(-extent * 0.34), snap(extent * 0.02), snap(extent * 0.38)]
        for idx, x in enumerate(rig_xs):
            scale = 1.0 + (0.08 * idx)
            y = grounded_y(prefab_catalog, string_lights, scale=scale) + 7.5
            props.append(
                make_instance(
                    local_id=f"string_lights_{idx:02}",
                    prefab_id_uuid=string_lights,
                    x=x,
                    y=y,
                    z=road_step * 0.3,
                    yaw_deg=90.0,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 2),
                )
            )

    market_building_prefabs = [
        prefab_id
        for prefab_id in [market_hall, container_shop, diner_bar, container_shop, stacked_home, motel_block]
        if prefab_id
    ]
    market_xs = [
        snap(-extent * 0.68),
        snap(-extent * 0.3),
        0.0,
        snap(extent * 0.34),
    ]
    for idx, x in enumerate(market_xs):
        if not market_building_prefabs:
            break
        prefab_id = market_building_prefabs[idx % len(market_building_prefabs)]
        scale = 0.96 + ((idx % 3) * 0.08)
        y = grounded_y(prefab_catalog, prefab_id, scale=scale)
        district_market.append(
            make_instance(
                local_id=f"market_building_{idx:02}",
                prefab_id_uuid=prefab_id,
                x=x,
                y=y,
                z=market_line_z,
                yaw_deg=180.0,
                scale=scale,
                tint_rgba=palette_pick_muted(idx, mix=0.55) if idx % 2 == 0 else None,
            )
        )

    if market_stall:
        stall_xs = [
            snap(-extent * 0.78),
            snap(-extent * 0.46),
            snap(-extent * 0.12),
            snap(extent * 0.2),
            snap(extent * 0.52),
        ]
        for idx, x in enumerate(stall_xs):
            scale = 0.9 + ((idx % 2) * 0.06)
            y = grounded_y(prefab_catalog, market_stall, scale=scale)
            district_market.append(
                make_instance(
                    local_id=f"market_stall_{idx:02}",
                    prefab_id_uuid=market_stall,
                    x=x,
                    y=y,
                    z=market_line_z - spacing * 0.72,
                    yaw_deg=180.0,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 1),
                )
            )

    if food_cart:
        cart_positions = [
            (snap(-extent * 0.52), market_line_z - spacing * 1.42, 180.0),
            (snap(-extent * 0.12), market_line_z - spacing * 1.42, 180.0),
            (snap(extent * 0.22), market_line_z - spacing * 1.42, 180.0),
            (snap(extent * 0.48), market_line_z - spacing * 1.42, 180.0),
            (-road_step * 0.76, street_edge + road_step * 0.16, 150.0),
            (road_step * 0.84, -street_edge - road_step * 0.16, -28.0),
        ]
        for idx, (x, z, yaw_deg) in enumerate(cart_positions):
            scale = 0.88
            y = grounded_y(prefab_catalog, food_cart, scale=scale)
            district_market.append(
                make_instance(
                    local_id=f"food_cart_{idx:02}",
                    prefab_id_uuid=food_cart,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 3) if idx < 4 else palette_pick_muted(idx + 3, mix=0.18),
                )
            )

    if awning:
        awning_xs = [snap(-extent * 0.68), snap(-extent * 0.3), 0.0, snap(extent * 0.34)]
        for idx, x in enumerate(awning_xs):
            scale = 0.94
            y = grounded_y(prefab_catalog, awning, scale=scale)
            props.append(
                make_instance(
                    local_id=f"awning_{idx:02}",
                    prefab_id_uuid=awning,
                    x=x,
                    y=y,
                    z=market_line_z - spacing * 0.22,
                    yaw_deg=180.0,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 4),
                )
            )

    if sign_neon:
        sign_xs = [snap(-extent * 0.72), snap(-extent * 0.12), snap(extent * 0.28), snap(extent * 0.6)]
        for idx, x in enumerate(sign_xs):
            scale = 0.82
            y = grounded_y(prefab_catalog, sign_neon, scale=scale)
            props.append(
                make_instance(
                    local_id=f"sign_neon_{idx:02}",
                    prefab_id_uuid=sign_neon,
                    x=x,
                    y=y,
                    z=market_line_z - spacing * 0.32,
                    yaw_deg=180.0,
                    scale=scale,
                    tint_rgba=palette_pick(idx),
                )
            )

    if sign_totem:
        sign_positions = [
            (snap(-extent * 0.9), market_line_z - spacing * 0.55, 180.0),
            (snap(extent * 0.58), market_line_z - spacing * 0.55, 180.0),
            (snap(extent * 0.42), housing_line_z + spacing * 0.4, 0.0),
            (snap(-extent * 0.84), garage_line_z + spacing * 0.45, 35.0),
        ]
        for idx, (x, z, yaw_deg) in enumerate(sign_positions):
            scale = 0.95
            y = grounded_y(prefab_catalog, sign_totem, scale=scale)
            props.append(
                make_instance(
                    local_id=f"sign_totem_{idx:02}",
                    prefab_id_uuid=sign_totem,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 2),
                )
            )

    garage_prefabs = [prefab_id for prefab_id in [garage_bay, repair_shed, recycler_tower] if prefab_id]
    garage_positions = [
        (snap(-extent * 0.82), garage_line_z, 25.0, 1.08),
        (snap(-extent * 0.52), garage_line_z + spacing * 0.18, 8.0, 0.96),
        (snap(-extent * 0.2), garage_line_z - spacing * 0.12, -12.0, 1.02),
    ]
    for idx, (x, z, yaw_deg, scale) in enumerate(garage_positions):
        if not garage_prefabs:
            break
        prefab_id = garage_prefabs[idx % len(garage_prefabs)]
        y = grounded_y(prefab_catalog, prefab_id, scale=scale)
        district_garage.append(
            make_instance(
                local_id=f"garage_building_{idx:02}",
                prefab_id_uuid=prefab_id,
                x=x,
                y=y,
                z=z,
                yaw_deg=yaw_deg,
                scale=scale,
                tint_rgba=palette_pick_muted(idx + 1, mix=0.6) if idx == 1 else None,
            )
        )

    housing_prefabs = [prefab_id for prefab_id in [stacked_home, container_shop, diner_bar, motel_block, stacked_home] if prefab_id]
    housing_positions = [
        (snap(extent * 0.16), housing_line_z, 0.0, 0.98),
        (snap(extent * 0.46), housing_line_z + spacing * 0.1, 0.0, 1.08),
        (snap(extent * 0.76), housing_line_z - spacing * 0.08, 0.0, 1.02),
        (snap(extent * 0.52), housing_line_z - spacing * 0.88, -18.0, 0.92),
    ]
    for idx, (x, z, yaw_deg, scale) in enumerate(housing_positions):
        if not housing_prefabs:
            break
        prefab_id = housing_prefabs[idx % len(housing_prefabs)]
        y = grounded_y(prefab_catalog, prefab_id, scale=scale)
        district_housing.append(
            make_instance(
                local_id=f"housing_building_{idx:02}",
                prefab_id_uuid=prefab_id,
                x=x,
                y=y,
                z=z,
                yaw_deg=yaw_deg,
                scale=scale,
                tint_rgba=palette_pick_muted(idx + 4, mix=0.62) if idx % 2 == 0 else None,
            )
        )

    utility_prefabs = [prefab_id for prefab_id in [water_tower, clinic_shop, drone_pad, watch_post] if prefab_id]
    utility_positions = [
        (utility_line_x, snap(extent * 0.72), 15.0, 1.08),
        (utility_line_x + spacing * 0.62, snap(extent * 0.48), 90.0, 0.92),
        (utility_line_x + spacing * 1.12, snap(extent * 0.84), 0.0, 0.88),
        (utility_line_x + spacing * 1.42, snap(extent * 0.58), 32.0, 0.82),
    ]
    for idx, (x, z, yaw_deg, scale) in enumerate(utility_positions):
        if not utility_prefabs:
            break
        prefab_id = utility_prefabs[idx % len(utility_prefabs)]
        y = grounded_y(prefab_catalog, prefab_id, scale=scale)
        district_utility.append(
            make_instance(
                local_id=f"utility_building_{idx:02}",
                prefab_id_uuid=prefab_id,
                x=x,
                y=y,
                z=z,
                yaw_deg=yaw_deg,
                scale=scale,
                tint_rgba=palette_pick_muted(idx + 2, mix=0.58) if idx == 1 else None,
            )
        )

    if water_tank_small:
        tank_positions = [
            (utility_line_x + spacing * 1.15, snap(extent * 0.36)),
            (utility_line_x + spacing * 1.62, snap(extent * 0.92)),
        ]
        for idx, (x, z) in enumerate(tank_positions):
            scale = 0.9
            y = grounded_y(prefab_catalog, water_tank_small, scale=scale)
            district_utility.append(
                make_instance(
                    local_id=f"water_tank_{idx:02}",
                    prefab_id_uuid=water_tank_small,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=0.0,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 5, mix=0.65) if idx == 1 else None,
                )
            )

    if solar_canopy:
        y = grounded_y(prefab_catalog, solar_canopy, scale=0.94)
        district_utility.append(
            make_instance(
                local_id="solar_canopy_main",
                prefab_id_uuid=solar_canopy,
                x=utility_line_x + spacing * 1.55,
                y=y,
                z=snap(extent * 0.34),
                yaw_deg=90.0,
                scale=0.94,
                tint_rgba=palette_pick_muted(7, mix=0.38),
            )
        )

    if junk_fence:
        fence_positions = [
            (utility_line_x + spacing * 0.3, snap(extent * 0.92), 90.0),
            (utility_line_x + spacing * 0.3, snap(extent * 0.62), 90.0),
            (utility_line_x + spacing * 0.3, snap(extent * 0.32), 90.0),
            (utility_line_x + spacing * 1.95, snap(extent * 0.92), 90.0),
            (utility_line_x + spacing * 1.95, snap(extent * 0.62), 90.0),
            (utility_line_x + spacing * 1.95, snap(extent * 0.32), 90.0),
            (utility_line_x + spacing * 1.12, snap(extent * 1.04), 0.0),
            (utility_line_x + spacing * 1.12, snap(extent * 0.18), 0.0),
        ]
        for idx, (x, z, yaw_deg) in enumerate(fence_positions):
            scale = 0.92
            y = grounded_y(prefab_catalog, junk_fence, scale=scale)
            district_utility.append(
                make_instance(
                    local_id=f"junk_fence_{idx:02}",
                    prefab_id_uuid=junk_fence,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                )
            )

    if sat_dish:
        for idx, (x, z) in enumerate(
            [
                (utility_line_x + spacing * 1.58, snap(extent * 0.68)),
                (snap(extent * 0.78), housing_line_z + spacing * 0.78),
            ]
        ):
            scale = 0.82
            y = grounded_y(prefab_catalog, sat_dish, scale=scale)
            props.append(
                make_instance(
                    local_id=f"sat_dish_{idx:02}",
                    prefab_id_uuid=sat_dish,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=-25.0 if idx == 0 else 30.0,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 2, mix=0.7) if idx == 1 else None,
                )
            )

    if antenna_pole:
        for idx, (x, z) in enumerate(
            [
                (utility_line_x + spacing * 1.76, snap(extent * 0.52)),
                (snap(-extent * 0.12), garage_line_z + spacing * 0.72),
            ]
        ):
            scale = 0.94
            y = grounded_y(prefab_catalog, antenna_pole, scale=scale)
            props.append(
                make_instance(
                    local_id=f"antenna_{idx:02}",
                    prefab_id_uuid=antenna_pole,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=0.0,
                    scale=scale,
                )
            )

    if generator_box:
        generator_positions = [
            (snap(-extent * 0.24), garage_line_z + spacing * 0.98),
            (utility_line_x + spacing * 1.28, snap(extent * 0.22)),
            (snap(extent * 0.68), housing_line_z + spacing * 0.76),
        ]
        for idx, (x, z) in enumerate(generator_positions):
            scale = 0.86
            y = grounded_y(prefab_catalog, generator_box, scale=scale)
            props.append(
                make_instance(
                    local_id=f"generator_{idx:02}",
                    prefab_id_uuid=generator_box,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=90.0 if idx == 0 else 0.0,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 3, mix=0.68) if idx == 2 else None,
                )
            )

    if barrel_cluster:
        barrel_positions = [
            (snap(-extent * 0.58), garage_line_z + spacing * 0.82),
            (snap(-extent * 0.34), garage_line_z + spacing * 0.56),
            (snap(-extent * 0.14), garage_line_z + spacing * 1.02),
            (snap(extent * 0.12), market_line_z - spacing * 0.9),
            (snap(extent * 0.72), market_line_z - spacing * 0.82),
        ]
        for idx, (x, z) in enumerate(barrel_positions):
            scale = 0.82
            y = grounded_y(prefab_catalog, barrel_cluster, scale=scale)
            props.append(
                make_instance(
                    local_id=f"barrels_{idx:02}",
                    prefab_id_uuid=barrel_cluster,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=18.0 * idx,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 5) if idx in (3, 4) else None,
                )
            )

    if crate_stack:
        crate_positions = [
            (snap(-extent * 0.76), garage_line_z + spacing * 0.32),
            (snap(-extent * 0.44), garage_line_z + spacing * 1.18),
            (snap(-extent * 0.02), market_line_z - spacing * 1.02),
            (snap(extent * 0.46), market_line_z - spacing * 1.08),
            (snap(extent * 0.54), housing_line_z + spacing * 0.54),
        ]
        for idx, (x, z) in enumerate(crate_positions):
            scale = 0.88
            y = grounded_y(prefab_catalog, crate_stack, scale=scale)
            props.append(
                make_instance(
                    local_id=f"crates_{idx:02}",
                    prefab_id_uuid=crate_stack,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=-12.0 * idx,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 1, mix=0.7) if idx == 2 else None,
                )
            )

    if scrap_heap:
        scrap_positions = [
            (snap(-extent * 0.82), garage_line_z - spacing * 0.32),
            (snap(-extent * 0.26), garage_line_z - spacing * 0.24),
            (snap(extent * 0.82), housing_line_z - spacing * 0.42),
        ]
        for idx, (x, z) in enumerate(scrap_positions):
            scale = 1.0 + (0.1 * idx)
            y = grounded_y(prefab_catalog, scrap_heap, scale=scale)
            props.append(
                make_instance(
                    local_id=f"scrap_heap_{idx:02}",
                    prefab_id_uuid=scrap_heap,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=25.0 * idx,
                    scale=scale,
                )
            )

    if bench:
        bench_positions = [
            (snap(-extent * 0.44), market_line_z - spacing * 1.74, 180.0),
            (snap(extent * 0.24), market_line_z - spacing * 1.74, 180.0),
            (snap(extent * 0.54), housing_line_z + spacing * 1.14, 0.0),
            (utility_line_x + spacing * 1.04, snap(extent * 0.12), -90.0),
            (-street_edge - spacing * 0.26, -road_step * 0.24, 90.0),
            (street_edge + spacing * 0.26, road_step * 0.26, -90.0),
        ]
        for idx, (x, z, yaw_deg) in enumerate(bench_positions):
            scale = 0.88
            y = grounded_y(prefab_catalog, bench, scale=scale)
            props.append(
                make_instance(
                    local_id=f"bench_{idx:02}",
                    prefab_id_uuid=bench,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 4, mix=0.76) if idx == 2 else None,
                )
            )

    if planter_drum:
        planter_positions = [
            (snap(extent * 0.26), housing_line_z + spacing * 0.72),
            (snap(extent * 0.74), housing_line_z + spacing * 0.66),
            (snap(-extent * 0.12), market_line_z - spacing * 1.58),
            (snap(extent * 0.86), market_line_z - spacing * 0.54),
            (-road_step * 0.48, street_edge + road_step * 0.2),
            (road_step * 0.48, street_edge + road_step * 0.24),
            (-road_step * 0.52, -street_edge - road_step * 0.22),
            (road_step * 0.52, -street_edge - road_step * 0.18),
        ]
        for idx, (x, z) in enumerate(planter_positions):
            scale = 0.78
            y = grounded_y(prefab_catalog, planter_drum, scale=scale)
            props.append(
                make_instance(
                    local_id=f"planter_{idx:02}",
                    prefab_id_uuid=planter_drum,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=0.0,
                    scale=scale,
                    tint_rgba=palette_pick(idx + 2),
                )
            )

    if ground_vehicles:
        parking_positions = [
            (snap(-extent * 0.78), road_step * 2.8, 180.0, 0.92),
            (snap(-extent * 0.42), road_step * 2.75, 180.0, 0.9),
            (snap(extent * 0.14), road_step * 2.7, 180.0, 0.95),
            (snap(extent * 0.58), road_step * 2.78, 180.0, 0.88),
            (snap(-extent * 0.86), -street_edge + road_step * 0.2, 0.0, 0.96),
            (snap(-extent * 0.54), -street_edge + road_step * 0.12, 0.0, 0.92),
            (snap(extent * 0.22), -street_edge + road_step * 0.08, 0.0, 0.84),
            (snap(extent * 0.74), -street_edge + road_step * 0.14, 0.0, 0.9),
            (snap(-extent * 0.68), garage_line_z + spacing * 0.86, 18.0, 0.94),
            (snap(-extent * 0.3), garage_line_z + spacing * 0.88, -22.0, 0.88),
            (snap(extent * 0.48), housing_line_z + spacing * 1.18, 10.0, 0.9),
            (utility_line_x + spacing * 0.86, snap(extent * 0.16), 90.0, 0.82),
        ]
        for idx, (x, z, yaw_deg, scale) in enumerate(parking_positions):
            prefab_id = ground_vehicles[idx % len(ground_vehicles)]
            y = grounded_y(prefab_catalog, prefab_id, scale=scale)
            vehicles_ground.append(
                make_instance(
                    local_id=f"ground_vehicle_{idx:02}",
                    prefab_id_uuid=prefab_id,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick(idx) if idx % 2 == 0 else None,
                )
            )

    if air_vehicles:
        air_positions = [
            (0.0, 0.0, 45.0, 1.02, 10.0),
            (snap(-extent * 0.38), snap(extent * 0.08), 65.0, 0.86, 7.5),
            (snap(extent * 0.42), snap(-extent * 0.18), -20.0, 0.82, 7.2),
            (utility_line_x + spacing * 1.2, snap(extent * 0.64), 90.0, 0.88, 8.0),
            (snap(-extent * 0.56), garage_line_z + spacing * 1.28, 15.0, 0.9, 7.8),
            (snap(extent * 0.8), market_line_z - spacing * 0.6, 180.0, 0.82, 6.6),
            (snap(extent * 0.2), market_line_z - spacing * 0.2, 145.0, 0.8, 6.2),
            (snap(-extent * 0.08), housing_line_z + spacing * 1.26, -70.0, 0.78, 6.8),
        ]
        for idx, (x, z, yaw_deg, scale, altitude) in enumerate(air_positions):
            prefab_id = air_vehicles[idx % len(air_vehicles)]
            y = grounded_y(prefab_catalog, prefab_id, scale=scale) + altitude
            vehicles_air.append(
                make_instance(
                    local_id=f"air_vehicle_{idx:02}",
                    prefab_id_uuid=prefab_id,
                    x=x,
                    y=y,
                    z=z,
                    yaw_deg=yaw_deg,
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 2, mix=0.34),
                )
            )

    if citizens:
        rng = random.Random(20260405)
        citizen_spots: list[tuple[float, float, float]] = []
        for x in [snap(-extent * 0.72), snap(-extent * 0.46), snap(-extent * 0.2), snap(extent * 0.08), snap(extent * 0.36), snap(extent * 0.66)]:
            citizen_spots.append((x, market_line_z - spacing * 1.55, 180.0))
        for x in [snap(-extent * 0.18), snap(0.0), snap(extent * 0.22), snap(extent * 0.54)]:
            citizen_spots.append((x, street_edge - spacing * 0.08, 180.0))
            citizen_spots.append((x, -street_edge + spacing * 0.08, 0.0))
        for z in [snap(-extent * 0.68), snap(-extent * 0.32), snap(extent * 0.24), snap(extent * 0.58)]:
            citizen_spots.append((street_edge - spacing * 0.08, z, -90.0))
        citizen_spots.extend(
            [
                (snap(-extent * 0.58), garage_line_z + spacing * 0.62, 35.0),
                (snap(-extent * 0.22), garage_line_z + spacing * 0.86, -22.0),
                (snap(extent * 0.52), housing_line_z + spacing * 0.92, 10.0),
                (utility_line_x + spacing * 1.08, snap(extent * 0.22), 85.0),
                (utility_line_x + spacing * 1.48, snap(extent * 0.74), -20.0),
                (snap(-extent * 0.02), 0.0, 120.0),
            ]
        )
        for idx, (x, z, yaw_deg) in enumerate(citizen_spots):
            prefab_id = citizens[idx % len(citizens)]
            scale = 0.96 + rng.uniform(-0.08, 0.07)
            y = grounded_y(prefab_catalog, prefab_id, scale=scale)
            tint = None
            if idx % 9 == 0:
                tint = palette_pick(idx + 1)
            elif idx % 7 == 0:
                tint = palette_pick_muted(idx + 2, mix=0.26)
            population_walk.append(
                make_instance(
                    local_id=f"citizen_{idx:03}",
                    prefab_id_uuid=prefab_id,
                    x=x + rng.uniform(-1.0, 1.0),
                    y=y,
                    z=z + rng.uniform(-0.8, 0.8),
                    yaw_deg=yaw_deg + rng.uniform(-40.0, 40.0),
                    scale=scale,
                    tint_rgba=tint,
                )
            )

    if animals:
        rng = random.Random(5150)
        animal_spots = [
            (utility_line_x + spacing * 1.12, snap(extent * 0.84)),
            (utility_line_x + spacing * 1.42, snap(extent * 0.56)),
            (utility_line_x + spacing * 1.64, snap(extent * 0.3)),
            (snap(extent * 0.64), housing_line_z + spacing * 0.32),
            (snap(extent * 0.26), housing_line_z + spacing * 0.44),
            (snap(-extent * 0.24), market_line_z - spacing * 1.22),
            (snap(extent * 0.08), market_line_z - spacing * 1.08),
            (snap(-extent * 0.72), garage_line_z + spacing * 0.18),
        ]
        for idx, (x, z) in enumerate(animal_spots):
            prefab_id = animals[idx % len(animals)]
            scale = 0.9 + rng.uniform(-0.06, 0.14)
            y = grounded_y(prefab_catalog, prefab_id, scale=scale)
            animal_life.append(
                make_instance(
                    local_id=f"animal_{idx:02}",
                    prefab_id_uuid=prefab_id,
                    x=x + rng.uniform(-0.9, 0.9),
                    y=y,
                    z=z + rng.uniform(-0.9, 0.9),
                    yaw_deg=rng.uniform(0.0, 360.0),
                    scale=scale,
                    tint_rgba=palette_pick_muted(idx + 4, mix=0.5) if idx in (2, 5) else None,
                )
            )

    if drone_units:
        rng = random.Random(9901)
        drone_spots = [
            (snap(-extent * 0.48), street_edge, 8.0),
            (0.0, 0.0, 10.0),
            (snap(extent * 0.44), -street_edge, 7.5),
            (utility_line_x + spacing * 1.28, snap(extent * 0.66), 9.0),
            (snap(-extent * 0.52), garage_line_z + spacing * 0.78, 6.5),
            (snap(extent * 0.62), market_line_z - spacing * 0.52, 8.2),
            (snap(extent * 0.36), housing_line_z + spacing * 0.88, 6.8),
            (snap(-extent * 0.14), market_line_z - spacing * 1.4, 7.4),
        ]
        for idx, (x, z, altitude) in enumerate(drone_spots):
            prefab_id = drone_units[idx % len(drone_units)]
            scale = 0.92 + (0.04 * (idx % 2))
            y = grounded_y(prefab_catalog, prefab_id, scale=scale) + altitude
            population_fly.append(
                make_instance(
                    local_id=f"drone_unit_{idx:02}",
                    prefab_id_uuid=prefab_id,
                    x=x + rng.uniform(-0.8, 0.8),
                    y=y,
                    z=z + rng.uniform(-0.8, 0.8),
                    yaw_deg=rng.uniform(0.0, 360.0),
                    scale=scale,
                    tint_rgba=palette_pick(idx + 3),
                )
            )

    layers["infra_roads"] = layer_doc_explicit("infra_roads", infra_roads)
    layers["infra_shoulders"] = layer_doc_explicit("infra_shoulders", infra_shoulders)
    layers["district_market"] = layer_doc_explicit("district_market", district_market)
    layers["district_garage"] = layer_doc_explicit("district_garage", district_garage)
    layers["district_housing"] = layer_doc_explicit("district_housing", district_housing)
    layers["district_utility"] = layer_doc_explicit("district_utility", district_utility)
    layers["props"] = layer_doc_explicit("props", props)
    layers["vehicles_ground"] = layer_doc_explicit("vehicles_ground", vehicles_ground)
    layers["vehicles_air"] = layer_doc_explicit("vehicles_air", vehicles_air)
    layers["population_walk"] = layer_doc_explicit("population_walk", population_walk)
    layers["population_fly"] = layer_doc_explicit("population_fly", population_fly)
    layers["animal_life"] = layer_doc_explicit("animal_life", animal_life)
    return layers


def chrome_asset_plan() -> list[tuple[str, str]]:
    return [
        ("road_tile", "A modular futuristic chrome road tile for a utopian city boulevard. Clean white/silver materials, subtle lane markings, embedded blue neon strips. No text, no logos."),
        ("sidewalk_tile", "A modular futuristic sidewalk tile: chrome + white ceramic, subtle hex micro-pattern, clean utopian design. No text."),
        ("plaza_tile", "A modular utopian chrome plaza tile: clean white/silver, hex micro pattern, faint blue seams, suitable for a large city plaza. No text."),
        ("crosswalk_tile", "A modular futuristic crosswalk tile: clean white stripes embedded into chrome road surface, subtle blue edge lights. No text."),
        ("streetlight_neon", "A slim futuristic neon streetlight: chrome pole, soft blue light, utopian city style. No text."),
        ("streetlight_old", "An old-style street lamp: classic shape, warm glass, restored and clean, with subtle chrome futuristic additions. No text."),
        ("bench_modern", "A modern futuristic public bench: chrome frame, white composite seat, minimal utopian design. No text."),
        ("bench_old", "A vintage public bench: cast iron and wood, clean and maintained, with subtle futuristic chrome reinforcements. No text."),
        ("billboard_holo", "A futuristic holographic billboard sign: chrome frame, translucent hologram panel, utopian city. No text."),
        ("holo_sign_pillar", "A holographic information pillar: chrome base, floating UI glow, soft blue/teal light, utopian city. No text."),
        ("fountain_chrome", "A sculptural utopian chrome fountain: clean white water effect, subtle blue lighting, centerpiece for a plaza. No text."),
        ("statue_abstract", "An abstract public art statue: chrome and white composite, elegant curves, futuristic utopian sculpture. No text."),
        ("kiosk_info", "A futuristic public information kiosk: chrome shell, glowing blue screen, minimal design. No text."),
        ("vendor_stall", "A small futuristic vendor stall / market booth: chrome frame, clean canopy, subtle neon accents. No text."),
        ("planter_tree", "A large planter with a small clean futuristic tree: chrome planter bowl, healthy greenery, utopian city. No text."),
        ("planter_flowers", "A planter with colorful flowers: clean chrome base, well-maintained, utopian city decoration. No text."),
        ("trash_bin", "A futuristic public trash bin: chrome and white, minimal design, clean and maintained. No text."),
        ("bollard", "A street safety bollard: chrome cylinder with subtle blue glow ring, utopian city. No text."),
        ("tower_chrome_tall", "A tall utopian chrome skyscraper tower: sleek clean design, subtle blue light seams, large windows, futuristic. No text."),
        ("tower_chrome_mid", "A mid-rise utopian chrome building with terraces: clean white and silver materials, futuristic. No text."),
        ("tower_chrome_spire", "A very tall chrome spire tower: needle-like silhouette, glowing accents, futuristic spaceport city. No text."),
        ("tower_chrome_twist", "A twisting modern skyscraper: chrome and glass, elegant spiral form, utopian futuristic city. No text."),
        ("tower_glass_arc", "A glass-and-chrome arc-shaped high-rise: clean futuristic architecture, soft blue internal lighting. No text."),
        ("residential_pods", "A residential building with modular balcony pods: chrome and white panels, clean utopian design, futuristic. No text."),
        ("hotel_sleek", "A sleek futuristic hotel building: chrome facade, vertical light strips, clean utopian city style. No text."),
        ("lab_research", "A futuristic research lab building: chrome + white composite, antenna arrays, clean utopian design. No text."),
        ("mall_plaza", "A low-rise futuristic commercial plaza/mall: clean chrome structure, canopies, open frontage, utopian city. No text."),
        ("skybridge_module", "An elevated skybridge / pedestrian walkway module: chrome frame, transparent floor panels, soft blue lights. No text."),
        ("building_old_brick", "An old-style brick and stone building with art-deco ornaments and subtle futuristic chrome additions, clean and maintained. No text."),
        ("building_old_artdeco", "A restored art-deco building: stone and metal ornaments, clean, with subtle neon chrome signage frames. No text."),
        ("building_old_clocktower", "A historic clocktower building: stone base, clean and maintained, with futuristic chrome conduits and blue lights. No text."),
        ("building_old_market", "An old market hall building: brick and iron structure, clean, with futuristic lighting and chrome details. No text."),
        ("building_old_factory", "A converted old factory building: brick, tall windows, clean, with futuristic chrome additions. No text."),
        ("building_old_shrine", "A small old shrine/chapel building: stone and wood, respectful, clean, with subtle futuristic lights. No text."),
        ("building_old_townhouse", "A narrow townhouse building: old brick facade, clean and maintained, subtle chrome future retrofit. No text."),
        ("dome_terminal", "A dome-shaped transit terminal building: chrome and glass, clean utopian spaceport architecture. No text."),
        ("hangar_spaceport", "A spaceport hangar building: large doors, chrome structure, clean, subtle blue runway lights. No text."),
        ("ship_starship_lander", "A small starship lander parked on a plaza: sleek chrome hull, soft blue lights, utopian interstellar era. No text."),
        ("vehicle_hovercar", "A ground hovercar vehicle: sleek chrome body, blue neon accents, futuristic utopian city car. No text."),
        ("vehicle_hovercar_taxi", "A futuristic hover taxi: chrome body, soft yellow accent stripe, clean utopian design. No text."),
        ("vehicle_hoverbike", "A sleek hoverbike: chrome frame, blue neon strip, futuristic utopian city. No text."),
        ("vehicle_cargo_truck", "A futuristic cargo truck: hover-capable, chrome and white panels, utilitarian but clean, utopian city. No text."),
        ("vehicle_service_van", "A futuristic service van: chrome, clean, maintenance vehicle for utopian city. No text."),
        ("vehicle_police_patrol", "A futuristic police patrol vehicle: chrome armor, blue light bar, clean utopian style, non-aggressive. No text."),
        ("vehicle_skybus", "A flying sky-bus for a utopian spaceport city: clean chrome, large windows, gentle blue lights. No text."),
        ("vehicle_aerial_taxi", "A small aerial taxi vehicle: compact chrome craft, blue lights, utopian city sky traffic. No text."),
        ("vehicle_drone_courier", "A courier drone vehicle: small flying cargo drone, chrome shell, soft blue lights. No text."),
        ("vehicle_shuttle", "A small passenger shuttle: hovering/flying craft, chrome and glass, utopian spaceport city. No text."),
        ("unit_robot_worker", "A friendly humanoid robot worker unit that can walk: clean chrome and white panels, futuristic utopian city. No text."),
        ("unit_robot_security", "A security robot unit that can walk: sleek chrome armor, blue visor, non-threatening but capable. No text."),
        ("unit_robot_medic", "A medical assistant robot unit: white and chrome, soft green/blue lights, friendly, can walk. No text."),
        ("unit_robot_vendor", "A vendor robot unit: small friendly robot with a tray, chrome body, can walk. No text."),
        ("unit_alien_diplomat", "A tall elegant alien diplomat unit that can walk: smooth bioluminescent skin, futuristic robes, friendly. No text."),
        ("unit_alien_merchant", "An alien merchant unit that can walk: short and round body, colorful fabric packs, friendly. No text."),
        ("unit_alien_scientist", "An alien scientist unit that can walk: calm posture, lab coat-like futuristic clothing, friendly. No text."),
        ("unit_alien_child", "A small alien child unit that can walk: cute proportions, curious, friendly, futuristic clothing. No text."),
        ("unit_human_civilian", "A human civilian unit that can walk: futuristic utopian clothing, diverse and friendly. No text."),
        ("unit_human_pilot", "A human pilot unit that can walk: clean futuristic flight suit, utopian spaceport era. No text."),
        ("unit_android_artist", "An android artist unit that can walk: chrome body, colorful accent scarf, friendly, utopian city. No text."),
        ("unit_alien_guardian", "A tall alien guardian unit that can walk: elegant armor, chrome accents, calm and protective. No text."),
        ("unit_drone_camera", "A small flying camera drone unit: chrome sphere with lenses, soft blue lights, utopian city. No text."),
        ("unit_drone_security", "A small security drone unit: chrome shell, blue lights, friendly but vigilant, hovering. No text."),
    ]


def wasteland_asset_plan() -> list[tuple[str, str]]:
    return [
        ("road_cracked_tile", "A modular cracked wasteland road tile for a compact sci-fi frontier town. Faded lane paint, patched asphalt, dust, salvage repairs, readable from above. No text."),
        ("shoulder_scrap_tile", "A modular roadside shoulder tile with packed dust, concrete patches, scrap metal edging, and colorful salvage paint accents. No text."),
        ("crosswalk_faded_tile", "A modular faded crossing tile with worn paint, patchwork metal insets, and frontier-town sci-fi wear. No text."),
        ("streetlamp_patchwork", "A patchwork street lamp made from salvaged metal and practical sci-fi fixtures, colorful bulbs, readable silhouette. No text."),
        ("string_lights", "A hanging strand of market lights for a colorful wasteland street, improvised wiring, festive bulbs, frontier sci-fi look. No text."),
        ("bench_scrap", "A street bench assembled from salvaged metal and wood planks, practical and lived-in. No text."),
        ("shop_sign_neon", "A small colorful sci-fi shop sign built from salvage metal and neon tubing, no letters, just symbolic shapes. No text."),
        ("shop_totem", "A vertical roadside shop totem made from patched metal plates, bulbs, and glowing shapes, colorful frontier style. No text."),
        ("market_stall", "A colorful market stall for a post-apocalyptic sci-fi town, patched tarps, salvaged frame, goods implied, inviting. No text."),
        ("food_cart", "A small roadside food or tea cart for a wasteland sci-fi town, colorful canopy, salvaged wheels, everyday-life feel. No text."),
        ("awning_patchwork", "A patchwork storefront awning made from tarps, salvaged sheets, and bright painted panels. No text."),
        ("barrel_cluster", "A cluster of storage barrels and drums, practical salvage-town clutter, some bright paint stripes. No text."),
        ("crate_stack", "A stack of shipping crates and supply boxes for a frontier market or garage. No text."),
        ("scrap_heap", "A sculptural heap of scrap parts, broken panels, and useful salvage for a repair yard. No text."),
        ("solar_canopy", "A compact solar canopy for a small wasteland town utility yard, salvaged but functional sci-fi technology. No text."),
        ("water_tank_small", "A small elevated water tank or cistern for a frontier settlement, patched metal, colorful maintenance marks. No text."),
        ("antenna_pole", "A thin communications antenna mast with salvaged braces, everyday sci-fi frontier utility. No text."),
        ("generator_box", "A practical generator or battery box for a wasteland town utility corner, patched panels and cables. No text."),
        ("planter_drum", "A colorful planter made from reused industrial drums with hardy plants, cheerful frontier street decoration. No text."),
        ("junk_fence", "A fence section built from scrap panels, poles, and salvaged mesh, modular and readable. No text."),
        ("sat_dish", "A small salvaged satellite dish for a sci-fi frontier roof or yard. No text."),
        ("garage_bay", "A repair garage building for a sci-fi wasteland town, salvaged metal doors, practical service bay, colorful details. No text."),
        ("repair_shed", "A compact mechanic shed with tools, patched walls, and lived-in sci-fi salvage-town character. No text."),
        ("container_shop", "A storefront built from stacked shipping containers and colorful salvage panels, clearly a small shop. No text."),
        ("stacked_home", "A small stacked home made from salvaged modules and patched walls, warm everyday-life feel, colorful cloth accents. No text."),
        ("market_hall", "A low market hall building for a colorful frontier town, patched roof, open frontage, sci-fi salvage style. No text."),
        ("clinic_shop", "A small clinic or supply shop building with practical sci-fi equipment and welcoming colorful accents. No text."),
        ("diner_bar", "A compact diner or tea bar building for a sci-fi wasteland town, cozy and colorful, mixed salvage and retro-future style. No text."),
        ("recycler_tower", "A recycling or salvage-processing tower building, functional and slightly improvised, frontier sci-fi town scale. No text."),
        ("water_tower", "A tall water tower structure for a small settlement, patched steel and colored support braces. No text."),
        ("drone_pad", "A compact drone landing pad or dispatch shack for a frontier town, practical sci-fi design. No text."),
        ("watch_post", "A small lookout or security post made from salvage metal and practical sci-fi equipment. No text."),
        ("motel_block", "A compact lodging block or bunkhouse for a frontier town, patched facade, colorful doors or awnings. No text."),
        ("vehicle_buggy", "A small wasteland buggy vehicle with exposed suspension, salvage armor, and colorful painted panels. No text."),
        ("vehicle_cargo_trike", "A three-wheeled cargo vehicle for a frontier market town, practical, colorful, and everyday. No text."),
        ("vehicle_salvage_truck", "A medium salvage truck with patched body panels, cargo racks, and frontier-town wear. No text."),
        ("vehicle_hover_sled", "A small hover sled utility vehicle with improvised sci-fi components and bright painted accents. No text."),
        ("vehicle_market_van", "A compact market delivery van for a frontier town, patched body and colorful canopy accents. No text."),
        ("vehicle_scout_bike", "A lightweight scout bike for dusty streets, retro-future wasteland aesthetic, colorful details. No text."),
        ("vehicle_sky_skiff", "A small flying skiff for local frontier travel, compact sci-fi silhouette, colorful maintenance paint. No text."),
        ("vehicle_courier_drone", "A courier drone craft for deliveries around a sci-fi wasteland town, practical and colorful. No text."),
        ("vehicle_patrol_drone", "A slightly heavier patrol drone craft for a small settlement, practical and non-militaristic. No text."),
        ("vehicle_cargo_quad", "A hovering quad-rotor cargo carrier for a frontier settlement, improvised sci-fi style. No text."),
        ("unit_robot_mechanic", "A mechanic robot unit that can walk, salvage-town friendly, practical tools, colorful work markings. No text."),
        ("unit_robot_vendor", "A vendor robot unit that can walk, friendly posture, carrying goods tray or packs, colorful market-town accents. No text."),
        ("unit_robot_sweeper", "A maintenance robot unit that can walk, small and practical, keeping the street tidy, colorful markings. No text."),
        ("unit_robot_guard", "A frontier-town guard robot unit that can walk, protective but calm, practical scavenged armor and colored lights. No text."),
        ("unit_human_scavenger", "A human scavenger unit that can walk, layered frontier clothing, everyday-town life, colorful accents. No text."),
        ("unit_human_shopkeeper", "A human shopkeeper unit that can walk, relaxed and friendly, frontier market clothing with bright details. No text."),
        ("unit_human_child", "A human child unit that can walk, lively and curious, adapted frontier clothing with bright colors. No text."),
        ("unit_android_clerk", "An android clerk unit that can walk, tidy and friendly, small-town sci-fi service role, colorful trim. No text."),
        ("unit_alien_trader", "An alien trader unit that can walk, approachable, carrying goods or packs, colorful frontier-town styling. No text."),
        ("unit_alien_farmer", "An alien rancher or grower unit that can walk, everyday-life posture, practical frontier gear and colorful cloth. No text."),
        ("unit_wasteland_dog", "A friendly wasteland dog-like animal unit that can walk, hardy and expressive, everyday-town companion. No text."),
        ("unit_pack_lizard", "A medium pack-lizard animal unit that can walk, domesticated frontier settlement animal with harness. No text."),
        ("unit_desert_bird", "A small desert bird animal unit that can walk, lively and colorful enough to read from gameplay view. No text."),
        ("unit_moss_goat", "A compact goat-like settlement animal unit that can walk, adapted to harsh frontier life, slightly colorful fur or gear. No text."),
        ("unit_drone_helper", "A small helper drone unit that hovers, friendly, practical town-service robot with colorful lights. No text."),
        ("unit_drone_observer", "A small observer drone unit that hovers, non-threatening, compact sci-fi frontier utility style. No text."),
    ]


def get_scene_profile(profile_id: str) -> SceneProfile:
    key = str(profile_id or "").strip().lower()
    if key == "wasteland_town":
        return SceneProfile(
            profile_id="wasteland_town",
            scene_prefix="showcase_scene_wasteland",
            run_dir_prefix="showcase_scene_wasteland",
            label_prefix="Showcase (Wasteland Town)",
            description=(
                "A compact post-apocalyptic sci-fi town built around two crossing streets: "
                "market stalls, repair yards, homes, utility structures, vehicles, robots, drones, "
                "animals, and everyday life. Generated by automation."
            ),
            floor_prompt=(
                "A mostly flat compact terrain for a colorful post-apocalyptic sci-fi wasteland town. "
                "Target about 60m x 60m. Packed dust, cracked concrete, scattered salvage patches, "
                "subtle warm earth tones with a few colorful painted traces. Keep it smooth enough for "
                "streets, buildings, and vehicles. No hills, no cliffs."
            ),
            min_terrain_size_m=56.0,
            layout_kind="wasteland_town",
            asset_plan=wasteland_asset_plan(),
        )
    if key in ("utopian_chrome", "chrome", "default"):
        return SceneProfile(
            profile_id="utopian_chrome",
            scene_prefix="showcase_scene",
            run_dir_prefix="showcase_scene",
            label_prefix="Showcase (Utopian Chrome)",
            description=(
                "A future-fiction interstellar city plaza: clean chrome towers, mixed old-style district, "
                "streets with vehicles, robots and aliens living together. Generated by automation."
            ),
            floor_prompt=(
                "A perfectly flat utopian chrome plaza ground for a futuristic spaceport city. "
                "Very large continuous ground plane: at least 320m x 320m. "
                "Subtle hexagonal micro-pattern, clean white/silver materials, faint blue neon seams, "
                "gentle wear but pristine overall. No bumps, no hills."
            ),
            min_terrain_size_m=260.0,
            layout_kind="utopian_chrome",
            asset_plan=chrome_asset_plan(),
        )
    raise RuntimeError(f"Unknown profile: {profile_id}")


def build_layout_layers_for_profile(
    profile: SceneProfile,
    *,
    prefab_catalog: dict[str, dict[str, Any]],
    assets: dict[str, str],
    layout_extent_m: float,
    plaza_extent_m: float,
) -> dict[str, dict[str, Any]]:
    if profile.layout_kind == "wasteland_town":
        return build_layout_layers_wasteland(
            prefab_catalog=prefab_catalog,
            assets=assets,
            layout_extent_m=layout_extent_m,
            plaza_extent_m=plaza_extent_m,
        )
    return build_layout_layers_chrome(
        prefab_catalog=prefab_catalog,
        assets=assets,
        layout_extent_m=layout_extent_m,
        plaza_extent_m=plaza_extent_m,
    )


def progress_camera_for_profile(
    profile: SceneProfile,
    *,
    layout_extent_m: float,
    plaza_extent_m: float,
) -> dict[str, float]:
    if profile.layout_kind == "wasteland_town":
        return {
            "x": 0.0,
            "y": max(10.0, layout_extent_m * 0.46),
            "z": 0.0,
            "yaw": 0.58,
            "pitch": -0.5,
            "zoom_t": 0.86,
        }
    return {
        "x": 0.0,
        "y": max(12.0, layout_extent_m * 0.11),
        "z": 0.0,
        "yaw": 0.75,
        "pitch": -0.38,
        "zoom_t": 0.92,
    }


def curated_shots_for_profile(
    profile: SceneProfile,
    *,
    layout_extent_m: float,
    plaza_extent_m: float,
) -> list[dict[str, float | str]]:
    if profile.layout_kind == "wasteland_town":
        return [
            {
                "label": "overview_final",
                "x": 0.0,
                "y": max(10.5, layout_extent_m * 0.48),
                "z": 0.0,
                "yaw": 0.58,
                "pitch": -0.5,
                "zoom_t": 0.86,
            },
            {
                "label": "crossroads_life",
                "x": 0.0,
                "y": 2.4,
                "z": -(plaza_extent_m * 0.55),
                "yaw": 1.86,
                "pitch": -0.16,
                "zoom_t": 0.04,
            },
            {
                "label": "market_row",
                "x": layout_extent_m * 0.12,
                "y": 2.8,
                "z": plaza_extent_m * 0.92,
                "yaw": -2.7,
                "pitch": -0.18,
                "zoom_t": 0.03,
            },
            {
                "label": "garage_yard",
                "x": -(layout_extent_m * 0.52),
                "y": 2.7,
                "z": -(layout_extent_m * 0.54),
                "yaw": 0.78,
                "pitch": -0.14,
                "zoom_t": 0.06,
            },
            {
                "label": "utility_corner",
                "x": -(layout_extent_m * 0.56),
                "y": 4.0,
                "z": layout_extent_m * 0.68,
                "yaw": 0.22,
                "pitch": -0.2,
                "zoom_t": 0.1,
            },
        ]
    return [
        {
            "label": "overview_final",
            "x": 0.0,
            "y": max(14.0, layout_extent_m * 0.12),
            "z": 0.0,
            "yaw": 0.75,
            "pitch": -0.38,
            "zoom_t": 0.92,
        },
        {
            "label": "street_plaza",
            "x": 0.0,
            "y": 2.6,
            "z": -(plaza_extent_m * 0.33),
            "yaw": 1.95,
            "pitch": -0.14,
            "zoom_t": 0.05,
        },
        {
            "label": "old_district",
            "x": layout_extent_m * 0.7,
            "y": 2.6,
            "z": layout_extent_m * 0.66,
            "yaw": -2.35,
            "pitch": -0.18,
            "zoom_t": 0.0,
        },
        {
            "label": "spaceport",
            "x": -(layout_extent_m * 0.8),
            "y": 5.2,
            "z": -(layout_extent_m * 0.8),
            "yaw": 0.65,
            "pitch": -0.22,
            "zoom_t": 0.15,
        },
    ]


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument(
        "--profile",
        default="utopian_chrome",
        help="Scene profile to build. Supported: utopian_chrome, wasteland_town.",
    )
    ap.add_argument(
        "--run-dir",
        default=None,
        help="Run artifacts directory (default: test/run_1/showcase_scene_<timestamp>)",
    )
    ap.add_argument("--realm-id", default="default")
    ap.add_argument("--scene-id", default=None, help="Optional explicit scene id (default: auto versioned)")
    ap.add_argument(
        "--config",
        default=str(Path("~/.gravimera/config.toml").expanduser()),
        help="Path to Gravimera config.toml (AI provider config lives here). Default: ~/.gravimera/config.toml",
    )
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0)
    ap.add_argument(
        "--startup-timeout-secs",
        type=float,
        default=900.0,
        help=(
            "Seconds to wait for the game to compile/start and print the Automation API base URL. "
            "First-time --release builds can take several minutes."
        ),
    )
    ap.add_argument(
        "--floor-prompt",
        default=None,
        help="Override the selected profile's GenFloor prompt.",
    )
    ap.add_argument(
        "--min-terrain-size-m",
        type=float,
        default=None,
        help="Override the selected profile's minimum acceptable terrain size.",
    )
    args = ap.parse_args()

    profile = get_scene_profile(args.profile)

    repo_root = Path(__file__).resolve().parents[1]
    run_dir = (
        Path(args.run_dir).expanduser().resolve()
        if args.run_dir
        else (repo_root / "test" / "run_1" / f"{profile.run_dir_prefix}_{_now_tag()}").resolve()
    )
    run_dir.mkdir(parents=True, exist_ok=True)
    shots_dir = run_dir / "shots"
    shots_dir.mkdir(parents=True, exist_ok=True)

    manifest_path = run_dir / "manifest.json"
    manifest: dict[str, Any]
    if manifest_path.exists():
        manifest = _read_json(manifest_path)
    else:
        manifest = {
            "version": MANIFEST_VERSION,
            "created_at": _now_tag(),
            "profile": profile.profile_id,
            "realm_id": str(args.realm_id),
            "scene_id": None,
            "floor_id_uuid": None,
            "scene_run_id": None,
            "assets": {},
            "shots_taken": 0,
            "skipped_assets": [],
            "asset_failures": {},
        }
        _write_json(manifest_path, manifest)

    config_path = Path(args.config).expanduser().resolve()
    if not config_path.exists():
        raise RuntimeError(f"Config not found: {config_path}")

    manifest_profile = str(manifest.get("profile") or "").strip()
    if manifest_profile:
        if manifest_profile != profile.profile_id:
            raise RuntimeError(
                f"Run dir {run_dir} belongs to profile={manifest_profile}, cannot reuse for profile={profile.profile_id}"
            )
    else:
        manifest["profile"] = profile.profile_id
        _write_json(manifest_path, manifest)

    stdout_path = run_dir / "gravimera_stdout.log"

    interrupted = False

    def _sigint(_signum: int, _frame: Any) -> None:
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, _sigint)

    # Optional automation token (auth) from config.
    token: str | None = None
    try:
        import tomllib

        cfg = tomllib.loads(_read_text(config_path))
        tok = ((cfg.get("automation") or {}).get("token") or "").strip()
        if tok:
            token = tok
    except Exception:
        token = None

    game = GameProcess(repo_root=repo_root, config_path=config_path, stdout_path=stdout_path)
    game.start()

    api_base = ""
    http: LocalHttp | None = None
    try:
        api_base = discover_base_url_from_log(stdout_path, timeout_secs=float(args.startup_timeout_secs))
        http = LocalHttp(api_base, token=token)
        wait_health(http)

        if interrupted:
            return 130

        # Pick or create scene id.
        realm_id = str(args.realm_id)
        scene_id = str(manifest.get("scene_id") or "").strip()
        if not scene_id:
            if args.scene_id and args.scene_id.strip():
                scene_id = args.scene_id.strip()
            else:
                existing = list_scenes(http, realm_id=realm_id)
                scene_id = pick_versioned_scene_id(existing, prefix=profile.scene_prefix)
            manifest["scene_id"] = scene_id
            _write_json(manifest_path, manifest)

        label = f"{profile.label_prefix} {scene_id}"
        description = profile.description
        print(f"scene_id={scene_id}")
        realm_scene_create_and_switch(
            http,
            realm_id=realm_id,
            scene_id=scene_id,
            label=label,
            description=description,
            dt_secs=args.dt_secs,
        )

        # Terrain (GenFloor). We try to ensure a sufficiently large ground plane so the city layout
        # doesn't spill into the void.
        grav_home = gravimera_default_home_dir()
        min_terrain_m = float(args.min_terrain_size_m or profile.min_terrain_size_m)
        floor_prompt = str(args.floor_prompt or profile.floor_prompt).strip()
        floor_id = str(manifest.get("floor_id_uuid") or "").strip()

        def _terrain_ok(terrain_id: str) -> bool:
            size = read_terrain_size_m(gravimera_home=grav_home, realm_id=realm_id, terrain_id_uuid=terrain_id)
            return bool(size and min(size[0], size[1]) >= min_terrain_m)

        if floor_id and not _terrain_ok(floor_id):
            size = read_terrain_size_m(gravimera_home=grav_home, realm_id=realm_id, terrain_id_uuid=floor_id)
            print(f"warn: existing terrain too small size_m={size}; rebuilding via GenFloor")
            floor_id = ""

        attempts = 0
        while not floor_id or not _terrain_ok(floor_id):
            if interrupted:
                return 130
            attempts += 1
            print(f"Generating GenFloor terrain… (attempt {attempts})")
            prompt = f"{floor_prompt} Size requirement: at least {min_terrain_m:.0f}m x {min_terrain_m:.0f}m."
            floor_id = build_genfloor_flat(http, dt_secs=args.dt_secs, prompt=prompt)
            manifest["floor_id_uuid"] = floor_id
            _write_json(manifest_path, manifest)
            size = read_terrain_size_m(gravimera_home=grav_home, realm_id=realm_id, terrain_id_uuid=floor_id)
            if size:
                print(f"terrain size_m={size[0]:.1f} x {size[1]:.1f}")
            if attempts >= 3:
                break

        # Back to realm build and apply terrain selection.
        set_mode(http, "build", dt_secs=args.dt_secs)
        select_scene_terrain(http, floor_id, dt_secs=args.dt_secs)

        # Layout scale derived from the terrain we ended up with.
        terrain_size = read_terrain_size_m(gravimera_home=grav_home, realm_id=realm_id, terrain_id_uuid=floor_id)
        if terrain_size:
            half = 0.5 * min(float(terrain_size[0]), float(terrain_size[1]))
            if profile.layout_kind == "wasteland_town":
                layout_extent_m = max(16.0, min(half - 6.0, 34.0))
            else:
                layout_extent_m = max(18.0, min(half - 8.0, 180.0))
        else:
            layout_extent_m = 24.0 if profile.layout_kind == "wasteland_town" else 140.0
        if profile.layout_kind == "wasteland_town":
            plaza_extent_m = max(9.0, min(14.0, layout_extent_m * 0.42))
        else:
            plaza_extent_m = max(20.0, min(55.0, layout_extent_m * 0.45))

        # Import scene sources (required for run_apply_patch).
        dirs = get_active_scene_dirs(http)
        if dirs["scene_id"] != scene_id:
            print(f"warn: active scene_id mismatch: expected={scene_id} got={dirs['scene_id']}")
        import_scene_sources(http, dirs["scene_src_dir"], dt_secs=args.dt_secs)

        run_id = ensure_run_id(manifest, profile_id=profile.profile_id)
        _write_json(manifest_path, manifest)

        asset_plan = profile.asset_plan

        prompt_by_key: dict[str, str] = {k: p for (k, p) in asset_plan}

        assets: dict[str, str] = dict(manifest.get("assets") or {})
        shots_taken = int(manifest.get("shots_taken") or 0)
        skipped_assets: set[str] = set(manifest.get("skipped_assets") or [])
        failures_raw = manifest.get("asset_failures") or {}
        failures: dict[str, int] = {str(k): int(v) for (k, v) in failures_raw.items()} if isinstance(failures_raw, dict) else {}

        def _save_manifest() -> None:
            manifest["assets"] = assets
            manifest["shots_taken"] = shots_taken
            manifest["skipped_assets"] = sorted(skipped_assets)
            manifest["asset_failures"] = failures
            _write_json(manifest_path, manifest)

        _save_manifest()

        # Ensure realm prefabs from prior runs are loaded into the in-memory library, so patches
        # can place them immediately after a restart.
        try:
            reload_realm_prefabs(http)
        except Exception as err:
            print(f"warn: prefabs/reload_realm failed (continuing): {err}")

        # If the manifest references prefabs that aren't present in the running library, clear
        # them so we regenerate instead of being stuck forever with an unplaceable prefab id.
        prefab_catalog_boot = get_prefab_catalog(http)
        for key, prefab_id_uuid in list(assets.items()):
            pid = str(prefab_id_uuid or "").strip()
            if pid and pid not in prefab_catalog_boot:
                print(f"warn: manifest prefab not in library; will regenerate: {key} -> {pid[:8]}")
                assets.pop(key, None)
        _save_manifest()

        # Layout patches as durable run steps. We re-apply as assets complete so the scene is
        # decorated incrementally while Gen3D continues running.
        status = scene_run_status(http, run_id)
        next_step = int(((status.get("status") or {}).get("next_step") or 1))
        print(f"scene run_id={run_id} next_step={next_step}")

        last_layout_asset_count = -1
        last_layout_time = 0.0

        def _asset_ready_count() -> int:
            return sum(1 for v in assets.values() if str(v).strip())

        def _apply_layout_and_shot(tag: str) -> None:
            nonlocal next_step, shots_taken, last_layout_asset_count, last_layout_time
            try:
                reload_realm_prefabs(http)
            except Exception as err:
                print(f"warn: prefabs/reload_realm failed before layout apply: {err}")
            prefab_catalog = get_prefab_catalog(http)
            layers = build_layout_layers_for_profile(
                profile,
                prefab_catalog=prefab_catalog,
                assets=assets,
                layout_extent_m=layout_extent_m,
                plaza_extent_m=plaza_extent_m,
            )
            patch_ops = []
            for layer_id in sorted(layers.keys()):
                patch_ops.append({"kind": "upsert_layer", "layer_id": layer_id, "doc": layers[layer_id]})

            resp = apply_run_step(http, run_id=run_id, step_no=next_step, patch_ops=patch_ops)
            result = resp.get("result") or {}
            applied = bool(result.get("applied"))
            if not applied:
                # Common recovery: after restart, prefabs may exist on disk but not in memory.
                # Reload and retry once before surfacing an error.
                try:
                    reload_realm_prefabs(http)
                    prefab_catalog = get_prefab_catalog(http)
                    layers = build_layout_layers_for_profile(
                        profile,
                        prefab_catalog=prefab_catalog,
                        assets=assets,
                        layout_extent_m=layout_extent_m,
                        plaza_extent_m=plaza_extent_m,
                    )
                    patch_ops = [
                        {"kind": "upsert_layer", "layer_id": layer_id, "doc": layers[layer_id]}
                        for layer_id in sorted(layers.keys())
                    ]
                    resp = apply_run_step(http, run_id=run_id, step_no=next_step, patch_ops=patch_ops)
                    result = resp.get("result") or {}
                    applied = bool(result.get("applied"))
                except Exception:
                    applied = False
                if not applied:
                    raise RuntimeError(f"run_apply_patch step failed: {resp}")
            print(f"layout applied (step={next_step}) tag={tag}")
            next_step += 1
            last_layout_asset_count = _asset_ready_count()
            last_layout_time = time.monotonic()

            # Let new instances appear, then take an overview shot.
            step(http, 3, dt_secs=args.dt_secs)
            safe = re.sub(r"[^A-Za-z0-9_-]+", "_", tag).strip("_") or "shot"
            shots_taken += 1
            out_path = shots_dir / f"{shots_taken:03d}_{safe}.png"
            progress_camera = progress_camera_for_profile(
                profile,
                layout_extent_m=layout_extent_m,
                plaza_extent_m=plaza_extent_m,
            )
            set_camera_and_shot(
                http,
                x=float(progress_camera["x"]),
                y=float(progress_camera["y"]),
                z=float(progress_camera["z"]),
                yaw=float(progress_camera["yaw"]),
                pitch=float(progress_camera["pitch"]),
                zoom_t=float(progress_camera["zoom_t"]),
                out_path=out_path,
                dt_secs=args.dt_secs,
            )
            _save_manifest()

        # Enqueue missing assets up-front (queue is FIFO).
        inflight: dict[str, str] = {}
        for key, prompt in asset_plan:
            if interrupted:
                return 130
            if key in skipped_assets:
                continue
            if key in assets and str(assets[key]).strip():
                continue
            print(f"Gen3D enqueue: {key}")
            try:
                task_id = enqueue_gen3d_task(http, kind="build", prompt=prompt)
            except Exception as err:
                failures[key] = failures.get(key, 0) + 1
                print(f"warn: failed to enqueue {key}: {err}")
                continue
            inflight[key] = task_id
            _save_manifest()

        # If resuming with assets already ready, apply once so the scene matches the manifest.
        if assets and _asset_ready_count() > 0:
            _apply_layout_and_shot("resume")

        new_assets_since_layout = 0
        applied_once = False
        max_retries = 3

        while inflight:
            if interrupted:
                return 130

            # Advance simulation; Gen3D tasks progress on frames.
            step(http, 12, dt_secs=args.dt_secs)

            # Poll tasks.
            for key, task_id in list(inflight.items()):
                if interrupted:
                    return 130
                try:
                    resp = http.json("GET", f"/v1/gen3d/tasks/{task_id}")
                    task = resp.get("task") or {}
                except Exception as err:
                    failures[key] = failures.get(key, 0) + 1
                    print(f"warn: task query failed for {key}: {err}")
                    inflight.pop(key, None)
                    if failures[key] <= max_retries:
                        print(f"retry enqueue (query-failed): {key} attempt {failures[key]}/{max_retries}")
                        inflight[key] = enqueue_gen3d_task(http, kind="build", prompt=prompt_by_key[key])
                    else:
                        skipped_assets.add(key)
                        print(f"skip: {key} after {failures[key]} errors")
                    _save_manifest()
                    continue

                state = str(task.get("state") or "")
                if state == "done":
                    prefab_id = str(task.get("result_prefab_id_uuid") or "").strip()
                    if prefab_id:
                        assets[key] = prefab_id
                        inflight.pop(key, None)
                        new_assets_since_layout += 1
                        print(f"Gen3D done: {key} -> {prefab_id[:8]}")
                        _save_manifest()
                elif state in ("failed", "canceled"):
                    err = task.get("error")
                    inflight.pop(key, None)
                    failures[key] = failures.get(key, 0) + 1
                    print(f"warn: Gen3D {key} ended state={state}: {err!r}")
                    if failures[key] <= max_retries:
                        print(f"retry enqueue: {key} attempt {failures[key]}/{max_retries}")
                        inflight[key] = enqueue_gen3d_task(http, kind="build", prompt=prompt_by_key[key])
                    else:
                        skipped_assets.add(key)
                        print(f"skip: {key} after {failures[key]} failures")
                    _save_manifest()

            # Apply layout occasionally so the UI shows incremental decoration progress.
            ready_count = _asset_ready_count()
            if profile.layout_kind == "wasteland_town":
                have_infra = bool(str(assets.get("road_cracked_tile") or "").strip()) and bool(
                    str(assets.get("shoulder_scrap_tile") or "").strip()
                )
            else:
                have_infra = bool(str(assets.get("road_tile") or "").strip()) and bool(
                    str(assets.get("sidewalk_tile") or assets.get("plaza_tile") or "").strip()
                )
            should_apply = have_infra and (
                (not applied_once)
                or (ready_count != last_layout_asset_count and new_assets_since_layout >= 4)
                or (ready_count != last_layout_asset_count and (time.monotonic() - last_layout_time) > 180.0)
            )
            if should_apply:
                _apply_layout_and_shot(f"progress_{ready_count:02}")
                applied_once = True
                new_assets_since_layout = 0

            time.sleep(0.05)

        # Final apply to ensure everything is placed.
        if _asset_ready_count() != last_layout_asset_count:
            _apply_layout_and_shot("final")

        # Final curated screenshots (overview + key districts).
        def _shot(
            *,
            label: str,
            x: float,
            y: float,
            z: float,
            yaw: float,
            pitch: float,
            zoom_t: float,
        ) -> None:
            nonlocal shots_taken
            safe = re.sub(r"[^A-Za-z0-9_-]+", "_", label).strip("_") or "shot"
            shots_taken += 1
            out_path = shots_dir / f"{shots_taken:03d}_{safe}.png"
            set_camera_and_shot(
                http,
                x=x,
                y=y,
                z=z,
                yaw=yaw,
                pitch=pitch,
                zoom_t=zoom_t,
                out_path=out_path,
                dt_secs=args.dt_secs,
            )
            _save_manifest()

        for shot in curated_shots_for_profile(
            profile,
            layout_extent_m=layout_extent_m,
            plaza_extent_m=plaza_extent_m,
        ):
            _shot(
                label=str(shot["label"]),
                x=float(shot["x"]),
                y=float(shot["y"]),
                z=float(shot["z"]),
                yaw=float(shot["yaw"]),
                pitch=float(shot["pitch"]),
                zoom_t=float(shot["zoom_t"]),
            )

        # Let brains attach in Play mode (existing engine behavior).
        set_mode(http, "play", dt_secs=args.dt_secs)
        http.json("POST", "/v1/resume", {})
        time.sleep(2.0)
        http.json("POST", "/v1/pause", {})
        step(http, 2, dt_secs=args.dt_secs)
        shots_taken += 1
        shot_play = shots_dir / f"{shots_taken:03d}_play_mode.png"
        http.json("POST", "/v1/screenshot", {"path": str(shot_play)}, timeout_secs=300.0)
        step(http, 3, dt_secs=args.dt_secs)
        _save_manifest()

        # Persist scene.
        http.json("POST", "/v1/scene/save", {})
        step(http, 3, dt_secs=args.dt_secs)

        print(f"OK: scene_id={scene_id} floor_id_uuid={floor_id} shots_dir={shots_dir}")
        return 0
    finally:
        if http is not None:
            try:
                http.json("POST", "/v1/shutdown", {}, timeout_secs=2.0, retries=1)
            except Exception:
                pass
        game.ensure_stopped()


if __name__ == "__main__":
    raise SystemExit(main())
