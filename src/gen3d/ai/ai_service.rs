use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bevy::log::warn;

use crate::config::{ClaudeConfig, GeminiConfig, OpenAiConfig};

use super::structured_outputs::Gen3dAiJsonSchemaKind;
use super::{agent_parsing, artifacts, parse};
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

fn expected_version_for_schema(kind: Gen3dAiJsonSchemaKind) -> u64 {
    match kind {
        Gen3dAiJsonSchemaKind::AgentStepV1 => 1,
        Gen3dAiJsonSchemaKind::PlanV1 => 8,
        Gen3dAiJsonSchemaKind::PlanOpsV1 => 1,
        Gen3dAiJsonSchemaKind::ComponentDraftV1 => 2,
        Gen3dAiJsonSchemaKind::ReviewDeltaV1 => 1,
        Gen3dAiJsonSchemaKind::DescriptorMetaV1 => 1,
        Gen3dAiJsonSchemaKind::MotionAuthoringV1 => 1,
    }
}

fn coerce_single_json_object_best_effort(
    kind: Gen3dAiJsonSchemaKind,
    text: &str,
) -> Option<String> {
    if matches!(kind, Gen3dAiJsonSchemaKind::AgentStepV1) {
        if let Ok(step) = agent_parsing::parse_agent_step(text) {
            if let Ok(out) = serde_json::to_string(&step) {
                return Some(out);
            }
        }
    }

    fn parse_value_lenient(text: &str) -> Option<serde_json::Value> {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            return Some(value);
        }
        if let Ok(value) = json5::from_str::<serde_json::Value>(text) {
            return Some(value);
        }
        let repaired = repair_over_escaped_quotes_outside_strings(text)?;
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(repaired.trim()) {
            return Some(value);
        }
        json5::from_str::<serde_json::Value>(repaired.trim()).ok()
    }

    let expected_version = expected_version_for_schema(kind);
    let candidates = parse::extract_json_objects(text, 32);
    if candidates.is_empty() {
        return None;
    }

    let mut last_object: Option<String> = None;
    let mut last_version_match: Option<String> = None;
    for candidate in candidates.into_iter() {
        let candidate = candidate.trim();
        let Some(value) = parse_value_lenient(candidate) else {
            continue;
        };
        if !value.is_object() {
            continue;
        }

        let Ok(canonical) = serde_json::to_string(&value) else {
            continue;
        };
        last_object = Some(canonical.clone());
        if value.get("version").and_then(|v| v.as_u64()) == Some(expected_version) {
            last_version_match = Some(canonical);
        }
    }

    last_version_match.or(last_object)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LenientJsonMode {
    Json5,
    RepairStrict,
    RepairJson5,
}

fn strip_backslash_quote_outside_strings(text: &str) -> Option<String> {
    let mut out = String::with_capacity(text.len());
    let mut changed = false;

    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escape = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escape {
                escape = false;
                continue;
            }
            match ch {
                '\\' => escape = true,
                '"' => in_string = false,
                _ => {}
            }
            continue;
        }

        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '\\' => {
                // JSON does not allow backslashes outside string literals. A common LLM mistake is
                // to output `\"key\"` in object key positions. If we see `\"` while we're not
                // inside a string, drop the backslash.
                if matches!(chars.peek(), Some('"')) {
                    changed = true;
                    out.push('"');
                    let _ = chars.next();
                } else {
                    out.push(ch);
                }
            }
            _ => out.push(ch),
        }
    }

    changed.then_some(out)
}

fn repair_over_escaped_quotes_outside_strings(text: &str) -> Option<String> {
    let mut current: Option<String> = None;
    let mut candidate = text.to_string();

    // Apply the repair multiple times to handle sequences like `\\\"key\\\"` (double-escaped)
    // that require more than one pass to fully normalize.
    for _ in 0..4 {
        let Some(next) = strip_backslash_quote_outside_strings(&candidate) else {
            break;
        };
        candidate = next;
        current = Some(candidate.clone());
    }

    current
}

fn parse_json_value_lenient(text: &str) -> Result<(serde_json::Value, LenientJsonMode), String> {
    let text = text.trim();

    if let Ok(value) = json5::from_str::<serde_json::Value>(text) {
        return Ok((value, LenientJsonMode::Json5));
    }

    let Some(repaired) = repair_over_escaped_quotes_outside_strings(text) else {
        return Err("No lenient parse candidates".into());
    };
    let repaired = repaired.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(repaired) {
        return Ok((value, LenientJsonMode::RepairStrict));
    }
    if let Ok(value) = json5::from_str::<serde_json::Value>(repaired) {
        return Ok((value, LenientJsonMode::RepairJson5));
    }
    Err("Failed to parse after repair".into())
}

fn sleep_with_cancel(duration: Duration, cancel: Option<&Arc<AtomicBool>>) -> bool {
    if duration.is_zero() {
        return cancel.is_some_and(|flag| flag.load(Ordering::Relaxed));
    }

    let start = std::time::Instant::now();
    let step = Duration::from_millis(50);
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return true;
        }

        let elapsed = start.elapsed();
        if elapsed >= duration {
            return false;
        }
        let remaining = duration.saturating_sub(elapsed);
        std::thread::sleep(step.min(remaining));
    }
}

fn enforce_structured_outputs_json_object(
    resp: &mut Gen3dAiTextResponse,
    kind: Gen3dAiJsonSchemaKind,
    ai: &Gen3dAiServiceConfig,
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<(), String> {
    let trimmed = resp.text.trim();
    let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(value) => value,
        Err(err) => {
            let hint = parse::extract_json_objects(trimmed, 3);

            if let Ok((value, mode)) = parse_json_value_lenient(trimmed) {
                if let Ok(canonical) = serde_json::to_string(&value) {
                    warn!(
                        "Gen3D: backend did not enforce structured outputs (parsed via {mode:?}); continuing best-effort. service={} base_url={} schema={kind:?} hint_objects={} err={}",
                        ai.service_label(),
                        ai.base_url(),
                        hint.len(),
                        err
                    );
                    artifacts::append_gen3d_run_log(
                        run_dir,
                        format!(
                            "structured_outputs_violation prefix={} schema={kind:?} service={} base_url={} mode={mode:?} hint_objects={} err={}",
                            artifact_prefix,
                            ai.service_label(),
                            ai.base_url(),
                            hint.len(),
                            err
                        ),
                    );
                    resp.text = canonical;
                    value
                } else {
                    value
                }
            } else if let Some(coerced) = coerce_single_json_object_best_effort(kind, trimmed) {
                warn!(
                    "Gen3D: backend did not enforce structured outputs; continuing best-effort. service={} base_url={} schema={kind:?} hint_objects={} err={}",
                    ai.service_label(),
                    ai.base_url(),
                    hint.len(),
                    err
                );
                artifacts::append_gen3d_run_log(
                    run_dir,
                    format!(
                        "structured_outputs_violation prefix={} schema={kind:?} service={} base_url={} hint_objects={} err={}",
                        artifact_prefix,
                        ai.service_label(),
                        ai.base_url(),
                        hint.len(),
                        err
                    ),
                );
                resp.text = coerced;
                serde_json::from_str(resp.text.trim()).map_err(|err2| {
                    format!(
                        "Gen3D: backend did not enforce structured outputs (response could not be coerced into a single JSON object). service={} base_url={} err={err2}",
                        ai.service_label(),
                        ai.base_url(),
                    )
                })?
            } else if hint.len() > 1 {
                return Err(format!(
                    "Gen3D: backend did not enforce structured outputs (multiple JSON objects detected). service={} base_url={} err={err}",
                    ai.service_label(),
                    ai.base_url(),
                ));
            } else {
                return Err(format!(
                    "Gen3D: backend did not enforce structured outputs (response is not a single JSON object). service={} base_url={} err={err}",
                    ai.service_label(),
                    ai.base_url(),
                ));
            }
        }
    };

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

    Ok(())
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
    // Some OpenAI-compatible providers can return truncated SSE streams (e.g. missing
    // `response.output_text.done`) while still returning HTTP 200. When structured outputs are
    // required, retry a few times before failing the entire Gen3D run.
    const MAX_STRUCTURED_OUTPUT_RETRIES: usize = 2;

    let mut session = session;
    let max_attempts = 1 + MAX_STRUCTURED_OUTPUT_RETRIES;
    let mut attempt = 0usize;
    let mut retry_delay = Duration::from_millis(200);

    loop {
        attempt = attempt.saturating_add(1);

        let call_session = if attempt > 1 && require_structured_outputs && expected_schema.is_some()
        {
            // Avoid `/responses` continuation while retrying; if the previous response was
            // truncated, continuing it may produce multiple concatenated objects.
            let mut session = session.clone();
            session.responses_previous_id = None;
            session
        } else {
            session.clone()
        };

        let mut resp = match ai {
            Gen3dAiServiceConfig::OpenAi(openai) => super::openai::generate_text_via_openai(
                progress,
                call_session,
                cancel.clone(),
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
                call_session,
                cancel.clone(),
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
                call_session,
                cancel.clone(),
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

        session = resp.session.clone();

        if require_structured_outputs && expected_schema.is_some() {
            let kind = expected_schema.unwrap();
            if let Err(err) = enforce_structured_outputs_json_object(
                &mut resp,
                kind,
                ai,
                run_dir,
                artifact_prefix,
            ) {
                if attempt < max_attempts {
                    warn!(
                        "Gen3D: structured outputs invalid; retrying (attempt {}/{}) service={} base_url={} schema={kind:?} err={}",
                        attempt,
                        max_attempts,
                        ai.service_label(),
                        ai.base_url(),
                        err
                    );
                    artifacts::append_gen3d_run_log(
                        run_dir,
                        format!(
                            "structured_outputs_retry prefix={} schema={kind:?} service={} base_url={} attempt={}/{} delay_ms={} err={}",
                            artifact_prefix,
                            ai.service_label(),
                            ai.base_url(),
                            attempt,
                            max_attempts,
                            retry_delay.as_millis(),
                            err
                        ),
                    );

                    if sleep_with_cancel(retry_delay, cancel.as_ref()) {
                        return Err("Cancelled".into());
                    }
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(2));
                    continue;
                }

                return Err(err);
            }
        }

        return Ok(resp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::agent::Gen3dAgentActionJsonV1;

    #[test]
    fn coerce_agent_step_prefers_non_done_candidate() {
        let text = r#"{"version":1,"status_summary":"first","actions":[{"kind":"tool_call","call_id":"call_1","tool_id":"get_tool_detail_v1","args":{"tool_id":"qa_v1"}}]}{"version":1,"status_summary":"second","actions":[{"kind":"done","reason":"stop"}]}"#;
        let coerced =
            coerce_single_json_object_best_effort(Gen3dAiJsonSchemaKind::AgentStepV1, text)
                .expect("coerce");
        let step: crate::gen3d::agent::Gen3dAgentStepJsonV1 =
            serde_json::from_str(&coerced).expect("parse coerced JSON");
        assert_eq!(step.version, 1);
        assert_eq!(step.status_summary, "first");
        assert_eq!(step.actions.len(), 1);
        assert!(matches!(
            step.actions[0],
            Gen3dAgentActionJsonV1::ToolCall { .. }
        ));
    }

    #[test]
    fn coerce_prefers_last_version_match_for_non_agent_schema() {
        let text = r#"{"version":7} noise {"version":8}"#;
        let coerced = coerce_single_json_object_best_effort(Gen3dAiJsonSchemaKind::PlanV1, text)
            .expect("coerce");
        let value: serde_json::Value = serde_json::from_str(&coerced).expect("parse coerced JSON");
        assert_eq!(value.get("version").and_then(|v| v.as_u64()), Some(8));
    }

    #[test]
    fn lenient_parsing_accepts_json5_trailing_commas_and_canonicalizes() {
        let text = r#"{version: 1, status_summary: "ok", actions: [],}"#;
        let (value, mode) = parse_json_value_lenient(text).expect("lenient parse");
        assert!(matches!(
            mode,
            LenientJsonMode::Json5 | LenientJsonMode::RepairJson5
        ));
        let canonical = serde_json::to_string(&value).expect("canonicalize");
        let parsed: serde_json::Value = serde_json::from_str(&canonical).expect("strict JSON");
        assert!(parsed.is_object());
    }

    #[test]
    fn lenient_parsing_repairs_over_escaped_keys_outside_strings() {
        let text = r#"{"a":1,"b":{\"c\":2}}"#;
        let (value, mode) = parse_json_value_lenient(text).expect("lenient parse");
        assert_eq!(mode, LenientJsonMode::RepairStrict);
        let canonical = serde_json::to_string(&value).expect("canonicalize");
        let parsed: serde_json::Value = serde_json::from_str(&canonical).expect("strict JSON");
        assert_eq!(parsed.get("a").and_then(|v| v.as_u64()), Some(1));
        assert_eq!(
            parsed
                .get("b")
                .and_then(|v| v.get("c"))
                .and_then(|v| v.as_u64()),
            Some(2)
        );
    }

    #[test]
    fn structured_outputs_retry_recovers_from_truncated_json() {
        let progress = Arc::new(Mutex::new(Gen3dAiProgress::default()));
        let ai = Gen3dAiServiceConfig::OpenAi(OpenAiConfig {
            base_url: "mock://gen3d".into(),
            model: "gpt-mock".into(),
            model_reasoning_effort: "high".into(),
            api_key: "test".into(),
        });

        let user_text = "__MOCK_TRUNCATE_ONCE__ test_id=structured_outputs_retry_recovers_from_truncated_json\nUser prompt:\nA warcar with a cannon.\nInput images:\n";

        let resp = generate_text_via_ai_service(
            &progress,
            Gen3dAiSessionState::default(),
            None,
            Some(Gen3dAiJsonSchemaKind::AgentStepV1),
            true,
            &ai,
            "high",
            "test system",
            user_text,
            &[],
            None,
            "agent_step",
        )
        .expect("should recover via retry");

        let parsed: serde_json::Value =
            serde_json::from_str(resp.text.trim()).expect("valid JSON after retry");
        assert!(parsed.is_object());
    }
}
