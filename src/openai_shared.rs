use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub(crate) const CURL_HTTP_STATUS_WRITEOUT_ARG: &str = "\n__GRAVIMERA_HTTP_STATUS__%{http_code}\n";
pub(crate) const CURL_HTTP_STATUS_MARKER: &str = "\n__GRAVIMERA_HTTP_STATUS__";

pub(crate) struct TempSecretFile {
    path: PathBuf,
}

impl TempSecretFile {
    pub(crate) fn create(prefix: &str, contents: &str) -> std::io::Result<Self> {
        use std::io::Write;

        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        let nonce = uuid::Uuid::new_v4();
        path.push(format!("gravimera_{prefix}_{pid}_{nonce}.txt"));

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(contents.as_bytes())?;
        file.flush()?;

        Ok(Self { path })
    }

    pub(crate) fn curl_header_arg(&self) -> String {
        format!("@{}", self.path.display())
    }
}

impl Drop for TempSecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn curl_auth_header_file(api_key: &str) -> Result<TempSecretFile, std::io::Error> {
    // IMPORTANT: do not pass secrets on the curl command line (visible via `ps`).
    // Use `curl -H @file` so argv contains only the temp file path.
    let api_key = api_key.replace(['\n', '\r'], "");
    let headers = format!("Authorization: Bearer {api_key}\n");
    TempSecretFile::create("openai_auth", &headers)
}

pub(crate) fn split_curl_http_status<'a>(stdout: &'a str, marker: &str) -> (&'a str, Option<u16>) {
    let Some(pos) = stdout.rfind(marker) else {
        return (stdout, None);
    };
    let (body, rest) = stdout.split_at(pos);
    let code_str = rest[marker.len()..].lines().next().unwrap_or("").trim();
    (body, code_str.parse::<u16>().ok())
}

pub(crate) fn extract_openai_responses_output_text(json: &Value) -> Option<String> {
    let output = json.get("output")?.as_array()?;

    fn extract_text_from_item(item: &Value) -> Option<String> {
        let Some(parts) = item.get("content").and_then(|v| v.as_array()) else {
            return None;
        };
        let mut out = String::new();
        for part in parts {
            let ty = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(ty, "output_text" | "text") {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
        }
        (!out.trim().is_empty()).then_some(out)
    }

    // Prefer the FINAL assistant message. Some OpenAI-compatible providers return multiple
    // `output` items in a single response; concatenating them can produce multiple JSON objects
    // back-to-back, which violates Gen3D's structured-output contract.
    let mut last_assistant_message: Option<&Value> = None;
    for item in output.iter() {
        if item.get("type").and_then(|v| v.as_str()) != Some("message") {
            continue;
        }
        if item.get("role").and_then(|v| v.as_str()) != Some("assistant") {
            continue;
        }
        last_assistant_message = Some(item);
    }
    if let Some(item) = last_assistant_message {
        if let Some(text) = extract_text_from_item(item) {
            return Some(text);
        }
    }

    // Fallback: best-effort extract from any output item with a `content` array.
    for item in output.iter().rev() {
        if let Some(text) = extract_text_from_item(item) {
            return Some(text);
        }
    }
    None
}

pub(crate) fn extract_openai_responses_sse_output_text(body: &str) -> Option<String> {
    // Best path: parse the final `response` object embedded in SSE and extract the final assistant
    // message output. This avoids:
    // - duplicating `delta` + `done` text, and
    // - concatenating multiple assistant messages/tool-ish outputs into a single string.
    let mut candidate_response: Option<Value> = None;
    for line in body.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(resp) = json.get("response") {
            candidate_response = Some(resp.clone());
            continue;
        }
        if json.get("output").is_some() || json.get("id").is_some() {
            candidate_response = Some(json);
        }
    }
    if let Some(resp) = candidate_response.as_ref() {
        if let Some(text) = extract_openai_responses_output_text(resp) {
            return Some(text);
        }
    }

    // Fallback: reconstruct output_text from SSE events. Keep a per-output_index buffer and
    // prefer the highest output index at the end (tends to correspond to the final assistant
    // message).
    let mut by_output_index: BTreeMap<i64, String> = BTreeMap::new();

    for line in body.lines() {
        let line = line.trim();
        let Some(data) = line.strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(json) = serde_json::from_str::<Value>(data) else {
            continue;
        };

        let output_index = json
            .get("output_index")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        match json.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|v| v.as_str()) {
                    by_output_index
                        .entry(output_index)
                        .or_default()
                        .push_str(delta);
                }
            }
            "response.output_text.done" => {
                if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                    // Providers often include the full final text here, so prefer it over
                    // previously-accumulated deltas to avoid duplication.
                    by_output_index.insert(output_index, text.to_string());
                }
            }
            // Some SSE streams include the full part payload instead of deltas.
            "response.content_part.added" | "response.content_part.done" => {
                let Some(part) = json.get("part") else {
                    continue;
                };
                if part.get("type").and_then(|v| v.as_str()) != Some("output_text") {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    by_output_index.insert(output_index, text.to_string());
                }
            }
            _ => {}
        }
    }

    by_output_index
        .iter()
        .rev()
        .find_map(|(_, text)| (!text.trim().is_empty()).then_some(text.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn responses_output_text_prefers_last_assistant_message() {
        let json = serde_json::json!({
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"output_text","text":"first"}]
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type":"output_text","text":"second"}]
                }
            ]
        });
        assert_eq!(
            extract_openai_responses_output_text(&json).as_deref(),
            Some("second")
        );
    }

    #[test]
    fn responses_sse_output_text_does_not_duplicate_delta_plus_done() {
        let body = r#"event: response.output_text.delta
data: {"type":"response.output_text.delta","output_index":1,"delta":"{\"a\":1}"}

event: response.output_text.done
data: {"type":"response.output_text.done","output_index":1,"text":"{\"a\":1}"}
"#;
        assert_eq!(
            extract_openai_responses_sse_output_text(body).as_deref(),
            Some("{\"a\":1}")
        );
    }

    #[test]
    fn responses_sse_output_text_prefers_highest_output_index() {
        let body = r#"event: response.output_text.done
data: {"type":"response.output_text.done","output_index":1,"text":"one"}

event: response.output_text.done
data: {"type":"response.output_text.done","output_index":3,"text":"three"}
"#;
        assert_eq!(
            extract_openai_responses_sse_output_text(body).as_deref(),
            Some("three")
        );
    }

    #[test]
    fn responses_sse_output_text_prefers_last_assistant_message_in_response_completed() {
        let body = r#"event: response.completed
data: {"type":"response.completed","response":{"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"first"}]},{"type":"message","role":"assistant","content":[{"type":"output_text","text":"second"}]}]}}
"#;
        assert_eq!(
            extract_openai_responses_sse_output_text(body).as_deref(),
            Some("second")
        );
    }
}
