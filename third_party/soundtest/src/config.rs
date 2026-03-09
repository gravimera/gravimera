use crate::cli;
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireApi {
    Auto,
    Responses,
    ChatCompletions,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub base_url: String,
    pub token: String,
    pub model: String,
    pub model_reasoning_effort: String,
    pub wire_api: WireApi,

    pub onnx_model_dir: Option<PathBuf>,
    pub onnx_voice: Option<PathBuf>,
    pub onnx_language_model: Option<PathBuf>,
    pub onnx_runtime: Option<PathBuf>,

    pub system_tts_binary: String,
}

#[derive(Debug, Deserialize, Default)]
struct SoundtestConfigFile {
    base_url: Option<String>,
    token: Option<String>,
    model: Option<String>,
    model_reasoning_effort: Option<String>,
    wire_api: Option<String>,

    onnx_model_dir: Option<String>,
    onnx_voice: Option<String>,
    onnx_language_model: Option<String>,
    onnx_runtime: Option<String>,

    system_tts_binary: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct CodexDefaults {
    base_url: Option<String>,
    token: Option<String>,
    model: Option<String>,
    reasoning_effort: Option<String>,
    wire_api: Option<String>,
}

pub fn default_config_path() -> PathBuf {
    if let Some(home) = dirs::home_dir() {
        return home.join(".soundtest").join("config.toml");
    }
    PathBuf::from(".soundtest").join("config.toml")
}

pub fn load_settings(cli: &cli::Cli) -> Result<Settings> {
    let config_path = cli.config.clone().unwrap_or_else(default_config_path);
    let config_dir = config_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let file_cfg = load_config_file(&config_path).with_context(|| {
        format!(
            "failed to read config file {}",
            config_path.to_string_lossy()
        )
    })?;

    let codex = load_codex_defaults().unwrap_or_default();

    let require_ai = !cli.no_ai && !matches!(cli.command, Some(cli::Command::Doctor));

    let base_url_opt = first_non_empty([
        cli.base_url.as_deref(),
        file_cfg.base_url.as_deref(),
        std::env::var("OPENAI_BASE_URL").ok().as_deref(),
        codex.base_url.as_deref(),
    ])
    .map(str::to_owned);
    let base_url = if require_ai {
        base_url_opt.ok_or_else(|| missing_setting_error("base_url", &config_path))?
    } else {
        base_url_opt.unwrap_or_else(|| "offline".to_owned())
    };

    let token_opt = first_non_empty([
        cli.token.as_deref(),
        file_cfg.token.as_deref(),
        std::env::var("OPENAI_API_KEY").ok().as_deref(),
        codex.token.as_deref(),
    ])
    .map(str::to_owned);
    let token = if require_ai {
        token_opt.ok_or_else(|| missing_setting_error("token", &config_path))?
    } else {
        token_opt.unwrap_or_default()
    };

    let model_opt = first_non_empty([
        cli.model.as_deref(),
        file_cfg.model.as_deref(),
        codex.model.as_deref(),
    ])
    .map(str::to_owned);
    let model = if require_ai {
        model_opt.ok_or_else(|| missing_setting_error("model", &config_path))?
    } else {
        model_opt.unwrap_or_else(|| "offline".to_owned())
    };

    let model_reasoning_effort = first_non_empty([
        cli.reasoning_effort.as_deref(),
        file_cfg.model_reasoning_effort.as_deref(),
        Some("medium"),
    ])
    .unwrap()
    .to_owned();

    let wire_api = parse_wire_api(first_non_empty([
        file_cfg.wire_api.as_deref(),
        codex.wire_api.as_deref(),
        Some("auto"),
    ]));

    let mut onnx_model_dir = if let Some(p) = cli.onnx_model_dir.clone() {
        Some(resolve_path(p, &cwd))
    } else if let Some(s) = file_cfg.onnx_model_dir.as_deref() {
        Some(resolve_path(PathBuf::from(s), &config_dir))
    } else {
        None
    };
    if onnx_model_dir.is_none() {
        let candidates = [
            cwd.join("models").join("chatterbox-multilingual-onnx"),
            cwd.join("models").join("chatterbox-multilingual-ONNX"),
        ];
        onnx_model_dir = candidates.into_iter().find(|p| p.is_dir());
    }

    let onnx_voice = if let Some(p) = cli.onnx_voice.clone() {
        Some(resolve_path(p, &cwd))
    } else if let Some(s) = file_cfg.onnx_voice.as_deref() {
        Some(resolve_path(PathBuf::from(s), &config_dir))
    } else {
        None
    };

    let onnx_language_model = if let Some(p) = cli.onnx_language_model.clone() {
        Some(resolve_path(p, &cwd))
    } else if let Some(s) = file_cfg.onnx_language_model.as_deref() {
        Some(resolve_path(PathBuf::from(s), &config_dir))
    } else {
        None
    };

    let onnx_runtime = if let Some(p) = cli.onnx_runtime.clone() {
        Some(resolve_path(p, &cwd))
    } else if let Some(s) = file_cfg.onnx_runtime.as_deref() {
        Some(resolve_path(PathBuf::from(s), &config_dir))
    } else {
        None
    };

    let system_tts_binary = first_non_empty([
        file_cfg.system_tts_binary.as_deref(),
        Some(default_system_tts_binary()),
    ])
    .map(|s| resolve_executable_spec(s, &config_dir))
    .unwrap();

    Ok(Settings {
        base_url,
        token,
        model,
        model_reasoning_effort,
        wire_api,
        onnx_model_dir,
        onnx_voice,
        onnx_language_model,
        onnx_runtime,
        system_tts_binary,
    })
}

fn load_config_file(path: &Path) -> Result<SoundtestConfigFile> {
    if !path.exists() {
        return Ok(SoundtestConfigFile::default());
    }
    let content = std::fs::read_to_string(path)?;
    let cfg: SoundtestConfigFile = toml::from_str(&content)?;
    Ok(cfg)
}

fn resolve_path(path: PathBuf, config_dir: &Path) -> PathBuf {
    let expanded = expand_tilde(path);
    if expanded.is_relative() {
        return config_dir.join(expanded);
    }
    expanded
}

fn resolve_executable_spec(spec: &str, config_dir: &Path) -> String {
    if looks_like_path(spec) {
        let resolved = resolve_path(PathBuf::from(spec), config_dir);
        return resolved.to_string_lossy().to_string();
    }
    spec.to_owned()
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/')
        || value.contains('\\')
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with(".\\")
        || value.starts_with("..\\")
}

fn expand_tilde(path: PathBuf) -> PathBuf {
    let home = dirs::home_dir();
    let Some(home) = home else { return path };

    let mut components = path.components();
    let Some(first) = components.next() else {
        return path;
    };
    if first.as_os_str() != "~" {
        return path;
    }

    let mut out = home;
    out.extend(components);
    out
}

fn missing_setting_error(setting: &str, config_path: &Path) -> anyhow::Error {
    anyhow!(
        "missing required setting `{}`. Create {} or provide `--{}`",
        setting,
        config_path.to_string_lossy(),
        setting.replace('_', "-")
    )
}

fn first_non_empty<'a, const N: usize>(values: [Option<&'a str>; N]) -> Option<&'a str> {
    for v in values {
        let Some(v) = v else { continue };
        let trimmed = v.trim();
        if !trimmed.is_empty() {
            return Some(trimmed);
        }
    }
    None
}

fn parse_wire_api(value: Option<&str>) -> WireApi {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "responses" => WireApi::Responses,
        "chat" | "chat_completions" | "chatcompletions" => WireApi::ChatCompletions,
        _ => WireApi::Auto,
    }
}

#[cfg(windows)]
fn default_system_tts_binary() -> &'static str {
    "powershell"
}

#[cfg(target_os = "macos")]
fn default_system_tts_binary() -> &'static str {
    "say"
}

#[cfg(not(any(windows, target_os = "macos")))]
fn default_system_tts_binary() -> &'static str {
    "say"
}

fn load_codex_defaults() -> Option<CodexDefaults> {
    let home = dirs::home_dir()?;
    let codex_dir = home.join(".codex");
    let config_path = codex_dir.join("config.toml");
    let auth_path = codex_dir.join("auth.json");

    let mut defaults = CodexDefaults::default();

    if config_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&config_path) {
            if let Ok(cfg) = toml::from_str::<CodexConfig>(&text) {
                defaults.model = cfg.model.clone();
                defaults.reasoning_effort = cfg.model_reasoning_effort.clone();

                if let Some((base_url, wire_api)) = cfg
                    .selected_provider()
                    .and_then(|p| Some((p.base_url, p.wire_api)))
                {
                    defaults.base_url = base_url;
                    defaults.wire_api = wire_api;
                }
            }
        }
    }

    if auth_path.exists() {
        if let Ok(text) = std::fs::read_to_string(&auth_path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                defaults.token = json
                    .get("OPENAI_API_KEY")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
            }
        }
    }

    if defaults.base_url.is_none() && defaults.token.is_none() && defaults.model.is_none() {
        return None;
    }

    Some(defaults)
}

#[derive(Debug, Deserialize)]
struct CodexConfig {
    model_provider: Option<String>,
    model: Option<String>,
    model_reasoning_effort: Option<String>,
    model_providers: Option<HashMap<String, CodexProvider>>,
}

#[derive(Debug, Deserialize)]
struct CodexProvider {
    base_url: Option<String>,
    wire_api: Option<String>,
}

impl CodexConfig {
    fn selected_provider(&self) -> Option<CodexProvider> {
        let providers = self.model_providers.as_ref()?;
        let name = self
            .model_provider
            .clone()
            .or_else(|| providers.keys().next().cloned())?;
        providers.get(&name).cloned()
    }
}

impl Clone for CodexProvider {
    fn clone(&self) -> Self {
        Self {
            base_url: self.base_url.clone(),
            wire_api: self.wire_api.clone(),
        }
    }
}
