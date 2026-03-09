# Soundtest CLI - Implementation Plan

This plan is written for iterating from an MVP to a higher-quality "object voice" system.

## Milestone 0 (MVP - implemented)

- CLI command: `soundtest speak <object> <text...>`
- Batch mode: `soundtest --speak <object> <message> [--speak <object> <message> ...]` (renders all sounds, then mixes + plays together)
- Config loading from `~/.soundtest/config.toml` with fallbacks:
  - `~/.codex/config.toml` + `~/.codex/auth.json`
  - `OPENAI_BASE_URL` / `OPENAI_API_KEY`
  - CLI overrides
- AI render planner (OpenAI-compatible):
  - supports `model` + `model_reasoning_effort` (default: `medium`)
  - `wire_api=auto` uses `/responses` then falls back to `/chat/completions`
- Local backends:
  - `onnx` (fully offline Chatterbox multilingual ONNX TTS)
  - `system` (system TTS rendered to WAV, then processed)
  - `procedural` (built-in token -> audio synthesis)
- Render plan schema:
  - `backend: system` + `text:` + effect knobs
  - `backend: procedural` + `proc:`
  - `auto` selects exactly one backend (no combo mode)
- Audio effects chain:
  - independent tempo (`speed`) and pitch (`pitch_semitones`)
  - EQ, reverb, distortion, limiter
- Logging:
  - JSONL at `~/.soundtest/soundtest.log.jsonl`
  - includes tool selection + timings + model/effort (never token)
- Tests:
  - `cargo test` covers render-plan parsing + DSP utilities + sanitizer module

## Milestone 1 (quality & control)

Effects quality:
- Improve pitch shifting quality (higher-quality resampler, optional formant-ish approaches).
- Improve time-stretch quality (more robust WSOLA, transient handling).
- Add optional "texture" effects: chorus, flanger, ring-mod, bitcrush, noise gate.

User controls (without removing AI automation):
- Add CLI overrides to bypass AI for effects (implemented):
  - `--preset`, `--amount`, `--speed`, `--pitch-semitones`, `--bass-db`, `--treble-db`, `--reverb`, `--distortion`
- Add `--no-ai` mode (implemented):
  - user supplies backend + text/proc + effect knobs directly
  - useful for repeatable demos and offline runs

Backend UX:
- Add `soundtest doctor` (implemented):
  - prints detected backends and resolved paths (`system_tts_binary`, procedural always)
  - prints platform notes (Windows uses PowerShell+SAPI; macOS uses `say`)

## Milestone 2 (presets & object mapping)

- Expand presets (e.g., `cat`, `dog`, `orc`, `demon`, `android`, `elf`, `whisper`).
- Add an internal mapping layer:
  - AI outputs `preset:` + `amount:` most of the time
  - only uses explicit knob overrides when needed
- Add "object hints" to the system prompt:
  - animals -> higher pitch / lighter reverb; giants -> slower + bass; radio -> band-limit + distortion.

## Milestone 3 (testing & packaging)

- Add offline unit tests for DSP utilities:
  - expected length changes for speed/pitch, non-panics on short buffers, limiter behavior.
- Add a CI-friendly integration test mode:
  - `--dry-run` verification only (no audio playback)
- Document platform notes:
  - Windows: requires `powershell.exe` and SAPI (built-in)
  - macOS: requires `say` (built-in)
