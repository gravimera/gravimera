use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentTraceRecordV1 {
    pub(crate) ts_ms: u64,
    pub(crate) event: AgentTraceEventV1,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum AgentTraceEventV1 {
    Info {
        message: String,
    },
    LlmRequest {
        artifact_prefix: String,
        artifact_dir: String,
        model: String,
        images: usize,
        system_text_file: Option<String>,
        user_text_file: Option<String>,
    },
    LlmResponse {
        artifact_prefix: String,
        artifact_dir: String,
        api: String,
        ok: bool,
        total_tokens: Option<u64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    ToolCall {
        call_id: String,
        tool_id: String,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        tool_id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<serde_json::Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
}

pub(crate) fn append_agent_trace_event_v1(run_dir: Option<&Path>, event: &AgentTraceEventV1) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join("agent_trace.jsonl");
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let record = AgentTraceRecordV1 {
        ts_ms,
        event: event.clone(),
    };
    let line = match serde_json::to_string(&record) {
        Ok(json) => json,
        Err(_) => return,
    };
    let mut line = line;
    line.push('\n');
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    use std::io::Write;
    let _ = file.write_all(line.as_bytes());
}

pub(crate) fn run_root_dir_from_pass_dir(pass_dir: &Path) -> Option<&Path> {
    // pass_dir = <run_id>/attempt_N/pass_M
    pass_dir.parent()?.parent()
}
