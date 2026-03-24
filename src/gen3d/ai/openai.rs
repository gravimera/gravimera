use bevy::log::{debug, error, warn};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::gen3d::agent::{
    append_agent_trace_event_v1, run_root_dir_from_pass_dir, AgentTraceEventV1,
};
use crate::openai_shared::{
    curl_auth_header_file, extract_openai_chat_completions_sse_last_json,
    extract_openai_chat_completions_sse_output_text, extract_openai_responses_output_text,
    extract_openai_responses_sse_output_text, split_curl_http_status, CURL_HTTP_STATUS_MARKER,
    CURL_HTTP_STATUS_WRITEOUT_ARG,
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
use super::structured_outputs::{json_schema_spec, Gen3dAiJsonSchemaKind};
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
const CURL_FIRST_BYTE_TIMEOUT_SECS: u32 = 120;
const CURL_IDLE_TIMEOUT_SECS: u32 = 300;
const CURL_HARD_TIMEOUT_SECS_DEFAULT: u32 = 1_200;
const CURL_HARD_TIMEOUT_SECS_STRUCTURED: u32 = 1_200;

const OPENAI_CAPABILITIES_CACHE_FILE_NAME: &str = "openai_capabilities_cache.json";
const OPENAI_CAPABILITIES_CACHE_VERSION: u32 = 1;

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
struct OpenAiCapabilitiesCacheV1 {
    version: u32,
    entries: Vec<OpenAiCapabilitiesCacheEntryV1>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize)]
struct OpenAiCapabilitiesCacheEntryV1 {
    base_url: String,
    model: String,
    responses_supported: Option<bool>,
    responses_stream_required: Option<bool>,
    responses_continuation_supported: Option<bool>,
    responses_background_supported: Option<bool>,
    responses_structured_outputs_supported: Option<bool>,
    chat_stream_required: Option<bool>,
    chat_structured_outputs_supported: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct OpenAiCapabilityFlagsSnapshot {
    responses_supported: Option<bool>,
    responses_stream_required: Option<bool>,
    responses_continuation_supported: Option<bool>,
    responses_background_supported: Option<bool>,
    responses_structured_outputs_supported: Option<bool>,
    chat_stream_required: Option<bool>,
    chat_structured_outputs_supported: Option<bool>,
}

impl OpenAiCapabilityFlagsSnapshot {
    fn from_session(session: &Gen3dAiSessionState) -> Self {
        Self {
            responses_supported: session.responses_supported,
            responses_stream_required: session.responses_stream_required,
            responses_continuation_supported: session.responses_continuation_supported,
            responses_background_supported: session.responses_background_supported,
            responses_structured_outputs_supported: session.responses_structured_outputs_supported,
            chat_stream_required: session.chat_stream_required,
            chat_structured_outputs_supported: session.chat_structured_outputs_supported,
        }
    }
}

fn openai_capabilities_cache_lock() -> &'static Mutex<()> {
    static LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn normalize_openai_base_url(value: &str) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn normalize_openai_model(value: &str) -> String {
    value.trim().to_string()
}

fn openai_capabilities_cache_path() -> PathBuf {
    crate::paths::gravimera_dir().join(OPENAI_CAPABILITIES_CACHE_FILE_NAME)
}

fn read_openai_capabilities_cache(path: &Path) -> OpenAiCapabilitiesCacheV1 {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return OpenAiCapabilitiesCacheV1 {
                version: OPENAI_CAPABILITIES_CACHE_VERSION,
                entries: Vec::new(),
            };
        }
        Err(err) => {
            warn!(
                "Gen3D: failed to read OpenAI capabilities cache {}: {err}",
                path.display()
            );
            return OpenAiCapabilitiesCacheV1 {
                version: OPENAI_CAPABILITIES_CACHE_VERSION,
                entries: Vec::new(),
            };
        }
    };

    let mut cache: OpenAiCapabilitiesCacheV1 = match serde_json::from_slice(&bytes) {
        Ok(cache) => cache,
        Err(err) => {
            warn!(
                "Gen3D: failed to parse OpenAI capabilities cache {}: {err}",
                path.display()
            );
            return OpenAiCapabilitiesCacheV1 {
                version: OPENAI_CAPABILITIES_CACHE_VERSION,
                entries: Vec::new(),
            };
        }
    };

    if cache.version != OPENAI_CAPABILITIES_CACHE_VERSION {
        warn!(
            "Gen3D: ignoring OpenAI capabilities cache {} due to version mismatch (have {}, want {})",
            path.display(),
            cache.version,
            OPENAI_CAPABILITIES_CACHE_VERSION
        );
        cache = OpenAiCapabilitiesCacheV1 {
            version: OPENAI_CAPABILITIES_CACHE_VERSION,
            entries: Vec::new(),
        };
    }

    for entry in cache.entries.iter_mut() {
        entry.base_url = normalize_openai_base_url(&entry.base_url);
        entry.model = normalize_openai_model(&entry.model);
    }

    cache
}

fn write_openai_capabilities_cache(
    path: &Path,
    cache: &OpenAiCapabilitiesCacheV1,
) -> std::io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "missing cache parent dir",
        ));
    };
    std::fs::create_dir_all(parent)?;

    let pretty = serde_json::to_vec_pretty(cache)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::Other, err))?;
    let tmp = parent.join(format!(
        "{}.tmp.{}",
        OPENAI_CAPABILITIES_CACHE_FILE_NAME,
        uuid::Uuid::new_v4()
    ));
    std::fs::write(&tmp, pretty)?;

    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path)
        }
        Err(err) => {
            let _ = std::fs::remove_file(&tmp);
            Err(err)
        }
    }
}

fn hydrate_session_capabilities_from_cache_path(
    session: &mut Gen3dAiSessionState,
    base_url: &str,
    model: &str,
    path: &Path,
) {
    let needs_hydration = session.responses_supported.is_none()
        || session.responses_stream_required.is_none()
        || session.responses_continuation_supported.is_none()
        || session.responses_background_supported.is_none()
        || session.responses_structured_outputs_supported.is_none()
        || session.chat_stream_required.is_none()
        || session.chat_structured_outputs_supported.is_none();
    if !needs_hydration {
        return;
    }

    let base_url = normalize_openai_base_url(base_url);
    let model = normalize_openai_model(model);
    if base_url.is_empty() || model.is_empty() {
        return;
    }

    let cache = read_openai_capabilities_cache(path);
    let Some(entry) = cache
        .entries
        .iter()
        .find(|e| e.base_url == base_url && e.model == model)
    else {
        return;
    };

    if session.responses_supported.is_none() {
        session.responses_supported = entry.responses_supported;
    }
    if session.responses_stream_required.is_none() {
        session.responses_stream_required = entry.responses_stream_required;
    }
    if session.responses_continuation_supported.is_none() {
        session.responses_continuation_supported = entry.responses_continuation_supported;
    }
    if session.responses_background_supported.is_none() {
        session.responses_background_supported = entry.responses_background_supported;
    }
    if session.responses_structured_outputs_supported.is_none() {
        session.responses_structured_outputs_supported =
            entry.responses_structured_outputs_supported;
    }
    if session.chat_stream_required.is_none() {
        session.chat_stream_required = entry.chat_stream_required;
    }
    if session.chat_structured_outputs_supported.is_none() {
        session.chat_structured_outputs_supported = entry.chat_structured_outputs_supported;
    }
}

fn hydrate_session_capabilities_from_cache(
    session: &mut Gen3dAiSessionState,
    base_url: &str,
    model: &str,
) {
    let path = openai_capabilities_cache_path();
    let _guard = openai_capabilities_cache_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    hydrate_session_capabilities_from_cache_path(session, base_url, model, &path);
}

fn persist_session_capabilities_to_cache_path(
    base_url: &str,
    model: &str,
    session: &Gen3dAiSessionState,
    path: &Path,
) {
    let base_url = normalize_openai_base_url(base_url);
    let model = normalize_openai_model(model);
    if base_url.is_empty() || model.is_empty() {
        return;
    }

    let mut cache = read_openai_capabilities_cache(path);

    let existing_idx = cache
        .entries
        .iter()
        .position(|e| e.base_url == base_url && e.model == model);
    let idx = match existing_idx {
        Some(idx) => idx,
        None => {
            cache.entries.push(OpenAiCapabilitiesCacheEntryV1 {
                base_url: base_url.clone(),
                model: model.clone(),
                ..Default::default()
            });
            cache.entries.len().saturating_sub(1)
        }
    };
    let Some(entry) = cache.entries.get_mut(idx) else {
        return;
    };
    entry.base_url = base_url;
    entry.model = model;
    if session.responses_supported.is_some() {
        entry.responses_supported = session.responses_supported;
    }
    if session.responses_stream_required.is_some() {
        entry.responses_stream_required = session.responses_stream_required;
    }
    if session.responses_continuation_supported.is_some() {
        entry.responses_continuation_supported = session.responses_continuation_supported;
    }
    if session.responses_background_supported.is_some() {
        entry.responses_background_supported = session.responses_background_supported;
    }
    if session.responses_structured_outputs_supported.is_some() {
        entry.responses_structured_outputs_supported =
            session.responses_structured_outputs_supported;
    }
    if session.chat_stream_required.is_some() {
        entry.chat_stream_required = session.chat_stream_required;
    }
    if session.chat_structured_outputs_supported.is_some() {
        entry.chat_structured_outputs_supported = session.chat_structured_outputs_supported;
    }

    if let Err(err) = write_openai_capabilities_cache(path, &cache) {
        warn!(
            "Gen3D: failed to persist OpenAI capabilities cache {}: {err}",
            path.display()
        );
    }
}

fn persist_session_capabilities_to_cache(
    base_url: &str,
    model: &str,
    session: &Gen3dAiSessionState,
) {
    let path = openai_capabilities_cache_path();
    let _guard = openai_capabilities_cache_lock()
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    persist_session_capabilities_to_cache_path(base_url, model, session, &path);
}

fn persist_session_capabilities_if_changed(
    base_url: &str,
    model: &str,
    before: OpenAiCapabilityFlagsSnapshot,
    session: &Gen3dAiSessionState,
) {
    let after = OpenAiCapabilityFlagsSnapshot::from_session(session);
    if after != before {
        persist_session_capabilities_to_cache(base_url, model, session);
    }
}

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
) -> Result<std::process::Output, OpenAiError> {
    if let Some(body) = stdin_body {
        if let Some(mut stdin) = child.stdin.take() {
            use std::io::Write;
            stdin.write_all(body).map_err(|err| OpenAiError {
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

    let stdout = child.stdout.take().ok_or_else(|| OpenAiError {
        summary: "Internal error: missing curl stdout pipe".into(),
        url: url.to_string(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;
    let stderr = child.stderr.take().ok_or_else(|| OpenAiError {
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
                return Err(OpenAiError {
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
            let elapsed_ms = elapsed.as_millis() as u64;
            let last_ms = stdout_last_activity_ms.load(Ordering::Relaxed);
            let since_last_ms = elapsed_ms.saturating_sub(last_ms);
            if since_last_ms > timeouts.idle.as_millis() as u64 {
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
        return Err(OpenAiError {
            summary: "Cancelled".into(),
            url: url.to_string(),
            status: None,
            body_preview: None,
            cancelled: true,
        });
    }

    if let Some(summary) = timed_out_summary {
        let _ = child.kill();
        let _ = child.wait();
        let _stdout = stdout_handle.join().unwrap_or_default();
        let stderr = stderr_handle.join().unwrap_or_default();
        let bytes = stdout_bytes_total.load(Ordering::Relaxed);
        let stderr_bytes = stderr_bytes_total.load(Ordering::Relaxed);
        let err = if stderr_bytes > 0 {
            let tail = String::from_utf8_lossy(&stderr);
            format!(
                "{summary} (stdout_bytes={bytes}, stderr_tail={})",
                truncate_for_ui(tail.trim(), 240)
            )
        } else {
            format!("{summary} (stdout_bytes={bytes})")
        };
        return Err(OpenAiError {
            summary: err,
            url: url.to_string(),
            status: None,
            body_preview: None,
            cancelled: false,
        });
    }

    let status = status.ok_or_else(|| OpenAiError {
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
    cancel: Option<Arc<AtomicBool>>,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
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

    let cancel = cancel.as_deref();
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled".into());
        }
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

    if base_url.starts_with("mock://gen3d") {
        #[cfg(any(test, debug_assertions))]
        {
            set_progress(progress, "Mocking OpenAI…");
            let resp = mock_generate_text_via_openai(
                progress,
                session.clone(),
                expected_schema,
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

        #[cfg(not(any(test, debug_assertions)))]
        {
            return Err("mock://gen3d base_url is only supported in tests and debug builds".into());
        }
    }

    hydrate_session_capabilities_from_cache(&mut session, base_url, model);
    let caps_before = OpenAiCapabilityFlagsSnapshot::from_session(&session);

    let (responses_summary, attempted_responses) = if session.responses_supported == Some(false) {
        // Avoid repeatedly attempting /responses (and logging warnings) once we've already
        // detected that the provider doesn't support it for this base_url/model pair.
        debug!(
            "Gen3D: skipping /responses (previously marked unsupported); using /chat/completions."
        );
        append_gen3d_run_log(
            run_dir,
            format!(
                "responses_skipped prefix={} reason=unsupported",
                artifact_prefix
            ),
        );
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
                error: Some("skipped_unsupported".into()),
            },
        );
        (
            OpenAiError::new("Responses API not supported".into()),
            false,
        )
    } else {
        let responses_summary = match openai_responses_flow(
            progress,
            &mut session,
            cancel,
            expected_schema,
            require_structured_outputs,
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
                persist_session_capabilities_if_changed(base_url, model, caps_before, &session);
                return Ok(resp);
            }
            Err(err) => {
                if err.cancelled {
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
                            error: Some("cancelled".into()),
                        },
                    );
                    append_gen3d_run_log(
                        run_dir,
                        format!("request_cancelled prefix={}", artifact_prefix),
                    );
                    persist_session_capabilities_if_changed(base_url, model, caps_before, &session);
                    return Err("Cancelled".into());
                }
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

        (responses_summary, true)
    };

    let chat_summary = match openai_chat_completions_flow(
        progress,
        &mut session,
        cancel,
        expected_schema,
        require_structured_outputs,
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
            persist_session_capabilities_if_changed(base_url, model, caps_before, &session);
            return Ok(resp);
        }
        Err(err) => {
            if err.cancelled {
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
                        error: Some("cancelled".into()),
                    },
                );
                append_gen3d_run_log(
                    run_dir,
                    format!("request_cancelled prefix={}", artifact_prefix),
                );
                persist_session_capabilities_if_changed(base_url, model, caps_before, &session);
                return Err("Cancelled".into());
            }
            if attempted_responses {
                warn!(
                    "Gen3D: /chat/completions attempt failed after /responses fallback: {}",
                    err.short()
                );
            } else {
                warn!(
                    "Gen3D: /chat/completions attempt failed (/responses unsupported): {}",
                    err.short()
                );
            }
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
    persist_session_capabilities_if_changed(base_url, model, caps_before, &session);
    Err(format!(
        "OpenAI request failed.\n/responses: {responses_summary}\n/chat/completions: {chat_summary}\n(See terminal logs for details.)"
    ))
}

#[cfg(any(test, debug_assertions))]
fn mock_generate_text_via_openai(
    _progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: Gen3dAiSessionState,
    _expected_schema: Option<Gen3dAiJsonSchemaKind>,
    _system_instructions: &str,
    user_text: &str,
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

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum MockKind {
        Warcar,
        Snake,
        Octopus,
        Mantis,
    }

    fn mock_kind_from_text(text: &str) -> MockKind {
        let t = text.to_ascii_lowercase();
        if t.contains("octopus") {
            return MockKind::Octopus;
        }
        if t.contains("mantis") {
            return MockKind::Mantis;
        }
        if t.contains("snake") {
            return MockKind::Snake;
        }
        if t.contains("warcar") || t.contains("cannon") || t.contains("tank") {
            return MockKind::Warcar;
        }
        MockKind::Warcar
    }

    fn components_for_kind(kind: MockKind) -> Vec<&'static str> {
        match kind {
            MockKind::Warcar => vec!["chassis", "wheels", "turret", "cannon", "details"],
            MockKind::Snake => vec!["body", "seg_0", "seg_1", "seg_2", "seg_3", "seg_4", "seg_5"],
            MockKind::Octopus => vec![
                "body",
                "tentacle_0",
                "tentacle_1",
                "tentacle_2",
                "tentacle_3",
                "tentacle_4",
                "tentacle_5",
                "tentacle_6",
                "tentacle_7",
            ],
            MockKind::Mantis => vec![
                "body", "head", "arm_l", "arm_r", "leg_fl", "leg_fr", "leg_ml", "leg_mr", "leg_bl",
                "leg_br",
            ],
        }
    }

    fn plan_json_for_kind(kind: MockKind) -> serde_json::Value {
        match kind {
            MockKind::Warcar => serde_json::json!({
                "version": 8,
                "mobility": { "kind": "ground", "max_speed": 6.0 },
                "collider": { "kind": "aabb_xz", "half_extents": [2.2, 3.8] },
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
            }),
            MockKind::Snake => serde_json::json!({
                "version": 8,
                "mobility": { "kind": "ground", "max_speed": 5.0 },
                "collider": { "kind": "circle_xz", "radius": 0.6 },
                "attack": { "kind": "none" },
                "assembly_notes": "Mock plan: segmented snake body for slither motion authoring.",
                "root_component": "body",
                "components": [
                    {
                        "name": "body",
                        "purpose": "Main snake body (root).",
                        "modeling_notes": "Long, low body; keep simple.",
                        "size": [0.8, 0.6, 2.2],
                        "anchors": [],
                        "attach_to": null
                    },
                    {
                        "name": "seg_0",
                        "purpose": "Body segment 0.",
                        "modeling_notes": "Repeatable segment.",
                        "size": [0.7, 0.5, 1.0],
                        "anchors": [],
                        "attach_to": {"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.75],"scale":[1.0,1.0,1.0]}}
                    },
                    {
                        "name": "seg_1",
                        "purpose": "Body segment 1.",
                        "modeling_notes": "Repeatable segment.",
                        "size": [0.65, 0.48, 0.95],
                        "anchors": [],
                        "attach_to": {"parent":"seg_0","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.7],"scale":[1.0,1.0,1.0]}}
                    },
                    {
                        "name": "seg_2",
                        "purpose": "Body segment 2.",
                        "modeling_notes": "Repeatable segment.",
                        "size": [0.62, 0.46, 0.9],
                        "anchors": [],
                        "attach_to": {"parent":"seg_1","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.65],"scale":[1.0,1.0,1.0]}}
                    },
                    {
                        "name": "seg_3",
                        "purpose": "Body segment 3.",
                        "modeling_notes": "Repeatable segment.",
                        "size": [0.58, 0.44, 0.85],
                        "anchors": [],
                        "attach_to": {"parent":"seg_2","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.6],"scale":[1.0,1.0,1.0]}}
                    },
                    {
                        "name": "seg_4",
                        "purpose": "Body segment 4.",
                        "modeling_notes": "Repeatable segment.",
                        "size": [0.55, 0.42, 0.8],
                        "anchors": [],
                        "attach_to": {"parent":"seg_3","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.55],"scale":[1.0,1.0,1.0]}}
                    },
                    {
                        "name": "seg_5",
                        "purpose": "Body segment 5 (tail).",
                        "modeling_notes": "Slightly taper to tail.",
                        "size": [0.5, 0.4, 0.75],
                        "anchors": [],
                        "attach_to": {"parent":"seg_4","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.0,0.5],"scale":[1.0,1.0,1.0]}}
                    }
                ]
            }),
            MockKind::Octopus => serde_json::json!({
                "version": 8,
                "mobility": { "kind": "ground", "max_speed": 4.0 },
                "collider": { "kind": "circle_xz", "radius": 0.9 },
                "attack": { "kind": "melee", "cooldown_secs": 0.8, "damage": 4, "range": 1.0, "radius": 0.6, "arc_degrees": 120.0 },
                "assembly_notes": "Mock plan: octopus body + 8 tentacles for undulation motion authoring.",
                "root_component": "body",
                "components": [
                    {
                        "name": "body",
                        "purpose": "Octopus mantle/body (root).",
                        "modeling_notes": "Bulbous body; keep simple.",
                        "size": [1.4, 1.1, 1.4],
                        "anchors": [],
                        "attach_to": null
                    },
                    {
                        "name": "tentacle_0",
                        "purpose": "Tentacle 0.",
                        "modeling_notes": "Simple taper.",
                        "size": [0.35, 0.35, 1.2],
                        "anchors": [],
                        "attach_to": {"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.8,-0.3,0.4],"scale":[1.0,1.0,1.0]}}
                    },
                    { "name": "tentacle_1", "purpose":"Tentacle 1.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.9,-0.3,0.0],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_2", "purpose":"Tentacle 2.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.8,-0.3,-0.4],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_3", "purpose":"Tentacle 3.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.4,-0.3,-0.8],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_4", "purpose":"Tentacle 4.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,-0.3,-0.9],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_5", "purpose":"Tentacle 5.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.4,-0.3,-0.8],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_6", "purpose":"Tentacle 6.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.8,-0.3,-0.4],"scale":[1.0,1.0,1.0]}} },
                    { "name": "tentacle_7", "purpose":"Tentacle 7.", "modeling_notes":"Simple taper.", "size":[0.35,0.35,1.2], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.9,-0.3,0.0],"scale":[1.0,1.0,1.0]}} }
                ]
            }),
            MockKind::Mantis => serde_json::json!({
                "version": 8,
                "mobility": { "kind": "ground", "max_speed": 5.5 },
                "collider": { "kind": "circle_xz", "radius": 0.85 },
                "attack": { "kind": "melee", "cooldown_secs": 0.6, "damage": 6, "range": 1.1, "radius": 0.55, "arc_degrees": 140.0 },
                "assembly_notes": "Mock plan: mantis body with 6 legs + 2 scythe arms (requires authored motion).",
                "root_component": "body",
                "components": [
                    {
                        "name": "body",
                        "purpose": "Mantis torso (root).",
                        "modeling_notes": "Tall-ish thorax + abdomen silhouette; keep readable.",
                        "size": [1.0, 1.2, 2.0],
                        "anchors": [],
                        "attach_to": null
                    },
                    { "name":"head", "purpose":"Head.", "modeling_notes":"Small head.", "size":[0.6,0.6,0.6], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.0,0.6,0.9],"scale":[1.0,1.0,1.0]}} },
                    { "name":"arm_l", "purpose":"Left scythe arm.", "modeling_notes":"Curved blade-like forelimb.", "size":[0.3,0.8,1.0], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.55,0.3,0.6],"scale":[1.0,1.0,1.0]}} },
                    { "name":"arm_r", "purpose":"Right scythe arm.", "modeling_notes":"Curved blade-like forelimb.", "size":[0.3,0.8,1.0], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.55,0.3,0.6],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_fl", "purpose":"Front left leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.8], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.6,-0.3,0.5],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_fr", "purpose":"Front right leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.8], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.6,-0.3,0.5],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_ml", "purpose":"Mid left leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.9], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.65,-0.35,0.0],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_mr", "purpose":"Mid right leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.9], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.65,-0.35,0.0],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_bl", "purpose":"Back left leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.9], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[-0.6,-0.35,-0.55],"scale":[1.0,1.0,1.0]}} },
                    { "name":"leg_br", "purpose":"Back right leg.", "modeling_notes":"Thin leg.", "size":[0.25,0.4,0.9], "anchors":[], "attach_to":{"parent":"body","parent_anchor":"origin","child_anchor":"origin","offset":{"pos":[0.6,-0.35,-0.55],"scale":[1.0,1.0,1.0]}} }
                ]
            }),
        }
    }

    fn parse_applies_to_from_user_text(user_text: &str) -> Option<(String, u32, String, u32)> {
        let mut run_id: Option<String> = None;
        let mut attempt: Option<u32> = None;
        let mut plan_hash: Option<String> = None;
        let mut assembly_rev: Option<u32> = None;

        for line in user_text.lines().map(|l| l.trim()) {
            if let Some(v) = line.strip_prefix("- run_id:") {
                run_id = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("- attempt:") {
                attempt = v.trim().parse::<u32>().ok();
            } else if let Some(v) = line.strip_prefix("- plan_hash:") {
                plan_hash = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("- assembly_rev:") {
                assembly_rev = v.trim().parse::<u32>().ok();
            }
        }

        Some((run_id?, attempt?, plan_hash?, assembly_rev?))
    }

    fn parse_child_components_from_motion_authoring_user_text(user_text: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for line in user_text.lines().map(|l| l.trim()) {
            let Some(rest) = line.strip_prefix("- child=") else {
                continue;
            };
            let Some(name) = rest.split_whitespace().next() else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            out.push(name.to_string());
        }
        out.sort();
        out.dedup();
        out
    }

    fn parse_required_anchor_names_from_component_user_text(user_text: &str) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut in_required = false;
        for line in user_text.lines().map(|l| l.trim()) {
            if line == "Required anchors for this component (MUST include all in output JSON):" {
                in_required = true;
                continue;
            }
            if !in_required {
                continue;
            }
            if !line.starts_with("- ") {
                if line.is_empty() {
                    continue;
                }
                if !out.is_empty() {
                    break;
                }
                continue;
            }
            let rest = line.trim_start_matches("- ").trim();
            let Some((name, _)) = rest.split_once(':') else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            out.push(name.to_string());
        }
        out.sort();
        out.dedup();
        out
    }

    fn maybe_truncate_mock_response_once(
        artifact_prefix: &str,
        user_text: &str,
        text: String,
    ) -> String {
        const MARKER_INVALID_DRAFT_OPS_ALWAYS: &str = "__MOCK_INVALID_DRAFT_OPS_ALWAYS__";
        if user_text.contains(MARKER_INVALID_DRAFT_OPS_ALWAYS)
            && artifact_prefix.starts_with("tool_draft_ops_")
        {
            return "{".to_string();
        }

        const MARKER: &str = "__MOCK_TRUNCATE_ONCE__";
        if !user_text.contains(MARKER) {
            return text;
        }

        use std::collections::HashSet;
        use std::hash::{Hash, Hasher};
        use std::sync::{Mutex, OnceLock};

        static TRUNCATED_KEYS: OnceLock<Mutex<HashSet<u64>>> = OnceLock::new();

        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        artifact_prefix.hash(&mut hasher);
        user_text.hash(&mut hasher);
        let key = hasher.finish();

        let lock = TRUNCATED_KEYS.get_or_init(|| Mutex::new(HashSet::new()));
        if let Ok(mut guard) = lock.lock() {
            if guard.insert(key) {
                return text.chars().take(64).collect();
            }
        }

        text
    }

    let text = if artifact_prefix == "prompt_intent" {
        let kind = mock_kind_from_text(user_text);
        let requires_attack = !matches!(kind, MockKind::Snake);
        serde_json::json!({
            "version": 1,
            "requires_attack": requires_attack,
        })
        .to_string()
    } else if artifact_prefix.starts_with("tool_plan_ops_") {
        // A "no-op" plan-ops patch. This exercises the PlanOps parsing/apply path offline.
        serde_json::json!({ "version": 1, "ops": [] }).to_string()
    } else if artifact_prefix.starts_with("tool_review_") {
        let (run_id, attempt, plan_hash, assembly_rev) = parse_applies_to_from_user_text(user_text)
            .unwrap_or_else(|| ("".into(), 0, "".into(), 0));
        serde_json::json!({
            "version": 1,
            "applies_to": { "run_id": run_id, "attempt": attempt, "plan_hash": plan_hash, "assembly_rev": assembly_rev },
            "actions": [
                { "kind": "accept" }
            ],
            "summary": "Mock: accept review."
        })
        .to_string()
    } else if artifact_prefix.starts_with("tool_draft_ops_") {
        fn find_first_part_id_by_component(user_text: &str) -> Option<(String, String, [f32; 3])> {
            let marker = "Component parts snapshots (JSON; includes part_id_uuid + recipes):";
            let after = user_text.split(marker).nth(1)?;
            for line in after.lines() {
                let line = line.trim();
                if !line.starts_with('{') {
                    continue;
                }
                let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
                    continue;
                };
                let component = value.get("component")?.as_str()?.trim().to_string();
                if component.is_empty() {
                    continue;
                }
                let parts = value.get("parts").and_then(|v| v.as_array())?;
                for part in parts {
                    let part_id = part
                        .get("part_id_uuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    if part_id.is_empty() || part_id == "null" {
                        continue;
                    }
                    let scale = part
                        .get("transform")
                        .and_then(|v| v.get("scale"))
                        .and_then(|v| v.as_array())
                        .and_then(|arr| {
                            (arr.len() == 3).then_some([
                                arr[0].as_f64().unwrap_or(1.0) as f32,
                                arr[1].as_f64().unwrap_or(1.0) as f32,
                                arr[2].as_f64().unwrap_or(1.0) as f32,
                            ])
                        })
                        .unwrap_or([1.0, 1.0, 1.0]);
                    return Some((component, part_id, scale));
                }
            }
            None
        }

        let Some((component, part_id_uuid, scale)) = find_first_part_id_by_component(user_text)
        else {
            return Err("mock://gen3d: tool_draft_ops_* missing component parts snapshots".into());
        };
        let new_scale = [scale[0], scale[1], scale[2] * 1.25];
        serde_json::json!({
            "version": 1,
            "ops": [
                {
                    "kind": "update_primitive_part",
                    "component": component,
                    "part_id_uuid": part_id_uuid,
                    "set_transform": { "scale": new_scale }
                }
            ]
        })
        .to_string()
    } else if artifact_prefix.starts_with("tool_plan_") {
        plan_json_for_kind(mock_kind_from_text(user_text)).to_string()
    } else if artifact_prefix.starts_with("tool_component") {
        // All components use the same small primitive set; the engine maps them into the current
        // planned component via its object_id.
        let required_anchor_names = parse_required_anchor_names_from_component_user_text(user_text);
        let anchors: Vec<serde_json::Value> = required_anchor_names
            .iter()
            .map(|name| {
                serde_json::json!({
                    "name": name,
                    "pos": [0.0, 0.0, 0.0],
                    "forward": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                })
            })
            .collect();
        serde_json::json!({
            "version": 2,
            "collider": null,
            "anchors": anchors,
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
    } else if artifact_prefix.starts_with("tool_motion_") {
        let (run_id, attempt, plan_hash, assembly_rev) = parse_applies_to_from_user_text(user_text)
            .ok_or_else(|| {
                "mock://gen3d failed to parse applies_to values from motion-authoring user_text"
                    .to_string()
            })?;

        fn parse_target_channel_from_motion_authoring_user_text(user_text: &str) -> Option<String> {
            for line in user_text.lines() {
                let line = line.trim();
                let Some(rest) = line.strip_prefix("target_channel:") else {
                    continue;
                };
                let ch = rest.trim();
                if ch.is_empty() || ch == "unknown" {
                    continue;
                }
                return Some(ch.to_string());
            }
            None
        }

        let target_channel = parse_target_channel_from_motion_authoring_user_text(user_text)
            .ok_or_else(|| {
                "mock://gen3d: motion authoring user_text missing target_channel".to_string()
            })?;

        let children = parse_child_components_from_motion_authoring_user_text(user_text);
        let targets: Vec<String> = if children.is_empty() {
            Vec::new()
        } else {
            children.into_iter().take(8).collect()
        };

        if targets.is_empty() {
            serde_json::json!({
              "version": 1,
              "applies_to": { "run_id": run_id, "attempt": attempt, "plan_hash": plan_hash, "assembly_rev": assembly_rev },
              "decision": "regen_geometry_required",
              "reason": "Mock: no attachment edges available to animate.",
              "replace_channels": [],
              "edges": [],
              "notes": null
            })
            .to_string()
        } else {
            let (driver, duration_units, keyframes) = match target_channel.as_str() {
                "move" => (
                    "move_phase",
                    1.0,
                    vec![
                        serde_json::json!({"t_units": 0.0, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 0.5, "delta": {"pos": [0.03, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 1.0, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                    ],
                ),
                "action" => (
                    "action_time",
                    1.2,
                    vec![
                        serde_json::json!({"t_units": 0.0, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 0.6, "delta": {"pos": [0.0, 0.01, 0.0], "rot_quat_xyzw": [0.0, 0.0, 0.08, 0.9968], "scale": null}}),
                        serde_json::json!({"t_units": 1.2, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                    ],
                ),
                "attack_primary" => (
                    "attack_time",
                    0.35,
                    vec![
                        serde_json::json!({"t_units": 0.0, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 0.18, "delta": {"pos": [0.0, 0.0, 0.02], "rot_quat_xyzw": [0.0, 0.08, 0.0, 0.9968], "scale": null}}),
                        serde_json::json!({"t_units": 0.35, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                    ],
                ),
                _ => (
                    "always",
                    1.6,
                    vec![
                        serde_json::json!({"t_units": 0.0, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 0.8, "delta": {"pos": [0.0, 0.015, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                        serde_json::json!({"t_units": 1.6, "delta": {"pos": [0.0, 0.0, 0.0], "rot_quat_xyzw": null, "scale": null}}),
                    ],
                ),
            };

            let mut edges: Vec<serde_json::Value> = Vec::new();
            for (idx, child) in targets.iter().enumerate() {
                let phase = idx as f32 * 0.08;
                edges.push(serde_json::json!({
                    "component": child,
                    "slots": [
                        {
                            "channel": target_channel.as_str(),
                            "driver": driver,
                            "speed_scale": 1.0,
                            "time_offset_units": phase,
                            "clip": {
                                "kind": "loop",
                                "duration_units": duration_units,
                                "keyframes": keyframes.clone(),
                            }
                        }
                    ]
                }));
            }

            serde_json::json!({
              "version": 1,
              "applies_to": { "run_id": run_id, "attempt": attempt, "plan_hash": plan_hash, "assembly_rev": assembly_rev },
              "decision": "author_clips",
              "reason": format!("Mock: bake simple per-edge `{}` loops.", target_channel.as_str()),
              "replace_channels": [target_channel.as_str()],
              "edges": edges,
              "notes": null
            })
            .to_string()
        }
    } else if artifact_prefix == "descriptor_meta" {
        serde_json::json!({
            "version": 1,
            "short": "Mock prefab (Gen3D).",
            "tags": ["mock", "gen3d"]
        })
        .to_string()
    } else {
        return Err(format!(
            "mock://gen3d has no response for artifact_prefix `{artifact_prefix}`"
        ));
    };

    let text = maybe_truncate_mock_response_once(artifact_prefix, user_text, text);

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

fn is_structured_outputs_rejected(err: &OpenAiError) -> bool {
    if !matches!(err.status, Some(400 | 404 | 422)) {
        return false;
    }
    let Some(preview) = err.body_preview.as_deref() else {
        return false;
    };
    let preview = preview.to_ascii_lowercase();

    let mentions_feature = preview.contains("response_format")
        || preview.contains("text.format")
        || (preview.contains("text") && preview.contains("format"))
        || preview.contains("json_schema")
        || preview.contains("schema");
    if !mentions_feature {
        return false;
    }

    preview.contains("unsupported")
        || preview.contains("unknown field")
        || preview.contains("unrecognized field")
        || preview.contains("not supported")
        || preview.contains("invalid")
}

fn is_stream_required(err: &OpenAiError) -> bool {
    if err.status != Some(400) {
        return false;
    }
    let Some(preview) = err.body_preview.as_deref() else {
        return false;
    };
    let preview = preview.to_ascii_lowercase();

    // Common gateway error (example):
    // {"detail":"Stream must be set to true"}
    if preview.contains("stream must be set to true") {
        return true;
    }

    // More tolerant matching for variants like "stream must be true".
    preview.contains("stream") && preview.contains("must") && preview.contains("true")
}

fn openai_error_message(json: &serde_json::Value) -> Option<&str> {
    json.get("error")?
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn openai_responses_flow(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    session: &mut Gen3dAiSessionState,
    cancel: Option<&AtomicBool>,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
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

    let url = crate::config::join_base_url(base_url, "responses");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
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

    fn is_responses_background_unsupported(err: &OpenAiError) -> bool {
        if err.status != Some(400) {
            return false;
        }
        let Some(preview) = err.body_preview.as_deref() else {
            return false;
        };
        let preview = preview.to_ascii_lowercase();
        (preview.contains("background") || preview.contains("store"))
            && (preview.contains("unsupported parameter")
                || preview.contains("unknown field")
                || preview.contains("unrecognized field")
                || preview.contains("not supported")
                || preview.contains("invalid request"))
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

        let schema_for_request = if expected_schema.is_some()
            && (require_structured_outputs
                || session.responses_structured_outputs_supported != Some(false))
        {
            expected_schema
        } else {
            None
        };

        let background_for_request =
            schema_for_request.is_some() && session.responses_background_supported != Some(false);

        let stream_for_request = session.responses_stream_required == Some(true);

        match openai_responses_curl(
            progress,
            cancel,
            base_url,
            api_key,
            model,
            stream_for_request,
            reasoning_effort,
            schema_for_request,
            system_instructions,
            user_text,
            images,
            image_paths,
            previous_response_id.as_deref(),
            background_for_request,
            run_dir,
            artifact_prefix,
            false,
        ) {
            Ok(body) => {
                if body.trim().is_empty() {
                    if !stream_for_request {
                        warn!("Gen3D: /responses returned an empty body; retrying with stream=true for this session.");
                        append_gen3d_run_log(
                            run_dir,
                            format!(
                                "responses_retry prefix={} reason=empty_body_stream_required",
                                artifact_prefix
                            ),
                        );
                        session.responses_supported = Some(true);
                        session.responses_stream_required = Some(true);
                        continue;
                    }
                    session.responses_supported = Some(false);
                    return Err(OpenAiError::new("/responses returned an empty body".into()));
                }
                session.responses_supported = Some(true);
                if background_for_request {
                    session.responses_background_supported = Some(true);
                }
                if schema_for_request.is_some() {
                    session.responses_structured_outputs_supported = Some(true);
                }
                break body;
            }
            Err(err) => {
                if err.cancelled {
                    return Err(err);
                }
                if background_for_request && is_responses_background_unsupported(&err) {
                    warn!("Gen3D: /responses background unsupported; retrying without it.");
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "responses_retry prefix={} reason=background_unsupported err={}",
                            artifact_prefix,
                            err.short()
                        ),
                    );
                    session.responses_supported = Some(true);
                    session.responses_background_supported = Some(false);
                    continue;
                }

                if attempted_previous_response_id && is_unsupported_previous_response_id(&err) {
                    warn!("Gen3D: /responses continuation unsupported (previous_response_id); retrying without it.");
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "responses_retry prefix={} reason=previous_response_id_unsupported err={}",
                            artifact_prefix,
                            err.short()
                        ),
                    );
                    session.responses_supported = Some(true);
                    session.responses_continuation_supported = Some(false);
                    success_used_previous_response_id = false;
                    previous_response_id = None;
                    continue;
                }

                if !stream_for_request && is_stream_required(&err) {
                    warn!("Gen3D: /responses requires stream=true; retrying with streaming enabled for this session.");
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "responses_retry prefix={} reason=stream_required err={}",
                            artifact_prefix,
                            err.short()
                        ),
                    );
                    session.responses_supported = Some(true);
                    session.responses_stream_required = Some(true);
                    continue;
                }

                if is_responses_endpoint_unsupported(&err) {
                    session.responses_supported = Some(false);
                    return Err(err);
                }

                if schema_for_request.is_some() && is_structured_outputs_rejected(&err) {
                    session.responses_structured_outputs_supported = Some(false);
                    if require_structured_outputs {
                        return Err(OpenAiError::new(format!(
                            "Structured outputs required, but provider rejected them: {}",
                            err.short()
                        )));
                    }
                    warn!("Gen3D: /responses structured outputs rejected; retrying without structured outputs for this session.");
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "responses_retry prefix={} reason=structured_outputs_rejected err={}",
                            artifact_prefix,
                            err.short()
                        ),
                    );
                    continue;
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
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "responses_retry prefix={} reason=transient_failure attempt={}/{} delay_ms={} err={}",
                            artifact_prefix,
                            attempt,
                            max_attempts,
                            retry_delay.as_millis(),
                            err.short()
                        ),
                    );
                    if sleep_with_cancel(retry_delay, cancel) {
                        return Err(OpenAiError::cancelled(url.clone()));
                    }
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

    let sse_output_text = extract_openai_responses_sse_output_text(&body);

    let mut json: serde_json::Value = match parse_openai_responses_json(&body) {
        Ok(json) => json,
        Err(err) => {
            // If we already reconstructed an SSE output text, do not retry just to obtain the
            // surrounding JSON envelope; the text itself is sufficient.
            if sse_output_text.is_some() {
                warn!(
                    "Gen3D: failed to parse /responses JSON envelope, but SSE output text was extracted; continuing. err={err}"
                );
                append_gen3d_run_log(
                    run_dir,
                    format!(
                        "responses_parse_warning prefix={} reason=json_envelope_parse_failed err={err}",
                        artifact_prefix
                    ),
                );
                serde_json::Value::Null
            } else if attempt < max_attempts {
                warn!(
                    "Gen3D: failed to parse /responses JSON; retrying (attempt {}/{}) err={err}",
                    attempt, max_attempts
                );
                append_gen3d_run_log(
                    run_dir,
                    format!(
                        "responses_retry prefix={} reason=parse_json attempt={}/{} delay_ms={} err={err}",
                        artifact_prefix,
                        attempt,
                        max_attempts,
                        retry_delay.as_millis(),
                    ),
                );
                if sleep_with_cancel(retry_delay, cancel) {
                    return Err(OpenAiError::cancelled(url.clone()));
                }
                let schema_for_request = if expected_schema.is_some()
                    && (require_structured_outputs
                        || session.responses_structured_outputs_supported != Some(false))
                {
                    expected_schema
                } else {
                    None
                };
                let background_for_request = schema_for_request.is_some()
                    && session.responses_background_supported != Some(false);
                let stream_for_request = session.responses_stream_required == Some(true);
                body = openai_responses_curl(
                    progress,
                    cancel,
                    base_url,
                    api_key,
                    model,
                    stream_for_request,
                    reasoning_effort,
                    schema_for_request,
                    system_instructions,
                    user_text,
                    images,
                    image_paths,
                    previous_response_id.as_deref(),
                    background_for_request,
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
    if !json.is_null() {
        write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_responses.json"), &json);
    }

    if let Some(text) = sse_output_text {
        let total_tokens = extract_openai_total_tokens(&json);

        if success_used_previous_response_id {
            session.responses_continuation_supported = Some(true);
        }
        if let Some(id) = json.get("id").and_then(|v| v.as_str()) {
            session.responses_previous_id = Some(id.to_string());
        }

        return Ok(Gen3dAiTextResponse {
            text,
            api: Gen3dAiApi::Responses,
            session: session.clone(),
            total_tokens,
        });
    }

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
        let poll_headers = curl_auth_header_file(api_key).map_err(|err| {
            OpenAiError::new(format!("Failed to create curl auth header file: {err}"))
        })?;

        loop {
            if start.elapsed() > timeout {
                return Err(OpenAiError::new(format!(
                    "/responses timed out after {}s",
                    GEN3D_RESPONSES_POLL_MAX_SECS
                )));
            }
            let url = crate::config::join_base_url(base_url, &format!("responses/{id}"));
            if sleep_with_cancel(delay, cancel) {
                return Err(OpenAiError::cancelled(url));
            }
            delay = (delay * 2).min(std::time::Duration::from_millis(
                GEN3D_RESPONSES_POLL_MAX_DELAY_MS,
            ));
            if is_cancelled(cancel) {
                return Err(OpenAiError::cancelled(url));
            }
            let _permit = crate::ai_limiter::acquire_permit_cancellable(cancel).map_err(|()| {
                OpenAiError {
                    summary: "Cancelled".into(),
                    url: url.clone(),
                    status: None,
                    body_preview: None,
                    cancelled: true,
                }
            })?;
            let mut cmd = std::process::Command::new("curl");
            crate::system_proxy::apply_system_proxy_to_curl_command(&mut cmd, &url);
            cmd.arg("-sS")
                .arg("--no-buffer")
                .arg("--connect-timeout")
                .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
                .arg("--max-time")
                .arg(CURL_HARD_TIMEOUT_SECS_DEFAULT.to_string())
                .arg("-H")
                .arg(poll_headers.curl_header_arg())
                .arg(&url)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());

            let poll = cmd
                .spawn()
                .map_err(|err| OpenAiError::new(format!("Failed to start curl: {err}")))?;
            let poll = wait_curl_with_byte_timeouts(
                poll,
                None,
                CurlByteTimeouts {
                    first_byte: std::time::Duration::from_secs(CURL_FIRST_BYTE_TIMEOUT_SECS.into()),
                    idle: std::time::Duration::from_secs(CURL_IDLE_TIMEOUT_SECS.into()),
                    hard: std::time::Duration::from_secs(CURL_HARD_TIMEOUT_SECS_DEFAULT.into()),
                },
                cancel,
                &url,
            )?;
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
    let text = extract_openai_responses_output_text(&json)
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
    cancel: Option<&AtomicBool>,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
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

    let url = crate::config::join_base_url(base_url, "chat/completions");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
    }

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
    let body = loop {
        attempt = attempt.saturating_add(1);
        if attempt > 1 {
            set_progress(
                progress,
                format!("Retrying /chat/completions… (attempt {attempt}/{max_attempts})"),
            );
        }

        let schema_for_request = if expected_schema.is_some()
            && (require_structured_outputs
                || session.chat_structured_outputs_supported != Some(false))
        {
            expected_schema
        } else {
            None
        };

        let stream_for_request = session.chat_stream_required == Some(true);

        match openai_chat_completions_curl(
            progress,
            session,
            cancel,
            base_url,
            api_key,
            model,
            stream_for_request,
            reasoning_effort,
            schema_for_request,
            system_instructions,
            user_text,
            images,
            image_paths,
            run_dir,
            artifact_prefix,
        ) {
            Ok(body) => {
                if schema_for_request.is_some() {
                    session.chat_structured_outputs_supported = Some(true);
                }
                break body;
            }
            Err(err) => {
                if err.cancelled {
                    return Err(err);
                }
                if !stream_for_request && is_stream_required(&err) {
                    warn!("Gen3D: /chat/completions requires stream=true; retrying with streaming enabled for this session.");
                    append_gen3d_run_log(
                        run_dir,
                        format!(
                            "chat_retry prefix={} reason=stream_required err={}",
                            artifact_prefix,
                            err.short()
                        ),
                    );
                    session.chat_stream_required = Some(true);
                    continue;
                }
                if schema_for_request.is_some() && is_structured_outputs_rejected(&err) {
                    session.chat_structured_outputs_supported = Some(false);
                    if require_structured_outputs {
                        return Err(OpenAiError::new(format!(
                            "Structured outputs required, but provider rejected them: {}",
                            err.short()
                        )));
                    }
                    warn!("Gen3D: /chat/completions structured outputs rejected; retrying without structured outputs for this session.");
                    continue;
                }
                if attempt < max_attempts
                    && (is_transient_chat_error(&err) || err.body_preview.is_none())
                {
                    warn!(
                        "Gen3D: /chat/completions transient failure; will retry (attempt {}/{}) err={}",
                        attempt,
                        max_attempts,
                        err.short()
                    );
                    if sleep_with_cancel(retry_delay, cancel) {
                        return Err(OpenAiError::cancelled(url.clone()));
                    }
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(4));
                    continue;
                }
                return Err(err);
            }
        }
    };

    write_gen3d_text_artifact(run_dir, format!("{artifact_prefix}_chat_raw.txt"), &body);

    let body_trim = body.trim();
    let json_opt: Option<serde_json::Value> = serde_json::from_str(body_trim).ok();

    if let Some(json) = json_opt.as_ref() {
        write_gen3d_json_artifact(run_dir, format!("{artifact_prefix}_chat.json"), json);
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
            format!("{artifact_prefix}_chat_sse_last.json"),
            last,
        );
    }

    let (text, total_tokens) = if let Some(json) = json_opt.as_ref() {
        if let Some(message) = openai_error_message(json) {
            return Err(OpenAiError {
                summary: format!("OpenAI error: {message}"),
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
            .ok_or_else(|| OpenAiError::new("/chat/completions returned no content".into()))?
            .to_string();
        (text, extract_openai_total_tokens(json))
    } else if let Some(text) = extract_openai_chat_completions_sse_output_text(body_trim) {
        let total_tokens = sse_last_json.as_ref().and_then(extract_openai_total_tokens);
        (text, total_tokens)
    } else if let Some(message) = sse_last_json.as_ref().and_then(openai_error_message) {
        return Err(OpenAiError {
            summary: format!("OpenAI error: {message}"),
            url,
            status: None,
            body_preview: Some(truncate_for_ui(body_trim, 1200)),
            cancelled: false,
        });
    } else {
        return Err(OpenAiError {
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
        total_tokens,
    })
}

#[derive(Clone, Debug)]
struct OpenAiError {
    summary: String,
    url: String,
    status: Option<u16>,
    body_preview: Option<String>,
    cancelled: bool,
}

impl OpenAiError {
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

fn build_openai_responses_request_json(
    model: &str,
    stream: bool,
    reasoning_effort: &str,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    previous_response_id: Option<&str>,
    background: bool,
) -> serde_json::Value {
    let mut input = serde_json::json!({
      "model": model,
      "stream": stream,
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
    if background {
        // Background mode enables polling via `GET /responses/<id>` when the response returns
        // `status=queued|in_progress`. This avoids client-side timeouts on long structured-output
        // generations (large schemas + high reasoning effort).
        //
        // OpenAI requires `store=true` for background responses; some OpenAI-compatible providers
        // may ignore or reject these fields (handled by feature-detection in `openai_responses_flow`).
        input["background"] = serde_json::json!(true);
        input["store"] = serde_json::json!(true);
    }
    if let Some(previous) = previous_response_id {
        input["previous_response_id"] = serde_json::json!(previous);
    }
    if reasoning_effort.trim() != "none" {
        input["reasoning"] = serde_json::json!({ "effort": reasoning_effort });
    }
    if let Some(kind) = expected_schema {
        let spec = json_schema_spec(kind);
        input["text"] = serde_json::json!({
            "format": {
                "type": "json_schema",
                "name": spec.name,
                "schema": spec.schema,
                "strict": true,
            }
        });
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

    input
}

fn build_openai_chat_completions_request_json(
    session: &Gen3dAiSessionState,
    model: &str,
    stream: bool,
    reasoning_effort: &str,
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
      "stream": stream,
      "messages": messages,
    });
    if reasoning_effort.trim() != "none" {
        body_json["reasoning_effort"] = serde_json::json!(reasoning_effort);
    }
    if let Some(kind) = expected_schema {
        let spec = json_schema_spec(kind);
        body_json["response_format"] = serde_json::json!({
            "type": "json_schema",
            "json_schema": {
                "name": spec.name,
                "schema": spec.schema,
                "strict": true,
            }
        });
    }
    body_json
}

fn openai_responses_curl(
    progress: &Arc<Mutex<Gen3dAiProgress>>,
    cancel: Option<&AtomicBool>,
    base_url: &str,
    api_key: &str,
    model: &str,
    stream: bool,
    reasoning_effort: &str,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    previous_response_id: Option<&str>,
    background: bool,
    run_dir: Option<&Path>,
    artifact_prefix: &str,
    probe_only: bool,
) -> Result<String, OpenAiError> {
    let url = crate::config::join_base_url(base_url, "responses");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
    }
    let input = build_openai_responses_request_json(
        model,
        stream,
        reasoning_effort,
        expected_schema,
        system_instructions,
        user_text,
        images,
        image_paths,
        previous_response_id,
        background,
    );

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
            "responses_send prefix={} url={} body_bytes={} images={} previous_response_id={} structured_outputs={} probe_only={}",
            artifact_prefix,
            url,
            body.len(),
            images.len(),
            previous_response_id.unwrap_or(""),
            expected_schema.is_some(),
            probe_only
        ),
    );
    set_progress(progress, "Waiting for AI slot…");
    let _permit =
        crate::ai_limiter::acquire_permit_cancellable(cancel).map_err(|()| OpenAiError {
            summary: "Cancelled".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: true,
        })?;
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
    }
    set_progress(progress, "Sending request…");
    let auth_headers = curl_auth_header_file(api_key).map_err(|err| OpenAiError {
        summary: format!("Failed to create curl auth header file: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;

    let hard_timeout_secs = if expected_schema.is_some() && !background {
        CURL_HARD_TIMEOUT_SECS_STRUCTURED
    } else {
        CURL_HARD_TIMEOUT_SECS_DEFAULT
    };
    let mut cmd = std::process::Command::new("curl");
    crate::system_proxy::apply_system_proxy_to_curl_command(&mut cmd, &url);
    cmd.arg("-sS")
        .arg("--no-buffer")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(hard_timeout_secs.to_string())
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

    let child = cmd.spawn().map_err(|err| OpenAiError {
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
            first_byte: std::time::Duration::from_secs(CURL_FIRST_BYTE_TIMEOUT_SECS.into()),
            idle: std::time::Duration::from_secs(CURL_IDLE_TIMEOUT_SECS.into()),
            hard: std::time::Duration::from_secs(hard_timeout_secs.into()),
        },
        cancel,
        &url,
    )?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(OpenAiError {
            summary: format!("curl exited with non-zero status:\n{stderr}"),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: false,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let (body, status_code) = split_curl_http_status(&stdout, CURL_HTTP_STATUS_MARKER);
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
            cancelled: false,
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
                cancelled: false,
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
    session: &Gen3dAiSessionState,
    cancel: Option<&AtomicBool>,
    base_url: &str,
    api_key: &str,
    model: &str,
    stream: bool,
    reasoning_effort: &str,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    system_instructions: &str,
    user_text: &str,
    images: &[(&'static str, Vec<u8>)],
    image_paths: &[PathBuf],
    run_dir: Option<&Path>,
    artifact_prefix: &str,
) -> Result<String, OpenAiError> {
    let url = crate::config::join_base_url(base_url, "chat/completions");
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
    }
    let body_json = build_openai_chat_completions_request_json(
        session,
        model,
        stream,
        reasoning_effort,
        expected_schema,
        system_instructions,
        user_text,
        images,
        image_paths,
    );
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
            "chat_send prefix={} url={} body_bytes={} images={} structured_outputs={}",
            artifact_prefix,
            url,
            body.len(),
            images.len(),
            expected_schema.is_some()
        ),
    );
    set_progress(progress, "Waiting for AI slot…");
    let _permit =
        crate::ai_limiter::acquire_permit_cancellable(cancel).map_err(|()| OpenAiError {
            summary: "Cancelled".into(),
            url: url.clone(),
            status: None,
            body_preview: None,
            cancelled: true,
        })?;
    if let Some(cancel) = cancel {
        if cancel.load(Ordering::Relaxed) {
            return Err(OpenAiError::cancelled(url));
        }
    }
    set_progress(progress, "Sending request…");
    let auth_headers = curl_auth_header_file(api_key).map_err(|err| OpenAiError {
        summary: format!("Failed to create curl auth header file: {err}"),
        url: url.clone(),
        status: None,
        body_preview: None,
        cancelled: false,
    })?;

    let hard_timeout_secs = if expected_schema.is_some() {
        CURL_HARD_TIMEOUT_SECS_STRUCTURED
    } else {
        CURL_HARD_TIMEOUT_SECS_DEFAULT
    };

    let mut cmd = std::process::Command::new("curl");
    crate::system_proxy::apply_system_proxy_to_curl_command(&mut cmd, &url);
    cmd.arg("-sS")
        .arg("--no-buffer")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(hard_timeout_secs.to_string())
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

    let child = cmd.spawn().map_err(|err| OpenAiError {
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
            first_byte: std::time::Duration::from_secs(CURL_FIRST_BYTE_TIMEOUT_SECS.into()),
            idle: std::time::Duration::from_secs(CURL_IDLE_TIMEOUT_SECS.into()),
            hard: std::time::Duration::from_secs(hard_timeout_secs.into()),
        },
        cancel,
        &url,
    )?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(OpenAiError {
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
            "Gen3D: missing HTTP status marker in curl GET output (truncated): {}",
            truncate_for_ui(&stdout, 800)
        );
        return Err(OpenAiError {
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
                cancelled: false,
            });
        }
    }

    Ok(body.trim().to_string())
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
        assert!(extract_openai_responses_output_text(&json).is_none());
        assert_eq!(openai_response_status(&json), Some("in_progress"));
        assert!(openai_response_has_pending_status(&json));
    }

    #[test]
    fn responses_request_includes_text_format_when_schema_provided() {
        let json = build_openai_responses_request_json(
            "gpt-test",
            false,
            "high",
            Some(Gen3dAiJsonSchemaKind::PlanV1),
            "sys",
            "user",
            &[],
            &[],
            None,
            false,
        );

        let format = json
            .get("text")
            .and_then(|v| v.get("format"))
            .expect("text.format should exist");
        assert_eq!(
            format.get("type").and_then(|v| v.as_str()),
            Some("json_schema")
        );
        assert_eq!(
            format.get("name").and_then(|v| v.as_str()),
            Some("gen3d_plan_v1")
        );
        assert_eq!(format.get("strict").and_then(|v| v.as_bool()), Some(true));
        assert!(format.get("schema").is_some());
    }

    #[test]
    fn responses_request_omits_text_format_when_schema_not_provided() {
        let json = build_openai_responses_request_json(
            "gpt-test",
            false,
            "high",
            None,
            "sys",
            "user",
            &[],
            &[],
            None,
            false,
        );
        assert!(json.get("text").is_none());
    }

    #[test]
    fn responses_request_enables_background_with_store_when_requested() {
        let json = build_openai_responses_request_json(
            "gpt-test",
            false,
            "high",
            Some(Gen3dAiJsonSchemaKind::PlanV1),
            "sys",
            "user",
            &[],
            &[],
            None,
            true,
        );
        assert_eq!(json.get("background").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(json.get("store").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn chat_request_includes_response_format_when_schema_provided() {
        let session = Gen3dAiSessionState::default();
        let json = build_openai_chat_completions_request_json(
            &session,
            "gpt-test",
            false,
            "high",
            Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1),
            "sys",
            "user",
            &[],
            &[],
        );

        let resp_format = json
            .get("response_format")
            .expect("response_format should exist");
        assert_eq!(
            resp_format.get("type").and_then(|v| v.as_str()),
            Some("json_schema")
        );
        let spec = resp_format
            .get("json_schema")
            .expect("response_format.json_schema should exist");
        assert_eq!(
            spec.get("name").and_then(|v| v.as_str()),
            Some("gen3d_review_delta_v1")
        );
        assert_eq!(spec.get("strict").and_then(|v| v.as_bool()), Some(true));
        assert!(spec.get("schema").is_some());
    }

    #[test]
    fn chat_request_omits_response_format_when_schema_not_provided() {
        let session = Gen3dAiSessionState::default();
        let json = build_openai_chat_completions_request_json(
            &session,
            "gpt-test",
            false,
            "high",
            None,
            "sys",
            "user",
            &[],
            &[],
        );
        assert!(json.get("response_format").is_none());
    }

    #[test]
    fn detects_structured_outputs_rejection_errors() {
        let err = OpenAiError {
            summary: "HTTP 400".into(),
            url: "https://example.invalid".into(),
            status: Some(400),
            body_preview: Some("Unknown field `response_format`".into()),
            cancelled: false,
        };
        assert!(is_structured_outputs_rejected(&err));

        let other = OpenAiError {
            summary: "HTTP 400".into(),
            url: "https://example.invalid".into(),
            status: Some(400),
            body_preview: Some("unknown field `previous_response_id`".into()),
            cancelled: false,
        };
        assert!(!is_structured_outputs_rejected(&other));
    }

    #[test]
    fn detects_stream_required_errors() {
        let err = OpenAiError {
            summary: "HTTP 400".into(),
            url: "https://example.invalid".into(),
            status: Some(400),
            body_preview: Some("{\"detail\":\"Stream must be set to true\"}".into()),
            cancelled: false,
        };
        assert!(is_stream_required(&err));

        let other = OpenAiError {
            summary: "HTTP 400".into(),
            url: "https://example.invalid".into(),
            status: Some(400),
            body_preview: Some("stream must be set to false".into()),
            cancelled: false,
        };
        assert!(!is_stream_required(&other));
    }

    #[test]
    fn hydrates_capabilities_by_base_url_and_model() {
        let dir = std::env::temp_dir().join(format!(
            "gravimera_openai_caps_test_{}",
            uuid::Uuid::new_v4()
        ));
        let path = dir.join(OPENAI_CAPABILITIES_CACHE_FILE_NAME);

        let cache = OpenAiCapabilitiesCacheV1 {
            version: OPENAI_CAPABILITIES_CACHE_VERSION,
            entries: vec![
                OpenAiCapabilitiesCacheEntryV1 {
                    base_url: "https://example.invalid/v1/".into(),
                    model: "gpt-a".into(),
                    responses_supported: Some(true),
                    responses_stream_required: Some(true),
                    responses_continuation_supported: Some(false),
                    responses_background_supported: Some(false),
                    responses_structured_outputs_supported: Some(true),
                    chat_stream_required: Some(true),
                    chat_structured_outputs_supported: Some(true),
                },
                OpenAiCapabilitiesCacheEntryV1 {
                    base_url: "https://example.invalid/v1".into(),
                    model: "gpt-b".into(),
                    responses_supported: Some(false),
                    responses_stream_required: Some(false),
                    responses_continuation_supported: Some(false),
                    responses_background_supported: Some(false),
                    responses_structured_outputs_supported: Some(false),
                    chat_stream_required: Some(false),
                    chat_structured_outputs_supported: Some(false),
                },
            ],
        };
        write_openai_capabilities_cache(&path, &cache).expect("write caps cache");

        let mut session = Gen3dAiSessionState::default();
        hydrate_session_capabilities_from_cache_path(
            &mut session,
            "https://example.invalid/v1",
            "gpt-a",
            &path,
        );
        assert_eq!(session.responses_supported, Some(true));
        assert_eq!(session.responses_stream_required, Some(true));
        assert_eq!(session.responses_continuation_supported, Some(false));
        assert_eq!(session.responses_background_supported, Some(false));
        assert_eq!(session.responses_structured_outputs_supported, Some(true));
        assert_eq!(session.chat_stream_required, Some(true));
        assert_eq!(session.chat_structured_outputs_supported, Some(true));

        // Does not override existing flags.
        let mut session2 = Gen3dAiSessionState::default();
        session2.responses_supported = Some(false);
        session2.responses_stream_required = Some(false);
        hydrate_session_capabilities_from_cache_path(
            &mut session2,
            "https://example.invalid/v1/",
            "gpt-a",
            &path,
        );
        assert_eq!(session2.responses_supported, Some(false));
        assert_eq!(session2.responses_stream_required, Some(false));
        assert_eq!(session2.responses_structured_outputs_supported, Some(true));
        assert_eq!(session2.chat_stream_required, Some(true));

        // Separate model key uses separate entry.
        let mut session3 = Gen3dAiSessionState::default();
        hydrate_session_capabilities_from_cache_path(
            &mut session3,
            "https://example.invalid/v1",
            "gpt-b",
            &path,
        );
        assert_eq!(session3.responses_supported, Some(false));
        assert_eq!(session3.responses_stream_required, Some(false));
        assert_eq!(session3.responses_structured_outputs_supported, Some(false));
        assert_eq!(session3.chat_stream_required, Some(false));
    }

    #[test]
    fn persists_capabilities_merging_known_flags() {
        let dir = std::env::temp_dir().join(format!(
            "gravimera_openai_caps_persist_test_{}",
            uuid::Uuid::new_v4()
        ));
        let path = dir.join(OPENAI_CAPABILITIES_CACHE_FILE_NAME);

        let mut session = Gen3dAiSessionState::default();
        session.responses_supported = Some(true);
        session.responses_stream_required = Some(true);
        session.responses_background_supported = Some(false);
        persist_session_capabilities_to_cache_path(
            "https://example.invalid/v1/",
            "gpt-a",
            &session,
            &path,
        );

        let cache = read_openai_capabilities_cache(&path);
        assert_eq!(cache.version, OPENAI_CAPABILITIES_CACHE_VERSION);
        let entry = cache
            .entries
            .iter()
            .find(|e| e.base_url == "https://example.invalid/v1" && e.model == "gpt-a")
            .expect("expected persisted entry");
        assert_eq!(entry.responses_supported, Some(true));
        assert_eq!(entry.responses_stream_required, Some(true));
        assert_eq!(entry.responses_background_supported, Some(false));

        // New information updates the entry without clearing other fields.
        let mut session2 = Gen3dAiSessionState::default();
        session2.responses_structured_outputs_supported = Some(false);
        persist_session_capabilities_to_cache_path(
            "https://example.invalid/v1",
            "gpt-a",
            &session2,
            &path,
        );
        let cache2 = read_openai_capabilities_cache(&path);
        let entry2 = cache2
            .entries
            .iter()
            .find(|e| e.base_url == "https://example.invalid/v1" && e.model == "gpt-a")
            .expect("expected updated entry");
        assert_eq!(entry2.responses_supported, Some(true));
        assert_eq!(entry2.responses_stream_required, Some(true));
        assert_eq!(entry2.responses_background_supported, Some(false));
        assert_eq!(entry2.responses_structured_outputs_supported, Some(false));
    }

    #[test]
    #[cfg(unix)]
    fn cancels_wait_curl_when_flag_set() {
        use std::process::Stdio;

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_for_thread = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(50));
            cancel_for_thread.store(true, Ordering::Relaxed);
        });

        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg("sleep 10")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let child = cmd.spawn().expect("spawn sleep");

        let start = std::time::Instant::now();
        let err = wait_curl_with_byte_timeouts(
            child,
            None,
            CurlByteTimeouts {
                first_byte: std::time::Duration::from_secs(5),
                idle: std::time::Duration::from_secs(5),
                hard: std::time::Duration::from_secs(5),
            },
            Some(cancel.as_ref()),
            "test://cancel",
        )
        .expect_err("expected cancellation error");
        assert!(err.cancelled);
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
