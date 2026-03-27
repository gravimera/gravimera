use bevy::log::{debug, error, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gen3d::agent::{
    append_agent_trace_event_v1, run_root_dir_from_artifact_dir, AgentTraceEventV1,
};
use crate::openai_shared::{
    curl_auth_header_file, extract_openai_chat_completions_sse_last_json,
    extract_openai_chat_completions_sse_output_text, split_curl_http_status,
    CURL_HTTP_STATUS_MARKER, CURL_HTTP_STATUS_WRITEOUT_ARG,
};

use super::super::{GEN3D_MAX_CHAT_HISTORY_MESSAGES, GEN3D_MAX_REQUEST_IMAGES};
use super::artifacts::{
    append_gen3d_run_log, write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::parse::extract_json_object;
use super::structured_outputs::Gen3dAiJsonSchemaKind;
use super::{
    set_progress, truncate_for_ui, Gen3dAiApi, Gen3dAiProgress, Gen3dAiSessionState,
    Gen3dAiTextResponse, Gen3dChatHistoryMessage,
};

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
// Curl timeout strategy:
// - `--connect-timeout`: fail fast if TCP/TLS can't connect.
// - First-byte timeout: fail fast if the provider never starts sending a body.
// - Idle timeout: allow long responses if bytes keep arriving, but abort if the transfer stalls.
// - Hard timeout: absolute safety net so a single request can't monopolize the whole build budget.
const CURL_IDLE_TIMEOUT_SECS: u32 = 300;
const CURL_HARD_TIMEOUT_SECS_DEFAULT: u32 = 1_200;

#[derive(Clone, Copy, Debug)]
struct CurlByteTimeouts {
    first_byte: std::time::Duration,
    idle: std::time::Duration,
    hard: std::time::Duration,
}

fn is_cancelled(cancel: Option<&AtomicBool>) -> bool {
    match cancel {
        Some(flag) => flag.load(Ordering::Relaxed),
        None => false,
    }
}

fn sleep_with_cancel(duration: std::time::Duration, cancel: Option<&AtomicBool>) -> bool {
    if duration.is_zero() {
        return is_cancelled(cancel);
    }

    let start = std::time::Instant::now();
    let step = std::time::Duration::from_millis(50);
    loop {
        if is_cancelled(cancel) {
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

fn read_stream_to_end(
    mut reader: impl std::io::Read,
    start: std::time::Instant,
    bytes_total: Arc<AtomicU64>,
    last_activity_ms: Arc<AtomicU64>,
    saw_any_byte: Option<Arc<AtomicBool>>,
) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        out.extend_from_slice(&buf[..n]);
        bytes_total.fetch_add(n as u64, Ordering::Relaxed);
        last_activity_ms.store(start.elapsed().as_millis() as u64, Ordering::Relaxed);
        if let Some(flag) = &saw_any_byte {
            flag.store(true, Ordering::Relaxed);
        }
    }
    out
}

fn wait_curl_with_byte_timeouts(
    mut child: std::process::Child,
    stdin_body: Option<&[u8]>,
    timeouts: CurlByteTimeouts,
    cancel: Option<&AtomicBool>,
    url: &str,
) -> Result<std::process::Output, MimoError> {
    if let Some(body) = stdin_body {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(body).map_err(|err| MimoError {
                summary: format!("Failed to write request to curl stdin: {err}"),
                url: url.to_string(),
                status: None,
                body_preview: None,
                cancelled: false,
            })?;
        }
    }

    let start = std::time::Instant::now();
    let stdout_bytes_total = Arc::new(AtomicU64::new(0));
    let stdout_last_activity_ms = Arc::new(AtomicU64::new(0));
    let stdout_saw_any_byte = Arc::new(AtomicBool::new(false));

    let stderr_bytes_total = Arc::new(AtomicU64::new(0));
    let stderr_last_activity_ms = Arc::new(AtomicU64::new(0));

    let stdout = child.stdout.take().ok_or_else(|| MimoError {
        summary: "Internal error: missing curl stdout pipe".into(),
        url: url.to_string(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;
    let stderr = child.stderr.take().ok_or_else(|| MimoError {
        summary: "Internal error: missing curl stderr pipe".into(),
        url: url.to_string(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;

    let stdout_handle = {
        let bytes_total = stdout_bytes_total.clone();
        let last_activity_ms = stdout_last_activity_ms.clone();
        let saw = stdout_saw_any_byte.clone();
        std::thread::spawn(move || {
            read_stream_to_end(stdout, start, bytes_total, last_activity_ms, Some(saw))
        })
    };
    let stderr_handle = {
        let bytes_total = stderr_bytes_total.clone();
        let last_activity_ms = stderr_last_activity_ms.clone();
        std::thread::spawn(move || {
            read_stream_to_end(stderr, start, bytes_total, last_activity_ms, None)
        })
    };

    let sleep_step = std::time::Duration::from_millis(50);
    let mut status: Option<std::process::ExitStatus> = None;
    let mut timed_out_summary: Option<String> = None;
    let mut cancelled = false;

    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {}
            Err(err) => {
                let _ = child.kill();
                return Err(MimoError {
                    summary: format!("Failed to poll curl status: {err}"),
                    url: url.to_string(),
                    status: None,
                    body_preview: None,
                    cancelled: false,
                });
            }
        }

        if let Some(cancel) = cancel {
            if cancel.load(Ordering::Relaxed) {
                cancelled = true;
                break;
            }
        }

        let elapsed = start.elapsed();
        if elapsed > timeouts.hard {
            timed_out_summary = Some(format!(
                "curl timed out (hard cap {}s)",
                timeouts.hard.as_secs()
            ));
            break;
        }

        if !stdout_saw_any_byte.load(Ordering::Relaxed) {
            if elapsed > timeouts.first_byte {
                timed_out_summary = Some(format!(
                    "curl timed out waiting for first response byte ({}s)",
                    timeouts.first_byte.as_secs()
                ));
                break;
            }
        } else {
            let last_activity_ms = stdout_last_activity_ms.load(Ordering::Relaxed);
            let idle_for_ms = elapsed.as_millis() as u64;
            if idle_for_ms.saturating_sub(last_activity_ms) > timeouts.idle.as_millis() as u64 {
                timed_out_summary = Some(format!(
                    "curl timed out waiting for more bytes (idle {}s)",
                    timeouts.idle.as_secs()
                ));
                break;
            }
        }

        std::thread::sleep(sleep_step);
    }

    if cancelled {
        let _ = child.kill();
        let _ = child.wait();
        let _stdout = stdout_handle.join().unwrap_or_default();
        let _stderr = stderr_handle.join().unwrap_or_default();
        return Err(MimoError::cancelled(url.to_string()));
    }

    if let Some(summary) = timed_out_summary {
        let _ = child.kill();
        let _ = child.wait();
        let _stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();
        let bytes = stdout_bytes_total.load(Ordering::Relaxed);
        let stderr_bytes = stderr_bytes_total.load(Ordering::Relaxed);
        if stderr_bytes > 0 {
            let tail = String::from_utf8_lossy(&stderr);
            return Err(MimoError {
                summary: format!(
                    "{summary} (stdout_bytes={bytes}, stderr_tail={})",
                    truncate_for_ui(tail.trim(), 240)
                ),
                url: url.to_string(),
                status: None,
                body_preview: None,
                cancelled: false,
            });
        }
        return Err(MimoError {
            summary: format!("{summary} (stdout_bytes={bytes})"),
            url: url.to_string(),
            status: None,
            body_preview: None,
            cancelled: false,
        });
    }

    let status = status.ok_or_else(|| MimoError {
        summary: "Internal error: missing curl exit status".into(),
        url: url.to_string(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

#[derive(Clone, Debug)]
struct MimoError {
    summary: String,
    url: String,
    status: Option<u16>,
    body_preview: Option<String>,
    cancelled: bool,
}

impl MimoError {
    fn new(summary: String) -> Self {
        Self {
            summary,
            url: String::new(),
            status: None,
            body_preview: None,
            cancelled: false,
        }
    }

    fn cancelled(url: String) -> Self {
        Self {
            summary: "Cancelled".into(),
            url,
            status: None,
            body_preview: None,
            cancelled: true,
        }
    }

    fn short(&self) -> String {
        if let Some(code) = self.status {
            format!("{} ({})", self.summary.trim(), code)
        } else {
            self.summary.trim().to_string()
        }
    }

    fn detail(&self) -> String {
        let mut out = self.short();
        if !self.url.trim().is_empty() {
            out.push_str(&format!("\nurl: {}", self.url.trim()));
        }
        if let Some(preview) = &self.body_preview {
            out.push_str(&format!("\nbody_preview: {preview}"));
        }
        out
    }
}

impl std::fmt::Display for MimoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.short())
    }
}

impl std::error::Error for MimoError {}

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

fn json_to_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|v| (v >= 0).then_some(v as u64)))
}

fn extract_mimo_token_usage(json: &serde_json::Value) -> (Option<u64>, Option<u64>, Option<u64>) {
    let Some(usage) = json.get("usage") else {
        return (None, None, None);
    };

    let total_tokens = usage.get("total_tokens").and_then(json_to_u64);
    let input_tokens = usage.get("prompt_tokens").and_then(json_to_u64);
    let output_tokens = usage.get("completion_tokens").and_then(json_to_u64);

    let total_tokens = total_tokens.or_else(|| match (input_tokens, output_tokens) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        (Some(i), None) => Some(i),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    });

    (input_tokens, output_tokens, total_tokens)
}

fn mimo_error_message(json: &serde_json::Value) -> Option<&str> {
    json.get("error")?
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn build_mimo_chat_completions_request_json(
    session: &Gen3dAiSessionState,
    model: &str,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
) -> serde_json::Value {
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

    // MiMo Structured Outputs: `response_format: { "type": "json_object" }`.
    if expected_schema.is_some() {
        body_json["response_format"] = serde_json::json!({
            "type": "json_object",
        });
    }

    body_json
}

fn mimo_chat_completions_curl(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &Gen3dAiSessionState,
    cancel: Option<&AtomicBool>,
    first_byte_timeout: std::time::Duration,
    base_url: &str,
    api_key: &str,
    model: &str,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<String, MimoError> {
    let url = crate::config::join_base_url(base_url, "chat/completions");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(MimoError::cancelled(url));
        }
    }

    let body_json = build_mimo_chat_completions_request_json(
        session,
        model,
        expected_schema,
        system_instructions,
        user_text,
        images,
        image_paths,
    );
    write_gen3d_json_artifact(
        run_dir,
        format!("{artifact_prefix}_mimo_chat_request.json"),
        &body_json,
    );

    let body = serde_json::to_vec(&body_json).map_err(|err| MimoError::new(err.to_string()))?;
    debug!(
        "Gen3D: sending MiMo curl request (url={}, model={}, body_bytes={})",
        url,
        model,
        body.len()
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "mimo_chat_send prefix={} url={} body_bytes={} images={} structured_outputs={}",
            artifact_prefix,
            url,
            body.len(),
            images.len(),
            expected_schema.is_some(),
        ),
    );

    set_progress(progress, "Waiting for AI slot…");
    let _permit =
        crate::ai_limiter::acquire_permit_cancellable(cancel).map_err(|()| MimoError {
            summary: "Cancelled".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: true,
        })?;
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(MimoError::cancelled(url));
        }
    }

    set_progress(progress, "Sending request…");
    let auth_headers = curl_auth_header_file(api_key).map_err(|err| MimoError {
        summary: format!("Failed to create curl auth header file: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;

    let mut cmd = std::process::Command::new("curl");
    crate::system_proxy::apply_system_proxy_to_curl_command(&mut cmd, &url);
    cmd.arg("-sS")
        .arg("--no-buffer")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(CURL_HARD_TIMEOUT_SECS_DEFAULT.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(auth_headers.curl_header_arg())
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg(CURL_HTTP_STATUS_WRITEOUT_ARG)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = cmd.spawn().map_err(|err| MimoError {
        summary: format!("Failed to start curl: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;

    let output = wait_curl_with_byte_timeouts(
        child,
        Some(&body),
        CurlByteTimeouts {
            first_byte: first_byte_timeout,
            idle: std::time::Duration::from_secs(CURL_IDLE_TIMEOUT_SECS.into()),
            hard: std::time::Duration::from_secs(CURL_HARD_TIMEOUT_SECS_DEFAULT.into()),
        },
        cancel,
        &url,
    )?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(MimoError {
            summary: format!("curl exited with non-zero status:\n{stderr}"),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: false,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let (body, status_code) = split_curl_http_status(&stdout, CURL_HTTP_STATUS_MARKER);
    if status_code.is_none() {
        warn!(
            "Gen3D: missing HTTP status marker in MiMo curl output (truncated): {}",
            truncate_for_ui(&stdout, 800)
        );
        return Err(MimoError {
            summary: "Missing HTTP status marker in curl output.".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: false,
        });
    }
    append_gen3d_run_log(
        run_dir,
        format!(
            "mimo_chat_recv prefix={} http_status={} body_chars={}",
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
                    "mimo_chat_error prefix={} http_status={} body_preview={}",
                    artifact_prefix,
                    code,
                    truncate_for_ui(body.trim(), 240)
                ),
            );
            return Err(MimoError {
                summary: format!("HTTP {code}"),
                url: url.clone(),
                status: Some(code),
                body_preview: Some(truncate_for_ui(body.trim(), 1200)),
                cancelled: false,
            });
        }
    }

    Ok(body.trim().to_string())
}

fn mimo_chat_completions_flow(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    cancel: Option<&AtomicBool>,
    first_byte_timeout: std::time::Duration,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    base_url: &str,
    api_key: &str,
    model: &str,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, MimoError> {
    set_progress(progress, "Calling MiMo /chat/completions…");

    let url = crate::config::join_base_url(base_url, "chat/completions");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(MimoError::cancelled(url));
        }
    }

    const MAX_CHAT_RETRIES: usize = 2;
    fn is_transient_chat_error(err: &MimoError) -> bool {
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
    let body = loop {
        attempt = attempt.saturating_add(1);
        if attempt > 1 {
            set_progress(
                progress,
                format!("Retrying MiMo /chat/completions… (attempt {attempt}/{max_attempts})"),
            );
        }

        match mimo_chat_completions_curl(
            progress,
            session,
            cancel,
            first_byte_timeout,
            base_url,
            api_key,
            model,
            expected_schema,
            system_instructions,
            user_text,
            images,
            image_paths,
            run_dir,
            artifact_prefix,
        ) {
            Ok(body) => break body,
            Err(err) => {
                if err.cancelled {
                    return Err(err);
                }
                if attempt < max_attempts
                    && (is_transient_chat_error(&err) || err.body_preview.is_none())
                {
                    warn!(
                        "Gen3D: MiMo /chat/completions transient failure; will retry (attempt {}/{}) err={}",
                        attempt,
                        max_attempts,
                        err.short()
                    );
                    if sleep_with_cancel(retry_delay, cancel) {
                        return Err(MimoError::cancelled(url.clone()));
                    }
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(4));
                    continue;
                }
                return Err(err);
            }
        }
    };

    write_gen3d_text_artifact(
        run_dir,
        format!("{artifact_prefix}_mimo_chat_raw.txt"),
        &body,
    );

    let body_trim = body.trim();
    let json_opt: Option<serde_json::Value> = serde_json::from_str(body_trim).ok();

    if let Some(json) = json_opt.as_ref() {
        write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_mimo_chat.json"), json);
    }

    let sse_last_json = if json_opt.is_none() {
        extract_openai_chat_completions_sse_last_json(body_trim).or_else(|| {
            extract_json_object(body_trim).and_then(|extracted| {
                serde_json::from_str::<serde_json::Value>(extracted.trim()).ok()
            })
        })
    } else {
        None
    };
    if let Some(last) = sse_last_json.as_ref() {
        write_gen3d_json_artifact(
            run_dir,
            format!("{artifact_prefix}_mimo_chat_sse_last.json"),
            last,
        );
    }

    let (text, input_tokens, output_tokens, total_tokens) = if let Some(json) = json_opt.as_ref()
    {
        if let Some(message) = mimo_error_message(json) {
            return Err(MimoError {
                summary: format!("MiMo error: {message}"),
                url,
                status: None,
                body_preview: Some(truncate_for_ui(body_trim, 1200)),
                cancelled: false,
            });
        }

        let text = json
            .get("choices")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|v| v.as_str())
            .ok_or_else(|| MimoError::new("/chat/completions returned no content".into()))?
            .to_string();
        let (input_tokens, output_tokens, total_tokens) = extract_mimo_token_usage(json);
        (text, input_tokens, output_tokens, total_tokens)
    } else if let Some(text) = extract_openai_chat_completions_sse_output_text(body_trim) {
        let (input_tokens, output_tokens, total_tokens) = sse_last_json
            .as_ref()
            .map(extract_mimo_token_usage)
            .unwrap_or((None, None, None));
        (text, input_tokens, output_tokens, total_tokens)
    } else if let Some(message) = sse_last_json.as_ref().and_then(mimo_error_message) {
        return Err(MimoError {
            summary: format!("MiMo error: {message}"),
            url,
            status: None,
            body_preview: Some(truncate_for_ui(body_trim, 1200)),
            cancelled: false,
        });
    } else {
        return Err(MimoError {
            summary: "/chat/completions returned no content".into(),
            url,
            status: None,
            body_preview: Some(truncate_for_ui(body_trim, 1200)),
            cancelled: false,
        });
    };

    // Keep a short chat history to give the model some context between runs.
    session.chat_history.push(Gen3dChatHistoryMessage {
        role: "assistant".into(),
        content: text.clone(),
    });
    if session.chat_history.len() > GEN3D_MAX_CHAT_HISTORY_MESSAGES {
        let keep = GEN3D_MAX_CHAT_HISTORY_MESSAGES;
        session
            .chat_history
            .drain(0..(session.chat_history.len().saturating_sub(keep)));
    }

    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::ChatCompletions,
        session: session.clone(),
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

pub(super) fn generate_text_via_mimo(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    mut session: Gen3dAiSessionState,
    cancel: Option<Arc<AtomicBool>>,
    first_byte_timeout: std::time::Duration,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    _require_structured_outputs: bool,
    base_url: &str,
    api_key: &str,
    model: &str,
    _reasoning_effort: &str,
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

    let cancel = cancel.as_deref();
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
    }

    let run_root_dir = run_dir.and_then(|dir| run_root_dir_from_artifact_dir(dir));
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
        "Gen3D: starting MiMo request (prefix={}, model={}, images={}, system_chars={}, user_chars={})",
        artifact_prefix,
        model,
        image_paths.len(),
        system_instructions.chars().count(),
        user_text.chars().count(),
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_start prefix={} service=mimo model={} images={} system_chars={} user_chars={}",
            artifact_prefix,
            model,
            image_paths.len(),
            system_instructions.chars().count(),
            user_text.chars().count()
        ),
    );

    let mut images = Vec::new();
    if !image_paths.is_empty() {
        set_progress(progress, "Reading images…");
        for (idx, path) in image_paths.iter().enumerate() {
            if let Some(cancel) = cancel {
                if cancel.load(Ordering::Relaxed) {
                    return Err("Cancelled".into());
                }
            }
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

    let resp = match mimo_chat_completions_flow(
        progress,
        &mut session,
        cancel,
        first_byte_timeout,
        expected_schema,
        base_url,
        api_key,
        model,
        system_instructions,
        user_text,
        &images,
        image_paths,
        run_dir,
        artifact_prefix,
    ) {
        Ok(resp) => resp,
        Err(err) => {
            error!("Gen3D: MiMo request failed: {}", err.detail());
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "mimo_chat_completions".into(),
                    ok: false,
                    total_tokens: None,
                    error: Some(err.short()),
                },
            );
            append_gen3d_run_log(
                run_dir,
                format!(
                    "request_failed prefix={} service=mimo err={}",
                    artifact_prefix,
                    err.short()
                ),
            );
            return Err(format!(
                "MiMo request failed.\n/chat/completions: {err}\n(See terminal logs for details.)"
            ));
        }
    };

    append_agent_trace_event_v1(
        run_root_dir,
        &AgentTraceEventV1::LlmResponse {
            artifact_prefix: artifact_prefix.to_string(),
            artifact_dir: run_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<none>".into()),
            api: "mimo_chat_completions".into(),
            ok: true,
            total_tokens: resp.total_tokens,
            error: None,
        },
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_done prefix={} service=mimo api=chat_completions tokens={}",
            artifact_prefix,
            resp.total_tokens.unwrap_or(0)
        ),
    );

    Ok(resp)
}
