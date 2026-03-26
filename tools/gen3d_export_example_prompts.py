#!/usr/bin/env python3
"""
Export Gen3D prompt history from the local Gravimera cache into a parsable Markdown file.

The Gen3D cache format has evolved over time. This script is intentionally dependency-free
and uses multiple fallbacks to locate:
  - user prompt text
  - short descriptor text (semantic summary)

It also deduplicates entries by normalized prompt text.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


def _read_text(path: Path) -> str:
    return path.read_text(encoding="utf-8", errors="replace")


def _load_json(path: Path) -> Any:
    return json.loads(_read_text(path))


def _iter_jsonl(path: Path) -> Iterable[dict[str, Any]]:
    with path.open("r", encoding="utf-8", errors="replace") as f:
        for raw in f:
            line = raw.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            if isinstance(obj, dict):
                yield obj


def _ms_to_iso(ms: int) -> str:
    dt = datetime.fromtimestamp(ms / 1000.0, tz=timezone.utc)
    return dt.isoformat().replace("+00:00", "Z")


def _normalize_prompt(text: str) -> str:
    # Normalize enough for stable dedupe without destroying meaningful formatting.
    text = text.replace("\r\n", "\n").replace("\r", "\n")
    lines = [line.rstrip() for line in text.split("\n")]
    return "\n".join(lines).strip()


def _pass_num(pass_dir: Path) -> int:
    m = re.match(r"^pass_(\d+)$", pass_dir.name)
    if not m:
        return 0
    return int(m.group(1))


def _sorted_pass_files(attempt_dir: Path, filename: str) -> list[Path]:
    out: list[Path] = []
    for pass_dir in attempt_dir.glob("pass_*"):
        if not pass_dir.is_dir():
            continue
        path = pass_dir / filename
        if path.is_file():
            out.append(path)
    out.sort(key=lambda p: _pass_num(p.parent))
    return out


def _find_attempt_dir(run_dir: Path) -> Path | None:
    attempt0 = run_dir / "attempt_0"
    if attempt0.is_dir():
        return attempt0
    attempts = [p for p in run_dir.glob("attempt_*") if p.is_dir()]
    if not attempts:
        return None

    def attempt_num(p: Path) -> int:
        m = re.match(r"^attempt_(\d+)$", p.name)
        return int(m.group(1)) if m else 0

    attempts.sort(key=attempt_num)
    return attempts[0]


def _extract_prompt_from_agent_step_user_text(text: str) -> str | None:
    lines = text.splitlines()
    for i, line in enumerate(lines):
        if line.strip() != "User prompt:":
            continue
        collected: list[str] = []
        for line2 in lines[i + 1 :]:
            if line2.startswith("Input images:"):
                break
            collected.append(line2)
        prompt = "\n".join(collected).strip("\n").strip()
        return prompt or None
    return None


def _extract_user_notes_from_prompt_intent(text: str) -> str | None:
    lines = text.splitlines()
    try:
        start = lines.index("User notes:")
    except ValueError:
        return None

    collected: list[str] = []
    for line in lines[start + 1 :]:
        if (
            line.startswith("Reference photo")
            or line.startswith("Question:")
            or line.startswith("Return JSON")
        ):
            break
        collected.append(line)
    notes = "\n".join(collected).strip("\n").strip()
    return notes or None


def _extract_prompt_from_descriptor_meta_user_text(text: str) -> str | None:
    # Common formats:
    #   "User prompt (context only; DO NOT copy as short):\n<...>"
    #   "User prompt:\n<...>"
    lines = text.splitlines()
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("User prompt") and stripped.endswith(":"):
            prompt = "\n".join(lines[i + 1 :]).strip("\n").strip()
            return prompt or None
        if stripped.startswith("User prompt:"):
            after = stripped.partition(":")[2].strip()
            if after:
                return after
    return None


def _find_prompt(run_dir: Path) -> str | None:
    attempt_dir = _find_attempt_dir(run_dir)
    if attempt_dir is not None:
        manifest = attempt_dir / "inputs_manifest.json"
        if manifest.is_file():
            try:
                data = _load_json(manifest)
                rel = data.get("user_prompt_file")
                if isinstance(rel, str):
                    prompt_path = attempt_dir / rel
                    if prompt_path.is_file():
                        prompt = _read_text(prompt_path).strip()
                        if prompt:
                            return prompt
            except Exception:
                pass

        # Some older caches include the file but not the manifest.
        prompt_path = attempt_dir / "inputs" / "user_prompt.txt"
        if prompt_path.is_file():
            prompt = _read_text(prompt_path).strip()
            if prompt:
                return prompt

        for path in _sorted_pass_files(attempt_dir, "agent_step_user_text.txt"):
            try:
                prompt = _extract_prompt_from_agent_step_user_text(_read_text(path))
                if prompt:
                    return prompt
            except Exception:
                continue

        for path in _sorted_pass_files(attempt_dir, "prompt_intent_user_text.txt"):
            try:
                notes = _extract_user_notes_from_prompt_intent(_read_text(path))
                if notes:
                    return notes
            except Exception:
                continue

        for path in _sorted_pass_files(attempt_dir, "descriptor_meta_user_text.txt"):
            try:
                prompt = _extract_prompt_from_descriptor_meta_user_text(_read_text(path))
                if prompt:
                    return prompt
            except Exception:
                continue

    events = run_dir / "info_store_v1" / "events.jsonl"
    if events.is_file():
        for obj in _iter_jsonl(events):
            if obj.get("kind") != "tool_call_start":
                continue
            data = obj.get("data")
            if not isinstance(data, dict):
                continue
            args = data.get("args")
            if not isinstance(args, dict):
                continue
            prompt = args.get("prompt")
            if isinstance(prompt, str) and prompt.strip():
                return prompt.strip()

    trace = run_dir / "agent_trace.jsonl"
    if trace.is_file():
        for obj in _iter_jsonl(trace):
            ev = obj.get("event")
            if not isinstance(ev, dict):
                continue
            if ev.get("kind") != "tool_call":
                continue
            args = ev.get("args")
            if not isinstance(args, dict):
                continue
            prompt = args.get("prompt")
            if isinstance(prompt, str) and prompt.strip():
                return prompt.strip()

    return None


def _short_from_kv_state_summary(run_dir: Path) -> str | None:
    kv = run_dir / "info_store_v1" / "kv.jsonl"
    if not kv.is_file():
        return None

    best_rev = -1
    best_short: str | None = None
    for obj in _iter_jsonl(kv):
        key = obj.get("key")
        if not isinstance(key, dict):
            continue
        if key.get("namespace") != "gen3d":
            continue
        key_name = key.get("key")
        if not isinstance(key_name, str) or not key_name.endswith("state_summary"):
            continue
        kv_rev = obj.get("kv_rev")
        if not isinstance(kv_rev, int):
            continue
        value = obj.get("value")
        if not isinstance(value, dict):
            continue
        descriptor_meta = value.get("descriptor_meta")
        if not isinstance(descriptor_meta, dict):
            continue

        short: str | None = None
        effective = descriptor_meta.get("effective")
        if isinstance(effective, dict) and isinstance(effective.get("short"), str):
            short = effective.get("short")
        if (
            not short
            and isinstance(descriptor_meta.get("seeded"), dict)
            and isinstance(descriptor_meta["seeded"].get("short"), str)
        ):
            short = descriptor_meta["seeded"].get("short")
        if (
            not short
            and isinstance(descriptor_meta.get("override"), dict)
            and isinstance(descriptor_meta["override"].get("short"), str)
        ):
            short = descriptor_meta["override"].get("short")

        if short and kv_rev > best_rev:
            best_rev = kv_rev
            best_short = short.strip()

    return best_short


def _short_from_descriptor_meta_responses(run_dir: Path) -> str | None:
    attempt_dir = _find_attempt_dir(run_dir)
    if attempt_dir is None:
        return None
    candidates = _sorted_pass_files(attempt_dir, "descriptor_meta_responses.json")
    if not candidates:
        return None

    for path in reversed(candidates):
        try:
            resp = _load_json(path)
        except Exception:
            continue
        output = resp.get("output")
        if not isinstance(output, list):
            continue
        for out_item in output:
            if not isinstance(out_item, dict) or out_item.get("type") != "message":
                continue
            content = out_item.get("content")
            if not isinstance(content, list):
                continue
            for c in content:
                if not isinstance(c, dict) or c.get("type") != "output_text":
                    continue
                txt = c.get("text")
                if not isinstance(txt, str) or not txt.strip():
                    continue
                try:
                    meta = json.loads(txt)
                except json.JSONDecodeError:
                    continue
                if not isinstance(meta, dict):
                    continue
                short = meta.get("short")
                if isinstance(short, str) and short.strip():
                    return short.strip()
    return None


def _short_from_set_descriptor_meta_trace(run_dir: Path) -> str | None:
    trace = run_dir / "agent_trace.jsonl"
    if not trace.is_file():
        return None
    for obj in _iter_jsonl(trace):
        ev = obj.get("event")
        if not isinstance(ev, dict):
            continue
        if ev.get("kind") != "tool_result":
            continue
        if ev.get("tool_id") != "set_descriptor_meta_v1":
            continue
        if ev.get("ok") is not True:
            continue
        result = ev.get("result")
        if not isinstance(result, dict):
            continue
        short = result.get("short")
        if isinstance(short, str) and short.strip():
            return short.strip()
    return None


def _find_short(run_dir: Path) -> str | None:
    short = _short_from_kv_state_summary(run_dir)
    if short:
        return short

    short = _short_from_descriptor_meta_responses(run_dir)
    if short:
        return short

    short = _short_from_set_descriptor_meta_trace(run_dir)
    if short:
        return short

    return None


def _created_at_iso(run_dir: Path) -> str | None:
    run_json = run_dir / "run.json"
    if not run_json.is_file():
        return None
    try:
        data = _load_json(run_json)
    except Exception:
        return None
    ms = data.get("created_at_ms")
    return _ms_to_iso(ms) if isinstance(ms, int) else None


@dataclass(frozen=True)
class RunRecord:
    run_id: str
    created_at: str | None
    prompt: str | None
    short: str | None


def _iter_run_records(cache_dir: Path) -> list[RunRecord]:
    run_records: list[RunRecord] = []
    for run_dir in sorted(cache_dir.iterdir()):
        if not run_dir.is_dir():
            continue
        if run_dir.name.startswith("."):
            continue
        run_id = run_dir.name
        created_at = _created_at_iso(run_dir)
        prompt = _find_prompt(run_dir)
        short = _find_short(run_dir)
        if prompt is not None:
            prompt = _normalize_prompt(prompt)
            if not prompt:
                prompt = None
        if short is not None:
            short = short.strip() or None
        if prompt is None and short is None:
            continue
        run_records.append(
            RunRecord(
                run_id=run_id,
                created_at=created_at,
                prompt=prompt,
                short=short,
            )
        )

    def sort_key(r: RunRecord) -> tuple[str, str]:
        return (r.created_at or "", r.run_id)

    run_records.sort(key=sort_key)
    return run_records


def _build_payload(*, cache_dir: Path, records: list[RunRecord]) -> dict[str, Any]:
    by_prompt: dict[str, list[RunRecord]] = {}
    missing_prompt: list[dict[str, Any]] = []
    for r in records:
        if r.prompt is None:
            missing_prompt.append(
                {
                    "created_at": r.created_at,
                    "run_id": r.run_id,
                    "short": r.short,
                }
            )
            continue
        by_prompt.setdefault(r.prompt, []).append(r)

    entries: list[dict[str, Any]] = []
    for prompt, runs in by_prompt.items():
        runs_sorted = sorted(runs, key=lambda r: (r.created_at or "", r.run_id))
        first_seen = next((r.created_at for r in runs_sorted if r.created_at), None)
        last_seen = next((r.created_at for r in reversed(runs_sorted) if r.created_at), None)

        latest_short = next((r.short for r in reversed(runs_sorted) if r.short), None)
        short_variants = sorted({r.short for r in runs_sorted if r.short})

        entry: dict[str, Any] = {
            "prompt": prompt,
            "run_count": len(runs_sorted),
            "run_ids": [r.run_id for r in runs_sorted],
        }
        if first_seen is not None:
            entry["first_seen"] = first_seen
        if last_seen is not None:
            entry["last_seen"] = last_seen
        if latest_short is not None:
            entry["short"] = latest_short
        if len(short_variants) > 1:
            entry["short_variants"] = short_variants
        entries.append(entry)

    entries.sort(key=lambda e: (e.get("first_seen", ""), e.get("prompt", "")))
    missing_prompt.sort(key=lambda e: (e.get("created_at") or "", e.get("run_id") or ""))

    return {
        "version": 1,
        "source_dir": str(cache_dir),
        "dedupe": {
            "by": "prompt_normalized_v1",
            "run_count_total": len(records),
            "unique_prompt_count": len(entries),
            "missing_prompt_run_count": len(missing_prompt),
        },
        "entries": entries,
        "missing_prompt_runs": missing_prompt,
    }


def _write_markdown(*, out_path: Path, payload: dict[str, Any]) -> None:
    out_path.parent.mkdir(parents=True, exist_ok=True)
    body = json.dumps(payload, ensure_ascii=False, indent=2, sort_keys=True)
    md = "\n".join(
        [
            "# Gen3D example prompts",
            "",
            "Auto-generated from local Gen3D cache runs.",
            "",
            "Regenerate:",
            f"- `python3 tools/gen3d_export_example_prompts.py --cache-dir {payload['source_dir']} --out {out_path}`",
            "",
            "```json",
            body,
            "```",
            "",
        ]
    )
    out_path.write_text(md, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description="Export Gen3D cached prompts into Markdown+JSON.")
    parser.add_argument(
        "--cache-dir",
        type=Path,
        default=Path("~/.gravimera/cache/gen3d").expanduser(),
        help="Gen3D cache directory (default: ~/.gravimera/cache/gen3d).",
    )
    parser.add_argument(
        "--out",
        type=Path,
        default=Path("assets/example_prompts.md"),
        help="Output Markdown path (default: assets/example_prompts.md).",
    )
    args = parser.parse_args()

    if not args.cache_dir.is_dir():
        print(f"error: cache dir not found: {args.cache_dir}", file=sys.stderr)
        return 2

    records = _iter_run_records(args.cache_dir)
    payload = _build_payload(cache_dir=args.cache_dir, records=records)
    _write_markdown(out_path=args.out, payload=payload)

    print(
        "ok:"
        f" runs={payload['dedupe']['run_count_total']}"
        f" unique_prompts={payload['dedupe']['unique_prompt_count']}"
        f" missing_prompt_runs={payload['dedupe']['missing_prompt_run_count']}"
        f" out={args.out}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
