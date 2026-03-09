use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

fn default_dsp_concurrency() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

#[derive(Debug, Parser)]
#[command(
    name = "soundtest",
    version,
    about = "AI-driven object-voice TTS and procedural sounds"
)]
pub struct Cli {
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[arg(long, global = true)]
    pub base_url: Option<String>,

    #[arg(long, global = true)]
    pub token: Option<String>,

    #[arg(long, global = true)]
    pub model: Option<String>,

    #[arg(long, global = true)]
    pub reasoning_effort: Option<String>,

    /// Disable AI planning and run fully offline.
    ///
    /// - `--backend onnx|system`: speaks the provided text directly.
    /// - `--backend procedural`: treats the provided text as procedural token text.
    #[arg(long, global = true)]
    pub no_ai: bool,

    /// Path to a Chatterbox multilingual ONNX model directory (fully offline TTS).
    #[arg(long, global = true)]
    pub onnx_model_dir: Option<PathBuf>,

    /// Optional reference voice WAV for ONNX TTS (defaults to `default_voice.wav` in the model dir).
    #[arg(long, global = true)]
    pub onnx_voice: Option<PathBuf>,

    /// Optional override for the Chatterbox language model ONNX file.
    ///
    /// Example:
    /// - `--onnx-language-model models/chatterbox-multilingual-onnx/onnx/language_model_q4f16.onnx`
    #[arg(long, global = true)]
    pub onnx_language_model: Option<PathBuf>,

    /// Optional ONNX Runtime dynamic library path for the `ort` crate (`load-dynamic`).
    #[arg(long, global = true)]
    pub onnx_runtime: Option<PathBuf>,

    /// Batch mode: repeat `--speak <OBJECT> <MESSAGE>` to enqueue multiple sounds.
    #[arg(long, value_names = ["OBJECT", "MESSAGE"], num_args = 2)]
    pub speak: Vec<String>,

    #[arg(long, value_enum, default_value_t = BackendChoice::Auto, global = true)]
    pub backend: BackendChoice,

    /// Override the effect preset for TTS backends (onnx/system).
    #[arg(long, global = true)]
    pub preset: Option<String>,

    /// Override the preset mix amount (0.0-1.0).
    #[arg(long, global = true)]
    pub amount: Option<f32>,

    /// Override speaking speed (0.4-1.8).
    #[arg(long, global = true)]
    pub speed: Option<f32>,

    /// Override pitch shift in semitones (-24..24).
    #[arg(long, global = true)]
    pub pitch_semitones: Option<f32>,

    /// Override bass EQ in dB (-12..18).
    #[arg(long, global = true)]
    pub bass_db: Option<f32>,

    /// Override treble EQ in dB (-12..18).
    #[arg(long, global = true)]
    pub treble_db: Option<f32>,

    /// Override reverb amount (0.0-1.0).
    #[arg(long, global = true)]
    pub reverb: Option<f32>,

    /// Override distortion amount (0.0-1.0).
    #[arg(long, global = true)]
    pub distortion: Option<f32>,

    #[arg(long, global = true)]
    pub dry_run: bool,

    #[arg(long, global = true)]
    pub verbose: bool,

    /// Output volume (0-100).
    #[arg(
        long,
        default_value_t = 100,
        global = true,
        value_parser = clap::value_parser!(u8).range(0..=100)
    )]
    pub volume: u8,

    /// Max in-flight AI planning calls when batching.
    #[arg(long, default_value_t = 8, global = true)]
    pub ai_concurrency: usize,

    /// Max in-flight TTS jobs when batching.
    #[arg(long, default_value_t = 2, global = true)]
    pub tts_concurrency: usize,

    /// Max in-flight DSP/procedural rendering jobs when batching.
    #[arg(long, default_value_t = default_dsp_concurrency(), global = true)]
    pub dsp_concurrency: usize,

    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Speak(SpeakArgs),
    /// Print detected backends and resolved paths.
    Doctor,
}

#[derive(Debug, Args)]
pub struct SpeakArgs {
    pub object: String,
    pub text: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendChoice {
    Auto,
    Onnx,
    System,
    Procedural,
}
