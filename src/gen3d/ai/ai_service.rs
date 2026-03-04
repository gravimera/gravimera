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
    ai: &Gen3dAiServiceConfig,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, String> {
    match ai {
        Gen3dAiServiceConfig::OpenAi(openai) => super::openai::generate_text_via_openai(
            progress,
            session,
            cancel,
            expected_schema,
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
            &claude.base_url,
            &claude.api_key,
            &claude.model,
            system_instructions,
            user_text,
            image_paths,
            run_dir,
            artifact_prefix,
        ),
    }
}
