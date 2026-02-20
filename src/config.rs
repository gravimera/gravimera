use bevy::prelude::Resource;
use std::path::{Path, PathBuf};

const CONFIG_FILE_NAME: &str = "config.toml";
const CONFIG_OVERRIDE_ENV: &str = "GRAVIMERA_CONFIG";

#[derive(Resource, Clone, Debug)]
pub(crate) struct AppConfig {
    pub(crate) openai: Option<OpenAiConfig>,
    pub(crate) log_path: Option<PathBuf>,
    pub(crate) scene_dat_path: Option<PathBuf>,
    pub(crate) gen3d_cache_dir: Option<PathBuf>,
    pub(crate) automation_enabled: bool,
    pub(crate) automation_bind: Option<String>,
    pub(crate) automation_token: Option<String>,
    pub(crate) automation_disable_local_input: bool,
    pub(crate) automation_pause_on_start: bool,
    pub(crate) refine_iterations: u32,
    pub(crate) gen3d_max_parallel_components: usize,
    pub(crate) gen3d_max_seconds: u64,
    pub(crate) gen3d_max_tokens: u64,
    pub(crate) gen3d_no_progress_max_steps: u32,
    pub(crate) gen3d_max_replans: u32,
    pub(crate) gen3d_max_regen_total: u32,
    pub(crate) gen3d_max_regen_per_component: u32,
    pub(crate) gen3d_save_pass_screenshots: bool,
    pub(crate) gen3d_review_appearance: bool,
    pub(crate) gen3d_reasoning_effort_plan: String,
    pub(crate) gen3d_reasoning_effort_agent_step: String,
    pub(crate) gen3d_reasoning_effort_component: String,
    pub(crate) gen3d_reasoning_effort_review: String,
    pub(crate) gen3d_reasoning_effort_repair: String,
    pub(crate) loaded_from: Option<PathBuf>,
    pub(crate) errors: Vec<String>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            openai: None,
            log_path: None,
            scene_dat_path: None,
            gen3d_cache_dir: None,
            automation_enabled: false,
            automation_bind: None,
            automation_token: None,
            automation_disable_local_input: true,
            automation_pause_on_start: true,
            refine_iterations: 1,
            gen3d_max_parallel_components: 10,
            gen3d_max_seconds: 60 * 60,
            gen3d_max_tokens: 10_000_000,
            gen3d_no_progress_max_steps: 12,
            gen3d_max_replans: 1,
            gen3d_max_regen_total: 16,
            gen3d_max_regen_per_component: 2,
            gen3d_save_pass_screenshots: !cfg!(test),
            gen3d_review_appearance: false,
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

    parse_log_path_into_config(&mut out, &text);
    parse_scene_dat_path_into_config(&mut out, &text);
    parse_gen3d_cache_dir_into_config(&mut out, &text);
    parse_automation_enabled_into_config(&mut out, &text);
    parse_automation_bind_into_config(&mut out, &text);
    parse_automation_disable_local_input_into_config(&mut out, &text);
    parse_automation_pause_on_start_into_config(&mut out, &text);
    parse_automation_token_into_config(&mut out, &text);
    parse_refine_iterations_into_config(&mut out, &text);
    parse_gen3d_max_parallel_components_into_config(&mut out, &text);
    parse_gen3d_max_seconds_into_config(&mut out, &text);
    parse_gen3d_max_tokens_into_config(&mut out, &text);
    parse_gen3d_no_progress_max_steps_into_config(&mut out, &text);
    parse_gen3d_max_replans_into_config(&mut out, &text);
    parse_gen3d_max_regen_total_into_config(&mut out, &text);
    parse_gen3d_max_regen_per_component_into_config(&mut out, &text);
    parse_gen3d_save_pass_screenshots_into_config(&mut out, &text);
    parse_gen3d_review_appearance_into_config(&mut out, &text);
    parse_gen3d_reasoning_effort_plan_into_config(&mut out, &text);
    parse_gen3d_reasoning_effort_agent_step_into_config(&mut out, &text);
    parse_gen3d_reasoning_effort_component_into_config(&mut out, &text);
    parse_gen3d_reasoning_effort_review_into_config(&mut out, &text);
    parse_gen3d_reasoning_effort_repair_into_config(&mut out, &text);
    populate_openai_config(&mut out, &text);

    out
}

fn load_config_from_path(path: &Path) -> AppConfig {
    let mut out = AppConfig::default();
    match std::fs::read_to_string(path) {
        Ok(text) => {
            out.loaded_from = Some(path.to_path_buf());
            parse_log_path_into_config(&mut out, &text);
            parse_scene_dat_path_into_config(&mut out, &text);
            parse_gen3d_cache_dir_into_config(&mut out, &text);
            parse_automation_enabled_into_config(&mut out, &text);
            parse_automation_bind_into_config(&mut out, &text);
            parse_automation_disable_local_input_into_config(&mut out, &text);
            parse_automation_pause_on_start_into_config(&mut out, &text);
            parse_automation_token_into_config(&mut out, &text);
            parse_refine_iterations_into_config(&mut out, &text);
            parse_gen3d_max_parallel_components_into_config(&mut out, &text);
            parse_gen3d_max_seconds_into_config(&mut out, &text);
            parse_gen3d_max_tokens_into_config(&mut out, &text);
            parse_gen3d_no_progress_max_steps_into_config(&mut out, &text);
            parse_gen3d_max_replans_into_config(&mut out, &text);
            parse_gen3d_max_regen_total_into_config(&mut out, &text);
            parse_gen3d_max_regen_per_component_into_config(&mut out, &text);
            parse_gen3d_save_pass_screenshots_into_config(&mut out, &text);
            parse_gen3d_review_appearance_into_config(&mut out, &text);
            parse_gen3d_reasoning_effort_plan_into_config(&mut out, &text);
            parse_gen3d_reasoning_effort_agent_step_into_config(&mut out, &text);
            parse_gen3d_reasoning_effort_component_into_config(&mut out, &text);
            parse_gen3d_reasoning_effort_review_into_config(&mut out, &text);
            parse_gen3d_reasoning_effort_repair_into_config(&mut out, &text);
            populate_openai_config(&mut out, &text);
        }
        Err(err) => out
            .errors
            .push(format!("Failed to read {}: {err}", path.display())),
    }
    out
}

fn populate_openai_config(out: &mut AppConfig, text: &str) {
    match parse_openai_config(text) {
        Ok(mut cfg) => {
            // Allow env override for convenience (keeps secrets out of files if desired).
            if cfg.api_key.trim().is_empty() {
                if let Ok(key) = std::env::var("OPENAI_API_KEY") {
                    cfg.api_key = key;
                }
            }
            if cfg.api_key.trim().is_empty() {
                out.errors.push(
                    "config.toml: missing `openai.OPENAI_API_KEY` (or env `OPENAI_API_KEY`)".into(),
                );
            } else {
                out.openai = Some(cfg);
            }
        }
        Err(err) => out.errors.push(err),
    }
}

fn parse_log_path_into_config(out: &mut AppConfig, text: &str) {
    match parse_log_path(text) {
        Ok(Some(path)) => {
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

fn parse_gen3d_no_progress_max_steps_into_config(out: &mut AppConfig, text: &str) {
    match parse_gen3d_no_progress_max_steps(text) {
        Ok(Some(value)) => out.gen3d_no_progress_max_steps = value,
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

fn parse_log_path(text: &str) -> Result<Option<PathBuf>, String> {
    let mut section: Option<String> = None;
    let mut log_path: Option<PathBuf> = None;

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
        if key != "log_path" {
            continue;
        }

        // Accept `log_path` at top-level, or under `[app]` / `[logging]` / `[openai]` for convenience.
        if let Some(sec) = section.as_deref() {
            if sec != "app" && sec != "logging" && sec != "openai" {
                continue;
            }
        }

        let value = value.trim();
        let value = if value.starts_with('"') || value.starts_with('\'') {
            parse_toml_string(value).ok_or_else(|| {
                format!(
                    "config.toml:{line_no}: expected a quoted string value for key `log_path` (example: log_path = \"./gravimera.log\")"
                )
            })?
        } else {
            // Be forgiving: accept unquoted path strings for `log_path`.
            value.to_string()
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            log_path = None;
        } else {
            log_path = Some(expand_tilde_path(trimmed));
        }
    }

    Ok(log_path)
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
                    "config.toml:{line_no}: expected an integer for `max_seconds` (example: max_seconds = 3600)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i128 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `max_seconds` (example: max_seconds = 3600)"
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

fn parse_gen3d_no_progress_max_steps(text: &str) -> Result<Option<u32>, String> {
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
        if key != "no_progress_max_steps" && key != "gen3d_no_progress_max_steps" {
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
                    "config.toml:{line_no}: expected an integer for `no_progress_max_steps` (example: no_progress_max_steps = 12)"
                )
            })?
        } else {
            value.to_string()
        };

        let parsed: i64 = value.trim().parse().map_err(|_| {
            format!(
                "config.toml:{line_no}: expected an integer for `no_progress_max_steps` (example: no_progress_max_steps = 12)"
            )
        })?;
        if parsed < 0 {
            return Err(format!(
                "config.toml:{line_no}: `no_progress_max_steps` must be >= 0 (0 disables the guard)"
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
            "OPENAI_API_KEY" => api_key = Some(value),
            _ => {}
        }
    }

    let base_url = base_url.unwrap_or_else(|| "https://api.openai.com/v1".into());
    let model = model.ok_or_else(|| "config.toml: missing `openai.model`".to_string())?;
    let effort = effort.unwrap_or_else(|| "high".into());
    let api_key = api_key.unwrap_or_default();

    Ok(OpenAiConfig {
        base_url,
        model,
        model_reasoning_effort: effort,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_gen3d_review_appearance;
    use super::parse_gen3d_save_pass_screenshots;

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
