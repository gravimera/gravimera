use bevy::log::{debug, error, warn};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::gen3d::agent::{
    append_agent_trace_event_v1, run_root_dir_from_pass_dir, AgentTraceEventV1,
};

use super::super::{
    GEN3D_MAX_CHAT_HISTORY_MESSAGES, GEN3D_MAX_REQUEST_IMAGES,
    GEN3D_RESPONSES_POLL_INITIAL_DELAY_MS, GEN3D_RESPONSES_POLL_MAX_DELAY_MS,
    GEN3D_RESPONSES_POLL_MAX_SECS,
};
use super::artifacts::{
    append_gen3d_run_log, write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::parse::extract_json_object;
use super::{
    set_progress, truncate_for_ui, Gen3dAiApi, Gen3dAiProgress, Gen3dAiSessionState,
    Gen3dAiTextResponse, Gen3dChatHistoryMessage,
};

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
const CURL_MAX_TIME_SECS: u32 = 600;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum ReasoningEffortRank {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
}

fn parse_reasoning_effort_rank(value: &str) -> Option<ReasoningEffortRank> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "none" => Some(ReasoningEffortRank::None),
        "low" => Some(ReasoningEffortRank::Low),
        "medium" => Some(ReasoningEffortRank::Medium),
        "high" => Some(ReasoningEffortRank::High),
        _ => None,
    }
}

fn rank_to_reasoning_effort(rank: ReasoningEffortRank) -> &'static str {
    match rank {
        ReasoningEffortRank::None => "none",
        ReasoningEffortRank::Low => "low",
        ReasoningEffortRank::Medium => "medium",
        ReasoningEffortRank::High => "high",
    }
}

pub(super) fn cap_reasoning_effort(config_effort: &str, cap: &str) -> String {
    let config_trim = config_effort.trim();
    let Some(config_rank) = parse_reasoning_effort_rank(config_trim) else {
        return config_trim.to_string();
    };
    let Some(cap_rank) = parse_reasoning_effort_rank(cap) else {
        return config_trim.to_string();
    };
    rank_to_reasoning_effort(config_rank.min(cap_rank)).to_string()
}

pub(super) fn generate_text_via_openai(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    mut session: Gen3dAiSessionState,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, String> {
    if image_paths.len() > GEN3D_MAX_REQUEST_IMAGES {
        return Err(format!(
            "Too many images: {} (max {GEN3D_MAX_REQUEST_IMAGES})",
            image_paths.len(),
        ));
    }

    let run_root_dir = run_dir.and_then(|dir| run_root_dir_from_pass_dir(dir));
    append_agent_trace_event_v1(
        run_root_dir,
        &AgentTraceEventV1::LlmRequest {
            artifact_prefix: artifact_prefix.to_string(),
            artifact_dir: run_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<none>".into()),
            model: model.to_string(),
            images: image_paths.len(),
            system_text_file: run_dir.map(|_| format!("{artifact_prefix}_system_text.txt")),
            user_text_file: run_dir.map(|_| format!("{artifact_prefix}_user_text.txt")),
        },
    );

    debug!(
        "Gen3D: starting OpenAI request (prefix={}, model={}, reasoning_effort={}, images={}, system_chars={}, user_chars={})",
        artifact_prefix,
        model,
        reasoning_effort,
        image_paths.len(),
        system_instructions.chars().count(),
        user_text.chars().count(),
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_start prefix={} model={} reasoning_effort={} images={} system_chars={} user_chars={}",
            artifact_prefix,
            model,
            reasoning_effort,
            image_paths.len(),
            system_instructions.chars().count(),
            user_text.chars().count()
        ),
    );

    let mut images = Vec::new();
    if !image_paths.is_empty() {
        set_progress(progress, "Reading images…");
        for (idx, path) in image_paths.iter().enumerate() {
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("<image>");
            set_progress(
                progress,
                format!("Reading image {}/{}: {name}", idx + 1, image_paths.len()),
            );
            let bytes = std::fs::read(path)
                .map_err(|err| format!("Failed to read image {}: {err}", path.display()))?;
            let ext = path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("png")
                .to_ascii_lowercase();
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                other => {
                    debug!("Gen3D: unsupported image extension {other}, defaulting to image/png");
                    "image/png"
                }
            };
            images.push((mime, bytes));
        }
    }

    if let Some(run_dir) = run_dir {
        write_gen3d_text_artifact(
            Some(run_dir),
            format!("{artifact_prefix}_user_text.txt"),
            user_text,
        );
        write_gen3d_text_artifact(
            Some(run_dir),
            format!("{artifact_prefix}_system_text.txt"),
            system_instructions,
        );
    }

    if base_url.starts_with("mock://gen3d") {
        #[cfg(test)]
        {
            set_progress(progress, "Mocking OpenAI…");
            let resp = mock_generate_text_via_openai(
                progress,
                session.clone(),
                system_instructions,
                user_text,
                image_paths,
                run_dir,
                artifact_prefix,
            )?;

            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "mock".into(),
                    ok: true,
                    total_tokens: resp.total_tokens,
                    error: None,
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!(
                    "request_done prefix={} api=mock tokens={}",
                    artifact_prefix,
                    resp.total_tokens.unwrap_or(0)
                ),
            );
            return Ok(resp);
        }

        #[cfg(not(test))]
        {
            return Err("mock://gen3d base_url is only supported in tests".into());
        }
    }

    let responses_summary = match openai_responses_flow(
        progress,
        &mut session,
        base_url,
        api_key,
        model,
        reasoning_effort,
        system_instructions,
        user_text,
        &images,
        image_paths,
        run_dir,
        artifact_prefix,
    ) {
        Ok(resp) => {
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "responses".into(),
                    ok: true,
                    total_tokens: resp.total_tokens,
                    error: None,
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!(
                    "request_done prefix={} api=responses tokens={}",
                    artifact_prefix,
                    resp.total_tokens.unwrap_or(0)
                ),
            );
            return Ok(resp);
        }
        Err(err) => {
            warn!(
                "Gen3D: /responses attempt failed; falling back to /chat/completions: {}",
                err.short()
            );
            debug!("Gen3D: /responses failed detail: {}", err.detail());
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "responses".into(),
                    ok: false,
                    total_tokens: None,
                    error: Some(err.short().to_string()),
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!(
                    "responses_failed prefix={} err={}",
                    artifact_prefix,
                    err.short()
                ),
            );
            err
        }
    };

    let chat_summary = match openai_chat_completions_flow(
        progress,
        &mut session,
        base_url,
        api_key,
        model,
        reasoning_effort,
        system_instructions,
        user_text,
        &images,
        image_paths,
        run_dir,
        artifact_prefix,
    ) {
        Ok(resp) => {
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "chat_completions".into(),
                    ok: true,
                    total_tokens: resp.total_tokens,
                    error: None,
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!(
                    "request_done prefix={} api=chat_completions tokens={}",
                    artifact_prefix,
                    resp.total_tokens.unwrap_or(0)
                ),
            );
            return Ok(resp);
        }
        Err(err) => {
            warn!(
                "Gen3D: /chat/completions attempt failed after /responses fallback: {}",
                err.short()
            );
            debug!("Gen3D: /chat/completions failed detail: {}", err.detail());
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "chat_completions".into(),
                    ok: false,
                    total_tokens: None,
                    error: Some(err.short().to_string()),
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!("chat_failed prefix={} err={}", artifact_prefix, err.short()),
            );
            err
        }
    };

    error!(
        "Gen3D: OpenAI request failed (both endpoints). /responses={} /chat/completions={}",
        responses_summary.short(),
        chat_summary.short()
    );
    Err(format!(
        "OpenAI request failed.\n/responses: {responses_summary}\n/chat/completions: {chat_summary}\n(See terminal logs for details.)"
    ))
}

#[cfg(test)]
fn mock_generate_text_via_openai(
    _progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    _system_instructions: &str,
    _user_text: &str,
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, String> {
    // A deterministic offline backend used by unit tests. This avoids network access and ensures
    // Gen3D agent/tool orchestration stays regression-tested.

    if !image_paths.is_empty() {
        // We don't support mock images yet; keep the mock minimal for now.
        return Err("mock://gen3d does not support images (tests should use prompt-only)".into());
    }

    let text = if artifact_prefix == "agent_step" {
        // Intentionally uses some common “wrong” arg spellings (name/component_id/base) to
        // regression-test tool-call tolerance.
        serde_json::json!({
            "version": 1,
            "status_summary": "Mock: build a warcar with a cannon via plan + components.",
            "actions": [
                {
                    "kind": "tool_call",
                    "call_id": "call_1_create_ws",
                    "tool_id": "create_workspace_v1",
                    "args": { "name": "warcar_preview", "base": "main" }
                },
                {
                    "kind": "tool_call",
                    "call_id": "call_2_set_ws",
                    "tool_id": "set_active_workspace_v1",
                    "args": { "name": "warcar_preview" }
                },
                {
                    "kind": "tool_call",
                    "call_id": "call_3_plan",
                    "tool_id": "llm_generate_plan_v1",
                    "args": {
                        "prompt": "A warcar with a cannon as weapon",
                        "style": "Voxel/Pixel Art",
                        "components": ["chassis","wheels","turret","cannon","details"]
                    }
                },
                {
                    "kind": "tool_call",
                    "call_id": "call_4_components",
                    "tool_id": "llm_generate_components_v1",
                    "args": {
                        "component_names": ["chassis","wheels","turret","cannon","details"]
                    }
                },
                {
                    "kind": "tool_call",
                    "call_id": "call_9_validate",
                    "tool_id": "validate_v1",
                    "args": {}
                },
                {
                    "kind": "tool_call",
                    "call_id": "call_10_smoke",
                    "tool_id": "smoke_check_v1",
                    "args": {}
                },
                {
                    "kind": "done",
                    "reason": "Mock Gen3D build completed."
                }
            ]
        })
        .to_string()
    } else if artifact_prefix.starts_with("tool_plan_") {
        serde_json::json!({
            "version": 6,
            "mobility": { "kind": "ground", "max_speed": 6.0 },
            "attack": {
                "kind": "ranged_projectile",
                "cooldown_secs": 0.8,
                "muzzle": { "component": "cannon", "anchor": "muzzle" },
                "projectile": {
                    "shape": "sphere",
                    "radius": 0.12,
                    "color": [1.0, 0.75, 0.2, 1.0],
                    "unlit": true,
                    "speed": 26.0,
                    "ttl_secs": 2.0,
                    "damage": 5,
                    "obstacle_rule": "bullets_blockers",
                    "spawn_energy_impact": false
                }
            },
            "assembly_notes": "Mock plan: keep structure simple and readable.",
            "root_component": "chassis",
            "components": [
                {
                    "name": "chassis",
                    "purpose": "Main armored body of the warcar.",
                    "modeling_notes": "Chunky low-poly proportions. Serves as the root component.",
                    "size": [4.0, 1.4, 7.0],
                    "anchors": [],
                    "attach_to": null
                },
                {
                    "name": "wheels",
                    "purpose": "Four big off-road wheels.",
                    "modeling_notes": "Simple cylinders; readable tread.",
                    "size": [5.0, 1.2, 7.5],
                    "anchors": [],
                    "attach_to": {
                        "parent": "chassis",
                        "parent_anchor": "origin",
                        "child_anchor": "origin",
                        "offset": { "pos": [0.0, -0.8, 0.0], "scale": [1.0, 1.0, 1.0] }
                    }
                },
                {
                    "name": "turret",
                    "purpose": "Roof turret base.",
                    "modeling_notes": "Low profile ring + mount.",
                    "size": [2.0, 0.8, 2.0],
                    "anchors": [],
                    "attach_to": {
                        "parent": "chassis",
                        "parent_anchor": "origin",
                        "child_anchor": "origin",
                        "offset": { "pos": [0.0, 1.1, -1.2], "scale": [1.0, 1.0, 1.0] }
                    }
                },
                {
                    "name": "cannon",
                    "purpose": "Cannon barrel and housing.",
                    "modeling_notes": "Short thick barrel; clearly a cannon.",
                    "size": [1.0, 1.0, 3.2],
                    "anchors": [
                        { "name": "muzzle", "pos": [0.0, 0.0, 1.6], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
                    ],
                    "attach_to": {
                        "parent": "turret",
                        "parent_anchor": "origin",
                        "child_anchor": "origin",
                        "offset": { "pos": [0.0, 0.2, 1.2], "scale": [1.0, 1.0, 1.0] }
                    }
                },
                {
                    "name": "details",
                    "purpose": "Small silhouette details (bumper/plates/exhaust).",
                    "modeling_notes": "Keep sparse; avoid clutter.",
                    "size": [4.5, 1.6, 7.5],
                    "anchors": [],
                    "attach_to": {
                        "parent": "chassis",
                        "parent_anchor": "origin",
                        "child_anchor": "origin",
                        "offset": { "pos": [0.0, 0.0, 0.0], "scale": [1.0, 1.0, 1.0] }
                    }
                }
            ]
        })
        .to_string()
    } else if artifact_prefix.starts_with("tool_component") {
        // All components use the same small primitive set; the engine maps them into the current
        // planned component via its object_id.
        serde_json::json!({
            "version": 2,
            "collider": null,
            "anchors": [],
            "parts": [
                {
                    "primitive": "cuboid",
                    "params": null,
                    "color": [0.45, 0.48, 0.52, 1.0],
                    "pos": [0.0, 0.0, 0.0],
                    "forward": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                    "scale": [1.0, 1.0, 1.0]
                },
                {
                    "primitive": "cylinder",
                    "params": null,
                    "color": [0.15, 0.15, 0.15, 1.0],
                    "pos": [0.6, -0.5, 0.9],
                    "forward": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                    "scale": [0.35, 0.35, 0.35]
                }
            ]
        })
        .to_string()
    } else {
        return Err(format!(
            "mock://gen3d has no response for artifact_prefix `{artifact_prefix}`"
        ));
    };

    if let Some(dir) = run_dir {
        write_gen3d_text_artifact(Some(dir), format!("{artifact_prefix}_mock.txt"), &text);
    }

    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::Responses,
        session,
        total_tokens: Some(123),
    })
}

pub(super) fn openai_response_status(json: &serde_json::Value) -> Option<&str> {
    json.get("status").and_then(|v| v.as_str())
}

pub(super) fn openai_response_has_pending_status(json: &serde_json::Value) -> bool {
    matches!(
        openai_response_status(json),
        Some("queued") | Some("in_progress") | Some("cancelling")
    )
}

pub(super) fn extract_openai_response_text(json: &serde_json::Value) -> Option<String> {
    let output = json.get("output")?.as_array()?;
    let mut out = String::new();
    for item in output {
        let Some(parts) = item.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for part in parts {
            if part.get("type").and_then(|v| v.as_str()) == Some("output_text") {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

fn json_to_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| (v >= 0).then_some(v as u64)))
}

fn extract_openai_total_tokens(json: &serde_json::Value) -> Option<u64> {
    let usage = json.get("usage")?;
    if let Some(value) = usage.get("total_tokens").and_then(json_to_u64) {
        return Some(value);
    }

    // Responses API typically uses `input_tokens`/`output_tokens`.
    let input = usage.get("input_tokens").and_then(json_to_u64).unwrap_or(0);
    let output = usage
        .get("output_tokens")
        .and_then(json_to_u64)
        .unwrap_or(0);
    if input.saturating_add(output) > 0 {
        return Some(input.saturating_add(output));
    }

    // Chat Completions commonly uses `prompt_tokens`/`completion_tokens`.
    let prompt = usage
        .get("prompt_tokens")
        .and_then(json_to_u64)
        .unwrap_or(0);
    let completion = usage
        .get("completion_tokens")
        .and_then(json_to_u64)
        .unwrap_or(0);
    if prompt.saturating_add(completion) > 0 {
        return Some(prompt.saturating_add(completion));
    }

    None
}

fn try_parse_openai_responses_sse(body: &str) -> Option<serde_json::Value> {
    let mut candidate: Option<serde_json::Value> = None;
    for line in body.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<serde_json::Value>(data) else {
            continue;
        };
        if let Some(resp) = json.get("response") {
            candidate = Some(resp.clone());
            continue;
        }
        if json.get("output").is_some() || json.get("id").is_some() {
            candidate = Some(json);
        }
    }
    candidate
}

fn parse_openai_responses_json(body: &str) -> Result<serde_json::Value, String> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return Err("Empty response body".into());
    }

    match serde_json::from_str::<serde_json::Value>(trimmed) {
        Ok(json) => return Ok(json),
        Err(err) => {
            // Some providers return streaming SSE even when the client doesn't request it.
            if let Some(json) = try_parse_openai_responses_sse(trimmed) {
                return Ok(json);
            }

            // Best-effort: extract the first JSON object in the payload.
            if let Some(extracted) = extract_json_object(trimmed) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(extracted.trim()) {
                    return Ok(json);
                }
            }

            Err(err.to_string())
        }
    }
}

fn openai_responses_flow(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, OpenAiError> {
    if session.responses_supported == Some(false) {
        return Err(OpenAiError::new("Responses API not supported".into()));
    }

    // Retries for transient provider/network failures. This is intentionally higher than you might
    // want for a typical request because Gen3D runs can take a long time and we prefer robustness
    // over failing an entire build on a momentary 5xx/timeout.
    const MAX_RESPONSES_RETRIES: usize = 6;

    set_progress(progress, "Calling /responses…");
    fn is_unsupported_previous_response_id(err: &OpenAiError) -> bool {
        if err.status != Some(400) {
            return false;
        }
        let Some(preview) = err.body_preview.as_deref() else {
            return false;
        };
        let preview = preview.to_ascii_lowercase();
        preview.contains("previous_response_id")
            && (preview.contains("unsupported parameter")
                || preview.contains("unknown field")
                || preview.contains("unrecognized field")
                || preview.contains("not supported"))
    }

    fn is_responses_endpoint_unsupported(err: &OpenAiError) -> bool {
        match err.status {
            Some(404 | 405 | 501) => true,
            _ => false,
        }
    }

    fn is_transient_responses_error(err: &OpenAiError) -> bool {
        match err.status {
            Some(408 | 409 | 425 | 429) => true,
            Some(code) if (500..=599).contains(&code) => true,
            _ => {
                let summary = err.summary.to_ascii_lowercase();
                summary.contains("timed out")
                    || summary.contains("timeout")
                    || summary.contains("connection reset")
                    || summary.contains("connection refused")
                    || summary.contains("failed to connect")
            }
        }
    }

    let mut previous_response_id = session.responses_previous_id.clone();
    if previous_response_id.is_some() && session.responses_continuation_supported == Some(false) {
        previous_response_id = None;
    }
    let attempted_previous_response_id = previous_response_id.is_some();
    let mut success_used_previous_response_id = attempted_previous_response_id;

    let max_attempts = 1 + MAX_RESPONSES_RETRIES;
    let mut attempt = 0usize;
    let mut retry_delay = std::time::Duration::from_millis(250);
    let mut body = loop {
        attempt = attempt.saturating_add(1);
        if attempt > 1 {
            set_progress(
                progress,
                format!("Retrying /responses… (attempt {attempt}/{max_attempts})"),
            );
        }

        match openai_responses_curl(
            progress,
            base_url,
            api_key,
            model,
            reasoning_effort,
            system_instructions,
            user_text,
            images,
            image_paths,
            previous_response_id.as_deref(),
            run_dir,
            artifact_prefix,
            false,
        ) {
            Ok(body) => {
                session.responses_supported = Some(true);
                break body;
            }
            Err(err) => {
                if attempted_previous_response_id && is_unsupported_previous_response_id(&err) {
                    warn!("Gen3D: /responses continuation unsupported (previous_response_id); retrying without it.");
                    session.responses_supported = Some(true);
                    session.responses_continuation_supported = Some(false);
                    success_used_previous_response_id = false;
                    previous_response_id = None;
                    continue;
                }

                if is_responses_endpoint_unsupported(&err) {
                    session.responses_supported = Some(false);
                    return Err(err);
                }

                if attempt < max_attempts
                    && (is_transient_responses_error(&err) || err.body_preview.is_none())
                {
                    warn!(
                        "Gen3D: /responses transient failure; will retry (attempt {}/{}) err={}",
                        attempt,
                        max_attempts,
                        err.short()
                    );
                    std::thread::sleep(retry_delay);
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(4));
                    continue;
                }

                // Keep `/responses` enabled; this may be a transient outage or provider-side issue.
                session.responses_supported = Some(true);
                return Err(err);
            }
        }
    };

    write_gen3d_text_artifact(
        run_dir,
        format!("{artifact_prefix}_responses_raw.txt"),
        &body,
    );

    let mut json: serde_json::Value = match parse_openai_responses_json(&body) {
        Ok(json) => json,
        Err(err) => {
            if attempt < max_attempts {
                warn!(
                    "Gen3D: failed to parse /responses JSON; retrying (attempt {}/{}) err={err}",
                    attempt, max_attempts
                );
                std::thread::sleep(retry_delay);
                body = openai_responses_curl(
                    progress,
                    base_url,
                    api_key,
                    model,
                    reasoning_effort,
                    system_instructions,
                    user_text,
                    images,
                    image_paths,
                    previous_response_id.as_deref(),
                    run_dir,
                    artifact_prefix,
                    false,
                )?;
                parse_openai_responses_json(&body).map_err(|err2| {
                    OpenAiError::new(format!("Failed to parse /responses JSON: {err2}"))
                })?
            } else {
                return Err(OpenAiError::new(format!(
                    "Failed to parse /responses JSON: {err}"
                )));
            }
        }
    };
    write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_responses.json"), &json);

    // Poll if in progress.
    if openai_response_has_pending_status(&json) {
        set_progress(progress, "Waiting for /responses result…");
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(GEN3D_RESPONSES_POLL_MAX_SECS);
        let mut delay = std::time::Duration::from_millis(GEN3D_RESPONSES_POLL_INITIAL_DELAY_MS);

        let id = json
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| OpenAiError::new("Missing /responses id".into()))?
            .to_string();

        loop {
            if start.elapsed() > timeout {
                return Err(OpenAiError::new(format!(
                    "/responses timed out after {}s",
                    GEN3D_RESPONSES_POLL_MAX_SECS
                )));
            }
            std::thread::sleep(delay);
            delay = (delay * 2).min(std::time::Duration::from_millis(
                GEN3D_RESPONSES_POLL_MAX_DELAY_MS,
            ));

            let url = crate::config::join_base_url(base_url, &format!("responses/{id}"));
            let poll = std::process::Command::new("curl")
                .arg("-sS")
                .arg("--connect-timeout")
                .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
                .arg("--max-time")
                .arg(CURL_MAX_TIME_SECS.to_string())
                .arg("-H")
                .arg(format!("Authorization: Bearer {api_key}"))
                .arg(&url)
                .output()
                .map_err(|err| OpenAiError::new(format!("Failed to start curl: {err}")))?;
            if !poll.status.success() {
                let stderr = String::from_utf8_lossy(&poll.stderr);
                return Err(OpenAiError::new(format!(
                    "curl exited with non-zero status:\n{stderr}"
                )));
            }
            body = String::from_utf8_lossy(&poll.stdout).to_string();
            json = parse_openai_responses_json(&body).map_err(|err| {
                OpenAiError::new(format!("Failed to parse /responses poll JSON: {err}"))
            })?;
            write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_poll.json"), &json);

            if !openai_response_has_pending_status(&json) {
                break;
            }
        }
    }

    let total_tokens = extract_openai_total_tokens(&json);
    let text = extract_openai_response_text(&json)
        .ok_or_else(|| OpenAiError::new("/responses returned no output text".into()))?;

    if success_used_previous_response_id {
        session.responses_continuation_supported = Some(true);
    }
    session.responses_previous_id = json
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::Responses,
        session: session.clone(),
        total_tokens,
    })
}

fn openai_chat_completions_flow(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, OpenAiError> {
    set_progress(progress, "Calling /chat/completions…");

    const MAX_CHAT_RETRIES: usize = 2;
    fn is_transient_chat_error(err: &OpenAiError) -> bool {
        match err.status {
            Some(408 | 409 | 425 | 429) => true,
            Some(code) if (500..=599).contains(&code) => true,
            _ => {
                let summary = err.summary.to_ascii_lowercase();
                summary.contains("timed out")
                    || summary.contains("timeout")
                    || summary.contains("connection reset")
                    || summary.contains("connection refused")
                    || summary.contains("failed to connect")
            }
        }
    }

    let max_attempts = 1 + MAX_CHAT_RETRIES;
    let mut attempt = 0usize;
    let mut retry_delay = std::time::Duration::from_millis(250);
    let json = loop {
        attempt = attempt.saturating_add(1);
        if attempt > 1 {
            set_progress(
                progress,
                format!("Retrying /chat/completions… (attempt {attempt}/{max_attempts})"),
            );
        }

        match openai_chat_completions_curl(
            progress,
            session,
            base_url,
            api_key,
            model,
            reasoning_effort,
            system_instructions,
            user_text,
            images,
            image_paths,
            run_dir,
            artifact_prefix,
        ) {
            Ok(json) => break json,
            Err(err) => {
                if attempt < max_attempts
                    && (is_transient_chat_error(&err) || err.body_preview.is_none())
                {
                    warn!(
                        "Gen3D: /chat/completions transient failure; will retry (attempt {}/{}) err={}",
                        attempt,
                        max_attempts,
                        err.short()
                    );
                    std::thread::sleep(retry_delay);
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(4));
                    continue;
                }
                return Err(err);
            }
        }
    };
    write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_chat.json"), &json);

    let text = json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| OpenAiError::new("/chat/completions returned no content".into()))?
        .to_string();

    let total_tokens = extract_openai_total_tokens(&json);
    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::ChatCompletions,
        session: session.clone(),
        total_tokens,
    })
}

#[derive(Clone, Debug)]
struct OpenAiError {
    summary: String,
    url: String,
    status: Option<u16>,
    body_preview: Option<String>,
}

impl OpenAiError {
    fn new(summary: String) -> Self {
        Self {
            summary,
            url: String::new(),
            status: None,
            body_preview: None,
        }
    }

    fn short(&self) -> String {
        if self.url.is_empty() {
            return self.summary.clone();
        }
        match self.status {
            Some(code) => format!("{} (url={}, status={})", self.summary, self.url, code),
            None => format!("{} (url={})", self.summary, self.url),
        }
    }

    fn detail(&self) -> String {
        let mut out = self.short();
        if let Some(preview) = &self.body_preview {
            out.push_str(&format!("\nbody_preview: {preview}"));
        }
        out
    }
}

impl std::fmt::Display for OpenAiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short())
    }
}

impl std::error::Error for OpenAiError {}

fn openai_responses_curl(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    previous_response_id: Option<&str>,
    run_dir: Option<&Path>,
    artifact_prefix: &str,
    probe_only: bool,
) -> Result<String, OpenAiError> {
    let url = crate::config::join_base_url(base_url, "responses");
    let mut input = serde_json::json!({
      "model": model,
      "stream": false,
      "input": [
        {
          "role": "system",
          "content": [
            {"type":"input_text", "text": system_instructions}
          ]
        },
        {
          "role": "user",
          "content": [
            {"type":"input_text", "text": user_text}
          ]
        }
      ],
    });
    if let Some(previous) = previous_response_id {
        input["previous_response_id"] = serde_json::json!(previous);
    }
    if reasoning_effort.trim() != "none" {
        input["reasoning"] = serde_json::json!({ "effort": reasoning_effort });
    }

    let content = input["input"][1]["content"].as_array_mut().unwrap();
    for (idx, (mime, bytes)) in images.iter().enumerate() {
        let b64 = base64_encode(bytes);
        let name = image_paths
            .get(idx)
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("<unknown>");
        content.push(serde_json::json!({
          "type": "input_text",
          "text": format!("Image {}: {name}", idx + 1),
        }));
        content.push(serde_json::json!({
          "type": "input_image",
          "image_url": format!("data:{mime};base64,{b64}"),
          "detail": "low",
        }));
    }

    write_gen3d_json_artifact(
        run_dir,
        format!("{artifact_prefix}_responses_request.json"),
        &input,
    );

    let body = serde_json::to_vec(&input).map_err(|err| OpenAiError::new(err.to_string()))?;
    debug!(
        "Gen3D: sending curl request (url={}, model={}, body_bytes={})",
        url,
        model,
        body.len()
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "responses_send prefix={} url={} body_bytes={} images={} previous_response_id={} probe_only={}",
            artifact_prefix,
            url,
            body.len(),
            images.len(),
            previous_response_id.unwrap_or(""),
            probe_only
        ),
    );
    set_progress(progress, "Sending request…");
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-sS")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(CURL_MAX_TIME_SECS.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg("\n__GRAVIMERA_HTTP_STATUS__%{http_code}\n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|err| OpenAiError {
        summary: format!("Failed to start curl: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
    })?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(&body).map_err(|err| OpenAiError {
            summary: format!("Failed to write request to curl stdin: {err}"),
            url: url.clone(),
            status: None,
            body_preview: None,
        })?;
    }

    let output = child.wait_with_output().map_err(|err| OpenAiError {
        summary: format!("Failed to wait for curl: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
    })?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(OpenAiError {
            summary: format!("curl exited with non-zero status:\n{stderr}"),
            url: url.clone(),
            status: None,
            body_preview: None,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    const STATUS_MARKER: &str = "\n__GRAVIMERA_HTTP_STATUS__";
    let (body, status_code) = split_curl_http_status(&stdout, STATUS_MARKER);
    let status = status_code;
    if status.is_none() {
        warn!(
            "Gen3D: missing HTTP status marker in curl output (truncated): {}",
            truncate_for_ui(&stdout, 800)
        );
        return Err(OpenAiError {
            summary: "Missing HTTP status marker in curl output.".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
        });
    }
    debug!(
        "Gen3D: curl completed (http_status={}, body_chars={})",
        status.unwrap_or(0),
        body.chars().count()
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "responses_recv prefix={} http_status={} body_chars={}",
            artifact_prefix,
            status.unwrap_or(0),
            body.chars().count()
        ),
    );

    if let Some(code) = status {
        if !(200..=299).contains(&code) {
            append_gen3d_run_log(
                run_dir,
                format!(
                    "responses_error prefix={} http_status={} body_preview={}",
                    artifact_prefix,
                    code,
                    truncate_for_ui(body.trim(), 240)
                ),
            );
            return Err(OpenAiError {
                summary: format!("HTTP {code}"),
                url: url.clone(),
                status: Some(code),
                body_preview: Some(truncate_for_ui(body.trim(), 1200)),
            });
        }
    }

    if probe_only {
        return Ok(body.trim().to_string());
    }

    Ok(body.trim().to_string())
}

fn openai_chat_completions_curl(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<serde_json::Value, OpenAiError> {
    let url = crate::config::join_base_url(base_url, "chat/completions");
    let mut messages = Vec::new();

    // A minimal session: keep some recent messages.
    if !session.chat_history.is_empty() {
        messages.extend(session.chat_history.iter().map(|m| {
            serde_json::json!({
              "role": m.role,
              "content": m.content,
            })
        }));
    }

    messages.push(serde_json::json!({
      "role": "system",
      "content": system_instructions,
    }));

    let mut user_content: Vec<serde_json::Value> = Vec::new();
    user_content.push(serde_json::json!({"type":"text","text": user_text}));
    for (idx, (mime, bytes)) in images.iter().enumerate() {
        let b64 = base64_encode(bytes);
        let name = image_paths
            .get(idx)
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("<unknown>");
        user_content.push(serde_json::json!({
          "type":"text",
          "text": format!("Image {}: {name}", idx + 1),
        }));
        user_content.push(serde_json::json!({
          "type": "image_url",
          "image_url": { "url": format!("data:{mime};base64,{b64}") },
        }));
    }
    messages.push(serde_json::json!({
      "role": "user",
      "content": user_content,
    }));

    let mut body_json = serde_json::json!({
      "model": model,
      "stream": false,
      "messages": messages,
    });
    if reasoning_effort.trim() != "none" {
        body_json["reasoning_effort"] = serde_json::json!(reasoning_effort);
    }
    write_gen3d_json_artifact(
        run_dir,
        format!("{artifact_prefix}_chat_request.json"),
        &body_json,
    );

    let body = serde_json::to_vec(&body_json).map_err(|err| OpenAiError::new(err.to_string()))?;
    debug!(
        "Gen3D: sending curl request (url={}, model={}, body_bytes={})",
        url,
        model,
        body.len()
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "chat_send prefix={} url={} body_bytes={} images={}",
            artifact_prefix,
            url,
            body.len(),
            images.len()
        ),
    );
    set_progress(progress, "Sending request…");

    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-sS")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(CURL_MAX_TIME_SECS.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(format!("Authorization: Bearer {api_key}"))
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg("\n__GRAVIMERA_HTTP_STATUS__%{http_code}\n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|err| OpenAiError {
        summary: format!("Failed to start curl: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
    })?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin.write_all(&body).map_err(|err| OpenAiError {
            summary: format!("Failed to write request to curl stdin: {err}"),
            url: url.clone(),
            status: None,
            body_preview: None,
        })?;
    }

    let output = child.wait_with_output().map_err(|err| OpenAiError {
        summary: format!("Failed to wait for curl: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
    })?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(OpenAiError {
            summary: format!("curl exited with non-zero status:\n{stderr}"),
            url: url.clone(),
            status: None,
            body_preview: None,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    const STATUS_MARKER: &str = "\n__GRAVIMERA_HTTP_STATUS__";
    let (body, status_code) = split_curl_http_status(&stdout, STATUS_MARKER);
    if status_code.is_none() {
        warn!(
            "Gen3D: missing HTTP status marker in curl GET output (truncated): {}",
            truncate_for_ui(&stdout, 800)
        );
        return Err(OpenAiError {
            summary: "Missing HTTP status marker in curl output.".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
        });
    }
    append_gen3d_run_log(
        run_dir,
        format!(
            "chat_recv prefix={} http_status={} body_chars={}",
            artifact_prefix,
            status_code.unwrap_or(0),
            body.chars().count()
        ),
    );

    if let Some(code) = status_code {
        if !(200..=299).contains(&code) {
            append_gen3d_run_log(
                run_dir,
                format!(
                    "chat_error prefix={} http_status={} body_preview={}",
                    artifact_prefix,
                    code,
                    truncate_for_ui(body.trim(), 240)
                ),
            );
            return Err(OpenAiError {
                summary: format!("HTTP {code}"),
                url: url.clone(),
                status: Some(code),
                body_preview: Some(truncate_for_ui(body.trim(), 1200)),
            });
        }
    }

    let json: serde_json::Value = serde_json::from_str(body.trim()).map_err(|err| OpenAiError {
        summary: format!("Failed to parse JSON: {err}"),
        url: url.clone(),
        status: status_code,
        body_preview: Some(truncate_for_ui(body.trim(), 1200)),
    })?;

    // Keep a short chat history to give the model some context between runs.
    let content = json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    session.chat_history.push(Gen3dChatHistoryMessage {
        role: "assistant".into(),
        content,
    });
    if session.chat_history.len() > GEN3D_MAX_CHAT_HISTORY_MESSAGES {
        let keep = GEN3D_MAX_CHAT_HISTORY_MESSAGES;
        session
            .chat_history
            .drain(0..(session.chat_history.len().saturating_sub(keep)));
    }

    Ok(json)
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    let mut chunks = bytes.chunks_exact(3);
    for chunk in &mut chunks {
        let n = ((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32);
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(TABLE[((n >> 6) & 63) as usize] as char);
        out.push(TABLE[(n & 63) as usize] as char);
    }
    let rem = chunks.remainder();
    match rem.len() {
        0 => {}
        1 => {
            let n = (rem[0] as u32) << 16;
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push('=');
            out.push('=');
        }
        2 => {
            let n = ((rem[0] as u32) << 16) | ((rem[1] as u32) << 8);
            out.push(TABLE[((n >> 18) & 63) as usize] as char);
            out.push(TABLE[((n >> 12) & 63) as usize] as char);
            out.push(TABLE[((n >> 6) & 63) as usize] as char);
            out.push('=');
        }
        _ => unreachable!(),
    }
    out
}

fn split_curl_http_status<'a>(stdout: &'a str, marker: &str) -> (&'a str, Option<u16>) {
    let Some(pos) = stdout.rfind(marker) else {
        return (stdout, None);
    };
    let (body, rest) = stdout.split_at(pos);
    let code_str = rest[marker.len()..].lines().next().unwrap_or("").trim();
    (body, code_str.parse::<u16>().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_in_progress_responses_without_output_text() {
        let json = serde_json::json!({
            "id": "resp_123",
            "status": "in_progress",
            "output": []
        });
        assert!(extract_openai_response_text(&json).is_none());
        assert_eq!(openai_response_status(&json), Some("in_progress"));
        assert!(openai_response_has_pending_status(&json));
    }
}
