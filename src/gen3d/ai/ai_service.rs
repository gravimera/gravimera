use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use crate::config::{ClaudeConfig, GeminiConfig, OpenAiConfig};

use super::structured_outputs::Gen3dAiJsonSchemaKind;
use super::{Gen3dAiProgress, Gen3dAiSessionState, Gen3dAiTextResponse};

#[derive(Clone, Debug)]
pub(super) enum Gen3dAiServiceConfig {
    OpenAi(OpenAiConfig),
    Gemini(GeminiConfig),
    Claude(ClaudeConfig),
}

impl Gen3dAiServiceConfig {
    pub(super) fn service_label(&self) -> &'static str {
        match self {
            Self::OpenAi(_) => "openai",
            Self::Gemini(_) => "gemini",
            Self::Claude(_) => "claude",
        }
    }

    pub(super) fn base_url(&self) -> &str {
        match self {
            Self::OpenAi(cfg) => cfg.base_url.as_str(),
            Self::Gemini(cfg) => cfg.base_url.as_str(),
            Self::Claude(cfg) => cfg.base_url.as_str(),
        }
    }

    pub(super) fn model(&self) -> &str {
        match self {
            Self::OpenAi(cfg) => cfg.model.as_str(),
            Self::Gemini(cfg) => cfg.model.as_str(),
            Self::Claude(cfg) => cfg.model.as_str(),
        }
    }

    pub(super) fn model_reasoning_effort(&self) -> &str {
        match self {
            Self::OpenAi(cfg) => cfg.model_reasoning_effort.as_str(),
            // Gemini does not have an OpenAI-style "reasoning_effort" request parameter, but the
            // rest of the Gen3D orchestration expects an effective effort string for logging +
            // budget capping. Treat Gemini as "high" here; the Gemini backend ignores this.
            Self::Gemini(_) => "high",
            // Claude does not support the OpenAI Responses `reasoning.effort` parameter, but keep
            // the same logging/budget surface.
            Self::Claude(_) => "high",
        }
    }
}

pub(super) fn generate_text_via_ai_service(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    cancel: Option<Arc<AtomicBool>>,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
    ai: &Gen3dAiServiceConfig,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, String> {
    let resp = match ai {
        Gen3dAiServiceConfig::OpenAi(openai) => super::openai::generate_text_via_openai(
            progress,
            session,
            cancel,
            expected_schema,
            require_structured_outputs,
            &openai.base_url,
            &openai.api_key,
            &openai.model,
            reasoning_effort,
            system_instructions,
            user_text,
            image_paths,
            run_dir,
            artifact_prefix,
        ),
        Gen3dAiServiceConfig::Gemini(gemini) => super::gemini::generate_text_via_gemini(
            progress,
            session,
            cancel,
            expected_schema,
            require_structured_outputs,
            &gemini.base_url,
            &gemini.api_key,
            &gemini.model,
            system_instructions,
            user_text,
            image_paths,
            run_dir,
            artifact_prefix,
        ),
        Gen3dAiServiceConfig::Claude(claude) => super::claude::generate_text_via_claude(
            progress,
            session,
            cancel,
            expected_schema,
            require_structured_outputs,
            &claude.base_url,
            &claude.api_key,
            &claude.model,
            system_instructions,
            user_text,
            image_paths,
            run_dir,
            artifact_prefix,
        ),
    }?;

    if require_structured_outputs && expected_schema.is_some() {
        let trimmed = resp.text.trim();
        let parsed: serde_json::Value = serde_json::from_str(trimmed).map_err(|err| {
            let hint = super::parse::extract_json_objects(trimmed, 3);
            if hint.len() > 1 {
                format!(
                    "Gen3D: backend did not enforce structured outputs (multiple JSON objects detected). service={} base_url={} err={err}",
                    ai.service_label(),
                    ai.base_url(),
                )
            } else {
                format!(
                    "Gen3D: backend did not enforce structured outputs (response is not a single JSON object). service={} base_url={} err={err}",
                    ai.service_label(),
                    ai.base_url(),
                )
            }
        })?;
        if !parsed.is_object() {
            return Err(format!(
                "Gen3D: backend did not enforce structured outputs (expected a JSON object, got {}). service={} base_url={}",
                match parsed {
                    serde_json::Value::Null => "null",
                    serde_json::Value::Bool(_) => "bool",
                    serde_json::Value::Number(_) => "number",
                    serde_json::Value::String(_) => "string",
                    serde_json::Value::Array(_) => "array",
                    serde_json::Value::Object(_) => "object",
                },
                ai.service_label(),
                ai.base_url(),
            ));
        }
    }

    Ok(resp)
}
