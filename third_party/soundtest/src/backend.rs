use crate::cli::BackendChoice;
use crate::chatterbox_onnx::{ChatterboxOnnx, ChatterboxOnnxConfig};
use crate::config::Settings;
use crate::effects::{self, EffectParams};
use crate::language;
use crate::logging;
use crate::render_plan::{BackendKind, RenderPlan};
use anyhow::{Context, Result, anyhow};
use rodio::Source;
use rodio::buffer::SamplesBuffer;
use serde_json::json;
use std::io::{BufReader, Read, Seek};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct BackendAvailability {
    pub system_tts: Option<SystemTtsAvailability>,
    pub onnx_tts: Option<OnnxTtsAvailability>,
    pub onnx_tts_error: Option<String>,
    pub procedural: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SystemTtsKind {
    #[cfg(windows)]
    PowerShellSapi,
    #[cfg(target_os = "macos")]
    Say,
}

#[derive(Debug, Clone)]
pub struct SystemTtsAvailability {
    pub binary: PathBuf,
    kind: SystemTtsKind,
}

#[derive(Debug, Clone)]
pub struct OnnxTtsAvailability {
    pub config: ChatterboxOnnxConfig,
    pub voice_path: PathBuf,
    pub runtime_path: PathBuf,
    pub language_model_path: PathBuf,
}

impl BackendAvailability {
    pub fn detect(settings: &Settings) -> Self {
        let system_tts = detect_system_tts(settings);
        let (onnx_tts, onnx_tts_error) = detect_onnx_tts(settings);

        Self {
            system_tts,
            onnx_tts,
            onnx_tts_error,
            procedural: true,
        }
    }

    pub fn available_backends_for_ai(&self) -> Vec<String> {
        let mut out = Vec::new();
        if self.onnx_tts.is_some() {
            out.push("onnx".to_owned());
        }
        if self.system_tts.is_some() {
            out.push("system".to_owned());
        }
        out.push("procedural".to_owned());
        out
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionPreview {
    pub plan_backend: BackendKind,
    pub requested_backend: BackendChoice,

    pub resolved_tts_backend: Option<String>,
    pub resolved_tts_tool: Option<String>,

    pub text: Option<String>,
    pub proc: Option<String>,
    pub effects: Option<EffectParams>,
}

#[derive(Debug, Clone)]
pub struct RenderedAudio {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
}

impl ExecutionPreview {
    pub fn format_tools(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "plan_backend={} requested_backend={}",
            format!("{:?}", self.plan_backend).to_ascii_lowercase(),
            format!("{:?}", self.requested_backend).to_ascii_lowercase()
        ));

        match (&self.resolved_tts_backend, &self.resolved_tts_tool) {
            (Some(backend), Some(tool)) => {
                out.push_str(&format!(
                    "\nresolved_tts_backend={backend}\nresolved_tts_tool={tool}"
                ));
            }
            _ => out.push_str("\nresolved_tts_backend=none"),
        }

        if self.plan_backend == BackendKind::Procedural {
            out.push_str("\nprocedural_tool=built-in");
        }

        if let Some(effects) = &self.effects {
            out.push_str(&format!("\neffects={}", effects.to_json()));
        }

        out
    }
}

pub fn preview_execution(
    settings: &Settings,
    plan: &RenderPlan,
    requested_backend: BackendChoice,
) -> Result<ExecutionPreview> {
    let availability = BackendAvailability::detect(settings);
    let allow_fallback = requested_backend == BackendChoice::Auto;
    let tts_backend = resolve_tts_backend(plan.backend, &availability, allow_fallback)?;

    let effects_params = match plan.backend {
        BackendKind::System | BackendKind::Onnx => Some(EffectParams::from_spec(&plan.effects)),
        BackendKind::Procedural => None,
    };

    Ok(ExecutionPreview {
        plan_backend: plan.backend,
        requested_backend,
        resolved_tts_backend: tts_backend.as_ref().map(|b| tts_backend_kind(b).to_owned()),
        resolved_tts_tool: tts_backend.as_ref().map(tts_backend_tool),
        text: plan.text.clone(),
        proc: plan.proc.clone(),
        effects: effects_params,
    })
}

pub fn synthesize_system_tts_mono(
    availability: &BackendAvailability,
    requested_backend: BackendChoice,
    text: &str,
) -> Result<RenderedAudio> {
    synthesize_tts_mono_for_plan(availability, requested_backend, BackendKind::System, text)
}

pub fn synthesize_tts_mono_for_plan(
    availability: &BackendAvailability,
    requested_backend: BackendChoice,
    plan_backend: BackendKind,
    text: &str,
) -> Result<RenderedAudio> {
    if plan_backend == BackendKind::Procedural {
        return Err(anyhow!("procedural backend cannot synthesize TTS audio"));
    }

    let allow_fallback = requested_backend == BackendChoice::Auto;
    let tts_backend = resolve_tts_backend(plan_backend, availability, allow_fallback)?
        .ok_or_else(|| anyhow!("TTS backend is not available"))?;
    let (samples, sample_rate) = synthesize_tts_mono(&tts_backend, text)?;
    Ok(RenderedAudio { samples, sample_rate })
}

pub async fn execute_render_plan(
    settings: &Settings,
    plan: &RenderPlan,
    requested_backend: BackendChoice,
    verbose: bool,
    volume: f32,
) -> Result<()> {
    let start = Instant::now();
    let availability = BackendAvailability::detect(settings);
    let audio = AudioOut::new(volume).context("failed to initialize audio output")?;

    let allow_fallback = requested_backend == BackendChoice::Auto;
    let tts_backend = resolve_tts_backend(plan.backend, &availability, allow_fallback)?;
    let effects_params = match plan.backend {
        BackendKind::System | BackendKind::Onnx => Some(EffectParams::from_spec(&plan.effects)),
        BackendKind::Procedural => None,
    };

    logging::info(
        "backend.start",
        json!({
            "plan_backend": format!("{:?}", plan.backend).to_ascii_lowercase(),
            "requested_backend": format!("{:?}", requested_backend).to_ascii_lowercase(),
            "resolved_tts_backend": tts_backend.as_ref().map(tts_backend_kind),
            "resolved_tts_tool": tts_backend.as_ref().map(tts_backend_tool),
            "available_backends": availability.available_backends_for_ai(),
            "volume": volume,
            "effects_spec": {
                "preset": &plan.effects.preset,
                "amount": plan.effects.amount,
                "speed": plan.effects.speed,
                "pitch_semitones": plan.effects.pitch_semitones,
                "bass_db": plan.effects.bass_db,
                "treble_db": plan.effects.treble_db,
                "reverb": plan.effects.reverb,
                "distortion": plan.effects.distortion,
            },
            "effects_params": effects_params.as_ref().map(|p| p.to_json()),
        }),
    );

    if verbose {
        eprintln!(
            "tools: {}",
            preview_execution(settings, plan, requested_backend)?.format_tools()
        );
    }

    match plan.backend {
        BackendKind::Procedural => {
            let tokens = plan
                .proc
                .as_deref()
                .ok_or_else(|| anyhow!("render plan missing `proc:` for procedural backend"))?;
            println!("{tokens}");

            let segment_start = Instant::now();
            logging::info(
                "backend.segment.start",
                json!({
                    "segment_index": 0,
                    "kind": "procedural",
                    "proc_chars": tokens.chars().count(),
                }),
            );
            if verbose {
                eprintln!("segment 0: procedural (built-in)");
            }

            let result = crate::procedural::play_token_text(&audio, tokens);
            if let Err(err) = result {
                logging::error(
                    "backend.segment.end",
                    json!({
                        "segment_index": 0,
                        "kind": "procedural",
                        "status": "error",
                        "duration_ms": segment_start.elapsed().as_millis(),
                        "error": format!("{err:#}"),
                    }),
                );
                logging::error(
                    "backend.end",
                    json!({
                        "status": "error",
                        "duration_ms": start.elapsed().as_millis(),
                        "error": format!("{err:#}"),
                    }),
                );
                return Err(err);
            }

            logging::info(
                "backend.segment.end",
                json!({
                    "segment_index": 0,
                    "kind": "procedural",
                    "status": "ok",
                    "duration_ms": segment_start.elapsed().as_millis(),
                }),
            );
        }
        BackendKind::System | BackendKind::Onnx => {
            let text = plan
                .text
                .as_deref()
                .ok_or_else(|| anyhow!("render plan missing `text:` for TTS backend"))?;
            println!("{text}");

            let Some(tts_backend) = tts_backend else {
                let err =
                    anyhow!("render plan requested TTS backend but no TTS backend is available");
                logging::error(
                    "backend.end",
                    json!({
                        "status": "error",
                        "duration_ms": start.elapsed().as_millis(),
                        "error": format!("{err:#}"),
                    }),
                );
                return Err(err);
            };

            let effects_params = effects_params.unwrap_or_else(EffectParams::neutral);

            let segment_start = Instant::now();
            logging::info(
                "backend.segment.start",
                json!({
                    "segment_index": 0,
                    "kind": "tts",
                    "tts_backend": tts_backend_kind(&tts_backend),
                    "tts_tool": tts_backend_tool(&tts_backend),
                    "text_chars": text.chars().count(),
                    "effects_params": effects_params.to_json(),
                }),
            );

            if verbose {
                eprintln!(
                    "segment 0: tts via {} (tool={})",
                    tts_backend_kind(&tts_backend),
                    tts_backend_tool(&tts_backend)
                );
                eprintln!("segment 0: effects {}", effects_params.to_json());
            }

            let tts_start = Instant::now();
            let (samples, sample_rate) = synthesize_tts_mono(&tts_backend, text)?;
            let tts_ms = tts_start.elapsed().as_millis();

            let fx_start = Instant::now();
            let processed = effects::apply_effects_mono(&samples, sample_rate, &effects_params);
            let fx_ms = fx_start.elapsed().as_millis();

            let play_start = Instant::now();
            let source = SamplesBuffer::new(1, sample_rate, processed);
            audio.play(source)?;
            let play_ms = play_start.elapsed().as_millis();

            logging::info(
                "backend.segment.end",
                json!({
                    "segment_index": 0,
                    "kind": "tts",
                    "status": "ok",
                    "duration_ms": segment_start.elapsed().as_millis(),
                    "tts_ms": tts_ms,
                    "effects_ms": fx_ms,
                    "play_ms": play_ms,
                }),
            );
        }
    }

    logging::info(
        "backend.end",
        json!({
            "status": "ok",
            "duration_ms": start.elapsed().as_millis(),
        }),
    );

    Ok(())
}

#[derive(Debug, Clone)]
enum TtsBackend {
    System(SystemTtsAvailability),
    Onnx(OnnxTtsAvailability),
}

fn tts_backend_kind(backend: &TtsBackend) -> &'static str {
    match backend {
        TtsBackend::System(_) => "system",
        TtsBackend::Onnx(_) => "onnx",
    }
}

fn tts_backend_tool(backend: &TtsBackend) -> String {
    match backend {
        TtsBackend::System(s) => s.binary.to_string_lossy().to_string(),
        TtsBackend::Onnx(o) => format!(
            "chatterbox_onnx(model_dir={}, voice={}, ort={}, lm={})",
            o.config.model_dir.to_string_lossy(),
            o.voice_path.to_string_lossy(),
            o.runtime_path.to_string_lossy(),
            o.language_model_path.to_string_lossy()
        ),
    }
}

fn resolve_tts_backend(
    plan_backend: BackendKind,
    availability: &BackendAvailability,
    allow_fallback: bool,
) -> Result<Option<TtsBackend>> {
    if plan_backend == BackendKind::Procedural {
        return Ok(None);
    }

    let system = availability.system_tts.clone().map(TtsBackend::System);
    let onnx = availability.onnx_tts.clone().map(TtsBackend::Onnx);

    match plan_backend {
        BackendKind::Onnx => {
            if onnx.is_some() {
                return Ok(onnx);
            }
            if allow_fallback && system.is_some() {
                return Ok(system);
            }
            let reason = availability
                .onnx_tts_error
                .as_deref()
                .unwrap_or("onnx TTS is not configured");
            Err(anyhow!("onnx TTS backend requested but not available: {reason}"))
        }
        BackendKind::System => {
            if system.is_some() {
                return Ok(system);
            }
            if allow_fallback && onnx.is_some() {
                return Ok(onnx);
            }
            Err(anyhow!(
                "system TTS backend requested but not available (set system_tts_binary or use onnx/procedural)"
            ))
        }
        BackendKind::Procedural => Ok(None),
    }
}

fn synthesize_tts_mono(backend: &TtsBackend, text: &str) -> Result<(Vec<f32>, u32)> {
    match backend {
        TtsBackend::System(system) => synthesize_with_system_tts(system, text),
        TtsBackend::Onnx(onnx) => synthesize_with_onnx_tts(onnx, text),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ChatterboxCacheKey {
    model_dir: PathBuf,
    onnx_runtime: Option<PathBuf>,
    language_model_path: Option<PathBuf>,
    exaggeration_1000: i32,
    max_new_tokens: usize,
}

struct ChatterboxCacheEntry {
    key: ChatterboxCacheKey,
    engine: ChatterboxOnnx,
}

static CHATTERBOX_ENGINE: OnceLock<Mutex<Option<ChatterboxCacheEntry>>> = OnceLock::new();

fn synthesize_with_onnx_tts(onnx: &OnnxTtsAvailability, text: &str) -> Result<(Vec<f32>, u32)> {
    let key = ChatterboxCacheKey {
        model_dir: onnx.config.model_dir.clone(),
        onnx_runtime: onnx.config.onnx_runtime.clone(),
        language_model_path: onnx.config.language_model_path.clone(),
        exaggeration_1000: (onnx.config.exaggeration * 1000.0).round() as i32,
        max_new_tokens: onnx.config.max_new_tokens,
    };

    let cache = CHATTERBOX_ENGINE.get_or_init(|| Mutex::new(None));
    let mut guard = cache
        .lock()
        .map_err(|_| anyhow!("chatterbox onnx cache mutex was poisoned"))?;

    let needs_reload = guard.as_ref().map(|c| c.key != key).unwrap_or(true);
    if needs_reload {
        let load_start = Instant::now();
        let engine = ChatterboxOnnx::load(&onnx.config)?;
        logging::info(
            "onnx.engine.load",
            json!({
                "duration_ms": load_start.elapsed().as_millis(),
                "model_dir": onnx.config.model_dir.to_string_lossy(),
                "language_model": onnx
                    .config
                    .language_model_path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .and_then(|n| n.to_str()),
            }),
        );
        *guard = Some(ChatterboxCacheEntry { key, engine });
    }

    let entry = guard
        .as_mut()
        .ok_or_else(|| anyhow!("internal error: ONNX cache entry missing"))?;

    let synth_start = Instant::now();
    let wav = entry.engine.synthesize_mono(&onnx.config, text)?;
    logging::info(
        "onnx.engine.synthesize",
        json!({
            "duration_ms": synth_start.elapsed().as_millis(),
            "text_chars": text.chars().count(),
        }),
    );
    Ok((wav, crate::chatterbox_onnx::SAMPLE_RATE))
}

fn resolve_executable(spec: &str) -> Option<PathBuf> {
    let path = PathBuf::from(spec);
    if path.exists() {
        return Some(path);
    }
    which::which(spec).ok()
}

fn detect_system_tts(settings: &Settings) -> Option<SystemTtsAvailability> {
    let binary = resolve_executable(&settings.system_tts_binary)?;

    #[cfg(windows)]
    {
        return Some(SystemTtsAvailability {
            binary,
            kind: SystemTtsKind::PowerShellSapi,
        });
    }

    #[cfg(target_os = "macos")]
    {
        return Some(SystemTtsAvailability {
            binary,
            kind: SystemTtsKind::Say,
        });
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = binary;
        None
    }
}

fn detect_onnx_tts(settings: &Settings) -> (Option<OnnxTtsAvailability>, Option<String>) {
    let Some(mut config) = ChatterboxOnnxConfig::from_settings(settings) else {
        return (None, None);
    };

    if !config.model_dir.is_dir() {
        return (
            None,
            Some(format!(
                "onnx_model_dir {} does not exist or is not a directory",
                config.model_dir.to_string_lossy()
            )),
        );
    }

    let tokenizer_path = config.model_dir.join("tokenizer.json");
    if !tokenizer_path.is_file() {
        return (
            None,
            Some(format!(
                "missing tokenizer.json in onnx_model_dir {}",
                config.model_dir.to_string_lossy()
            )),
        );
    }

    let voice_path = config
        .voice_path
        .clone()
        .unwrap_or_else(|| config.model_dir.join("default_voice.wav"));
    if !voice_path.is_file() {
        return (
            None,
            Some(format!(
                "missing voice WAV at {} (set onnx_voice or provide default_voice.wav)",
                voice_path.to_string_lossy()
            )),
        );
    }

    let onnx_dir = config.model_dir.join("onnx");
    let speech_encoder = onnx_dir.join("speech_encoder.onnx");
    let embed_tokens = onnx_dir.join("embed_tokens.onnx");
    let conditional_decoder = onnx_dir.join("conditional_decoder.onnx");
    if !speech_encoder.is_file() {
        return (None, Some(format!("missing {}", speech_encoder.to_string_lossy())));
    }
    let speech_encoder_data = speech_encoder.with_extension("onnx_data");
    if !speech_encoder_data.is_file() {
        return (
            None,
            Some(format!(
                "missing {} (external weights for speech_encoder.onnx)",
                speech_encoder_data.to_string_lossy()
            )),
        );
    }
    if !embed_tokens.is_file() {
        return (None, Some(format!("missing {}", embed_tokens.to_string_lossy())));
    }
    let embed_tokens_data = embed_tokens.with_extension("onnx_data");
    if !embed_tokens_data.is_file() {
        return (
            None,
            Some(format!(
                "missing {} (external weights for embed_tokens.onnx)",
                embed_tokens_data.to_string_lossy()
            )),
        );
    }
    if !conditional_decoder.is_file() {
        return (
            None,
            Some(format!("missing {}", conditional_decoder.to_string_lossy())),
        );
    }
    let conditional_decoder_data = conditional_decoder.with_extension("onnx_data");
    if !conditional_decoder_data.is_file() {
        return (
            None,
            Some(format!(
                "missing {} (external weights for conditional_decoder.onnx)",
                conditional_decoder_data.to_string_lossy()
            )),
        );
    }

    let language_model_path = config.resolved_language_model_path();
    if !language_model_path.is_file() {
        return (
            None,
            Some(format!(
                "missing language model ONNX (looked for {})",
                language_model_path.to_string_lossy()
            )),
        );
    }
    let language_model_data = language_model_path.with_extension("onnx_data");
    if !language_model_data.is_file() {
        return (
            None,
            Some(format!(
                "missing {} (external weights for {})",
                language_model_data.to_string_lossy(),
                language_model_path.file_name().and_then(|n| n.to_str()).unwrap_or("language model")
            )),
        );
    }
    config.language_model_path = Some(language_model_path.clone());

    let runtime_path = crate::chatterbox_onnx::resolve_onnxruntime_dylib_path(
        config.onnx_runtime.as_deref(),
        Some(&config.model_dir),
    );

    if !runtime_path.is_file() {
        return (
            None,
            Some(format!(
                "missing ONNX Runtime dylib at {} (set onnx_runtime or ORT_DYLIB_PATH)",
                runtime_path.to_string_lossy()
            )),
        );
    }
    if config.onnx_runtime.is_none() {
        config.onnx_runtime = Some(runtime_path.clone());
    }

    let availability = OnnxTtsAvailability {
        config,
        voice_path,
        runtime_path,
        language_model_path,
    };
    (Some(availability), None)
}

fn synthesize_with_system_tts(
    system: &SystemTtsAvailability,
    text: &str,
) -> Result<(Vec<f32>, u32)> {
    match system.kind {
        #[cfg(windows)]
        SystemTtsKind::PowerShellSapi => synthesize_windows_sapi_powershell(&system.binary, text),
        #[cfg(target_os = "macos")]
        SystemTtsKind::Say => synthesize_macos_say(&system.binary, text),
    }
}

#[cfg(windows)]
fn synthesize_windows_sapi_powershell(powershell: &PathBuf, text: &str) -> Result<(Vec<f32>, u32)> {
    let dir = tempfile::tempdir()?;
    let wav_path = dir.path().join("soundtest_system.wav");
    let text_path = dir.path().join("soundtest_text.txt");
    std::fs::write(&text_path, text)?;

    let script = r#"
$ErrorActionPreference = 'Stop'
$wavPath  = $env:SOUNDTEST_WAV_PATH
$textPath = $env:SOUNDTEST_TEXT_PATH
if (-not $wavPath)  { throw "missing env SOUNDTEST_WAV_PATH" }
if (-not $textPath) { throw "missing env SOUNDTEST_TEXT_PATH" }
$text = Get-Content -LiteralPath $textPath -Raw -Encoding UTF8
$voice  = New-Object -ComObject SAPI.SpVoice
$stream = New-Object -ComObject SAPI.SpFileStream
$stream.Open($wavPath, 3, $true)
$voice.AudioOutputStream = $stream
$null = $voice.Speak($text)
$stream.Close()
"#;

    let output = Command::new(powershell)
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-Command")
        .arg(script)
        .env("SOUNDTEST_WAV_PATH", &wav_path)
        .env("SOUNDTEST_TEXT_PATH", &text_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run {}", powershell.to_string_lossy()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "system TTS (SAPI via PowerShell) failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let file = std::fs::File::open(&wav_path)?;
    decode_audio_to_mono_f32(file)
}

#[cfg(target_os = "macos")]
fn synthesize_macos_say(say: &PathBuf, text: &str) -> Result<(Vec<f32>, u32)> {
    let dir = tempfile::tempdir()?;
    let wav_path = dir.path().join("soundtest_system.wav");
    let text_path = dir.path().join("soundtest_text.txt");
    std::fs::write(&text_path, text)?;

    let detected_lang = language::decide_language_code(text);
    let voice_candidates = macos_say_voice_candidates(say, detected_lang);

    let mut last_err: Option<anyhow::Error> = None;
    for voice in &voice_candidates {
        let mut cmd = Command::new(say);
        if let Some(voice) = voice {
            cmd.arg("-v").arg(voice);
        }
        let output = cmd
            .arg("-f")
            .arg(&text_path)
            .arg("-o")
            .arg(&wav_path)
            .arg("--data-format=LEI16@22050")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .with_context(|| format!("failed to run {}", say.to_string_lossy()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            last_err = Some(anyhow!(
                "system TTS (say) failed{} (exit {}): {}",
                voice
                    .as_deref()
                    .map(|v| format!(" with voice {v:?}"))
                    .unwrap_or_default(),
                output.status.code().unwrap_or(-1),
                stderr.trim()
            ));
            continue;
        }

        let file = std::fs::File::open(&wav_path)?;
        let (samples, sample_rate) = decode_audio_to_mono_f32(file)?;
        if audio_looks_suspiciously_short(text, &samples, sample_rate) {
            let voice_desc = voice
                .as_deref()
                .map(|v| format!(" for voice {v:?}"))
                .unwrap_or_else(|| " for default voice".to_owned());
            last_err = Some(anyhow!(
                "system TTS (say) produced suspiciously short audio{voice_desc} (lang={}). \
This usually means the selected macOS voice can't speak the input language. \
Install a matching voice in macOS settings, or check available voices with `say -v '?'`.",
                detected_lang.unwrap_or("unknown")
            ));
            continue;
        }
        return Ok((samples, sample_rate));
    }

    Err(last_err.unwrap_or_else(|| anyhow!("system TTS (say) failed")))
}

#[cfg(target_os = "macos")]
#[derive(Debug, Clone)]
struct SayVoice {
    name: String,
    locale: String,
}

#[cfg(target_os = "macos")]
static MACOS_SAY_VOICES: OnceLock<Vec<SayVoice>> = OnceLock::new();

#[cfg(target_os = "macos")]
fn macos_say_voice_candidates(say: &PathBuf, lang: Option<&'static str>) -> Vec<Option<String>> {
    let mut out: Vec<Option<String>> = Vec::new();
    let Some(lang) = lang else {
        out.push(None);
        return out;
    };

    if lang != "en" {
        let voices = MACOS_SAY_VOICES.get_or_init(|| list_macos_say_voices(say).unwrap_or_default());
        let mut names = select_macos_say_voice_names(voices, lang);
        names.truncate(5);
        out.extend(names.into_iter().map(Some));
    }
    out.push(None);
    out
}

#[cfg(target_os = "macos")]
fn list_macos_say_voices(say: &PathBuf) -> Result<Vec<SayVoice>> {
    let output = Command::new(say)
        .arg("-v")
        .arg("?")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run {}", say.to_string_lossy()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "`say -v '?'` failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_macos_say_voice_list(&stdout))
}

#[cfg(target_os = "macos")]
fn parse_macos_say_voice_list(text: &str) -> Vec<SayVoice> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let left = line.split_once('#').map(|(l, _)| l).unwrap_or(line).trim();
        if left.is_empty() {
            continue;
        }

        let mut parts: Vec<&str> = left.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }
        let locale = parts.pop().unwrap().to_owned();
        let name = parts.join(" ");
        if name.is_empty() || locale.is_empty() {
            continue;
        }
        out.push(SayVoice { name, locale });
    }
    out
}

#[cfg(target_os = "macos")]
fn select_macos_say_voice_names(voices: &[SayVoice], lang: &str) -> Vec<String> {
    let lang = lang.to_ascii_lowercase();
    let preferred_locales = preferred_macos_say_locales(&lang);

    let mut matches: Vec<&SayVoice> = voices
        .iter()
        .filter(|v| say_locale_base_lang(&v.locale) == lang)
        .collect();
    matches.sort_by(|a, b| compare_macos_say_voice(a, b, preferred_locales));

    let mut out = Vec::new();
    for voice in matches {
        if out.len() >= 8 {
            break;
        }
        if !out.iter().any(|v| v == &voice.name) {
            out.push(voice.name.clone());
        }
    }
    out
}

#[cfg(target_os = "macos")]
fn preferred_macos_say_locales(lang: &str) -> &'static [&'static str] {
    match lang {
        "zh" => &["zh_CN", "zh_TW", "zh_HK"],
        "pt" => &["pt_BR", "pt_PT"],
        "es" => &["es_ES", "es_MX"],
        "fr" => &["fr_FR", "fr_CA"],
        "en" => &["en_US", "en_GB"],
        "ar" => &["ar_001"],
        "he" => &["he_IL"],
        "hi" => &["hi_IN"],
        "ja" => &["ja_JP"],
        "ko" => &["ko_KR"],
        "ru" => &["ru_RU"],
        "th" => &["th_TH"],
        "el" => &["el_GR"],
        _ => &[],
    }
}

#[cfg(target_os = "macos")]
fn say_locale_base_lang(locale: &str) -> String {
    locale
        .split(|c| c == '_' || c == '-')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

#[cfg(target_os = "macos")]
fn compare_macos_say_voice(
    a: &SayVoice,
    b: &SayVoice,
    preferred_locales: &[&str],
) -> std::cmp::Ordering {
    let a_locale_rank = locale_pref_rank(&a.locale, preferred_locales);
    let b_locale_rank = locale_pref_rank(&b.locale, preferred_locales);

    a_locale_rank
        .cmp(&b_locale_rank)
        .then_with(|| voice_name_penalty(&a.name).cmp(&voice_name_penalty(&b.name)))
        .then_with(|| a.locale.cmp(&b.locale))
        .then_with(|| a.name.cmp(&b.name))
}

#[cfg(target_os = "macos")]
fn locale_pref_rank(locale: &str, preferred_locales: &[&str]) -> usize {
    preferred_locales
        .iter()
        .position(|p| p.eq_ignore_ascii_case(locale))
        .unwrap_or(usize::MAX)
}

#[cfg(target_os = "macos")]
fn voice_name_penalty(name: &str) -> (u8, u8) {
    let has_paren = u8::from(name.contains('(') || name.contains(')'));
    let has_space = u8::from(name.contains(' '));
    (has_paren, has_space)
}

#[cfg(target_os = "macos")]
fn audio_looks_suspiciously_short(text: &str, samples: &[f32], sample_rate: u32) -> bool {
    if samples.is_empty() || sample_rate == 0 {
        return true;
    }
    if text.trim().is_empty() {
        return true;
    }
    let chars = text.chars().filter(|c| c.is_alphanumeric()).count();
    if chars <= 1 {
        return false;
    }

    // If text turns into <100ms of audio, it's usually the wrong voice.
    // (E.g. default en_US voice on macOS often yields a tiny silent-ish clip for many scripts.)
    samples.len() < (sample_rate as usize / 10)
}

fn decode_audio_to_mono_f32<R>(reader: R) -> Result<(Vec<f32>, u32)>
where
    R: Read + Seek + Send + Sync + 'static,
{
    let decoder = rodio::Decoder::new(BufReader::new(reader))?;
    let sample_rate = decoder.sample_rate();
    let channels = decoder.channels() as usize;
    let samples: Vec<f32> = decoder.convert_samples().collect();

    if channels <= 1 {
        return Ok((samples, sample_rate));
    }

    let frames = samples.len() / channels;
    let mut mono = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut sum = 0.0f32;
        for ch in 0..channels {
            sum += samples[frame * channels + ch];
        }
        mono.push(sum / channels as f32);
    }

    Ok((mono, sample_rate))
}

pub struct AudioOut {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    volume: f32,
}

impl AudioOut {
    pub fn new(volume: f32) -> Result<Self> {
        let (_stream, handle) = rodio::OutputStream::try_default()?;
        Ok(Self {
            _stream,
            handle,
            volume: volume.clamp(0.0, 1.0),
        })
    }

    pub fn play<S>(&self, source: S) -> Result<()>
    where
        S: Source + Send + 'static,
        S::Item: rodio::Sample + Send,
        f32: rodio::cpal::FromSample<S::Item>,
    {
        let sink = rodio::Sink::try_new(&self.handle)?;
        sink.set_volume(self.volume);
        sink.append(source);
        sink.sleep_until_end();
        Ok(())
    }
}
