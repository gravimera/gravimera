use bevy::prelude::Resource;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_OVERRIDE_ENV: &str = "GRAVIMERA_CONFIG";

#[derive(Resource, Clone, Debug)]
pub(crate) struct AppConfig {
    pub(crate) openai: Option<OpenAiConfig>,
    pub(crate) gemini: Option<GeminiConfig>,
    pub(crate) claude: Option<ClaudeConfig>,
    pub(crate) log_path: Option<PathBuf>,
    pub(crate) log_level: bevy::log::Level,
    pub(crate) scene_dat_path: Option<PathBuf>,
    pub(crate) gen3d_cache_dir: Option<PathBuf>,
    pub(crate) gen3d_ai_service: Gen3dAiService,
    pub(crate) gen3d_orchestrator: Gen3dOrchestrator,
    pub(crate) intelligence_service_enabled: bool,
    pub(crate) intelligence_service_mode: IntelligenceServiceMode,
    pub(crate) intelligence_service_addr: Option<String>,
    pub(crate) intelligence_service_token: Option<String>,
    pub(crate) intelligence_service_debug_spawn_unit: bool,
    pub(crate) automation_enabled: bool,
    pub(crate) automation_bind: Option<String>,
    pub(crate) automation_token: Option<String>,
    pub(crate) automation_disable_local_input: bool,
    pub(crate) automation_pause_on_start: bool,
    pub(crate) refine_iterations: u32,
    pub(crate) gen3d_max_parallel_components: usize,
    pub(crate) gen3d_max_seconds: u64,
    pub(crate) gen3d_max_tokens: u64,
    pub(crate) gen3d_mock_delay_seconds: u64,
    pub(crate) gen3d_review_delta_rounds_max: u32,
    pub(crate) gen3d_no_progress_tries_max: u32,
    pub(crate) gen3d_inspection_steps_max: u32,
    pub(crate) gen3d_max_replans: u32,
    pub(crate) gen3d_max_regen_total: u32,
    pub(crate) gen3d_max_regen_per_component: u32,
    pub(crate) gen3d_save_pass_screenshots: bool,
    pub(crate) gen3d_review_appearance: bool,
    pub(crate) gen3d_require_structured_outputs: bool,
    pub(crate) gen3d_reasoning_effort_plan: String,
    pub(crate) gen3d_reasoning_effort_agent_step: String,
    pub(crate) gen3d_reasoning_effort_component: String,
    pub(crate) gen3d_reasoning_effort_review: String,
    pub(crate) gen3d_reasoning_effort_repair: String,
    pub(crate) loaded_from: Option<PathBuf>,
    pub(crate) errors: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum IntelligenceServiceMode {
    Disabled,
    #[default]
    Embedded,
    Sidecar,
}

impl IntelligenceServiceMode {
    pub(crate) fn enabled(&self) -> bool {
        !matches!(self, Self::Disabled)
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            openai: None,
            gemini: None,
            claude: None,
            log_path: Some(crate::paths::gravimera_dir().join("gravimera.log")),
            log_level: bevy::log::Level::INFO,
            scene_dat_path: None,
            gen3d_cache_dir: None,
            gen3d_ai_service: Gen3dAiService::OpenAi,
            gen3d_orchestrator: Gen3dOrchestrator::Agent,
            intelligence_service_enabled: false,
            intelligence_service_mode: IntelligenceServiceMode::Embedded,
            intelligence_service_addr: None,
            intelligence_service_token: None,
            intelligence_service_debug_spawn_unit: false,
            automation_enabled: false,
            automation_bind: None,
            automation_token: None,
            automation_disable_local_input: true,
            automation_pause_on_start: true,
            refine_iterations: 1,
            gen3d_max_parallel_components: 10,
            gen3d_max_seconds: 60 * 30,
            gen3d_max_tokens: 10_000_000,
            gen3d_mock_delay_seconds: 0,
            gen3d_review_delta_rounds_max: 2,
            gen3d_no_progress_tries_max: 3,
            gen3d_inspection_steps_max: 12,
            gen3d_max_replans: 1,
            gen3d_max_regen_total: 16,
            gen3d_max_regen_per_component: 2,
            gen3d_save_pass_screenshots: !cfg!(test),
            gen3d_review_appearance: false,
            gen3d_require_structured_outputs: true,
            gen3d_reasoning_effort_plan: "high".into(),
            gen3d_reasoning_effort_agent_step: "high".into(),
            gen3d_reasoning_effort_component: "high".into(),
            gen3d_reasoning_effort_review: "high".into(),
            gen3d_reasoning_effort_repair: "high".into(),
            loaded_from: None,
            errors: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OpenAiConfig {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) model_reasoning_effort: String,
    pub(crate) api_key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct GeminiConfig {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) api_key: String,
}

#[derive(Clone, Debug)]
pub(crate) struct ClaudeConfig {
    pub(crate) base_url: String,
    pub(crate) model: String,
    pub(crate) api_key: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Gen3dAiService {
    #[default]
    OpenAi,
    Gemini,
    Claude,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Gen3dOrchestrator {
    #[default]
    Agent,
    Pipeline,
}

pub(crate) fn default_config_path() -> PathBuf {
    crate::paths::default_config_path()
}

pub(crate) fn load_config_with_override(config_path: Option<&Path>) -> AppConfig {
    if let Some(path) = config_path {
        return load_config_from_path(path);
    }

    if let Ok(path) = std::env::var(CONFIG_OVERRIDE_ENV) {
        let path = path.trim();
        if !path.is_empty() {
            return load_config_from_path(Path::new(path));
        }
    }

    load_config_default()
}

fn load_config_default() -> AppConfig {
    let mut out = AppConfig::default();

    // Default: `~/.gravimera/config.toml` (override via `--config` or env `GRAVIMERA_CONFIG`).
    // Fallbacks:
    // - legacy: `config.toml` next to the running binary
    // - dev-friendly: `./config.toml` (when running via `cargo run`)
    let mut candidates = Vec::new();
    candidates.push(default_config_path());
    if let Some(path) = crate::paths::legacy_path_next_to_exe(CONFIG_FILE_NAME) {
        candidates.push(path);
    }
    candidates.push(PathBuf::from(CONFIG_FILE_NAME));

    let mut loaded_text = None;
    for path in candidates {
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                out.loaded_from = Some(path);
                loaded_text = Some(text);
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => out
                .errors
                .push(format!("Failed to read {}: {err}", path.display())),
        }
    }

    let Some(text) = loaded_text else {
        return out;
    };

    parse_config_text_into(&mut out, &text);

    out
}

fn load_config_from_path(path: &Path) -> AppConfig {
    let mut out = AppConfig::default();
    match std::fs::read_to_string(path) {
        Ok(text) => {
            out.loaded_from = Some(path.to_path_buf());
            parse_config_text_into(&mut out, &text);
        }
        Err(err) => out
            .errors
            .push(format!("Failed to read {}: {err}", path.display())),
    }
    out
}

fn parse_config_text_into(out: &mut AppConfig, text: &str) {
    parse_log_path_into_config(out, text);
    parse_log_level_into_config(out, text);
    parse_scene_dat_path_into_config(out, text);
    parse_gen3d_cache_dir_into_config(out, text);
    parse_gen3d_ai_service_into_config(out, text);
    parse_gen3d_orchestrator_into_config(out, text);
    parse_intelligence_service_enabled_into_config(out, text);
    parse_intelligence_service_mode_into_config(out, text);
    parse_intelligence_service_addr_into_config(out, text);
    parse_intelligence_service_token_into_config(out, text);
    parse_intelligence_service_debug_spawn_unit_into_config(out, text);
    parse_automation_enabled_into_config(out, text);
    parse_automation_bind_into_config(out, text);
    parse_automation_disable_local_input_into_config(out, text);
    parse_automation_pause_on_start_into_config(out, text);
    parse_automation_token_into_config(out, text);
    parse_refine_iterations_into_config(out, text);
    parse_gen3d_max_parallel_components_into_config(out, text);
    parse_gen3d_max_seconds_into_config(out, text);
    parse_gen3d_max_tokens_into_config(out, text);
    parse_gen3d_mock_delay_seconds_into_config(out, text);
    parse_gen3d_review_delta_rounds_max_into_config(out, text);
    parse_gen3d_no_progress_tries_max_into_config(out, text);
    parse_gen3d_inspection_steps_max_into_config(out, text);
    parse_gen3d_max_replans_into_config(out, text);
    parse_gen3d_max_regen_total_into_config(out, text);
    parse_gen3d_max_regen_per_component_into_config(out, text);
    parse_gen3d_save_pass_screenshots_into_config(out, text);
    parse_gen3d_review_appearance_into_config(out, text);
    parse_gen3d_require_structured_outputs_into_config(out, text);
    parse_gen3d_reasoning_effort_plan_into_config(out, text);
    parse_gen3d_reasoning_effort_agent_step_into_config(out, text);
    parse_gen3d_reasoning_effort_component_into_config(out, text);
    parse_gen3d_reasoning_effort_review_into_config(out, text);
    parse_gen3d_reasoning_effort_repair_into_config(out, text);
    populate_openai_config(out, text);
    populate_gemini_config(out, text);
    populate_claude_config(out, text);
}

fn populate_openai_config(out: &mut AppConfig, text: &str) {
    if !config_has_section(text, "openai") {
        if matches!(out.gen3d_ai_service, Gen3dAiService::OpenAi) {
            out.errors.push(
                "config.toml: missing [openai] section (required when [gen3d].ai_service = \"openai\")"
                    .into(),
            );
        }
        return;
    }

    match parse_openai_config(text) {
        Ok(mut cfg) => {
            let allow_empty_key_for_mock = cfg.base_url.trim().starts_with("mock://gen3d")
                && cfg!(any(test, debug_assertions));
            // Allow env override for convenience (keeps secrets out of files if desired).
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                if allow_empty_key_for_mock {
                    out.openai = Some(cfg);
                } else {
                    out.errors.push(
                        "config.toml: missing `openai.token` / `openai.OPENAI_API_KEY` (or env `OPENAI_API_KEY`)".into(),
                    );
                }
            } else {
                out.openai = Some(cfg);
            }
        }
        Err(err) => out.errors.push(err),
    }
}

fn populate_gemini_config(out: &mut AppConfig, text: &str) {
    if !config_has_section(text, "gemini") {
        if matches!(out.gen3d_ai_service, Gen3dAiService::Gemini) {
            out.errors.push(
                "config.toml: missing [gemini] section (required when [gen3d].ai_service = \"gemini\")"
                    .into(),
            );
        }
        return;
    }

    match parse_gemini_config(text) {
        Ok(mut cfg) => {
            let allow_empty_key_for_mock = cfg.base_url.trim().starts_with("mock://gen3d")
                && cfg!(any(test, debug_assertions));
            // Allow env override for convenience (keeps secrets out of files if desired).
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("X_GOOG_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("GEMINI_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                if allow_empty_key_for_mock {
                    out.gemini = Some(cfg);
                } else {
                    out.errors.push(
                        "config.toml: missing `gemini.token` / `gemini.X_GOOG_API_KEY` (or env `X_GOOG_API_KEY` / `GEMINI_API_KEY`)".into(),
                    );
                }
            } else {
                out.gemini = Some(cfg);
            }
        }
        Err(err) => out.errors.push(err),
    }
}

fn populate_claude_config(out: &mut AppConfig, text: &str) {
    if !config_has_section(text, "claude") {
        if matches!(out.gen3d_ai_service, Gen3dAiService::Claude) {
            out.errors.push(
                "config.toml: missing [claude] section (required when [gen3d].ai_service = \"claude\")"
                    .into(),
            );
        }
        return;
    }

    match parse_claude_config(text) {
        Ok(mut cfg) => {
            let allow_empty_key_for_mock = cfg.base_url.trim().starts_with("mock://gen3d")
                && cfg!(any(test, debug_assertions));
            // Allow env override for convenience (keeps secrets out of files if desired).
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("CLAUDE_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                if allow_empty_key_for_mock {
                    out.claude = Some(cfg);
                } else {
                    out.errors.push(
                        "config.toml: missing `claude.token` / `claude.ANTHROPIC_API_KEY` (or env `ANTHROPIC_API_KEY` / `CLAUDE_API_KEY`)".into(),
                    );
                }
            } else {
                out.claude = Some(cfg);
            }
        }
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_ai_service_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_ai_service(text) {
        Ok(Some(value)) => out.gen3d_ai_service = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_orchestrator_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_orchestrator(text) {
        Ok(Some(value)) => out.gen3d_orchestrator = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_log_path_into_config(out: &mut AppConfig, text: &str) {
    match parse_log_path(text) {
        Ok(Some(ParsedLogPath::Disabled)) => out.log_path = None,
        Ok(Some(ParsedLogPath::Path(path))) => {
            let resolved = if path.is_relative() {
                match out.loaded_from.as_ref().and_then(|p| p.parent()) {
                    Some(dir) => dir.join(&path),
                    None => path,
                }
            } else {
                path
            };
            out.log_path = Some(resolved);
        }
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_log_level_into_config(out: &mut AppConfig, text: &str) {
    match parse_log_level(text) {
        Ok(Some(value)) => out.log_level = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_scene_dat_path_into_config(out: &mut AppConfig, text: &str) {
    match parse_scene_dat_path(text) {
        Ok(Some(path)) => {
            let resolved = if path.is_relative() {
                match out.loaded_from.as_ref().and_then(|p| p.parent()) {
                    Some(dir) => dir.join(&path),
                    None => path,
                }
            } else {
                path
            };
            out.scene_dat_path = Some(resolved);
        }
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_cache_dir_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_cache_dir(text) {
        Ok(Some(path)) => {
            let resolved = if path.is_relative() {
                match out.loaded_from.as_ref().and_then(|p| p.parent()) {
                    Some(dir) => dir.join(&path),
                    None => path,
                }
            } else {
                path
            };
            out.gen3d_cache_dir = Some(resolved);
        }
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_automation_enabled_into_config(out: &mut AppConfig, text: &str) {
    match parse_automation_enabled(text) {
        Ok(Some(value)) => out.automation_enabled = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_automation_bind_into_config(out: &mut AppConfig, text: &str) {
    match parse_automation_bind(text) {
        Ok(Some(value)) => out.automation_bind = Some(value),
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_automation_disable_local_input_into_config(out: &mut AppConfig, text: &str) {
    match parse_automation_disable_local_input(text) {
        Ok(Some(value)) => out.automation_disable_local_input = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_automation_pause_on_start_into_config(out: &mut AppConfig, text: &str) {
    match parse_automation_pause_on_start(text) {
        Ok(Some(value)) => out.automation_pause_on_start = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_automation_token_into_config(out: &mut AppConfig, text: &str) {
    match parse_automation_token(text) {
        Ok(Some(value)) => out.automation_token = Some(value),
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_intelligence_service_enabled_into_config(out: &mut AppConfig, text: &str) {
    match parse_intelligence_service_enabled(text) {
        Ok(Some(value)) => out.intelligence_service_enabled = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_intelligence_service_mode_into_config(out: &mut AppConfig, text: &str) {
    match parse_intelligence_service_mode(text) {
        Ok(Some(value)) => out.intelligence_service_mode = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_intelligence_service_addr_into_config(out: &mut AppConfig, text: &str) {
    match parse_intelligence_service_addr(text) {
        Ok(Some(value)) => out.intelligence_service_addr = Some(value),
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_intelligence_service_token_into_config(out: &mut AppConfig, text: &str) {
    match parse_intelligence_service_token(text) {
        Ok(Some(value)) => out.intelligence_service_token = Some(value),
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_intelligence_service_debug_spawn_unit_into_config(out: &mut AppConfig, text: &str) {
    match parse_intelligence_service_debug_spawn_unit(text) {
        Ok(Some(value)) => out.intelligence_service_debug_spawn_unit = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_refine_iterations_into_config(out: &mut AppConfig, text: &str) {
    // Backwards-compat: also accept `gen3d_auto_refine_passes`.
    let legacy = parse_gen3d_auto_refine_passes(text)
        .ok()
        .flatten()
        .unwrap_or(out.refine_iterations);
    out.refine_iterations = legacy;

    match parse_refine_iterations(text) {
        Ok(Some(value)) => out.refine_iterations = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_parallel_components_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_parallel_components(text) {
        Ok(Some(value)) => out.gen3d_max_parallel_components = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_seconds_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_seconds(text) {
        Ok(Some(value)) => out.gen3d_max_seconds = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_tokens_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_tokens(text) {
        Ok(Some(value)) => out.gen3d_max_tokens = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_mock_delay_seconds_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_mock_delay_seconds(text) {
        Ok(Some(value)) => out.gen3d_mock_delay_seconds = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_review_delta_rounds_max_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_review_delta_rounds_max(text) {
        Ok(Some(value)) => out.gen3d_review_delta_rounds_max = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_no_progress_tries_max_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_no_progress_tries_max(text) {
        Ok(Some(value)) => out.gen3d_no_progress_tries_max = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_inspection_steps_max_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_inspection_steps_max(text) {
        Ok(Some(value)) => out.gen3d_inspection_steps_max = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_replans_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_replans(text) {
        Ok(Some(value)) => out.gen3d_max_replans = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_regen_total_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_regen_total(text) {
        Ok(Some(value)) => out.gen3d_max_regen_total = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_max_regen_per_component_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_max_regen_per_component(text) {
        Ok(Some(value)) => out.gen3d_max_regen_per_component = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_save_pass_screenshots_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_save_pass_screenshots(text) {
        Ok(Some(value)) => out.gen3d_save_pass_screenshots = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_review_appearance_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_review_appearance(text) {
        Ok(Some(value)) => out.gen3d_review_appearance = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_require_structured_outputs_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_require_structured_outputs(text) {
        Ok(Some(value)) => out.gen3d_require_structured_outputs = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_reasoning_effort_plan_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_reasoning_effort_plan(text) {
        Ok(Some(value)) => out.gen3d_reasoning_effort_plan = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_reasoning_effort_agent_step_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_reasoning_effort_agent_step(text) {
        Ok(Some(value)) => out.gen3d_reasoning_effort_agent_step = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_reasoning_effort_component_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_reasoning_effort_component(text) {
        Ok(Some(value)) => out.gen3d_reasoning_effort_component = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_reasoning_effort_review_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_reasoning_effort_review(text) {
        Ok(Some(value)) => out.gen3d_reasoning_effort_review = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_gen3d_reasoning_effort_repair_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_reasoning_effort_repair(text) {
        Ok(Some(value)) => out.gen3d_reasoning_effort_repair = value,
        Ok(None) => {}
        Err(err) => out.errors.push(err),
    }
}

fn parse_log_path(text: &str) -> Result<Option<ParsedLogPath>, String> {
    let mut section: Option<String> = None;
    let mut log_path: Option<ParsedLogPath> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "path" {
            continue;
        }

        // Only accept `[log].path`.
        if section.as_deref() != Some("log") {
            continue;
        }

        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string value for key `log.path` (example: [log]\\npath = \"./gravimera.log\")"
                )
            })?
        } else {
            // Be forgiving: accept unquoted path strings for `log.path`.
            value.to_string()
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            log_path = Some(ParsedLogPath::Disabled);
        } else {
            log_path = Some(ParsedLogPath::Path(expand_tilde_path(trimmed)));
        }
    }

    Ok(log_path)
}

fn parse_log_level(text: &str) -> Result<Option<bevy::log::Level>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bevy::log::Level> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "level" {
            continue;
        }

        // Only accept `[log].level`.
        if section.as_deref() != Some("log") {
            continue;
        }

        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string value for key `log.level` (example: [log]\\nlevel = \"info\")"
                )
            })?
        } else {
            // Be forgiving: accept unquoted strings.
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }

        out = match trimmed.to_ascii_lowercase().as_str() {
            "error" => Some(bevy::log::Level::ERROR),
            "warn" | "warning" => Some(bevy::log::Level::WARN),
            "info" => Some(bevy::log::Level::INFO),
            "debug" => Some(bevy::log::Level::DEBUG),
            "trace" => Some(bevy::log::Level::TRACE),
            _ => {
                return Err(format!(
                    "config.toml:{line_no}: invalid `log.level` value `{trimmed}` (expected: \"error\" | \"warn\" | \"info\" | \"debug\" | \"trace\")"
                ))
            }
        };
    }

    Ok(out)
}

enum ParsedLogPath {
    Disabled,
    Path(PathBuf),
}

fn parse_scene_dat_path(text: &str) -> Result<Option<PathBuf>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<PathBuf> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "scene_dat_path" {
            continue;
        }

        // Accept `scene_dat_path` at top-level, or under `[scene]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "scene" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string value for key `scene_dat_path` (example: scene_dat_path = \"./scene.dat\")"
                )
            })?
        } else {
            // Be forgiving: accept unquoted path strings.
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(expand_tilde_path(trimmed));
        }
    }

    Ok(out)
}

fn parse_gen3d_cache_dir(text: &str) -> Result<Option<PathBuf>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<PathBuf> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "gen3d_cache_dir" && key != "cache_dir" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string value for key `gen3d_cache_dir` (example: gen3d_cache_dir = \"./gen3d_cache\")"
                )
            })?
        } else {
            // Be forgiving: accept unquoted path strings.
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(expand_tilde_path(trimmed));
        }
    }

    Ok(out)
}

fn parse_gen3d_ai_service(text: &str) -> Result<Option<Gen3dAiService>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<Gen3dAiService> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "ai_service" && key != "gen3d_ai_service" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_string(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a quoted string value for `gen3d.ai_service` (example: [gen3d]\\nai_service = \"openai\")"
            )
        })?;
        let parsed = parsed.trim().to_ascii_lowercase();
        if parsed.is_empty() {
            out = None;
            continue;
        }

        out = Some(match parsed.as_str() {
            "openai" => Gen3dAiService::OpenAi,
            "gemini" => Gen3dAiService::Gemini,
            "claude" => Gen3dAiService::Claude,
            other => {
                return Err(format!(
                    "config.toml:{line_no}: unsupported `gen3d.ai_service` value {other:?} (expected \"openai\", \"gemini\", or \"claude\")"
                ));
            }
        });
    }

    Ok(out)
}

fn parse_gen3d_orchestrator(text: &str) -> Result<Option<Gen3dOrchestrator>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<Gen3dOrchestrator> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "orchestrator" && key != "gen3d_orchestrator" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_string(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a quoted string value for `gen3d.orchestrator` (example: [gen3d]\\norchestrator = \"pipeline\")"
            )
        })?;
        let parsed = parsed.trim().to_ascii_lowercase();
        if parsed.is_empty() {
            out = None;
            continue;
        }

        out = Some(match parsed.as_str() {
            "agent" => Gen3dOrchestrator::Agent,
            "pipeline" => Gen3dOrchestrator::Pipeline,
            other => {
                return Err(format!(
                    "config.toml:{line_no}: unsupported `gen3d.orchestrator` value {other:?} (expected \"agent\" or \"pipeline\")"
                ));
            }
        });
    }

    Ok(out)
}

fn parse_automation_enabled(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "automation_enabled"
            && !(section.as_deref() == Some("automation") && key == "enabled")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "automation" && sec != "app" && key == "automation_enabled" {
                continue;
            }
            if sec != "automation" && key == "enabled" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `automation.enabled` (example: [automation]\\nenabled = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_intelligence_service_enabled(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "intelligence_service_enabled"
            && !(section.as_deref() == Some("intelligence_service") && key == "enabled")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "intelligence_service"
                && sec != "app"
                && key == "intelligence_service_enabled"
            {
                continue;
            }
            if sec != "intelligence_service" && key == "enabled" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `intelligence_service.enabled` (example: [intelligence_service]\\nenabled = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_intelligence_service_mode(text: &str) -> Result<Option<IntelligenceServiceMode>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<IntelligenceServiceMode> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "intelligence_service_mode"
            && !(section.as_deref() == Some("intelligence_service") && key == "mode")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "intelligence_service" && sec != "app" && key == "intelligence_service_mode" {
                continue;
            }
            if sec != "intelligence_service" && key == "mode" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a string for `intelligence_service.mode` (example: [intelligence_service]\\nmode = \"embedded\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
            continue;
        }

        let mode = match trimmed.to_ascii_lowercase().as_str() {
            "disabled" | "off" | "none" => IntelligenceServiceMode::Disabled,
            "embedded" => IntelligenceServiceMode::Embedded,
            "sidecar" | "standalone" | "remote" | "external" => IntelligenceServiceMode::Sidecar,
            _ => {
                return Err(format!(
                    "config.toml:{line_no}: invalid `intelligence_service.mode` value `{trimmed}` (expected: \"embedded\" | \"sidecar\" | \"disabled\")"
                ));
            }
        };

        out = Some(mode);
    }

    Ok(out)
}

fn parse_intelligence_service_addr(text: &str) -> Result<Option<String>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<String> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "intelligence_service_addr"
            && !(section.as_deref() == Some("intelligence_service") && key == "addr")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "intelligence_service" && sec != "app" && key == "intelligence_service_addr" {
                continue;
            }
            if sec != "intelligence_service" && key == "addr" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a string for `intelligence_service.addr` (example: [intelligence_service]\\naddr = \"127.0.0.1:8792\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(trimmed.to_string());
        }
    }

    Ok(out)
}

fn parse_intelligence_service_token(text: &str) -> Result<Option<String>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<String> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "intelligence_service_token"
            && !(section.as_deref() == Some("intelligence_service") && key == "token")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "intelligence_service" && sec != "app" && key == "intelligence_service_token"
            {
                continue;
            }
            if sec != "intelligence_service" && key == "token" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a string for `intelligence_service.token` (example: [intelligence_service]\\ntoken = \"CHANGE_ME\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(trimmed.to_string());
        }
    }

    Ok(out)
}

fn parse_intelligence_service_debug_spawn_unit(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "intelligence_service_debug_spawn_unit"
            && !(section.as_deref() == Some("intelligence_service") && key == "debug_spawn_unit")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "intelligence_service"
                && sec != "app"
                && key == "intelligence_service_debug_spawn_unit"
            {
                continue;
            }
            if sec != "intelligence_service" && key == "debug_spawn_unit" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `intelligence_service.debug_spawn_unit` (example: [intelligence_service]\\ndebug_spawn_unit = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_automation_bind(text: &str) -> Result<Option<String>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<String> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "automation_bind" && !(section.as_deref() == Some("automation") && key == "bind")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "automation" && sec != "app" && key == "automation_bind" {
                continue;
            }
            if sec != "automation" && key == "bind" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a string for `automation.bind` (example: [automation]\\nbind = \"127.0.0.1:8791\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(trimmed.to_string());
        }
    }

    Ok(out)
}

fn parse_automation_disable_local_input(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "automation_disable_local_input"
            && !(section.as_deref() == Some("automation") && key == "disable_local_input")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "automation" && sec != "app" && key == "automation_disable_local_input" {
                continue;
            }
            if sec != "automation" && key == "disable_local_input" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `automation.disable_local_input` (example: [automation]\\ndisable_local_input = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_automation_pause_on_start(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "automation_pause_on_start"
            && !(section.as_deref() == Some("automation") && key == "pause_on_start")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "automation" && sec != "app" && key == "automation_pause_on_start" {
                continue;
            }
            if sec != "automation" && key == "pause_on_start" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `automation.pause_on_start` (example: [automation]\\npause_on_start = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_automation_token(text: &str) -> Result<Option<String>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<String> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "automation_token"
            && !(section.as_deref() == Some("automation") && key == "token")
        {
            continue;
        }

        if let Some(sec) = section.as_deref() {
            if sec != "automation" && sec != "app" && key == "automation_token" {
                continue;
            }
            if sec != "automation" && key == "token" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a string for `automation.token` (example: [automation]\\ntoken = \"secret\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
        } else {
            out = Some(trimmed.to_string());
        }
    }

    Ok(out)
}

fn parse_gen3d_max_parallel_components(text: &str) -> Result<Option<usize>, String> {
    const MAX_ALLOWED: usize = 64;

    let mut section: Option<String> = None;
    let mut out: Option<usize> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_parallel_components" && key != "gen3d_max_parallel_components" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_parallel_components` (example: max_parallel_components = 10)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_parallel_components` (example: max_parallel_components = 10)"
            )
        })?;
        if parsed <= 0 {
            return Err(format!(
                "config.toml:{line_no}: `max_parallel_components` must be >= 1"
            ));
        }

        out = Some((parsed as usize).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_max_seconds(text: &str) -> Result<Option<u64>, String> {
    const MAX_ALLOWED: u64 = 24 * 60 * 60;

    let mut section: Option<String> = None;
    let mut out: Option<u64> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_seconds" && key != "gen3d_max_seconds" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_seconds` (example: max_seconds = 1800)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i128 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_seconds` (example: max_seconds = 1800)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `max_seconds` must be >= 0 (0 disables the time budget)"
            ));
        }

        out = Some((parsed as u64).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_save_pass_screenshots(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "save_pass_screenshots" && key != "gen3d_save_pass_screenshots" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `gen3d.save_pass_screenshots` (example: [gen3d]\\nsave_pass_screenshots = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_gen3d_review_appearance(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "review_appearance" && key != "gen3d_review_appearance" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `gen3d.review_appearance` (example: [gen3d]\\nreview_appearance = false)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_gen3d_require_structured_outputs(text: &str) -> Result<Option<bool>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<bool> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "require_structured_outputs" && key != "gen3d_require_structured_outputs" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let parsed = parse_toml_bool(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a boolean for `gen3d.require_structured_outputs` (example: [gen3d]\\nrequire_structured_outputs = true)"
            )
        })?;
        out = Some(parsed);
    }

    Ok(out)
}

fn parse_gen3d_max_tokens(text: &str) -> Result<Option<u64>, String> {
    const MAX_ALLOWED: u64 = 100_000_000;

    let mut section: Option<String> = None;
    let mut out: Option<u64> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_tokens" && key != "gen3d_max_tokens" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_tokens` (example: max_tokens = 10000000)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i128 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_tokens` (example: max_tokens = 10000000)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `max_tokens` must be >= 0 (0 disables the token budget)"
            ));
        }

        out = Some((parsed as u64).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_mock_delay_seconds(text: &str) -> Result<Option<u64>, String> {
    const MAX_ALLOWED: u64 = 60 * 60;

    let mut section: Option<String> = None;
    let mut out: Option<u64> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "mock_delay_seconds" && key != "gen3d_mock_delay_seconds" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `mock_delay_seconds` (example: mock_delay_seconds = 60)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i128 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `mock_delay_seconds` (example: mock_delay_seconds = 60)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `mock_delay_seconds` must be >= 0"
            ));
        }

        out = Some((parsed as u64).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_review_delta_rounds_max(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 2;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "review_delta_rounds_max" && key != "gen3d_review_delta_rounds_max" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `review_delta_rounds_max` (example: review_delta_rounds_max = 2)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `review_delta_rounds_max` (example: review_delta_rounds_max = 2)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `review_delta_rounds_max` must be >= 0"
            ));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_no_progress_tries_max(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 10_000;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "no_progress_tries_max" && key != "gen3d_no_progress_tries_max" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `no_progress_tries_max` (example: no_progress_tries_max = 3)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `no_progress_tries_max` (example: no_progress_tries_max = 3)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `no_progress_tries_max` must be >= 0 (0 disables the guard)"
            ));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_inspection_steps_max(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 10_000;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "inspection_steps_max" && key != "gen3d_inspection_steps_max" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `inspection_steps_max` (example: inspection_steps_max = 12)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `inspection_steps_max` (example: inspection_steps_max = 12)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `inspection_steps_max` must be >= 0 (0 disables the guard)"
            ));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_max_replans(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 16;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_replans" && key != "gen3d_max_replans" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_replans` (example: max_replans = 1)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_replans` (example: max_replans = 1)"
            )
        })?;
        if parsed < 0 {
            return Err(format!("config.toml:{line_no}: `max_replans` must be >= 0"));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_max_regen_total(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 1024;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_regen_total" && key != "gen3d_max_regen_total" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_regen_total` (example: max_regen_total = 16)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_regen_total` (example: max_regen_total = 16)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `max_regen_total` must be >= 0"
            ));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_max_regen_per_component(text: &str) -> Result<Option<u32>, String> {
    const MAX_ALLOWED: u32 = 64;

    let mut section: Option<String> = None;
    let mut out: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "max_regen_per_component" && key != "gen3d_max_regen_per_component" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `max_regen_per_component` (example: max_regen_per_component = 2)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_regen_per_component` (example: max_regen_per_component = 2)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `max_regen_per_component` must be >= 0"
            ));
        }

        out = Some((parsed as u32).min(MAX_ALLOWED));
    }

    Ok(out)
}

fn parse_gen3d_reasoning_effort_value(
    text: &str,
    accepted_keys: &[&str],
    display_key: &str,
) -> Result<Option<String>, String> {
    let mut section: Option<String> = None;
    let mut out: Option<String> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !accepted_keys.iter().any(|k| k == &key) {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            out = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string for `{display_key}` (example: {display_key} = \"medium\")"
                )
            })?
        } else {
            value.to_string()
        };

        let trimmed = value.trim();
        if trimmed.is_empty() {
            out = None;
            continue;
        }

        let normalized = trimmed.to_ascii_lowercase();
        match normalized.as_str() {
            "none" | "low" | "medium" | "high" => out = Some(normalized),
            _ => {
                return Err(format!(
                    "config.toml:{line_no}: invalid `{display_key}` value `{trimmed}` (expected one of: \"none\", \"low\", \"medium\", \"high\")"
                ));
            }
        }
    }

    Ok(out)
}

fn parse_gen3d_reasoning_effort_plan(text: &str) -> Result<Option<String>, String> {
    parse_gen3d_reasoning_effort_value(
        text,
        &["reasoning_effort_plan", "gen3d_reasoning_effort_plan"],
        "reasoning_effort_plan",
    )
}

fn parse_gen3d_reasoning_effort_agent_step(text: &str) -> Result<Option<String>, String> {
    parse_gen3d_reasoning_effort_value(
        text,
        &[
            "reasoning_effort_agent_step",
            "gen3d_reasoning_effort_agent_step",
        ],
        "reasoning_effort_agent_step",
    )
}

fn parse_gen3d_reasoning_effort_component(text: &str) -> Result<Option<String>, String> {
    parse_gen3d_reasoning_effort_value(
        text,
        &[
            "reasoning_effort_component",
            "gen3d_reasoning_effort_component",
        ],
        "reasoning_effort_component",
    )
}

fn parse_gen3d_reasoning_effort_review(text: &str) -> Result<Option<String>, String> {
    parse_gen3d_reasoning_effort_value(
        text,
        &["reasoning_effort_review", "gen3d_reasoning_effort_review"],
        "reasoning_effort_review",
    )
}

fn parse_gen3d_reasoning_effort_repair(text: &str) -> Result<Option<String>, String> {
    parse_gen3d_reasoning_effort_value(
        text,
        &["reasoning_effort_repair", "gen3d_reasoning_effort_repair"],
        "reasoning_effort_repair",
    )
}

fn parse_gen3d_auto_refine_passes(text: &str) -> Result<Option<u32>, String> {
    let mut section: Option<String> = None;
    let mut passes: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "gen3d_auto_refine_passes" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            passes = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `gen3d_auto_refine_passes` (example: gen3d_auto_refine_passes = 1)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `gen3d_auto_refine_passes` (example: gen3d_auto_refine_passes = 1)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `gen3d_auto_refine_passes` must be >= 0"
            ));
        }
        passes = Some(parsed as u32);
    }

    Ok(passes)
}

fn parse_refine_iterations(text: &str) -> Result<Option<u32>, String> {
    let mut section: Option<String> = None;
    let mut passes: Option<u32> = None;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let name = line.trim_matches(&['[', ']'][..]).trim();
            section = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key != "refine_iterations" {
            continue;
        }

        // Accept at top-level, or under `[gen3d]` / `[app]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "gen3d" && sec != "app" {
                continue;
            }
        }

        let value = value.trim();
        if value.is_empty() {
            passes = None;
            continue;
        }

        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected an integer for `refine_iterations` (example: refine_iterations = 1)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `refine_iterations` (example: refine_iterations = 1)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `refine_iterations` must be >= 0"
            ));
        }
        passes = Some(parsed as u32);
    }

    Ok(passes)
}

fn expand_tilde_path(value: &str) -> PathBuf {
    crate::paths::expand_tilde_path(value)
}

fn config_has_section(text: &str, target: &str) -> bool {
    let target = target.trim();
    if target.is_empty() {
        return false;
    }

    for raw_line in text.lines() {
        let line = strip_comment(raw_line).trim();
        if !(line.starts_with('[') && line.ends_with(']')) {
            continue;
        }
        let section = line.trim_matches(&['[', ']'][..]).trim();
        if section == target {
            return true;
        }
    }

    false
}

fn parse_openai_config(text: &str) -> Result<OpenAiConfig, String> {
    let mut in_openai = false;
    let mut base_url = None::<String>;
    let mut model = None::<String>;
    let mut effort = None::<String>;
    let mut api_key = None::<String>;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_matches(&['[', ']'][..]).trim();
            in_openai = section == "openai";
            continue;
        }
        if !in_openai {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = parse_toml_string(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a quoted string value for key `{key}` (example: {key} = \"...\")"
            )
        })?;

        match key {
            "base_url" => base_url = Some(value),
            "model" => model = Some(value),
            "model_reasoning_effort" => effort = Some(value),
            "token" | "api_key" => api_key = Some(value),
            "OPENAI_API_KEY" => api_key = Some(value),
            _ => {}
        }
    }

    let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".into());
    let model = model.unwrap_or_else(|| "gpt-5.4".into());
    let effort = effort.unwrap_or_else(|| "high".into());
    let api_key = api_key.unwrap_or_default();

    Ok(OpenAiConfig {
        base_url,
        model,
        model_reasoning_effort: effort,
        api_key,
    })
}

fn parse_gemini_config(text: &str) -> Result<GeminiConfig, String> {
    let mut in_gemini = false;
    let mut base_url = None::<String>;
    let mut model = None::<String>;
    let mut api_key = None::<String>;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_matches(&['[', ']'][..]).trim();
            in_gemini = section == "gemini";
            continue;
        }
        if !in_gemini {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = parse_toml_string(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a quoted string value for key `{key}` (example: {key} = \"...\")"
            )
        })?;

        match key {
            "base_url" => base_url = Some(value),
            "model" => model = Some(value),
            "token" | "api_key" => api_key = Some(value),
            "X_GOOG_API_KEY" | "x_goog_api_key" | "GEMINI_API_KEY" => api_key = Some(value),
            _ => {}
        }
    }

    let base_url =
        base_url.unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".into());
    let model = model.unwrap_or_else(|| "gemini-3.1-pro-preview".into());
    let api_key = api_key.unwrap_or_default();

    Ok(GeminiConfig {
        base_url,
        model,
        api_key,
    })
}

fn parse_claude_config(text: &str) -> Result<ClaudeConfig, String> {
    let mut in_claude = false;
    let mut base_url = None::<String>;
    let mut model = None::<String>;
    let mut api_key = None::<String>;

    for (line_no, raw_line) in text.lines().enumerate() {
        let line_no = line_no + 1;
        let line = strip_comment(raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            let section = line.trim_matches(&['[', ']'][..]).trim();
            in_claude = section == "claude";
            continue;
        }
        if !in_claude {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();
        let value = parse_toml_string(value).ok_or_else(|| {
            format!(
                "config.toml:{line_no}: expected a quoted string value for key `{key}` (example: {key} = \"...\")"
            )
        })?;

        match key {
            "base_url" => base_url = Some(value),
            "model" => model = Some(value),
            "token" | "api_key" => api_key = Some(value),
            "ANTHROPIC_API_KEY" | "CLAUDE_API_KEY" => api_key = Some(value),
            _ => {}
        }
    }

    let base_url = base_url.unwrap_or_else(|| "https://api.anthropic.com/v1".into());
    let model = model.unwrap_or_else(|| "claude-opus-4-5".into());
    let api_key = api_key.unwrap_or_default();

    Ok(ClaudeConfig {
        base_url,
        model,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_gen3d_mock_delay_seconds;
    use super::parse_gen3d_require_structured_outputs;
    use super::parse_gen3d_review_appearance;
    use super::parse_gen3d_save_pass_screenshots;
    use super::{
        parse_config_text_into, parse_gen3d_orchestrator, parse_intelligence_service_mode,
        AppConfig, Gen3dAiService, Gen3dOrchestrator, IntelligenceServiceMode,
    };
    use std::path::PathBuf;

    #[test]
    fn parses_gen3d_save_pass_screenshots_from_gen3d_section() {
        let text = r#"
        [gen3d]
        save_pass_screenshots = true
        "#;
        assert_eq!(parse_gen3d_save_pass_screenshots(text).unwrap(), Some(true));
    }

    #[test]
    fn parses_gen3d_save_pass_screenshots_from_top_level() {
        let text = r#"
        gen3d_save_pass_screenshots = false
        "#;
        assert_eq!(
            parse_gen3d_save_pass_screenshots(text).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn parses_gen3d_review_appearance_from_gen3d_section() {
        let text = r#"
        [gen3d]
        review_appearance = true
        "#;
        assert_eq!(parse_gen3d_review_appearance(text).unwrap(), Some(true));
    }

    #[test]
    fn parses_gen3d_review_appearance_from_top_level() {
        let text = r#"
        gen3d_review_appearance = false
        "#;
        assert_eq!(parse_gen3d_review_appearance(text).unwrap(), Some(false));
    }

    #[test]
    fn parses_gen3d_require_structured_outputs_from_gen3d_section() {
        let text = r#"
        [gen3d]
        require_structured_outputs = false
        "#;
        assert_eq!(
            parse_gen3d_require_structured_outputs(text).unwrap(),
            Some(false)
        );
    }

    #[test]
    fn parses_gen3d_require_structured_outputs_from_top_level() {
        let text = r#"
        gen3d_require_structured_outputs = true
        "#;
        assert_eq!(
            parse_gen3d_require_structured_outputs(text).unwrap(),
            Some(true)
        );
    }

    #[test]
    fn parses_gen3d_mock_delay_seconds_from_gen3d_section() {
        let text = r#"
        [gen3d]
        mock_delay_seconds = 60
        "#;
        assert_eq!(parse_gen3d_mock_delay_seconds(text).unwrap(), Some(60));
    }

    #[test]
    fn parses_gen3d_mock_delay_seconds_from_top_level() {
        let text = r#"
        gen3d_mock_delay_seconds = 120
        "#;
        assert_eq!(parse_gen3d_mock_delay_seconds(text).unwrap(), Some(120));
    }

    #[test]
    fn parses_gen3d_ai_service_and_gemini_config() {
        let text = r#"
        [gen3d]
        ai_service = "gemini"

        [gemini]
        base_url = "mock://gen3d"
        model = "mock"
        token = "mock"
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.gen3d_ai_service, Gen3dAiService::Gemini);
        assert!(cfg.gemini.is_some());
        assert!(cfg.openai.is_none());
        assert!(cfg.claude.is_none());
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn parses_gen3d_orchestrator_from_gen3d_section() {
        let text = r#"
        [gen3d]
        orchestrator = "pipeline"
        "#;
        assert_eq!(
            parse_gen3d_orchestrator(text).unwrap(),
            Some(Gen3dOrchestrator::Pipeline)
        );
    }

    #[test]
    fn parses_gen3d_orchestrator_from_top_level() {
        let text = r#"
        gen3d_orchestrator = "agent"
        "#;
        assert_eq!(
            parse_gen3d_orchestrator(text).unwrap(),
            Some(Gen3dOrchestrator::Agent)
        );
    }

    #[test]
    fn parses_log_path_from_log_section() {
        let text = r#"
        [log]
        path = "./gravimera.log"

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        token = ""
        "#;

        let mut cfg = AppConfig::default();
        cfg.loaded_from = Some(PathBuf::from("testdata/config.toml"));
        parse_config_text_into(&mut cfg, text);
        assert_eq!(
            cfg.log_path,
            Some(PathBuf::from("testdata").join("./gravimera.log"))
        );
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn disables_file_logging_when_log_path_is_empty() {
        let text = r#"
        [log]
        path = ""

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        token = ""
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.log_path, None);
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn parses_log_level_from_log_section() {
        let text = r#"
        [log]
        level = "debug"

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        token = ""
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.log_level, bevy::log::Level::DEBUG);
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn rejects_invalid_log_level() {
        let text = r#"
        [log]
        level = "nope"

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        token = ""
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert!(
            cfg.errors
                .iter()
                .any(|e| e.contains("invalid `log.level` value")),
            "expected log level parse error, got: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn parses_gen3d_ai_service_and_claude_config() {
        let text = r#"
        [gen3d]
        ai_service = "claude"

        [claude]
        base_url = "mock://gen3d"
        model = "mock"
        token = "mock"
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.gen3d_ai_service, Gen3dAiService::Claude);
        assert!(cfg.claude.is_some());
        assert!(cfg.openai.is_none());
        assert!(cfg.gemini.is_none());
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn parses_openai_token_alias() {
        let text = r#"
        [gen3d]
        ai_service = "openai"

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        token = "mock"
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.gen3d_ai_service, Gen3dAiService::OpenAi);
        assert!(cfg.openai.is_some());
        assert!(cfg.gemini.is_none());
        assert!(cfg.claude.is_none());
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn allows_mock_openai_without_token() {
        let text = r#"
        [gen3d]
        ai_service = "openai"

        [openai]
        base_url = "mock://gen3d"
        model = "mock"
        "#;

        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert_eq!(cfg.gen3d_ai_service, Gen3dAiService::OpenAi);
        assert!(cfg.openai.is_some());
        assert!(
            cfg.errors.is_empty(),
            "unexpected config errors: {:?}",
            cfg.errors
        );
    }

    #[test]
    fn default_intelligence_service_mode_is_embedded() {
        assert_eq!(
            AppConfig::default().intelligence_service_mode,
            IntelligenceServiceMode::Embedded
        );
    }

    #[test]
    fn default_intelligence_service_enabled_is_false() {
        assert!(!AppConfig::default().intelligence_service_enabled);
    }

    #[test]
    fn parses_intelligence_service_mode_from_section() {
        let text = r#"
        [intelligence_service]
        mode = "sidecar"
        "#;
        assert_eq!(
            parse_intelligence_service_mode(text).unwrap(),
            Some(IntelligenceServiceMode::Sidecar)
        );
    }

    #[test]
    fn parses_intelligence_service_mode_synonym_standalone() {
        let text = r#"
        [intelligence_service]
        mode = "standalone"
        "#;
        assert_eq!(
            parse_intelligence_service_mode(text).unwrap(),
            Some(IntelligenceServiceMode::Sidecar)
        );
    }

    #[test]
    fn parses_intelligence_service_mode_invalid_value_is_error() {
        let text = r#"
        [intelligence_service]
        mode = "wat"
        "#;
        let err = parse_intelligence_service_mode(text).unwrap_err();
        assert!(
            err.contains("invalid `intelligence_service.mode` value"),
            "err={err}"
        );
    }

    #[test]
    fn parses_intelligence_service_enabled_true_from_section() {
        let text = r#"
        [intelligence_service]
        enabled = true
        "#;
        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert!(cfg.intelligence_service_enabled);
    }

    #[test]
    fn parses_intelligence_service_enabled_and_mode_together() {
        let text = r#"
        [intelligence_service]
        mode = "sidecar"
        enabled = true
        "#;
        let mut cfg = AppConfig::default();
        parse_config_text_into(&mut cfg, text);
        assert!(cfg.intelligence_service_enabled);
        assert_eq!(
            cfg.intelligence_service_mode,
            IntelligenceServiceMode::Sidecar
        );
    }
}

fn strip_comment(line: &str) -> &str {
    let mut min = line.len();
    if let Some(pos) = line.find('#') {
        min = min.min(pos);
    }
    if let Some(pos) = line.find(';') {
        min = min.min(pos);
    }
    &line[..min]
}

fn parse_toml_string(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(rest) = value.strip_prefix('"') {
        let (inner, tail) = parse_basic_toml_string(rest)?;
        if !tail.trim().is_empty() {
            return None;
        }
        return Some(unescape_basic(inner));
    }
    if let Some(rest) = value.strip_prefix('\'') {
        let (inner, tail) = rest.split_once('\'')?;
        if !tail.trim().is_empty() {
            return None;
        }
        return Some(inner.to_string());
    }
    None
}

fn parse_toml_bool(value: &str) -> Option<bool> {
    let value = value.trim();
    let value = if value.starts_with('"') || value.starts_with('\'') {
        parse_toml_string(value)?
    } else {
        value.to_string()
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn parse_basic_toml_string(rest: &str) -> Option<(&str, &str)> {
    let mut escaped = false;
    for (idx, ch) in rest.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' => escaped = true,
            '"' => {
                let inner = &rest[..idx];
                let tail = &rest[idx + 1..];
                return Some((inner, tail));
            }
            _ => {}
        }
    }
    None
}

fn unescape_basic(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        let Some(next) = chars.next() else {
            break;
        };
        match next {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '"' => out.push('"'),
            other => {
                out.push('\\');
                out.push(other);
            }
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn join_base_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{base}/{path}")
}

#[allow(dead_code)]
pub(crate) fn config_path_display(path: &Path) -> String {
    path.display().to_string()
}
