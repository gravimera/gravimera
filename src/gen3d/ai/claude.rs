use bevy::log::{debug, error, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gen3d::agent::{
    append_agent_trace_event_v1, run_root_dir_from_pass_dir, AgentTraceEventV1,
};
use crate::openai_shared::{
    split_curl_http_status, TempSecretFile, CURL_HTTP_STATUS_MARKER, CURL_HTTP_STATUS_WRITEOUT_ARG,
};

use super::artifacts::{
    append_gen3d_run_log, write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::structured_outputs::Gen3dAiJsonSchemaKind;
use super::{
    set_progress, truncate_for_ui, Gen3dAiApi, Gen3dAiProgress, Gen3dAiSessionState,
    Gen3dAiTextResponse,
};

use super::super::GEN3D_MAX_REQUEST_IMAGES;

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
const CURL_FIRST_BYTE_TIMEOUT_SECS: u32 = 120;
const CURL_IDLE_TIMEOUT_SECS: u32 = 300;
const CURL_HARD_TIMEOUT_SECS_DEFAULT: u32 = 1_200;

const CLAUDE_MAX_TOKENS_DEFAULT: u32 = 8_192;

#[derive(Clone, Copy, Debug)]
struct CurlByteTimeouts {
    first_byte: std::time::Duration,
    idle: std::time::Duration,
    hard: std::time::Duration,
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
) -> Result<std::process::Output, String> {
    if let Some(body) = stdin_body {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin
                .write_all(body)
                .map_err(|err| format!("Claude: failed to write request to curl stdin: {err}"))?;
        }
    }

    let start = std::time::Instant::now();
    let stdout_bytes_total = Arc::new(AtomicU64::new(0));
    let stdout_last_activity_ms = Arc::new(AtomicU64::new(0));
    let stdout_saw_any_byte = Arc::new(AtomicBool::new(false));

    let stderr_bytes_total = Arc::new(AtomicU64::new(0));
    let stderr_last_activity_ms = Arc::new(AtomicU64::new(0));

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| format!("Claude: internal error: missing curl stdout pipe ({url})"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("Claude: internal error: missing curl stderr pipe ({url})"))?;

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

    loop {
        match child.try_wait() {
            Ok(Some(s)) => {
                status = Some(s);
                break;
            }
            Ok(None) => {}
            Err(err) => {
                let _ = child.kill();
                return Err(format!("Claude: failed to poll curl status ({url}): {err}"));
            }
        }

        if let Some(cancel) = cancel {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                let _stdout = stdout_handle.join().unwrap_or_default();
                let _stderr = stderr_handle.join().unwrap_or_default();
                return Err("Cancelled".into());
            }
        }

        let elapsed = start.elapsed();
        if elapsed > timeouts.hard {
            timed_out_summary = Some(format!(
                "Claude: curl timed out (hard cap {}s)",
                timeouts.hard.as_secs()
            ));
            break;
        }

        if !stdout_saw_any_byte.load(Ordering::Relaxed) {
            if elapsed > timeouts.first_byte {
                timed_out_summary = Some(format!(
                    "Claude: curl timed out waiting for first response byte ({}s)",
                    timeouts.first_byte.as_secs()
                ));
                break;
            }
        } else {
            let elapsed_ms = elapsed.as_millis() as u64;
            let last_ms = stdout_last_activity_ms.load(Ordering::Relaxed);
            let since_last_ms = elapsed_ms.saturating_sub(last_ms);
            if since_last_ms > timeouts.idle.as_millis() as u64 {
                timed_out_summary = Some(format!(
                    "Claude: curl timed out waiting for more bytes (idle {}s)",
                    timeouts.idle.as_secs()
                ));
                break;
            }
        }

        std::thread::sleep(sleep_step);
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
            return Err(format!(
                "{summary} (stdout_bytes={bytes}, stderr_tail={})",
                truncate_for_ui(tail.trim(), 240)
            ));
        }
        return Err(format!("{summary} (stdout_bytes={bytes})"));
    }

    let status = status
        .ok_or_else(|| format!("Claude: internal error: missing curl exit status ({url})"))?;
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn curl_x_api_key_header_file(api_key: &str) -> Result<TempSecretFile, std::io::Error> {
    // IMPORTANT: do not pass secrets on the curl command line (visible via `ps`).
    // Use `curl -H @file` so argv contains only the temp file path.
    let api_key = api_key.replace(['\n', '\r'], "");
    let headers = format!("x-api-key: {api_key}\n");
    TempSecretFile::create("claude_auth", &headers)
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

fn extract_claude_stream_output(body: &str) -> (Option<String>, Option<u64>) {
    let mut out = String::new();
    let mut input_tokens: Option<u64> = None;
    let mut output_tokens: Option<u64> = None;

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

        let ty = json.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match ty {
            "message_start" => {
                if let Some(usage) = json.get("message").and_then(|v| v.get("usage")) {
                    if input_tokens.is_none() {
                        input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64());
                    }
                    if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        output_tokens = Some(output_tokens.unwrap_or(0).max(v));
                    }
                }
            }
            "message_delta" => {
                if let Some(usage) = json.get("usage") {
                    if input_tokens.is_none() {
                        input_tokens = usage.get("input_tokens").and_then(|v| v.as_u64());
                    }
                    if let Some(v) = usage.get("output_tokens").and_then(|v| v.as_u64()) {
                        output_tokens = Some(output_tokens.unwrap_or(0).max(v));
                    }
                }
            }
            "content_block_start" => {
                if let Some(block) = json.get("content_block") {
                    if block.get("type").and_then(|v| v.as_str()) == Some("text") {
                        if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                            out.push_str(text);
                        }
                    }
                }
            }
            "content_block_delta" => {
                if let Some(delta) = json.get("delta") {
                    if let Some(text) = delta.get("text").and_then(|v| v.as_str()) {
                        out.push_str(text);
                    }
                }
            }
            _ => {}
        }
    }

    let total_tokens = match (input_tokens, output_tokens) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        (Some(i), None) => Some(i),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    };

    ((!out.trim().is_empty()).then_some(out), total_tokens)
}

pub(super) fn generate_text_via_claude(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    cancel: Option<Arc<AtomicBool>>,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
    base_url: &str,
    api_key: &str,
    model: &str,
    system_instructions: &str,
    user_text: &str,
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<Gen3dAiTextResponse, String> {
    if require_structured_outputs && expected_schema.is_some() {
        return Err("Gen3D requires strict Structured Outputs, but the Claude backend does not yet support schema-enforced JSON outputs. Use the OpenAI backend or disable [gen3d].require_structured_outputs.".into());
    }

    if image_paths.len() > GEN3D_MAX_REQUEST_IMAGES {
        return Err(format!(
            "Too many images: {} (max {GEN3D_MAX_REQUEST_IMAGES})",
            image_paths.len(),
        ));
    }

    let cancel_flag = cancel.as_deref();
    if let Some(cancel) = cancel_flag {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
    }

    let url = crate::config::join_base_url(base_url, "messages");
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
        "Gen3D: starting Claude request (prefix={}, model={}, images={}, system_chars={}, user_chars={})",
        artifact_prefix,
        model,
        image_paths.len(),
        system_instructions.chars().count(),
        user_text.chars().count(),
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_start prefix={} service=claude model={} images={} url={}",
            artifact_prefix,
            model,
            image_paths.len(),
            url
        ),
    );

    let mut images = Vec::new();
    if !image_paths.is_empty() {
        set_progress(progress, "Reading images…");
        for (idx, path) in image_paths.iter().enumerate() {
            if let Some(cancel) = cancel_flag {
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

    if base_url.starts_with("mock://gen3d") {
        #[cfg(any(test, debug_assertions))]
        {
            set_progress(progress, "Mocking Claude…");
            let text = format!(
                "{{\"version\":1,\"mock\":true,\"service\":\"claude\",\"echo\":{}}}",
                serde_json::to_string(user_text).unwrap_or_else(|_| "\"\"".into())
            );
            let resp = Gen3dAiTextResponse {
                text,
                api: Gen3dAiApi::ClaudeMessages,
                session,
                total_tokens: Some(1),
            };
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
                    "request_done prefix={} service=claude api=mock tokens={}",
                    artifact_prefix,
                    resp.total_tokens.unwrap_or(0)
                ),
            );
            return Ok(resp);
        }

        #[cfg(not(any(test, debug_assertions)))]
        {
            return Err("mock://gen3d base_url is only supported in tests and debug builds".into());
        }
    }

    set_progress(progress, "Requesting Claude…");

    let mut parts: Vec<serde_json::Value> = Vec::new();
    parts.push(serde_json::json!({ "type": "text", "text": user_text }));
    for (idx, (mime, bytes)) in images.iter().enumerate() {
        let b64 = base64_encode(bytes);
        let name = image_paths
            .get(idx)
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("<unknown>");
        parts.push(serde_json::json!({
            "type": "text",
            "text": format!("Image {}: {name}", idx + 1),
        }));
        parts.push(serde_json::json!({
            "type": "image",
            "source": {
                "type": "base64",
                "media_type": mime,
                "data": b64,
            }
        }));
    }

    let mut req = serde_json::json!({
        "model": model,
        "max_tokens": CLAUDE_MAX_TOKENS_DEFAULT,
        "stream": true,
        "temperature": 0.2,
        "messages": [
            {
                "role": "user",
                "content": parts,
            }
        ]
    });
    if !system_instructions.trim().is_empty() {
        req["system"] = serde_json::json!(system_instructions);
    }

    write_gen3d_json_artifact(
        run_dir,
        format!("{artifact_prefix}_claude_request.json"),
        &req,
    );

    let body =
        serde_json::to_vec(&req).map_err(|err| format!("Claude: failed to encode JSON: {err}"))?;
    debug!(
        "Gen3D: sending Claude curl request (url={}, model={}, body_bytes={})",
        url,
        model,
        body.len()
    );

    let headers = curl_x_api_key_header_file(api_key)
        .map_err(|err| format!("Claude: failed to create curl auth header file: {err}"))?;

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
        .arg(headers.curl_header_arg())
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg(CURL_HTTP_STATUS_WRITEOUT_ARG)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|err| format!("Claude: failed to start curl: {err}"))?;

    let output = wait_curl_with_byte_timeouts(
        child,
        Some(&body),
        CurlByteTimeouts {
            first_byte: std::time::Duration::from_secs(CURL_FIRST_BYTE_TIMEOUT_SECS.into()),
            idle: std::time::Duration::from_secs(CURL_IDLE_TIMEOUT_SECS.into()),
            hard: std::time::Duration::from_secs(CURL_HARD_TIMEOUT_SECS_DEFAULT.into()),
        },
        cancel_flag,
        &url,
    )?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        append_gen3d_run_log(
            run_dir,
            format!(
                "claude_curl_nonzero prefix={} status={} stderr_tail={}",
                artifact_prefix,
                output.status,
                truncate_for_ui(stderr.trim(), 240)
            ),
        );
        return Err(format!(
            "Claude curl exited with non-zero status.\nURL: {url}\n{stderr}"
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let (body, status_code) = split_curl_http_status(&stdout, CURL_HTTP_STATUS_MARKER);
    if status_code.is_none() {
        warn!(
            "Gen3D: missing HTTP status marker in Claude curl output (truncated): {}",
            truncate_for_ui(stdout.trim(), 240)
        );
    }
    append_gen3d_run_log(
        run_dir,
        format!(
            "claude_recv prefix={} http_status={} body_chars={}",
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
                    "claude_error prefix={} http_status={} body_preview={}",
                    artifact_prefix,
                    code,
                    truncate_for_ui(body.trim(), 240)
                ),
            );
            append_agent_trace_event_v1(
                run_root_dir,
                &AgentTraceEventV1::LlmResponse {
                    artifact_prefix: artifact_prefix.to_string(),
                    artifact_dir: run_dir
                        .map(|d| d.display().to_string())
                        .unwrap_or_else(|| "<none>".into()),
                    api: "claude".into(),
                    ok: false,
                    total_tokens: None,
                    error: Some(format!("HTTP {code}")),
                },
            );
            return Err(format!(
                "Claude request failed (HTTP {code}).\nURL: {url}\n{}",
                truncate_for_ui(body.trim(), 1200)
            ));
        }
    }

    if let Some(run_dir) = run_dir {
        write_gen3d_text_artifact(
            Some(run_dir),
            format!("{artifact_prefix}_claude_raw.txt"),
            body,
        );
    }

    let (text_opt, total_tokens) = extract_claude_stream_output(body);
    let text = text_opt.ok_or_else(|| {
        error!(
            "Gen3D: Claude stream returned no output text (prefix={}, http_status={})",
            artifact_prefix,
            status_code.unwrap_or(0)
        );
        format!(
            "Claude returned no output text. (HTTP {})",
            status_code.unwrap_or(0)
        )
    })?;

    append_agent_trace_event_v1(
        run_root_dir,
        &AgentTraceEventV1::LlmResponse {
            artifact_prefix: artifact_prefix.to_string(),
            artifact_dir: run_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<none>".into()),
            api: "claude".into(),
            ok: true,
            total_tokens,
            error: None,
        },
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_done prefix={} service=claude api=messages tokens={}",
            artifact_prefix,
            total_tokens.unwrap_or(0)
        ),
    );

    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::ClaudeMessages,
        session,
        total_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::extract_claude_stream_output;

    #[test]
    fn parses_stream_output_text_from_sse() {
        let body = r#"
event: message_start
data: {"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":0}}}

event: content_block_start
data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}

event: content_block_delta
data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":" world"}}

event: message_delta
data: {"type":"message_delta","usage":{"output_tokens":42}}

event: message_stop
data: {"type":"message_stop"}
        "#;
        let (text, tokens) = extract_claude_stream_output(body);
        assert_eq!(text.unwrap(), "Hello world");
        assert_eq!(tokens, Some(52));
    }
}
