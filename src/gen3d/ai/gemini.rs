use bevy::log::{debug, error, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gen3d::agent::{
    append_agent_trace_event_v1, run_root_dir_from_artifact_dir, AgentTraceEventV1,
};
use crate::openai_shared::{
    split_curl_http_status, TempSecretFile, CURL_HTTP_STATUS_MARKER, CURL_HTTP_STATUS_WRITEOUT_ARG,
};

use super::artifacts::{
    append_gen3d_run_log, write_gen3d_json_artifact, write_gen3d_text_artifact,
};
use super::structured_outputs::{json_schema_spec, Gen3dAiJsonSchemaKind};
use super::{
    set_progress, truncate_for_ui, Gen3dAiApi, Gen3dAiProgress, Gen3dAiSessionState,
    Gen3dAiTextResponse,
};

use super::super::GEN3D_MAX_REQUEST_IMAGES;

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
const CURL_IDLE_TIMEOUT_SECS: u32 = 300;
const CURL_HARD_TIMEOUT_SECS_DEFAULT: u32 = 1_200;

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
                .map_err(|err| format!("Gemini: failed to write request to curl stdin: {err}"))?;
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
        .ok_or_else(|| format!("Gemini: internal error: missing curl stdout pipe ({url})"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("Gemini: internal error: missing curl stderr pipe ({url})"))?;

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
                return Err(format!("Gemini: failed to poll curl status ({url}): {err}"));
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
                "Gemini: curl timed out (hard cap {}s)",
                timeouts.hard.as_secs()
            ));
            break;
        }

        if !stdout_saw_any_byte.load(Ordering::Relaxed) {
            if elapsed > timeouts.first_byte {
                timed_out_summary = Some(format!(
                    "Gemini: curl timed out waiting for first response byte ({}s)",
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
                    "Gemini: curl timed out waiting for more bytes (idle {}s)",
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
        .ok_or_else(|| format!("Gemini: internal error: missing curl exit status ({url})"))?;
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    Ok(std::process::Output {
        status,
        stdout,
        stderr,
    })
}

fn curl_x_goog_api_key_header_file(api_key: &str) -> Result<TempSecretFile, std::io::Error> {
    // IMPORTANT: do not pass secrets on the curl command line (visible via `ps`).
    // Use `curl -H @file` so argv contains only the temp file path.
    let api_key = api_key.replace(['\n', '\r'], "");
    let headers = format!("x-goog-api-key: {api_key}\n");
    TempSecretFile::create("gemini_auth", &headers)
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

fn extract_gemini_stream_output(
    body: &str,
) -> (Option<String>, Option<u64>, Option<u64>, Option<u64>) {
    let mut out = String::new();
    let mut input_tokens = None::<u64>;
    let mut output_tokens = None::<u64>;
    let mut total_tokens = None::<u64>;

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

        if input_tokens.is_none() || output_tokens.is_none() || total_tokens.is_none() {
            if let Some(usage) = json.get("usageMetadata") {
                if input_tokens.is_none() {
                    input_tokens = usage.get("promptTokenCount").and_then(|v| v.as_u64());
                }
                if output_tokens.is_none() {
                    output_tokens = usage.get("candidatesTokenCount").and_then(|v| v.as_u64());
                }
                if total_tokens.is_none() {
                    total_tokens = usage.get("totalTokenCount").and_then(|v| v.as_u64());
                }
            }
        }

        let Some(candidates) = json.get("candidates").and_then(|v| v.as_array()) else {
            continue;
        };
        let Some(first) = candidates.first() else {
            continue;
        };

        let parts_value = first
            .get("content")
            .and_then(|v| v.get("parts"))
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Null);
        match parts_value {
            serde_json::Value::Array(parts) => {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        out.push_str(text);
                    }
                }
            }
            serde_json::Value::Object(map) => {
                if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
            _ => {}
        }
    }

    let total_tokens = total_tokens.or_else(|| match (input_tokens, output_tokens) {
        (Some(i), Some(o)) => Some(i.saturating_add(o)),
        (Some(i), None) => Some(i),
        (None, Some(o)) => Some(o),
        (None, None) => None,
    });

    (
        (!out.trim().is_empty()).then_some(out),
        input_tokens,
        output_tokens,
        total_tokens,
    )
}

fn is_gemini_structured_outputs_rejected(body: &str) -> bool {
    let body = body.to_ascii_lowercase();
    let mentions_feature = body.contains("responsejsonschema")
        || body.contains("response_json_schema")
        || body.contains("responseschema")
        || body.contains("response_schema")
        || body.contains("responsemimetype")
        || body.contains("response_mime_type")
        || body.contains("schema");
    if !mentions_feature {
        return false;
    }

    body.contains("unknown field")
        || body.contains("unrecognized field")
        || body.contains("unsupported")
        || body.contains("not supported")
        || body.contains("invalid")
        || body.contains("invalid argument")
}

fn build_gemini_stream_generate_content_request_json(
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    include_response_json_schema: bool,
    system_instructions: &str,
    user_text: &str,
    images: &[(&str, Vec<u8>)],
    image_paths: &[PathBuf],
) -> serde_json::Value {
    let mut generation_config = serde_json::json!({
        "temperature": 0.2,
    });

    if expected_schema.is_some() {
        generation_config["response_mime_type"] = serde_json::json!("application/json");
    }
    if include_response_json_schema {
        if let Some(kind) = expected_schema {
            let spec = json_schema_spec(kind);
            generation_config["response_json_schema"] = spec.schema;
        }
    }

    let mut parts: Vec<serde_json::Value> = Vec::new();
    parts.push(serde_json::json!({ "text": user_text }));
    for (idx, (mime, bytes)) in images.iter().enumerate() {
        let b64 = base64_encode(bytes);
        let name = image_paths
            .get(idx)
            .and_then(|p| p.file_name().and_then(|s| s.to_str()))
            .unwrap_or("<unknown>");
        parts.push(serde_json::json!({
            "text": format!("Image {}: {name}", idx + 1),
        }));
        parts.push(serde_json::json!({
            "inline_data": { "mime_type": mime, "data": b64 }
        }));
    }

    let mut req = serde_json::json!({
        "generationConfig": generation_config,
        "contents": [
            {
                "role": "user",
                "parts": parts,
            }
        ]
    });

    if !system_instructions.trim().is_empty() {
        req["system_instruction"] = serde_json::json!({
            "parts": [
                { "text": system_instructions }
            ]
        });
    }

    req
}

pub(super) fn generate_text_via_gemini(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    cancel: Option<Arc<AtomicBool>>,
    first_byte_timeout: std::time::Duration,
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

    let url = crate::config::join_base_url(
        base_url,
        &format!("models/{model}:streamGenerateContent?alt=sse"),
    );

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
        "Gen3D: starting Gemini request (prefix={}, model={}, images={}, system_chars={}, user_chars={})",
        artifact_prefix,
        model,
        image_paths.len(),
        system_instructions.chars().count(),
        user_text.chars().count(),
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_start prefix={} service=gemini model={} images={} url={}",
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
            set_progress(progress, "Mocking Gemini…");
            let text = format!(
                "{{\"version\":1,\"mock\":true,\"service\":\"gemini\",\"echo\":{}}}",
                serde_json::to_string(user_text).unwrap_or_else(|_| "\"\"".into())
            );
            let resp = Gen3dAiTextResponse {
                text,
                api: Gen3dAiApi::GeminiStreamGenerateContent,
                session,
                input_tokens: Some(1),
                output_tokens: Some(0),
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
                    "request_done prefix={} service=gemini api=mock tokens={}",
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

    set_progress(progress, "Requesting Gemini…");

    let headers = curl_x_goog_api_key_header_file(api_key)
        .map_err(|err| format!("Gemini: failed to create curl auth header file: {err}"))?;

    let mut include_response_json_schema = expected_schema.is_some();
    let body = loop {
        let req = build_gemini_stream_generate_content_request_json(
            expected_schema,
            include_response_json_schema,
            system_instructions,
            user_text,
            &images,
            image_paths,
        );

        let request_artifact = if include_response_json_schema {
            format!("{artifact_prefix}_gemini_request.json")
        } else {
            format!("{artifact_prefix}_gemini_request_retry_no_schema.json")
        };
        write_gen3d_json_artifact(run_dir, request_artifact, &req);

        let request_body = serde_json::to_vec(&req)
            .map_err(|err| format!("Gemini: failed to encode JSON: {err}"))?;
        debug!(
            "Gen3D: sending Gemini curl request (url={}, model={}, body_bytes={})",
            url,
            model,
            request_body.len()
        );

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
            .map_err(|err| format!("Gemini: failed to start curl: {err}"))?;

        let output = wait_curl_with_byte_timeouts(
            child,
            Some(&request_body),
            CurlByteTimeouts {
                first_byte: first_byte_timeout,
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
                    "gemini_curl_nonzero prefix={} status={} stderr_tail={}",
                    artifact_prefix,
                    output.status,
                    truncate_for_ui(stderr.trim(), 240)
                ),
            );
            return Err(format!(
                "Gemini curl exited with non-zero status.\nURL: {url}\n{stderr}"
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let (body, status_code) = split_curl_http_status(&stdout, CURL_HTTP_STATUS_MARKER);
        if status_code.is_none() {
            warn!(
                "Gen3D: missing HTTP status marker in Gemini curl output (truncated): {}",
                truncate_for_ui(stdout.trim(), 240)
            );
        }
        append_gen3d_run_log(
            run_dir,
            format!(
                "gemini_recv prefix={} http_status={} body_chars={}",
                artifact_prefix,
                status_code.unwrap_or(0),
                body.chars().count()
            ),
        );

        if let Some(code) = status_code {
            if !(200..=299).contains(&code) {
                if include_response_json_schema
                    && expected_schema.is_some()
                    && !require_structured_outputs
                    && code == 400
                    && is_gemini_structured_outputs_rejected(body)
                {
                    warn!(
                        "Gen3D: Gemini structured outputs rejected; retrying without response_json_schema."
                    );
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "gemini_retry prefix={} reason=structured_outputs_rejected http_status={} body_preview={}",
                            artifact_prefix,
                            code,
                            truncate_for_ui(body.trim(), 240)
                        ),
                    );
                    include_response_json_schema = false;
                    continue;
                }

                append_gen3d_run_log(
                    run_dir,
                    format!(
                        "gemini_error prefix={} http_status={} body_preview={}",
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
                        api: "gemini".into(),
                        ok: false,
                        total_tokens: None,
                        error: Some(format!("HTTP {code}")),
                    },
                );
                return Err(format!(
                    "Gemini request failed (HTTP {code}).\nURL: {url}\n{}",
                    truncate_for_ui(body.trim(), 1200)
                ));
            }
        }

        break body.to_string();
    };

    if let Some(run_dir) = run_dir {
        write_gen3d_text_artifact(
            Some(run_dir),
            format!("{artifact_prefix}_gemini_raw.txt"),
            &body,
        );
    }

    let (text_opt, input_tokens, output_tokens, total_tokens) = extract_gemini_stream_output(&body);
    let text = text_opt.ok_or_else(|| {
        error!(
            "Gen3D: Gemini stream returned no output text (prefix={})",
            artifact_prefix
        );
        "Gemini returned no output text.".to_string()
    })?;

    append_agent_trace_event_v1(
        run_root_dir,
        &AgentTraceEventV1::LlmResponse {
            artifact_prefix: artifact_prefix.to_string(),
            artifact_dir: run_dir
                .map(|d| d.display().to_string())
                .unwrap_or_else(|| "<none>".into()),
            api: "gemini".into(),
            ok: true,
            total_tokens,
            error: None,
        },
    );
    append_gen3d_run_log(
        run_dir,
        format!(
            "request_done prefix={} service=gemini api=stream_generate_content tokens={}",
            artifact_prefix,
            total_tokens.unwrap_or(0)
        ),
    );

    Ok(Gen3dAiTextResponse {
        text,
        api: Gen3dAiApi::GeminiStreamGenerateContent,
        session,
        input_tokens,
        output_tokens,
        total_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        build_gemini_stream_generate_content_request_json, extract_gemini_stream_output,
        Gen3dAiJsonSchemaKind,
    };

    #[test]
    fn parses_stream_output_text_from_sse() {
        let body = r#"
data: {"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}

data: {"candidates":[{"content":{"parts":[{"text":" world"}]}}],"usageMetadata":{"totalTokenCount":42}}
        "#;
        let (text, input_tokens, output_tokens, total_tokens) = extract_gemini_stream_output(body);
        assert_eq!(text.unwrap(), "Hello world");
        assert_eq!(input_tokens, None);
        assert_eq!(output_tokens, None);
        assert_eq!(total_tokens, Some(42));
    }

    #[test]
    fn structured_outputs_request_includes_response_json_schema() {
        let req = build_gemini_stream_generate_content_request_json(
            Some(Gen3dAiJsonSchemaKind::PlanV1),
            true,
            "You are a test system prompt",
            "Return a plan JSON",
            &[],
            &[],
        );
        let gen = req.get("generationConfig").expect("generationConfig");
        assert_eq!(
            gen.get("response_mime_type").and_then(|v| v.as_str()),
            Some("application/json")
        );
        assert!(gen.get("response_json_schema").is_some());
    }

    #[test]
    fn structured_outputs_retry_can_omit_response_json_schema() {
        let req = build_gemini_stream_generate_content_request_json(
            Some(Gen3dAiJsonSchemaKind::PlanV1),
            false,
            "",
            "Return a plan JSON",
            &[],
            &[],
        );
        let gen = req.get("generationConfig").expect("generationConfig");
        assert_eq!(
            gen.get("response_mime_type").and_then(|v| v.as_str()),
            Some("application/json")
        );
        assert!(gen.get("response_json_schema").is_none());
    }

    #[test]
    fn non_schema_requests_do_not_force_json_output() {
        let req = build_gemini_stream_generate_content_request_json(None, true, "", "Hi", &[], &[]);
        let gen = req.get("generationConfig").expect("generationConfig");
        assert!(gen.get("response_mime_type").is_none());
        assert!(gen.get("response_json_schema").is_none());
    }
}
