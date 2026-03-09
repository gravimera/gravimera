use crate::backend::BackendAvailability;
use crate::cli::BackendChoice;
use crate::config::{Settings, WireApi};
use crate::logging;
use crate::render_plan::{BackendKind, RenderPlan};
use anyhow::{Context, Result, anyhow};
use reqwest::Url;
use serde_json::json;
use std::time::Instant;

pub struct RenderRequest<'a> {
    pub object: &'a str,
    pub message: &'a str,
    pub requested_backend: BackendChoice,
    pub availability: BackendAvailability,
}

#[derive(Debug, Clone)]
pub struct RenderPlanResult {
    pub plan: RenderPlan,
    pub wire_api_used: String,
    pub duration_ms: u128,
    pub output_chars: usize,
}

#[derive(Debug, Clone)]
pub struct OpenAiClient {
    base_url: Url,
    token: String,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(base_url: &str, token: &str) -> Result<Self> {
        let mut url =
            Url::parse(base_url).with_context(|| format!("invalid base_url `{}`", base_url))?;
        if !url.path().ends_with('/') {
            let new_path = format!("{}/", url.path().trim_end_matches('/'));
            url.set_path(&new_path);
        }
        let http = reqwest::Client::builder()
            .user_agent("soundtest/0.1.0")
            .build()?;
        Ok(Self {
            base_url: url,
            token: token.to_owned(),
            http,
        })
    }

    pub async fn generate_render_plan(
        &self,
        settings: &Settings,
        request: &RenderRequest<'_>,
    ) -> Result<RenderPlanResult> {
        let start = Instant::now();
        logging::info(
            "ai.request",
            json!({
                "base_url": self.base_url.as_str(),
                "wire_api": format!("{:?}", settings.wire_api).to_ascii_lowercase(),
                "model": &settings.model,
                "reasoning_effort": &settings.model_reasoning_effort,
                "object": request.object,
                "message_chars": request.message.chars().count(),
            }),
        );

        let system_prompt = build_system_prompt(request);
        let user_prompt = format!("object: {}\nmessage: {}", request.object, request.message);

        let output_result: Result<(&str, String)> = match settings.wire_api {
            WireApi::Responses => self
                .call_responses(settings, &system_prompt, &user_prompt)
                .await
                .map(|t| ("responses", t)),
            WireApi::ChatCompletions => self
                .call_chat_completions(settings, &system_prompt, &user_prompt)
                .await
                .map(|t| ("chat_completions", t)),
            WireApi::Auto => match self
                .call_responses(settings, &system_prompt, &user_prompt)
                .await
            {
                Ok(text) => Ok(("responses", text)),
                Err(err) => {
                    let chat = self
                        .call_chat_completions(settings, &system_prompt, &user_prompt)
                        .await;
                    match chat {
                        Ok(text) => Ok(("chat_completions", text)),
                        Err(chat_err) => Err(anyhow!(
                            "responses call failed ({err}); chat.completions call also failed ({chat_err})"
                        )),
                    }
                }
            },
        };

        let (wire_api_used, output) = match output_result {
            Ok(v) => v,
            Err(err) => {
                logging::error(
                    "ai.response",
                    json!({
                        "status": "error",
                        "duration_ms": start.elapsed().as_millis(),
                        "error": format!("{err:#}"),
                    }),
                );
                return Err(err);
            }
        };

        let plan =
            RenderPlan::parse(&output).unwrap_or_else(|_| RenderPlan::fallback_text(&output));
        let plan = enforce_requested_backend(plan, request.requested_backend);
        let duration_ms = start.elapsed().as_millis();
        let output_chars = output.chars().count();

        logging::info(
            "ai.response",
            json!({
                "status": "ok",
                "duration_ms": duration_ms,
                "wire_api_used": wire_api_used,
                "output_chars": output_chars,
                "plan_backend": format!("{:?}", plan.backend).to_ascii_lowercase(),
                "plan_has_text": plan.text.as_deref().is_some_and(|t| !t.trim().is_empty()),
                "plan_has_proc": plan.proc.as_deref().is_some_and(|p| !p.trim().is_empty()),
                "plan_preset": &plan.effects.preset,
                "plan_amount": plan.effects.amount,
            }),
        );

        Ok(RenderPlanResult {
            plan,
            wire_api_used: wire_api_used.to_owned(),
            duration_ms,
            output_chars,
        })
    }

    async fn call_responses(
        &self,
        settings: &Settings,
        system: &str,
        user: &str,
    ) -> Result<String> {
        let url = self.endpoint_url("responses")?;
        let input_messages = json!([
            { "role": "system", "content": [ { "type": "input_text", "text": system } ] },
            { "role": "user", "content": [ { "type": "input_text", "text": user } ] },
        ]);

        let body_with_reasoning = json!({
            "model": settings.model,
            "input": input_messages.clone(),
            "reasoning": { "effort": settings.model_reasoning_effort },
            "stream": false,
        });

        let body_without_reasoning = json!({
            "model": settings.model,
            "input": input_messages,
            "stream": false,
        });

        let first = self
            .http
            .post(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .bearer_auth(&self.token)
            .json(&body_with_reasoning)
            .send()
            .await?;

        let resp = if first.status().is_success() {
            first
        } else if first.status().as_u16() == 400 {
            self.http
                .post(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .bearer_auth(&self.token)
                .json(&body_without_reasoning)
                .send()
                .await?
        } else {
            first
        };

        parse_responses_output(resp).await
    }

    async fn call_chat_completions(
        &self,
        settings: &Settings,
        system: &str,
        user: &str,
    ) -> Result<String> {
        let url = self.endpoint_url("chat/completions")?;

        let body_with_reasoning = json!({
            "model": settings.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "reasoning_effort": settings.model_reasoning_effort,
            "stream": false,
        });

        let body_without_reasoning = json!({
            "model": settings.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
            "stream": false,
        });

        let first = self
            .http
            .post(url.clone())
            .header(reqwest::header::ACCEPT, "application/json")
            .bearer_auth(&self.token)
            .json(&body_with_reasoning)
            .send()
            .await?;

        let resp = if first.status().is_success() {
            first
        } else if first.status().as_u16() == 400 {
            self.http
                .post(url)
                .header(reqwest::header::ACCEPT, "application/json")
                .bearer_auth(&self.token)
                .json(&body_without_reasoning)
                .send()
                .await?
        } else {
            first
        };

        parse_chat_completions_output(resp).await
    }

    fn endpoint_url(&self, endpoint: &str) -> Result<Url> {
        let base = self.base_url.as_str();
        if base.trim_end_matches('/').ends_with(endpoint) {
            return Ok(Url::parse(base)?);
        }
        Ok(self.base_url.join(endpoint)?)
    }
}

async fn parse_responses_output(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "AI API error {}: {}",
            status.as_u16(),
            truncate(&text, 400)
        ));
    }
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        return extract_responses_output_text(&value)
            .ok_or_else(|| anyhow!("AI response missing output text: {}", truncate(&text, 400)));
    }

    parse_sse_responses_output(&text)
        .with_context(|| format!("unexpected AI response format: {}", truncate(&text, 400)))
}

async fn parse_chat_completions_output(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(anyhow!(
            "AI API error {}: {}",
            status.as_u16(),
            truncate(&text, 400)
        ));
    }
    let value: serde_json::Value = serde_json::from_str(&text)
        .with_context(|| format!("unexpected AI response format: {}", truncate(&text, 400)))?;

    let content = value
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|msg| msg.get("content"))
        .and_then(|c| c.as_str());

    content.map(str::to_owned).ok_or_else(|| {
        anyhow!(
            "AI response missing choices[0].message.content: {}",
            truncate(&text, 400)
        )
    })
}

fn parse_sse_responses_output(body: &str) -> Result<String> {
    let mut completed: Option<serde_json::Value> = None;
    let mut last_with_response: Option<serde_json::Value> = None;

    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let data = line.trim_start_matches("data:").trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(event) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };

        let event_type = event.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let has_response = event.get("response").is_some();
        if event_type == "response.completed" {
            completed = Some(event.clone());
        }
        if has_response {
            last_with_response = Some(event);
        }
    }

    let event = completed
        .or(last_with_response)
        .ok_or_else(|| anyhow!("no SSE data events"))?;

    if let Some(response) = event.get("response") {
        return extract_responses_output_text(response)
            .ok_or_else(|| anyhow!("SSE response missing output text"));
    }

    extract_responses_output_text(&event).ok_or_else(|| anyhow!("SSE event missing output text"))
}

fn extract_responses_output_text(value: &serde_json::Value) -> Option<String> {
    if let Some(out) = value.get("output_text").and_then(|v| v.as_str()) {
        return Some(out.to_owned());
    }

    if let Some(output) = value.get("output").and_then(|v| v.as_array()) {
        let mut parts: Vec<String> = Vec::new();
        for item in output {
            if let Some(content) = item.get("content").and_then(|v| v.as_array()) {
                for part in content {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        parts.push(text.to_owned());
                    }
                }
            }
        }
        if !parts.is_empty() {
            return Some(parts.join(""));
        }
    }

    None
}

fn build_system_prompt(request: &RenderRequest<'_>) -> String {
    let mut available = request.availability.available_backends_for_ai();
    available.sort();
    available.dedup();
    let available_list = available.join(", ");
    let requested = match request.requested_backend {
        BackendChoice::Auto => "auto".to_owned(),
        BackendChoice::Onnx => "onnx".to_owned(),
        BackendChoice::System => "system".to_owned(),
        BackendChoice::Procedural => "procedural".to_owned(),
    };

    let mut prompt = String::new();
    prompt.push_str("You are an audio render planner.\n");
    prompt.push_str("Your job: speak the user's message using TTS, then apply audio effects to match the object's voice.\n");
    prompt.push_str("Text should remain intelligible and preserve the user's meaning, but may be rewritten to fit the object's voice.\n");
    prompt.push_str(
        "Return ONLY a render plan in plain text. No markdown, no quotes, no explanations.\n\n",
    );
    prompt.push_str("Available backends:\n");
    prompt.push_str(&format!("- {}\n\n", available_list));

    prompt.push_str("Available effect presets (for TTS backends):\n");
    prompt.push_str("- neutral, dragon, robot, fairy, giant, ghost, radio\n\n");

    prompt.push_str("Output format:\n");
    prompt.push_str("backend: <onnx|system|procedural>\n");
    prompt.push_str("text: <text to speak>                       (required for onnx/system)\n");
    prompt.push_str("proc: <token text for procedural synthesis> (required for procedural)\n");
    prompt.push_str("preset: <neutral|dragon|robot|fairy|giant|ghost|radio>   (only for onnx/system)\n");
    prompt.push_str("amount: <0.0-1.0>                            (only for onnx/system)\n");
    prompt.push_str("speed: <0.4-1.8>                              (optional; 1.0 normal; <1 slower; >1 faster)\n");
    prompt.push_str(
        "pitch_semitones: <-24..24>                    (optional; negative lowers voice)\n",
    );
    prompt.push_str("bass_db: <-12..18>                            (optional)\n");
    prompt.push_str("treble_db: <-12..18>                          (optional)\n");
    prompt.push_str("reverb: <0.0-1.0>                             (optional)\n");
    prompt.push_str("distortion: <0.0-1.0>                         (optional)\n\n");

    prompt.push_str("Hard rule:\n");
    prompt.push_str("- Choose EXACTLY ONE backend.\n");
    prompt.push_str(
        "- If backend is procedural: output ONLY proc: lines. No text: or effect lines.\n",
    );
    prompt.push_str("- If backend is onnx/system: output text: and effect lines. Do NOT output proc:.\n\n");

    prompt.push_str("Rules for text: lines:\n");
    prompt.push_str("- Keep the same meaning and language as the user's message.\n");
    prompt.push_str("- You MAY rephrase, add/remove a few words, or add short interjections to match the object's voice.\n");
    prompt.push_str("- Do not add new facts, instructions, or unrelated content.\n");
    prompt.push_str("- Keep it concise (roughly <= 1.5x the user's message length).\n");
    prompt.push_str("- Minor punctuation and pauses are allowed.\n\n");
    prompt.push_str("Style hints (optional):\n");
    prompt.push_str("- animals: you may add a short, pronounceable interjection (with vowels) like \"mrow\", \"woof\", \"rawr\".\n");
    prompt.push_str("- robot: terse phrasing, clipped pauses.\n");
    prompt.push_str("- dragon/giant: grand, ancient tone.\n");
    prompt.push_str("- fairy: bright, playful tone.\n");
    prompt.push_str("- ghost: airy, whispery tone.\n\n");

    prompt.push_str("Rules for effects:\n");
    prompt.push_str("- Be bold when needed: mythical/monster voices should use more extreme pitch/bass/reverb/distortion within the allowed ranges.\n");
    prompt.push_str("- Keep speech intelligible; avoid maxing every knob at once.\n\n");
    prompt.push_str("Preset tips (when you need strong character):\n");
    prompt.push_str("- dragon: pitch_semitones -10..-16, speed 0.7..0.95, bass_db 12..18, reverb 0.4..0.8, distortion 0.2..0.6\n");
    prompt.push_str(
        "- giant:  pitch_semitones -12..-18, speed 0.6..0.9,  bass_db 14..18, reverb 0.2..0.5\n",
    );
    prompt.push_str("- robot:  treble_db 10..18, distortion 0.25..0.7, speed 1.0..1.25, pitch_semitones -4..4\n");
    prompt.push_str(
        "- fairy:  pitch_semitones 8..16, speed 1.1..1.4, treble_db 10..18, reverb 0.2..0.6\n",
    );
    prompt.push_str(
        "- ghost:  reverb 0.6..1.0, pitch_semitones -4..-12, treble_db 6..18, bass_db -12..0\n\n",
    );

    prompt.push_str("Rules for proc: lines:\n");
    prompt.push_str("- Use sound tokens and punctuation for timing.\n");
    prompt.push_str("- Prefer tokens from: beep, boop, whirr, buzz, click, wind, whoosh, rustle, creak, rumble, crack.\n\n");

    if request.requested_backend != BackendChoice::Auto {
        prompt.push_str(&format!(
            "The user requested backend: {}. You MUST set backend: {} and only output compatible lines.\n",
            requested, requested
        ));
    } else {
        prompt.push_str("Auto selection:\n");
        prompt.push_str("- Choose backend ONLY from the Available backends list above.\n");
        prompt.push_str("- Prefer onnx for speaking voices when available (fully offline and consistent).\n");
        prompt.push_str("- Otherwise prefer system for speaking voices when available.\n");
        prompt.push_str(
            "- For robots/devices: onnx/system with robot effects, or procedural if non-speaking.\n",
        );
        prompt.push_str("- For static/nature objects (tree, mountain): prefer procedural.\n");
        prompt.push_str(
            "- For animals: prefer onnx/system (with animal-like interjections) if available; else procedural.\n",
        );
    }

    prompt
}

fn enforce_requested_backend(mut plan: RenderPlan, requested: BackendChoice) -> RenderPlan {
    let requested_backend = match requested {
        BackendChoice::Auto => return plan,
        BackendChoice::Onnx => BackendKind::Onnx,
        BackendChoice::System => BackendKind::System,
        BackendChoice::Procedural => BackendKind::Procedural,
    };

    plan.backend = requested_backend;

    match requested_backend {
        BackendKind::Procedural => {
            plan.text = None;
            if plan
                .proc
                .as_deref()
                .map(|p| p.trim().is_empty())
                .unwrap_or(true)
            {
                plan.proc = Some("wind...".to_owned());
            }
        }
        BackendKind::System | BackendKind::Onnx => {
            plan.proc = None;
        }
    }

    plan
}

fn truncate(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }

    let mut out = String::new();
    let mut chars = text.chars();

    for _ in 0..max {
        match chars.next() {
            Some(ch) => out.push(ch),
            None => return text.to_owned(),
        }
    }

    if chars.next().is_some() {
        out.push_str("...");
    }

    out
}
