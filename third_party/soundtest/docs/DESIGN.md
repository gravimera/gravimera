# Soundtest CLI - Design

## What it does

`soundtest` is a Rust CLI that turns `(object, message)` into **local audio**:

1. An OpenAI-compatible model produces a small plain-text **render plan**.
2. The program executes the plan using exactly one **local backend**:
   - `onnx` (fully offline ONNX TTS via Chatterbox multilingual model)
   - `system` (system TTS rendered to a WAV file, then processed)
   - `procedural` (built-in synthesis for non-speaking/static objects)
3. For `system` TTS, the model may rewrite the text a bit for character, then an **effects chain** (dragon/robot/etc) is applied. **Tempo and pitch are independently controllable.**

Non-goals:
- Real recorded animal samples.
- Vendor/cloud TTS services.

## CLI

Single:

```
soundtest speak <object> <text...>
```

Diagnostics:

```
soundtest doctor
```

Batch (renders all sounds first, then mixes + plays them at the same time):

```
soundtest --speak <object> <message> [--speak <object> <message> ...]
```

Key flags:
- `--config <path>`: config file path (default: `~/.soundtest/config.toml`)
- `--base-url <url>` / `--token <token>` / `--model <name>`: override AI settings
- `--reasoning-effort <low|medium|high>`: default is `medium`
- `--backend <auto|onnx|system|procedural>`: default is `auto`
- `--no-ai`: disable AI planning and run fully offline
- `--onnx-model-dir <dir>`: Chatterbox multilingual ONNX model directory (enables `onnx` backend)
- `--onnx-voice <path>`: reference voice WAV for ONNX TTS (defaults to `default_voice.wav` in the model dir)
- `--onnx-language-model <path>`: override the Chatterbox language model ONNX file (optional)
- `--onnx-runtime <path>`: ONNX Runtime dynamic library path (`onnxruntime.dll` / `libonnxruntime.dylib`)
- Effects overrides (TTS backends only): `--preset`, `--amount`, `--speed`, `--pitch-semitones`, `--bass-db`, `--treble-db`, `--reverb`, `--distortion`
- `--volume <0..100>`: output volume (default: `100`)
- `--dry-run`: print plan + tools preview; do not play audio
- `--verbose`: print selected tools/effects and extra diagnostics (never prints token)
- `--ai-concurrency <n>` / `--tts-concurrency <n>` / `--dsp-concurrency <n>`: pipeline limits for batch mode

Stdout behavior:
- For TTS plans: prints the final `text:` that will be spoken.
- For procedural plans: prints the `proc:` token text.
- For batch runs: prints one line per item in input order: `<object>: <text/proc>`.

To see what tools are used without playing audio:

```
soundtest speak dragon "Hello" --dry-run --verbose
```

To run fully offline (no AI) with ONNX TTS:

```
soundtest speak dragon "Hello" --no-ai --backend onnx --onnx-model-dir /path/to/chatterbox-model
```

To print ONNX timing breakdowns (tokenizer / speech encoder / language model / decoder), set:

```
SOUNDTEST_ONNX_TIMINGS=1 soundtest speak dragon "Hello" --no-ai --backend onnx
```

To bootstrap a local Chatterbox multilingual ONNX test model + ONNX Runtime:

macOS (Apple Silicon):

```
bash scripts/bootstrap_chatterbox_multilingual_onnx.sh
```

Windows:

```
powershell -ExecutionPolicy Bypass -File scripts/bootstrap_chatterbox_multilingual_onnx.ps1
```

By default these scripts download from `hf-mirror.com` (set `SOUNDTEST_HF_BASE` to use a different Hugging Face base URL).

After bootstrapping (from repo root), you can test with:

```
cargo run -- speak dragon "Hello" --no-ai
```

To build a self-contained folder you can zip/copy to another machine (includes the binary + model assets):

macOS/Linux:

```
bash scripts/build_dist.sh
```

Windows:

```
powershell -ExecutionPolicy Bypass -File scripts/build_dist.ps1
```

The output is written to `./dist/soundtest-<triple>/` and can be run directly:

```
./soundtest doctor
./soundtest speak dragon "Hello" --no-ai --backend onnx
```

To render and play many voices at once:

```
soundtest --speak dog "How is the weather today?" --speak cow "What's your name?"
```

## Configuration

Default location: `~/.soundtest/config.toml`

Example:

```toml
# AI settings (optional when using `--no-ai` or `soundtest doctor`)
base_url = "https://api.openai.com/v1"
token = "..."
model = "gpt-5.2"
model_reasoning_effort = "medium" # default

# Optional: OpenAI wire API selection
# - "auto" tries /responses first, then falls back to /chat/completions
wire_api = "auto" # auto|responses|chat_completions

# Optional: fully offline ONNX TTS (Chatterbox multilingual)
# Model dir layout:
# - tokenizer.json
# - default_voice.wav (or set onnx_voice)
# - onnx/speech_encoder.onnx
# - onnx/speech_encoder.onnx_data
# - onnx/embed_tokens.onnx
# - onnx/embed_tokens.onnx_data
# - onnx/conditional_decoder.onnx
# - onnx/conditional_decoder.onnx_data
# - onnx/language_model*.onnx
# - onnx/language_model*.onnx_data
onnx_model_dir = "/path/to/chatterbox-model"
onnx_voice = "/path/to/voice.wav" # optional
onnx_runtime = "/path/to/libonnxruntime.dylib" # optional; can also use ORT_DYLIB_PATH

# Optional: system TTS tool override
# - Windows default: "powershell"
# - macOS default: "say"
system_tts_binary = "powershell"
```

Credential/config fallback order:
1. CLI flags (`--base-url`, `--token`, `--model`, `--reasoning-effort`)
2. `~/.soundtest/config.toml`
3. Env vars `OPENAI_BASE_URL` / `OPENAI_API_KEY`
4. Codex CLI defaults in `~/.codex/config.toml` + `~/.codex/auth.json`

## Backend availability detection

At runtime the program detects:
- `system`: available on Windows and macOS if `system_tts_binary` resolves to an executable
  - Windows: uses `powershell.exe` to drive SAPI and render a WAV file
  - macOS: uses `say` to render a WAV file
- `onnx`: available when `onnx_model_dir` is set and the required model files + voice WAV + ONNX Runtime dylib are present
  - Uses ONNX Runtime via the `ort` crate with `load-dynamic`
  - You can point to the dylib via `onnx_runtime` or `ORT_DYLIB_PATH` (recommended for packaging)
- `procedural`: always available

The detected list is embedded into the AI prompt so `auto` selection chooses only usable backends.

## AI API (OpenAI-compatible)

Depending on `wire_api`, the client uses:
- `POST {base_url}/responses`
- `POST {base_url}/chat/completions`

`wire_api=auto` tries `/responses` first and falls back to `/chat/completions`.

`model_reasoning_effort` is sent when supported; on 400 errors the client retries once without it.

## Render plan format (model output)

The model must return ONLY plain text. No markdown. No explanation.

### System TTS plan

```
backend: system
text: <text to speak>                       (required)
preset: <neutral|dragon|robot|fairy|giant|ghost|radio>
amount: <0.0-1.0>
speed: <0.4-1.8>                             (optional; 1.0 normal)
pitch_semitones: <-24..24>                   (optional; negative lowers voice)
bass_db: <-12..18>                           (optional)
treble_db: <-12..18>                         (optional)
reverb: <0.0-1.0>                             (optional)
distortion: <0.0-1.0>                         (optional)
```

### ONNX TTS plan

Same fields as `system`, but with `backend: onnx`:

```
backend: onnx
text: <text to speak>                       (required)
preset: <neutral|dragon|robot|fairy|giant|ghost|radio>
amount: <0.0-1.0>
...
```

### Procedural plan

```
backend: procedural
proc: <token text for procedural synthesis>  (required)
```

Rules:
- Choose EXACTLY ONE backend.
- If `backend: procedural`, the program ignores any TTS/effect fields.
- If `backend: system`, the program ignores any `proc:` fields.
- `text:` should preserve the user's meaning and language, but may be lightly rewritten to match the object's voice.

## Effects engine

For TTS backends (`onnx`/`system`), the audio pipeline is:

1. Render speech audio:
   - `system`: run system TTS to render a temporary WAV file.
   - `onnx`: run local ONNX inference to produce a waveform (24 kHz).
2. Convert to mono `f32` samples (decode WAV for `system`; `onnx` is already `f32`).
3. Apply an effects chain:
   - **Pitch shift** (semitones) via linear resampling.
   - **Tempo (speed)** via a WSOLA-style time-stretcher.
   - **EQ** via low-shelf (bass) and high-shelf (treble) biquads.
   - **Reverb** via a lightweight comb/allpass network.
   - **Distortion** via a soft clip stage.
   - **Limiter** to prevent clipping.
4. Play audio via `rodio`.

Pitch and tempo independence:
- Pitch shifting changes duration by the pitch factor.
- A compensating time-stretch is applied so the final duration matches `speed` while pitch matches `pitch_semitones`.

## Observability (CLI + logs)

CLI:
- `--dry-run --verbose` prints the render plan and resolved local tool (`powershell.exe`/`say`) and final effect parameters.
- `--verbose` also prints per-segment details during playback.

Logs:
- JSONL at `~/.soundtest/soundtest.log.jsonl`
- Includes: AI timings, model + effort, wire API used, selected tools, effect parameters, and per-segment durations.
- The API token is never printed or logged.

## Testing

- Unit tests cover render plan parsing and the (currently unused) sanitizer module.
- `cargo test` is fully offline and does not call the AI API.
