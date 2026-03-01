use serde_json::Value;
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
    let mut out = String::new();
    for item in output {
        let Some(parts) = item.get("content").and_then(|v| v.as_array()) else {
            continue;
        };
        for part in parts {
            let ty = part.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if matches!(ty, "output_text" | "text") {
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
        }
    }
    (!out.trim().is_empty()).then_some(out)
}

pub(crate) fn extract_openai_responses_sse_output_text(body: &str) -> Option<String> {
    let mut out = String::new();
    let mut saw_delta = false;

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

        match json.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "response.output_text.delta" => {
                if let Some(delta) = json.get("delta").and_then(|v| v.as_str()) {
                    saw_delta = true;
                    out.push_str(delta);
                }
            }
            "response.output_text.done" => {
                if let Some(text) = json.get("text").and_then(|v| v.as_str()) {
                    saw_delta = true;
                    out.push_str(text);
                }
            }
            // Some SSE streams include the full part payload instead of deltas.
            "response.content_part.added" | "response.content_part.done" => {
                if saw_delta {
                    continue;
                }
                let Some(part) = json.get("part") else {
                    continue;
                };
                if part.get("type").and_then(|v| v.as_str()) != Some("output_text") {
                    continue;
                }
                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                    out.push_str(text);
                }
            }
            _ => {}
        }
    }

    (!out.trim().is_empty()).then_some(out)
}
