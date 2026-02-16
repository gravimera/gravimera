#!/usr/bin/env python3
"""
Run a real (rendered) Gen3D build via the local Automation HTTP API.

This is intentionally a "semantic" driver that:
- Starts the game with a given config.toml (Automation enabled)
- Enters Gen3D workshop
- Sets a prompt + builds
- Saves the draft into the world
- Moves + fires while capturing screenshots/frames to the run cache folder

The output artifacts are saved under:
  <gen3d_cache>/<run_id>/external_screenshots_*/

Note: This script does NOT print or handle secrets; your OpenAI key stays in config.toml/env.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import signal
import subprocess
import sys
import time
import urllib.error
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import Any


def _now_ms() -> int:
    return int(time.time() * 1000)


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
    raise ValueError("config.toml: missing [automation].bind (example: bind = \"127.0.0.1:8792\")")


def _parse_scene_dat_path(config_text: str) -> str | None:
    # Best-effort parse (no external TOML dependency).
    in_scene = False
    for raw in config_text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            in_scene = line == "[scene]"
            continue
        if not in_scene:
            continue
        m = re.match(r'scene_dat_path\s*=\s*"([^"]+)"\s*$', line)
        if m:
            return m.group(1)
    return None


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


@dataclass
class RunResult:
    run_id: str
    run_dir: Path
    instance_id_uuid: str


class GameProcess:
    def __init__(self, *, bin_path: Path, config_path: Path, workdir: Path, stdout_path: Path):
        self._bin_path = bin_path
        self._config_path = config_path
        self._workdir = workdir
        self._stdout_path = stdout_path
        self._proc: subprocess.Popen[bytes] | None = None

    def start(self) -> None:
        self._stdout_path.parent.mkdir(parents=True, exist_ok=True)
        out = open(self._stdout_path, "wb")
        self._proc = subprocess.Popen(
            [str(self._bin_path), "--config", str(self._config_path)],
            cwd=str(self._workdir),
            stdout=out,
            stderr=subprocess.STDOUT,
        )

    def is_running(self) -> bool:
        return self._proc is not None and self._proc.poll() is None

    def terminate(self) -> None:
        if self._proc is None:
            return
        if self._proc.poll() is not None:
            return
        try:
            self._proc.terminate()
        except Exception:
            pass

    def kill(self) -> None:
        if self._proc is None:
            return
        if self._proc.poll() is not None:
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


def run_one(
    *,
    api_base: str,
    prompt: str,
    build_timeout_secs: float,
    save_early: bool,
    world_move_offset: tuple[float, float],
    frames_per_capture: int,
    capture_count: int,
    dt_secs: float,
) -> RunResult:
    def post(path: str, body: dict[str, Any]) -> dict[str, Any]:
        # `/v1/step` can legitimately take longer while render captures or other work is in-flight.
        if path == "/v1/step":
            timeout = 300.0
        elif path in ("/v1/gen3d/save", "/v1/screenshot", "/v1/shutdown"):
            timeout = 300.0
        else:
            timeout = 30.0
        return _http_json("POST", f"{api_base}{path}", body, timeout=timeout)

    def get(path: str) -> dict[str, Any]:
        return _http_json("GET", f"{api_base}{path}", None, timeout=30.0)

    # Enter Gen3D workshop.
    post("/v1/mode", {"mode": "gen3d"})
    post("/v1/step", {"frames": 3, "dt_secs": dt_secs})
    post("/v1/gen3d/prompt", {"prompt": prompt})

    build = post("/v1/gen3d/build", {})
    run_id = str(build.get("run_id") or "").strip()
    if not run_id:
        raise RuntimeError(f"gen3d/build returned no run_id: {build}")

    status_log: list[dict[str, Any]] = []
    t0 = time.monotonic()
    last_print = 0.0
    last_pass = None
    last_status = None
    run_dir: Path | None = None

    while True:
        # Drive progress by stepping frames.
        post("/v1/step", {"frames": 10, "dt_secs": dt_secs})

        # Be tolerant of transient HTTP errors during long-running builds.
        status = None
        last_err: Exception | None = None
        for attempt in range(6):
            try:
                status = get("/v1/gen3d/status")
                break
            except RuntimeError as err:
                last_err = err
                msg = str(err)
                if any(code in msg for code in ("HTTP 502", "HTTP 503", "HTTP 504")):
                    time.sleep(0.2 * (attempt + 1))
                    continue
                raise
        if status is None:
            raise RuntimeError(f"Failed to fetch /v1/gen3d/status after retries: {last_err}")

        status_log.append({"t": round(time.monotonic() - t0, 3), **status})

        run_dir_raw = status.get("run_dir") or ""
        if run_dir_raw:
            run_dir = Path(run_dir_raw)
        build_complete = bool(status.get("build_complete"))
        draft_ready = bool(status.get("draft_ready"))
        running = bool(status.get("running"))
        cur_pass = status.get("pass")
        cur_status = status.get("status")
        cur_error = status.get("error")

        # Print only on changes (avoid huge logs).
        if cur_pass != last_pass or cur_status != last_status:
            # Sanitize status to one line, short.
            msg = str(cur_status or "").replace("\n", " ").strip()
            if len(msg) > 140:
                msg = msg[:137] + "…"
            print(f"pass={cur_pass} running={running} complete={build_complete}/{draft_ready} status={msg}")
            last_pass = cur_pass
            last_status = cur_status

        if save_early:
            # A "usable draft" is enough (root + at least one non-projectile primitive part).
            # This mode is faster but can produce incomplete models if the agent hasn't finished.
            if draft_ready:
                if not build_complete:
                    print(
                        "note: draft_ready=true but build_complete=false; proceeding to Save early"
                    )
                break
        else:
            # For a full end-to-end test (including animations/aim/combat), wait for the agent to
            # finish the build (or best-effort stop due to budgets).
            if build_complete:
                if not draft_ready:
                    raise RuntimeError(
                        f"Gen3D build completed but produced no usable draft (draft_ready=false). error={cur_error!r} status={cur_status!r}"
                    )
                break

        if not running and not build_complete:
            raise RuntimeError(
                f"Gen3D build stopped unexpectedly (running=false, build_complete=false). error={cur_error!r} status={cur_status!r}"
            )

        if time.monotonic() - t0 > build_timeout_secs:
            print(f"warn: build timeout reached ({build_timeout_secs:.1f}s); requesting /v1/gen3d/stop and attempting best-effort Save")
            try:
                post("/v1/gen3d/stop", {})
            except Exception:
                pass

            # Give the game a short grace window to transition to build_complete.
            grace_t0 = time.monotonic()
            final_status = status
            while time.monotonic() - grace_t0 < 10.0:
                try:
                    post("/v1/step", {"frames": 5, "dt_secs": dt_secs})
                    final_status = get("/v1/gen3d/status")
                    if final_status.get("build_complete") or not final_status.get("running"):
                        break
                except Exception:
                    pass
                time.sleep(0.2)

            if bool(final_status.get("draft_ready")):
                print(
                    "note: proceeding with best-effort Save after timeout; build_complete="
                    f"{bool(final_status.get('build_complete'))} running={bool(final_status.get('running'))}"
                )
                status = final_status
                break

            raise RuntimeError(
                f"Timed out waiting for Gen3D build after {build_timeout_secs:.1f}s. Last status: {status}"
            )

        # Avoid hammering the API too fast.
        now = time.monotonic()
        if now - last_print < 0.05:
            time.sleep(0.05)
        last_print = now

    if run_dir is None:
        raise RuntimeError("Missing run_dir from gen3d/status")

    # Save into world.
    save = post("/v1/gen3d/save", {})
    instance_id = str(save.get("instance_id_uuid") or "").strip()
    if not instance_id:
        raise RuntimeError(f"gen3d/save returned no instance_id_uuid: {save}")

    # Stop the agent after saving so the test doesn't keep consuming time/tokens.
    try:
        post("/v1/gen3d/stop", {})
    except Exception:
        pass

    # Switch back to build mode.
    post("/v1/mode", {"mode": "build"})
    post("/v1/step", {"frames": 5, "dt_secs": dt_secs})

    post("/v1/select", {"instance_ids": [instance_id]})

    # Create external screenshot dirs under run_dir.
    world_dir = run_dir / "external_screenshots_world"
    anim_dir = run_dir / "external_screenshots_anim"
    world_dir.mkdir(parents=True, exist_ok=True)
    anim_dir.mkdir(parents=True, exist_ok=True)

    # Write driver status trace to run_dir for debugging.
    (run_dir / "driver_status.jsonl").write_text(
        "\n".join(json.dumps(row, ensure_ascii=False) for row in status_log) + "\n",
        encoding="utf-8",
    )

    post("/v1/screenshot", {"path": str(world_dir / "spawn.png")})

    # Move: find current pos, then offset.
    state = get("/v1/state")
    pos = None
    has_attack = False
    attack_kind = None
    for obj in state.get("objects", []):
        if obj.get("instance_id_uuid") == instance_id:
            pos = obj.get("pos")
            has_attack = bool(obj.get("has_attack"))
            attack_kind = obj.get("attack_kind")
            break
    if not pos or len(pos) != 3:
        pos = [0.0, 0.0, 0.0]
    x, _, z = float(pos[0]), float(pos[1]), float(pos[2])
    dx, dz = world_move_offset
    dest_x = x + dx
    dest_z = z + dz
    try:
        post("/v1/move", {"x": dest_x, "z": dest_z})
    except RuntimeError as err:
        msg = str(err)
        if "HTTP 409" in msg:
            print(f"skip move: {msg}")
        else:
            raise

    # Capture movement frames.
    for i in range(capture_count):
        post("/v1/step", {"frames": frames_per_capture, "dt_secs": dt_secs})
        post("/v1/screenshot", {"path": str(anim_dir / f"frame_{i:02}.png")})
        if i == 0:
            post("/v1/screenshot", {"path": str(world_dir / "move_1s.png")})

    if has_attack:
        fire_dir = run_dir / "external_screenshots_anim_fire"
        fire_dir.mkdir(parents=True, exist_ok=True)

        # While firing, keep issuing a move order so we test "move direction vs attention direction"
        # (unit moves one way while aiming/firing another way).
        fire_move_x = dest_x + dx
        fire_move_z = dest_z + dz
        try:
            post("/v1/move", {"x": fire_move_x, "z": fire_move_z})
        except RuntimeError as err:
            msg = str(err)
            if "HTTP 409" in msg:
                print(f"skip fire-move: {msg}")
            else:
                raise

        # Fire at a point offset to +X so the aim direction differs from the move direction.
        fire_x = fire_move_x + 8.0
        fire_z = fire_move_z
        post("/v1/fire", {"active": True, "target": {"kind": "point", "x": fire_x, "z": fire_z}})
        for i in range(capture_count):
            post("/v1/step", {"frames": frames_per_capture, "dt_secs": dt_secs})
            post("/v1/screenshot", {"path": str(fire_dir / f"frame_{i:02}.png")})
            if i == 0:
                post("/v1/screenshot", {"path": str(world_dir / "fire_1s.png")})
        post("/v1/fire", {"active": False})
    else:
        print(f"skip fire: has_attack=false attack_kind={attack_kind!r}")

    # Best-effort mp4 encoding (optional).
    ffmpeg = shutil.which("ffmpeg")
    if ffmpeg:
        subprocess.run(
            [
                ffmpeg,
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-framerate",
                "10",
                "-i",
                str(anim_dir / "frame_%02d.png"),
                "-pix_fmt",
                "yuv420p",
                str(anim_dir / "move_anim.mp4"),
            ],
            check=False,
        )
        if has_attack:
            fire_dir = run_dir / "external_screenshots_anim_fire"
            subprocess.run(
                [
                    ffmpeg,
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-y",
                    "-framerate",
                    "10",
                    "-i",
                    str(fire_dir / "frame_%02d.png"),
                    "-pix_fmt",
                    "yuv420p",
                    str(fire_dir / "fire_anim.mp4"),
                ],
                check=False,
            )

    return RunResult(run_id=run_id, run_dir=run_dir, instance_id_uuid=instance_id)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--config", required=True, help="Path to config.toml (automation must be enabled)")
    ap.add_argument("--bin", default=None, help="Path to gravimera binary (default: target/debug/gravimera)")
    ap.add_argument("--workdir", default=None, help="Working directory for the game process (default: config dir)")
    ap.add_argument("--build-timeout-secs", type=float, default=3600.0, help="Timeout for Gen3D build")
    ap.add_argument(
        "--save-early",
        action="store_true",
        help="Save once draft_ready=true (faster but can yield incomplete models). Default waits for build_complete=true.",
    )
    ap.add_argument("--dt-secs", type=float, default=1.0 / 60.0, help="Fixed dt for /v1/step")
    ap.add_argument("--prompt", action="append", default=[], help="Prompt text (repeatable). If omitted, read from stdin.")
    ap.add_argument(
        "--reset-scene",
        action="store_true",
        help="Delete configured [scene].scene_dat_path before running (test isolation).",
    )
    ap.add_argument(
        "--reset-scene-each-prompt",
        action="store_true",
        help="In non-single-session mode, delete scene.dat before EACH prompt (strong isolation; does not accumulate units across prompts).",
    )
    ap.add_argument(
        "--single-session",
        action="store_true",
        help="Run all prompts in a single game session (saves all units into the same world).",
    )
    ap.add_argument("--move-dx", type=float, default=6.0)
    ap.add_argument("--move-dz", type=float, default=2.0)
    ap.add_argument("--frames-per-capture", type=int, default=6)
    ap.add_argument("--capture-count", type=int, default=20)
    args = ap.parse_args()

    config_path = Path(args.config).expanduser().resolve()
    config_text = _read_text(config_path)
    bind = _parse_automation_bind(config_text)
    api_base = f"http://{bind}"
    scene_dat_path_raw = _parse_scene_dat_path(config_text)
    scene_dat_path: Path | None = None
    if scene_dat_path_raw:
        scene_dat_path = Path(scene_dat_path_raw)
        if not scene_dat_path.is_absolute():
            scene_dat_path = (config_path.parent / scene_dat_path).resolve()

    bin_path = Path(args.bin) if args.bin else Path("target/debug/gravimera")
    if not bin_path.is_absolute():
        bin_path = (Path.cwd() / bin_path).resolve()

    workdir = Path(args.workdir).expanduser().resolve() if args.workdir else config_path.parent
    stdout_path = workdir / "gravimera_stdout.log"

    prompts = list(args.prompt)
    if not prompts:
        stdin = sys.stdin.read().strip()
        if stdin:
            prompts = [line.strip() for line in stdin.splitlines() if line.strip()]
    if not prompts:
        raise SystemExit("No prompts provided. Use --prompt or pipe prompts via stdin.")

    # Ctrl+C handling for clean shutdown.
    interrupted = False

    def _sigint(_signum: int, _frame: Any) -> None:
        nonlocal interrupted
        interrupted = True

    signal.signal(signal.SIGINT, _sigint)

    def reset_scene_file() -> None:
        if not args.reset_scene:
            return
        if scene_dat_path is None:
            print("warn: --reset-scene requested but config.toml has no [scene].scene_dat_path")
            return
        try:
            scene_dat_path.unlink(missing_ok=True)
            print(f"reset: deleted scene file: {scene_dat_path}")
        except Exception as err:
            print(f"warn: failed to delete scene file {scene_dat_path}: {err}")

    def wait_health(game: GameProcess) -> None:
        t0 = time.monotonic()
        while True:
            if not game.is_running():
                raise RuntimeError(f"Game exited early. See {stdout_path}")
            try:
                health = _http_json("GET", f"{api_base}/v1/health", None, timeout=0.5)
                if health.get("ok"):
                    break
            except Exception:
                pass
            if time.monotonic() - t0 > 30.0:
                raise RuntimeError(
                    f"Timed out waiting for Automation API on {api_base}. See {stdout_path}"
                )
            time.sleep(0.1)

    def shutdown_game(game: GameProcess) -> None:
        try:
            _http_json("POST", f"{api_base}/v1/shutdown", {}, timeout=2.0)
        except Exception:
            pass
        game.ensure_stopped()

    if args.reset_scene:
        reset_scene_file()

    if args.single_session:
        game = GameProcess(bin_path=bin_path, config_path=config_path, workdir=workdir, stdout_path=stdout_path)
        game.start()
        try:
            wait_health(game)
            for idx, prompt in enumerate(prompts):
                if interrupted:
                    break
                print(f"=== Gen3D real test ({idx+1}/{len(prompts)}): {prompt} ===")

                # Move offsets per prompt to avoid overlap in screenshots.
                dx = args.move_dx + idx * 3.0
                dz = args.move_dz + idx * 2.0

                result = run_one(
                    api_base=api_base,
                    prompt=prompt,
                    build_timeout_secs=args.build_timeout_secs,
                    save_early=args.save_early,
                    world_move_offset=(dx, dz),
                    frames_per_capture=args.frames_per_capture,
                    capture_count=args.capture_count,
                    dt_secs=args.dt_secs,
                )
                print(
                    f"OK: run_id={result.run_id} run_dir={result.run_dir} instance_id_uuid={result.instance_id_uuid}"
                )
        finally:
            shutdown_game(game)
    else:
        for idx, prompt in enumerate(prompts):
            if interrupted:
                break
            print(f"=== Gen3D real test ({idx+1}/{len(prompts)}): {prompt} ===")
            if args.reset_scene_each_prompt:
                reset_scene_file()

            game = GameProcess(bin_path=bin_path, config_path=config_path, workdir=workdir, stdout_path=stdout_path)
            game.start()
            try:
                wait_health(game)
                result = run_one(
                    api_base=api_base,
                    prompt=prompt,
                    build_timeout_secs=args.build_timeout_secs,
                    save_early=args.save_early,
                    world_move_offset=(args.move_dx, args.move_dz),
                    frames_per_capture=args.frames_per_capture,
                    capture_count=args.capture_count,
                    dt_secs=args.dt_secs,
                )
                print(
                    f"OK: run_id={result.run_id} run_dir={result.run_dir} instance_id_uuid={result.instance_id_uuid}"
                )
            finally:
                shutdown_game(game)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
