use crate::language;
use crate::logging;
use anyhow::{Context, Result, anyhow};
use ndarray::{Array1, Array2, Array4, Axis};
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::Tensor;
use rodio::Source;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;
use tokenizers::Tokenizer;

pub const SAMPLE_RATE: u32 = 24_000;

const START_SPEECH_TOKEN: i64 = 6561;
const STOP_SPEECH_TOKEN: i64 = 6562;

const NUM_HIDDEN_LAYERS: usize = 30;
const NUM_KEY_VALUE_HEADS: usize = 16;
const HEAD_DIM: usize = 64;

const DEFAULT_MAX_NEW_TOKENS: usize = 1024;
const DEFAULT_EXAGGERATION: f32 = 0.5;
const REPETITION_PENALTY: f32 = 1.2;

#[derive(Debug, Clone)]
pub struct ChatterboxOnnxConfig {
    pub model_dir: PathBuf,
    pub voice_path: Option<PathBuf>,
    pub onnx_runtime: Option<PathBuf>,
    pub language_model_path: Option<PathBuf>,
    pub exaggeration: f32,
    pub max_new_tokens: usize,
}

impl ChatterboxOnnxConfig {
    pub fn from_settings(settings: &crate::config::Settings) -> Option<Self> {
        let model_dir = settings.onnx_model_dir.clone()?;
        Some(Self {
            model_dir,
            voice_path: settings.onnx_voice.clone(),
            onnx_runtime: settings.onnx_runtime.clone(),
            language_model_path: settings.onnx_language_model.clone(),
            exaggeration: DEFAULT_EXAGGERATION,
            max_new_tokens: DEFAULT_MAX_NEW_TOKENS,
        })
    }

    pub(crate) fn resolved_voice_path(&self) -> PathBuf {
        self.voice_path
            .clone()
            .unwrap_or_else(|| self.model_dir.join("default_voice.wav"))
    }

    pub(crate) fn resolved_language_model_path(&self) -> PathBuf {
        self.language_model_path
            .clone()
            .unwrap_or_else(|| pick_language_model_path(&self.model_dir))
    }
}

pub struct ChatterboxOnnx {
    tokenizer: Tokenizer,
    speech_encoder: Session,
    embed_tokens: Session,
    language_model: Session,
    conditional_decoder: Session,
    past_input_names: Vec<String>,
    exaggeration: f32,
    max_new_tokens: usize,
}

#[derive(Debug, Clone)]
pub struct OnnxTtsTimings {
    pub voice_decode_ms: u128,
    pub voice_resample_ms: u128,
    pub tokenizer_ms: u128,
    pub embed_prompt_ms: u128,
    pub speech_encoder_ms: u128,
    pub language_model_ms: u128,
    pub language_model_steps: usize,
    pub embed_step_ms: u128,
    pub conditional_decoder_ms: u128,
    pub total_ms: u128,

    pub input_chars: usize,
    pub input_tokens: usize,
    pub speech_tokens: usize,
    pub stop_token_hit: bool,

    pub output_samples: usize,
    pub output_seconds: f64,
    pub realtime_factor: f64,
}

impl ChatterboxOnnx {
    pub fn load(cfg: &ChatterboxOnnxConfig) -> Result<Self> {
        ensure_ort_ready(cfg.onnx_runtime.as_deref(), Some(&cfg.model_dir))?;

        let tokenizer_path = cfg.model_dir.join("tokenizer.json");
        let tokenizer = Tokenizer::from_file(&tokenizer_path).map_err(|err| {
            anyhow!(
                "failed to load tokenizer from {} (expected tokenizer.json in the model dir): {err}",
                tokenizer_path.to_string_lossy()
            )
        })?;

        let speech_encoder_path = cfg.model_dir.join("onnx").join("speech_encoder.onnx");
        let embed_tokens_path = cfg.model_dir.join("onnx").join("embed_tokens.onnx");
        let conditional_decoder_path = cfg.model_dir.join("onnx").join("conditional_decoder.onnx");
        let language_model_path = cfg.resolved_language_model_path();

        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4);

        let speech_encoder = load_session(&speech_encoder_path, threads)
            .with_context(|| format!("failed to load {}", speech_encoder_path.to_string_lossy()))?;
        let embed_tokens = load_session(&embed_tokens_path, threads)
            .with_context(|| format!("failed to load {}", embed_tokens_path.to_string_lossy()))?;
        let conditional_decoder = load_session(&conditional_decoder_path, threads).with_context(
            || format!("failed to load {}", conditional_decoder_path.to_string_lossy()),
        )?;
        let language_model = load_session(&language_model_path, threads)
            .with_context(|| format!("failed to load {}", language_model_path.to_string_lossy()))?;

        let past_input_names = build_past_input_names();

        Ok(Self {
            tokenizer,
            speech_encoder,
            embed_tokens,
            language_model,
            conditional_decoder,
            past_input_names,
            exaggeration: cfg.exaggeration,
            max_new_tokens: cfg.max_new_tokens,
        })
    }

    pub fn synthesize_mono(&mut self, cfg: &ChatterboxOnnxConfig, text: &str) -> Result<Vec<f32>> {
        let (wav, _timings) = self.synthesize_mono_with_timings(cfg, text)?;
        Ok(wav)
    }

    pub fn synthesize_mono_with_timings(
        &mut self,
        cfg: &ChatterboxOnnxConfig,
        text: &str,
    ) -> Result<(Vec<f32>, OnnxTtsTimings)> {
        let total_start = Instant::now();

        let voice_path = cfg.resolved_voice_path();
        let voice_decode_start = Instant::now();
        let (mut voice_samples, voice_sr) =
            load_audio_mono_f32(&voice_path).with_context(|| {
                format!("failed to load ONNX voice audio from {}", voice_path.to_string_lossy())
            })?;
        let voice_decode_ms = voice_decode_start.elapsed().as_millis();

        let voice_resample_start = Instant::now();
        if voice_sr != SAMPLE_RATE {
            let factor = voice_sr as f32 / SAMPLE_RATE as f32;
            voice_samples = resample_linear_mono(&voice_samples, factor);
        }
        let voice_resample_ms = voice_resample_start.elapsed().as_millis();

        let (wav, mut timings) = self.synthesize_from_voice_samples_with_timings(&voice_samples, text)?;
        timings.voice_decode_ms = voice_decode_ms;
        timings.voice_resample_ms = voice_resample_ms;
        timings.total_ms = total_start.elapsed().as_millis();

        let output_seconds = wav.len() as f64 / SAMPLE_RATE as f64;
        timings.output_samples = wav.len();
        timings.output_seconds = output_seconds;
        timings.realtime_factor = if output_seconds > 0.0 {
            (timings.total_ms as f64) / (output_seconds * 1000.0)
        } else {
            f64::INFINITY
        };

        logging::info(
            "onnx.tts.timings",
            json!({
                "voice_decode_ms": timings.voice_decode_ms,
                "voice_resample_ms": timings.voice_resample_ms,
                "tokenizer_ms": timings.tokenizer_ms,
                "embed_prompt_ms": timings.embed_prompt_ms,
                "speech_encoder_ms": timings.speech_encoder_ms,
                "language_model_ms": timings.language_model_ms,
                "language_model_steps": timings.language_model_steps,
                "embed_step_ms": timings.embed_step_ms,
                "conditional_decoder_ms": timings.conditional_decoder_ms,
                "total_ms": timings.total_ms,
                "input_chars": timings.input_chars,
                "input_tokens": timings.input_tokens,
                "speech_tokens": timings.speech_tokens,
                "stop_token_hit": timings.stop_token_hit,
                "output_samples": timings.output_samples,
                "output_seconds": timings.output_seconds,
                "realtime_factor": timings.realtime_factor,
                "voice_path": voice_path.to_string_lossy(),
            }),
        );

        if std::env::var("SOUNDTEST_ONNX_TIMINGS").ok().as_deref() == Some("1") {
            eprintln!(
                "onnx timings: total={}ms rtf={:.1} output={:.2}s lm={}ms (steps={}, {:.1}ms/step) speech_encoder={}ms decoder={}ms",
                timings.total_ms,
                timings.realtime_factor,
                timings.output_seconds,
                timings.language_model_ms,
                timings.language_model_steps,
                if timings.language_model_steps > 0 {
                    timings.language_model_ms as f64 / timings.language_model_steps as f64
                } else {
                    0.0
                },
                timings.speech_encoder_ms,
                timings.conditional_decoder_ms,
            );
        }

        Ok((wav, timings))
    }

    fn synthesize_from_voice_samples_with_timings(
        &mut self,
        voice: &[f32],
        text: &str,
    ) -> Result<(Vec<f32>, OnnxTtsTimings)> {
        let tokenizer_ms: u128;
        let embed_prompt_ms: u128;
        let speech_encoder_ms: u128;
        let mut language_model_ms: u128 = 0;
        let mut embed_step_ms: u128 = 0;
        let conditional_decoder_ms: u128;

        let text = text.trim();
        if text.is_empty() {
            return Err(anyhow!("TTS text was empty"));
        }

        let lang = language::decide_language_code(text).unwrap_or("en");
        let prepared_text = prepare_language(lang, text);
        let input_chars = prepared_text.chars().count();

        let tokenizer_start = Instant::now();
        let encoding = self
            .tokenizer
            .encode(prepared_text, true)
            .map_err(|err| anyhow!("tokenizer.encode failed: {err}"))?;
        tokenizer_ms = tokenizer_start.elapsed().as_millis();
        let ids_u32 = encoding.get_ids();
        if ids_u32.is_empty() {
            return Err(anyhow!("tokenizer produced no tokens"));
        }

        let input_ids: Vec<i64> = ids_u32.iter().map(|&id| id as i64).collect();
        let position_ids = build_position_ids(&input_ids);

        let input_ids_arr = Array2::<i64>::from_shape_vec((1, input_ids.len()), input_ids)?;
        let position_ids_arr = Array2::<i64>::from_shape_vec((1, position_ids.len()), position_ids)?;
        let exaggeration_arr = Array1::<f32>::from_vec(vec![self.exaggeration]);

        let input_embeds = {
            let embed_prompt_start = Instant::now();
            let embed_outputs = self.embed_tokens.run(ort::inputs! {
                "input_ids" => Tensor::from_array(input_ids_arr)?,
                "position_ids" => Tensor::from_array(position_ids_arr)?,
                "exaggeration" => Tensor::from_array(exaggeration_arr)?,
            })?;
            embed_prompt_ms = embed_prompt_start.elapsed().as_millis();
            embed_outputs[0]
                .try_extract_array::<f32>()
                .context("embed_tokens output[0] was not a float tensor")?
                .into_dimensionality::<ndarray::Ix3>()
                .context("embed_tokens output[0] had an unexpected shape")?
                .to_owned()
        };

        let audio_values = Array2::<f32>::from_shape_vec((1, voice.len()), voice.to_vec())?;
        let speech_encoder_start = Instant::now();
        let speech_outputs = self.speech_encoder.run(ort::inputs! {
            "audio_values" => Tensor::from_array(audio_values)?,
        })?;
        speech_encoder_ms = speech_encoder_start.elapsed().as_millis();

        let mut it = speech_outputs.into_iter();
        let cond_emb = it
            .next()
            .ok_or_else(|| anyhow!("speech_encoder missing cond_emb output"))?
            .1;
        let prompt_token = it
            .next()
            .ok_or_else(|| anyhow!("speech_encoder missing prompt_token output"))?
            .1;
        let speaker_embeddings = it
            .next()
            .ok_or_else(|| anyhow!("speech_encoder missing speaker_embeddings output"))?
            .1;
        let speaker_features = it
            .next()
            .ok_or_else(|| anyhow!("speech_encoder missing speaker_features output"))?
            .1;

        let cond_emb = cond_emb
            .try_extract_array::<f32>()
            .context("speech_encoder cond_emb was not a float tensor")?
            .into_dimensionality::<ndarray::Ix3>()
            .context("speech_encoder cond_emb had an unexpected shape")?
            .to_owned();

        let prompt_token_view = prompt_token
            .try_extract_array::<i64>()
            .context("speech_encoder prompt_token was not an int64 tensor")?;
        let prompt_token_vec: Vec<i64> = prompt_token_view.iter().copied().collect();

        let inputs_embeds = ndarray::concatenate(Axis(1), &[cond_emb.view(), input_embeds.view()])
            .map_err(|err| anyhow!("failed to concatenate embeddings: {err}"))?;

        let (batch, seq_len, _hidden) = inputs_embeds.dim();
        if batch != 1 {
            return Err(anyhow!(
                "unexpected batch size {batch} (expected 1) from ONNX model"
            ));
        }

        let mut attention_mask_vec: Vec<i64> = vec![1; seq_len];
        let mut attention_mask =
            Array2::<i64>::from_shape_vec((1, attention_mask_vec.len()), attention_mask_vec.clone())?;

        let mut generated: Vec<i64> = vec![START_SPEECH_TOKEN];
        let mut generated_set: HashSet<i64> = HashSet::new();
        generated_set.insert(START_SPEECH_TOKEN);

        // Initial (empty) KV cache.
        let mut past: Vec<ort::value::DynValue> = Vec::new();
        let empty_past: Array4<f32> =
            Array4::zeros((1, NUM_KEY_VALUE_HEADS, 0, HEAD_DIM));
        for _ in 0..(NUM_HIDDEN_LAYERS * 2) {
            // Use the ndarray path so we can represent a 0-length sequence dimension.
            past.push(Tensor::<f32>::from_array(empty_past.clone())?.into_dyn());
        }

        let mut next_inputs_embeds = inputs_embeds;

        for step in 0..self.max_new_tokens {
            let embeds_tensor = Tensor::<f32>::from_array(next_inputs_embeds)?;
            let mask_tensor = Tensor::<i64>::from_array(attention_mask)?;

            let mut lm_inputs = ort::inputs! {
                "inputs_embeds" => embeds_tensor,
                "attention_mask" => mask_tensor,
            };
            for (name, value) in self.past_input_names.iter().zip(past.iter()) {
                lm_inputs.push((name.as_str().into(), value.into()));
            }

            let lm_step_start = Instant::now();
            let lm_outputs = self.language_model.run(lm_inputs)?;
            language_model_ms += lm_step_start.elapsed().as_millis();

            let logits = lm_outputs[0]
                .try_extract_array::<f32>()
                .context("language_model logits was not a float tensor")?
                .into_dimensionality::<ndarray::Ix3>()
                .context("language_model logits had an unexpected shape")?;
            let logits_seq_len = logits.shape()[1];
            if logits_seq_len == 0 {
                return Err(anyhow!("language_model returned empty logits"));
            }
            let last_logits = logits.index_axis(Axis(1), logits_seq_len - 1);
            let last_logits = last_logits.index_axis(Axis(0), 0);

            let mut best_token: i64 = 0;
            let mut best_score: f32 = f32::NEG_INFINITY;
            for (token_idx, &score_raw) in last_logits.iter().enumerate() {
                let token_idx = token_idx as i64;
                let mut score = score_raw;
                if generated_set.contains(&token_idx) {
                    score = if score < 0.0 {
                        score * REPETITION_PENALTY
                    } else {
                        score / REPETITION_PENALTY
                    };
                }
                if score > best_score {
                    best_score = score;
                    best_token = token_idx;
                }
            }

            generated.push(best_token);
            generated_set.insert(best_token);

            // Past KV outputs are ordered: layer0.key, layer0.value, layer1.key, layer1.value, ...
            past = lm_outputs
                .into_iter()
                .skip(1)
                .map(|(_, v)| v)
                .collect::<Vec<_>>();

            if best_token == STOP_SPEECH_TOKEN {
                break;
            }

            let pos_id = (step + 1) as i64;
            let next_token_arr = Array2::<i64>::from_shape_vec((1, 1), vec![best_token])?;
            let pos_arr = Array2::<i64>::from_shape_vec((1, 1), vec![pos_id])?;
            let exaggeration_arr = Array1::<f32>::from_vec(vec![self.exaggeration]);

            let embed_step_start = Instant::now();
            let embed_outputs = self.embed_tokens.run(ort::inputs! {
                "input_ids" => Tensor::from_array(next_token_arr)?,
                "position_ids" => Tensor::from_array(pos_arr)?,
                "exaggeration" => Tensor::from_array(exaggeration_arr)?,
            })?;
            embed_step_ms += embed_step_start.elapsed().as_millis();
            let embed = embed_outputs[0]
                .try_extract_array::<f32>()
                .context("embed_tokens output[0] was not a float tensor")?
                .into_dimensionality::<ndarray::Ix3>()
                .context("embed_tokens output[0] had an unexpected shape")?
                .to_owned();
            next_inputs_embeds = embed;

            attention_mask_vec.push(1);
            attention_mask =
                Array2::<i64>::from_shape_vec((1, attention_mask_vec.len()), attention_mask_vec.clone())?;
        }

        let stop_token_hit = generated.last().copied() == Some(STOP_SPEECH_TOKEN);
        let language_model_steps = generated.len().saturating_sub(1);

        // Drop start token and trailing stop token (if present).
        let mut speech_tokens: Vec<i64> = generated.into_iter().skip(1).collect();
        if speech_tokens.last().copied() == Some(STOP_SPEECH_TOKEN) {
            speech_tokens.pop();
        }
        if speech_tokens.is_empty() {
            return Err(anyhow!("ONNX TTS produced no speech tokens"));
        }

        let mut final_tokens: Vec<i64> = Vec::with_capacity(prompt_token_vec.len() + speech_tokens.len());
        final_tokens.extend(prompt_token_vec);
        final_tokens.extend(speech_tokens);

        let decoder_token_count = final_tokens.len();
        let speech_tokens_arr =
            Array2::<i64>::from_shape_vec((1, decoder_token_count), final_tokens)?;
        let conditional_decoder_start = Instant::now();
        let decoder_outputs = self.conditional_decoder.run(ort::inputs! {
            "speech_tokens" => Tensor::from_array(speech_tokens_arr)?,
            "speaker_embeddings" => &speaker_embeddings,
            "speaker_features" => &speaker_features,
        })?;
        conditional_decoder_ms = conditional_decoder_start.elapsed().as_millis();

        let wav_view = decoder_outputs[0]
            .try_extract_array::<f32>()
            .context("conditional_decoder output[0] was not a float tensor")?;
        let wav: Vec<f32> = wav_view.iter().copied().collect();
        if wav.is_empty() {
            return Err(anyhow!("conditional_decoder produced empty audio"));
        }

        Ok((
            wav,
            OnnxTtsTimings {
                voice_decode_ms: 0,
                voice_resample_ms: 0,
                tokenizer_ms,
                embed_prompt_ms,
                speech_encoder_ms,
                language_model_ms,
                language_model_steps,
                embed_step_ms,
                conditional_decoder_ms,
                total_ms: 0,
                input_chars,
                input_tokens: ids_u32.len(),
                speech_tokens: decoder_token_count,
                stop_token_hit,
                output_samples: 0,
                output_seconds: 0.0,
                realtime_factor: 0.0,
            },
        ))
    }
}

fn ensure_ort_ready(runtime_path: Option<&Path>, model_dir: Option<&Path>) -> Result<()> {
    static ORT_READY: OnceLock<Result<(), String>> = OnceLock::new();
    let result = ORT_READY.get_or_init(|| {
        let path = resolve_onnxruntime_dylib_path(runtime_path, model_dir);
        let init = ort::init_from(&path)
            .map_err(|e| format!("failed to load ONNX Runtime from {}: {e}", path.display()))?;
        init.with_name("soundtest").with_telemetry(false).commit();
        Ok(())
    });
    match result {
        Ok(()) => Ok(()),
        Err(msg) => Err(anyhow!("{msg}")),
    }
}

pub(crate) fn resolve_onnxruntime_dylib_path(
    runtime_path: Option<&Path>,
    model_dir: Option<&Path>,
) -> PathBuf {
    if let Some(p) = runtime_path {
        return p.to_path_buf();
    }
    if let Ok(p) = std::env::var("ORT_DYLIB_PATH") {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }

    let default_name = default_onnxruntime_library_name();

    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(default_name);
            if candidate.is_file() {
                return candidate;
            }
        }
    }

    if let Some(model_dir) = model_dir {
        let candidate = model_dir.join(default_name);
        if candidate.is_file() {
            return candidate;
        }
    }

    PathBuf::from(default_name)
}

fn default_onnxruntime_library_name() -> &'static str {
    #[cfg(windows)]
    {
        return "onnxruntime.dll";
    }
    #[cfg(target_os = "macos")]
    {
        return "libonnxruntime.dylib";
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        return "libonnxruntime.so";
    }
}

fn load_session(path: &Path, threads: usize) -> Result<Session> {
    Session::builder()?
        .with_optimization_level(GraphOptimizationLevel::Level3)?
        .with_intra_threads(threads)?
        .commit_from_file(path)
        .map_err(|e| e.into())
}

fn pick_language_model_path(model_dir: &Path) -> PathBuf {
    let onnx_dir = model_dir.join("onnx");
    let mut candidates = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&onnx_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.starts_with("language_model") && name.ends_with(".onnx") {
                candidates.push(path);
            }
        }
    }

    fn rank(file_name: &str) -> usize {
        let n = file_name.to_ascii_lowercase();
        if n.contains("q4_0") {
            0
        } else if n.contains("q4_1") {
            1
        } else if n.contains("q4") {
            2
        } else if n.contains("int8") {
            3
        } else if n.contains("fp16") {
            4
        } else {
            10
        }
    }

    candidates.sort_by(|a, b| {
        let a_name = a.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let b_name = b.file_name().and_then(|n| n.to_str()).unwrap_or("");
        rank(a_name).cmp(&rank(b_name)).then_with(|| a_name.cmp(b_name))
    });
    candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| onnx_dir.join("language_model_fp16.onnx"))
}

fn build_past_input_names() -> Vec<String> {
    let mut out = Vec::with_capacity(NUM_HIDDEN_LAYERS * 2);
    for layer in 0..NUM_HIDDEN_LAYERS {
        out.push(format!("past_key_values.{layer}.key"));
        out.push(format!("past_key_values.{layer}.value"));
    }
    out
}

fn build_position_ids(input_ids: &[i64]) -> Vec<i64> {
    input_ids
        .iter()
        .enumerate()
        .map(|(i, &id)| if id >= START_SPEECH_TOKEN { 0 } else { i as i64 - 1 })
        .collect()
}

fn prepare_language(lang: &str, text: &str) -> String {
    let mut t = text.to_owned();
    if lang.eq_ignore_ascii_case("ko") {
        t = decompose_korean(&t);
    }
    format!("[{}]{}", lang, t)
}

fn load_audio_mono_f32(path: &Path) -> Result<(Vec<f32>, u32)> {
    let file = std::fs::File::open(path)?;
    let decoder = rodio::Decoder::new(std::io::BufReader::new(file))?;
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

fn resample_linear_mono(input: &[f32], factor: f32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    if factor <= 0.0 || (factor - 1.0).abs() < 1e-6 {
        return input.to_owned();
    }

    let out_len = ((input.len() as f32) / factor).ceil().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f32 * factor;
        let idx = pos.floor() as usize;
        let frac = pos - (idx as f32);
        let s0 = input.get(idx).copied().unwrap_or(0.0);
        let s1 = input.get(idx + 1).copied().unwrap_or(s0);
        out.push(s0 * (1.0 - frac) + s1 * frac);
    }
    out
}

fn decompose_korean(text: &str) -> String {
    const HANGUL_BASE: u32 = 0xAC00;
    const HANGUL_END: u32 = 0xD7A3;
    const CHOSEONG: [&str; 19] = [
        "ㄱ", "ㄲ", "ㄴ", "ㄷ", "ㄸ", "ㄹ", "ㅁ", "ㅂ", "ㅃ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅉ",
        "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ",
    ];
    const JUNGSEONG: [&str; 21] = [
        "ㅏ", "ㅐ", "ㅑ", "ㅒ", "ㅓ", "ㅔ", "ㅕ", "ㅖ", "ㅗ", "ㅘ", "ㅙ", "ㅚ", "ㅛ", "ㅜ",
        "ㅝ", "ㅞ", "ㅟ", "ㅠ", "ㅡ", "ㅢ", "ㅣ",
    ];
    const JONGSEONG: [&str; 28] = [
        "", "ㄱ", "ㄲ", "ㄳ", "ㄴ", "ㄵ", "ㄶ", "ㄷ", "ㄹ", "ㄺ", "ㄻ", "ㄼ", "ㄽ", "ㄾ",
        "ㄿ", "ㅀ", "ㅁ", "ㅂ", "ㅄ", "ㅅ", "ㅆ", "ㅇ", "ㅈ", "ㅊ", "ㅋ", "ㅌ", "ㅍ", "ㅎ",
    ];

    let mut out = String::new();
    for ch in text.chars() {
        let cp = ch as u32;
        if (HANGUL_BASE..=HANGUL_END).contains(&cp) {
            let s_index = cp - HANGUL_BASE;
            let cho = (s_index / (21 * 28)) as usize;
            let jung = ((s_index % (21 * 28)) / 28) as usize;
            let jong = (s_index % 28) as usize;
            out.push_str(CHOSEONG.get(cho).copied().unwrap_or(""));
            out.push_str(JUNGSEONG.get(jung).copied().unwrap_or(""));
            if let Some(tail) = JONGSEONG.get(jong).copied() {
                out.push_str(tail);
            }
        } else {
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_ids_match_reference_logic() {
        let ids = vec![10, 20, START_SPEECH_TOKEN, 30];
        assert_eq!(build_position_ids(&ids), vec![-1, 0, 0, 2]);
    }

    #[test]
    fn korean_decomposition_keeps_non_hangul() {
        let s = "ABC 123";
        assert_eq!(decompose_korean(s), s);
    }
}
