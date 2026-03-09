use bevy::prelude::Resource;
use soundtest::cli::{BackendChoice, Cli, Command, SpeakArgs};
use soundtest::config;
use soundtest::effects::EffectSpec;
use soundtest::render_plan::{BackendKind, RenderPlan};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MetaSpeakVoice {
    Dog,
    Cow,
    Dragon,
}

impl MetaSpeakVoice {
    pub const fn all() -> [Self; 3] {
        [Self::Dog, Self::Cow, Self::Dragon]
    }

    pub const fn id_str(self) -> &'static str {
        match self {
            Self::Dog => "dog",
            Self::Cow => "cow",
            Self::Dragon => "dragon",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Dog => "Dog",
            Self::Cow => "Cow",
            Self::Dragon => "Dragon",
        }
    }

    pub fn effect_spec(self) -> EffectSpec {
        match self {
            Self::Dog => EffectSpec {
                preset: "neutral".to_string(),
                amount: 1.0,
                speed: Some(1.08),
                pitch_semitones: Some(2.5),
                bass_db: Some(-1.0),
                treble_db: Some(2.0),
                reverb: Some(0.03),
                distortion: Some(0.02),
            },
            Self::Cow => EffectSpec {
                preset: "giant".to_string(),
                amount: 0.7,
                speed: Some(0.82),
                pitch_semitones: Some(-6.0),
                bass_db: Some(5.0),
                treble_db: Some(-2.0),
                reverb: Some(0.08),
                distortion: Some(0.05),
            },
            Self::Dragon => EffectSpec {
                preset: "dragon".to_string(),
                amount: 1.0,
                speed: None,
                pitch_semitones: None,
                bass_db: None,
                treble_db: None,
                reverb: None,
                distortion: None,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetaSpeakRequest {
    pub voice: MetaSpeakVoice,
    pub content: String,
    pub volume: f32,
}

#[derive(Debug, Clone)]
pub struct MetaSpeakOutcome {
    pub backend: String,
}

pub trait MetaSpeakAdapter: Send + Sync {
    fn speak(&self, request: MetaSpeakRequest) -> Result<MetaSpeakOutcome, String>;
}

#[derive(Default)]
pub struct SoundtestMetaSpeakAdapter;

impl SoundtestMetaSpeakAdapter {
    fn build_cli() -> Cli {
        Cli {
            config: None,
            base_url: None,
            token: None,
            model: None,
            reasoning_effort: None,
            no_ai: true,
            onnx_model_dir: None,
            onnx_voice: None,
            onnx_language_model: None,
            onnx_runtime: None,
            speak: Vec::new(),
            backend: BackendChoice::Auto,
            preset: None,
            amount: None,
            speed: None,
            pitch_semitones: None,
            bass_db: None,
            treble_db: None,
            reverb: None,
            distortion: None,
            dry_run: false,
            verbose: false,
            volume: 100,
            ai_concurrency: 1,
            tts_concurrency: 1,
            dsp_concurrency: 1,
            command: Some(Command::Speak(SpeakArgs {
                object: "meta".to_string(),
                text: vec!["meta".to_string()],
            })),
        }
    }

    fn choose_tts_backend(has_onnx: bool, has_system: bool) -> Option<BackendKind> {
        if has_onnx {
            return Some(BackendKind::Onnx);
        }
        if has_system {
            return Some(BackendKind::System);
        }
        None
    }

    pub fn choose_tts_backend_for_tests(has_onnx: bool, has_system: bool) -> Option<BackendKind> {
        Self::choose_tts_backend(has_onnx, has_system)
    }
}

impl MetaSpeakAdapter for SoundtestMetaSpeakAdapter {
    fn speak(&self, request: MetaSpeakRequest) -> Result<MetaSpeakOutcome, String> {
        let content = request.content.trim();
        if content.is_empty() {
            return Err("Speak content is empty.".to_string());
        }

        let cli = Self::build_cli();
        let settings = config::load_settings(&cli).map_err(|err| err.to_string())?;
        let availability = soundtest::backend::BackendAvailability::detect(&settings);
        let Some(backend_kind) = Self::choose_tts_backend(
            availability.onnx_tts.is_some(),
            availability.system_tts.is_some(),
        ) else {
            let onnx_reason = availability
                .onnx_tts_error
                .as_deref()
                .unwrap_or("onnx not configured")
                .to_string();
            return Err(format!(
                "No TTS backend available (need ONNX or system TTS). ONNX: {onnx_reason}"
            ));
        };

        let plan = RenderPlan {
            backend: backend_kind,
            text: Some(content.to_string()),
            proc: None,
            effects: request.voice.effect_spec(),
            raw: format!("backend: {:?}\\ntext: {content}", backend_kind),
        };

        bevy::tasks::block_on(soundtest::backend::execute_render_plan(
            &settings,
            &plan,
            BackendChoice::Auto,
            false,
            request.volume.clamp(0.0, 1.0),
        ))
        .map_err(|err| err.to_string())?;

        let backend = match backend_kind {
            BackendKind::Onnx => "onnx",
            BackendKind::System => "system",
            BackendKind::Procedural => "procedural",
        }
        .to_string();

        Ok(MetaSpeakOutcome { backend })
    }
}

#[derive(Resource, Clone)]
pub struct MetaSpeakRuntime {
    adapter: Arc<dyn MetaSpeakAdapter>,
}

impl Default for MetaSpeakRuntime {
    fn default() -> Self {
        Self {
            adapter: Arc::new(SoundtestMetaSpeakAdapter),
        }
    }
}

impl MetaSpeakRuntime {
    pub fn adapter(&self) -> Arc<dyn MetaSpeakAdapter> {
        self.adapter.clone()
    }
}
