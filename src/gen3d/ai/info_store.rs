use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn b64_urlsafe_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
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

fn b64_urlsafe_decode(text: &str) -> Result<Vec<u8>, String> {
    fn val(ch: u8) -> Option<u8> {
        match ch {
            b'A'..=b'Z' => Some(ch - b'A'),
            b'a'..=b'z' => Some(ch - b'a' + 26),
            b'0'..=b'9' => Some(ch - b'0' + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }

    let s = text.trim();
    if s.is_empty() {
        return Err("Cursor is empty.".into());
    }
    if s.len() > 16 * 1024 {
        return Err("Cursor is too long.".into());
    }

    let bytes = s.as_bytes();
    if bytes.len() % 4 != 0 {
        return Err("Cursor is not valid base64 (length not multiple of 4).".into());
    }

    let mut out: Vec<u8> = Vec::with_capacity(bytes.len() / 4 * 3);
    let mut i = 0;
    while i < bytes.len() {
        let a = bytes[i];
        let b = bytes[i + 1];
        let c = bytes[i + 2];
        let d = bytes[i + 3];
        i += 4;

        let Some(va) = val(a) else {
            return Err("Cursor has invalid base64 character.".into());
        };
        let Some(vb) = val(b) else {
            return Err("Cursor has invalid base64 character.".into());
        };
        let vc = if c == b'=' { None } else { val(c) };
        let vd = if d == b'=' { None } else { val(d) };
        if (c == b'=' && d != b'=') || (c != b'=' && d == b'=' && vc.is_none()) {
            return Err("Cursor has invalid base64 padding.".into());
        }
        if c != b'=' && vc.is_none() {
            return Err("Cursor has invalid base64 character.".into());
        }
        if d != b'=' && vd.is_none() {
            return Err("Cursor has invalid base64 character.".into());
        }

        let vc = vc.unwrap_or(0);
        let vd = vd.unwrap_or(0);
        let n = ((va as u32) << 18) | ((vb as u32) << 12) | ((vc as u32) << 6) | (vd as u32);
        out.push(((n >> 16) & 0xFF) as u8);
        if c != b'=' {
            out.push(((n >> 8) & 0xFF) as u8);
        }
        if d != b'=' {
            out.push((n & 0xFF) as u8);
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoKvKey {
    pub(super) namespace: String,
    pub(super) key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoProvenance {
    pub(super) tool_id: String,
    pub(super) call_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoKvRecord {
    pub(super) kv_rev: u64,
    pub(super) written_at_ms: u64,
    pub(super) attempt: u32,
    pub(super) pass: u32,
    pub(super) assembly_rev: u32,
    pub(super) workspace_id: String,
    pub(super) key: InfoKvKey,
    pub(super) value: serde_json::Value,
    pub(super) summary: String,
    pub(super) bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) written_by: Option<InfoProvenance>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum InfoEventKindV1 {
    ToolCallStart,
    ToolCallResult,
    EngineLog,
    BudgetStop,
    Warning,
    Error,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoEvent {
    pub(super) event_id: u64,
    pub(super) ts_ms: u64,
    pub(super) attempt: u32,
    pub(super) pass: u32,
    pub(super) assembly_rev: u32,
    pub(super) kind: InfoEventKindV1,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) tool_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) call_id: Option<String>,
    pub(super) message: String,
    pub(super) data: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoBlob {
    pub(super) blob_id: String,
    pub(super) created_at_ms: u64,
    pub(super) attempt: u32,
    pub(super) pass: u32,
    pub(super) assembly_rev: u32,
    pub(super) content_type: String,
    pub(super) bytes: u64,
    pub(super) labels: Vec<String>,
    pub(super) storage: InfoBlobStorageV1,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum InfoBlobStorageV1 {
    RunCacheFile { relative_path: String },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoPage {
    #[serde(default)]
    pub(super) limit: u32,
    #[serde(default)]
    pub(super) cursor: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InfoPageOut<T> {
    pub(super) items: Vec<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) next_cursor: Option<String>,
    pub(super) truncated: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OffsetCursorV1 {
    v: u32,
    kind: String,
    params_sig: String,
    offset: usize,
}

fn encode_offset_cursor(kind: &str, params_sig: &str, offset: usize) -> String {
    let cursor = OffsetCursorV1 {
        v: 1,
        kind: kind.to_string(),
        params_sig: params_sig.to_string(),
        offset,
    };
    let json = serde_json::to_vec(&cursor).unwrap_or_default();
    b64_urlsafe_encode(&json)
}

fn decode_offset_cursor(kind: &str, params_sig: &str, cursor: &str) -> Result<usize, String> {
    let raw = b64_urlsafe_decode(cursor)?;
    let decoded: OffsetCursorV1 = serde_json::from_slice(&raw)
        .map_err(|err| format!("Cursor is not valid JSON: {err}"))?;
    if decoded.v != 1 {
        return Err(format!("Unsupported cursor version {}.", decoded.v));
    }
    if decoded.kind != kind {
        return Err("Cursor does not match this tool (kind mismatch).".into());
    }
    if decoded.params_sig != params_sig {
        return Err("Cursor does not match this request (sort/filters mismatch).".into());
    }
    Ok(decoded.offset)
}

fn page_slice<T: Clone>(items: &[T], kind: &str, params_sig: &str, limit: usize, offset: usize) -> InfoPageOut<T> {
    if offset >= items.len() {
        return InfoPageOut {
            items: Vec::new(),
            next_cursor: None,
            truncated: false,
        };
    }
    let end = (offset + limit).min(items.len());
    let out_items = items[offset..end].to_vec();
    let truncated = end < items.len();
    let next_cursor = truncated.then(|| encode_offset_cursor(kind, params_sig, end));
    InfoPageOut {
        items: out_items,
        next_cursor,
        truncated,
    }
}

fn stable_params_sig(value: &serde_json::Value) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
}

fn is_kv_key_char_ok(ch: char) -> bool {
    matches!(ch, 'a'..='z' | '0'..='9' | '.' | '_' | '-')
}

fn validate_kv_key(namespace: &str, key: &str) -> Result<(), String> {
    let namespace = namespace.trim();
    let key = key.trim();
    if namespace.is_empty() {
        return Err("Info KV namespace is empty.".into());
    }
    if key.is_empty() {
        return Err("Info KV key is empty.".into());
    }
    if namespace.len() > 32 {
        return Err("Info KV namespace is too long.".into());
    }
    if key.len() > 128 {
        return Err("Info KV key is too long.".into());
    }
    if !namespace.is_ascii() || !key.is_ascii() {
        return Err("Info KV namespace/key must be ASCII.".into());
    }
    if namespace.to_ascii_lowercase() != namespace {
        return Err("Info KV namespace must be lowercase ASCII.".into());
    }
    if key.to_ascii_lowercase() != key {
        return Err("Info KV key must be lowercase ASCII.".into());
    }
    if key.chars().any(|ch| !is_kv_key_char_ok(ch)) {
        return Err("Info KV key has invalid characters (allowed: a-z 0-9 . _ -).".into());
    }
    Ok(())
}

pub(super) struct Gen3dInfoStore {
    run_dir: PathBuf,
    kv_path: PathBuf,
    events_path: PathBuf,
    blobs_path: PathBuf,

    next_kv_rev: u64,
    next_event_id: u64,

    kv_records: Vec<InfoKvRecord>,
    kv_by_rev: HashMap<u64, usize>,
    kv_latest_by_key: HashMap<InfoKvKey, usize>,
    kv_by_key: HashMap<InfoKvKey, Vec<usize>>,

    events: Vec<InfoEvent>,
    events_by_id: HashMap<u64, usize>,

    blobs: Vec<InfoBlob>,
    blobs_by_id: HashMap<String, usize>,
}

impl Gen3dInfoStore {
    pub(super) fn open_or_create(run_dir: &Path) -> Result<Self, String> {
        if !run_dir.is_dir() {
            return Err(format!(
                "Gen3D run dir does not exist or is not a directory: {}",
                run_dir.display()
            ));
        }

        let store_dir = run_dir.join("info_store_v1");
        std::fs::create_dir_all(&store_dir).map_err(|err| {
            format!(
                "Failed to create Info Store dir {}: {err}",
                store_dir.display()
            )
        })?;

        let kv_path = store_dir.join("kv.jsonl");
        let events_path = store_dir.join("events.jsonl");
        let blobs_path = store_dir.join("blobs.jsonl");

        let mut store = Self {
            run_dir: run_dir.to_path_buf(),
            kv_path,
            events_path,
            blobs_path,
            next_kv_rev: 1,
            next_event_id: 1,
            kv_records: Vec::new(),
            kv_by_rev: HashMap::new(),
            kv_latest_by_key: HashMap::new(),
            kv_by_key: HashMap::new(),
            events: Vec::new(),
            events_by_id: HashMap::new(),
            blobs: Vec::new(),
            blobs_by_id: HashMap::new(),
        };

        store.rebuild_from_disk()?;
        Ok(store)
    }

    pub(super) fn run_dir(&self) -> &Path {
        self.run_dir.as_path()
    }

    fn rebuild_from_disk(&mut self) -> Result<(), String> {
        self.kv_records.clear();
        self.kv_by_rev.clear();
        self.kv_latest_by_key.clear();
        self.kv_by_key.clear();
        self.events.clear();
        self.events_by_id.clear();
        self.blobs.clear();
        self.blobs_by_id.clear();
        self.next_kv_rev = 1;
        self.next_event_id = 1;

        if self.kv_path.exists() {
            let text = std::fs::read_to_string(&self.kv_path).map_err(|err| {
                format!("Failed to read {}: {err}", self.kv_path.display())
            })?;
            for (line_idx, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let record: InfoKvRecord = serde_json::from_str(line).map_err(|err| {
                    format!(
                        "Failed to parse {} line {} as KV record: {err}",
                        self.kv_path.display(),
                        line_idx + 1
                    )
                })?;
                self.insert_kv_record(record)?;
            }
        }

        if self.events_path.exists() {
            let text = std::fs::read_to_string(&self.events_path).map_err(|err| {
                format!("Failed to read {}: {err}", self.events_path.display())
            })?;
            for (line_idx, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let event: InfoEvent = serde_json::from_str(line).map_err(|err| {
                    format!(
                        "Failed to parse {} line {} as event: {err}",
                        self.events_path.display(),
                        line_idx + 1
                    )
                })?;
                self.insert_event(event)?;
            }
        }

        if self.blobs_path.exists() {
            let text = std::fs::read_to_string(&self.blobs_path).map_err(|err| {
                format!("Failed to read {}: {err}", self.blobs_path.display())
            })?;
            for (line_idx, line) in text.lines().enumerate() {
                let line = line.trim();
                if line.is_empty() {
                    continue;
                }
                let blob: InfoBlob = serde_json::from_str(line).map_err(|err| {
                    format!(
                        "Failed to parse {} line {} as blob: {err}",
                        self.blobs_path.display(),
                        line_idx + 1
                    )
                })?;
                self.insert_blob(blob)?;
            }
        }

        Ok(())
    }

    fn append_jsonl(&self, path: &Path, value: &serde_json::Value) -> Result<(), String> {
        let line = serde_json::to_string(value).map_err(|err| format!("Failed to serialize JSON: {err}"))?;
        let mut line = line;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
        use std::io::Write;
        file.write_all(line.as_bytes())
            .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
        Ok(())
    }

    fn insert_kv_record(&mut self, record: InfoKvRecord) -> Result<(), String> {
        validate_kv_key(record.key.namespace.as_str(), record.key.key.as_str())?;
        if self.kv_by_rev.contains_key(&record.kv_rev) {
            return Err(format!("Duplicate kv_rev {} in store.", record.kv_rev));
        }
        let idx = self.kv_records.len();
        self.kv_records.push(record.clone());
        self.kv_by_rev.insert(record.kv_rev, idx);

        match self.kv_latest_by_key.get(&record.key) {
            Some(&prev_idx) => {
                let prev_rev = self.kv_records.get(prev_idx).map(|r| r.kv_rev).unwrap_or(0);
                if record.kv_rev >= prev_rev {
                    self.kv_latest_by_key.insert(record.key.clone(), idx);
                }
            }
            None => {
                self.kv_latest_by_key.insert(record.key.clone(), idx);
            }
        }
        self.kv_by_key.entry(record.key.clone()).or_default().push(idx);
        self.next_kv_rev = self.next_kv_rev.max(record.kv_rev.saturating_add(1));
        Ok(())
    }

    fn insert_event(&mut self, event: InfoEvent) -> Result<(), String> {
        if self.events_by_id.contains_key(&event.event_id) {
            return Err(format!("Duplicate event_id {} in store.", event.event_id));
        }
        let idx = self.events.len();
        self.events.push(event.clone());
        self.events_by_id.insert(event.event_id, idx);
        self.next_event_id = self.next_event_id.max(event.event_id.saturating_add(1));
        Ok(())
    }

    fn insert_blob(&mut self, blob: InfoBlob) -> Result<(), String> {
        if self.blobs_by_id.contains_key(&blob.blob_id) {
            return Err(format!("Duplicate blob_id {} in store.", blob.blob_id));
        }
        let idx = self.blobs.len();
        self.blobs.push(blob.clone());
        self.blobs_by_id.insert(blob.blob_id.clone(), idx);
        Ok(())
    }

    pub(super) fn kv_put(
        &mut self,
        attempt: u32,
        pass: u32,
        assembly_rev: u32,
        workspace_id: &str,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
        summary: String,
        written_by: Option<InfoProvenance>,
    ) -> Result<InfoKvRecord, String> {
        validate_kv_key(namespace, key)?;
        let bytes = serde_json::to_vec(&value)
            .map(|v| v.len() as u64)
            .unwrap_or(0);

        let record = InfoKvRecord {
            kv_rev: self.next_kv_rev,
            written_at_ms: now_ms(),
            attempt,
            pass,
            assembly_rev,
            workspace_id: workspace_id.trim().to_string(),
            key: InfoKvKey {
                namespace: namespace.trim().to_string(),
                key: key.trim().to_string(),
            },
            value,
            summary,
            bytes,
            written_by,
        };
        self.next_kv_rev = self.next_kv_rev.saturating_add(1);

        let json = serde_json::to_value(&record).unwrap_or(serde_json::Value::Null);
        self.append_jsonl(&self.kv_path, &json)?;
        self.insert_kv_record(record.clone())?;
        Ok(record)
    }

    pub(super) fn kv_latest_record(&self, namespace: &str, key: &str) -> Option<&InfoKvRecord> {
        let k = InfoKvKey {
            namespace: namespace.trim().to_string(),
            key: key.trim().to_string(),
        };
        let idx = self.kv_latest_by_key.get(&k).copied()?;
        self.kv_records.get(idx)
    }

    pub(super) fn kv_latest_entries(&self) -> Vec<(&InfoKvKey, &InfoKvRecord)> {
        let mut out: Vec<(&InfoKvKey, &InfoKvRecord)> = Vec::with_capacity(self.kv_latest_by_key.len());
        for (key, idx) in self.kv_latest_by_key.iter() {
            if let Some(record) = self.kv_records.get(*idx) {
                out.push((key, record));
            }
        }
        out
    }

    pub(super) fn kv_records_for_key(&self, namespace: &str, key: &str) -> Vec<&InfoKvRecord> {
        let k = InfoKvKey {
            namespace: namespace.trim().to_string(),
            key: key.trim().to_string(),
        };
        let Some(indices) = self.kv_by_key.get(&k) else {
            return Vec::new();
        };
        indices.iter().filter_map(|&idx| self.kv_records.get(idx)).collect()
    }

    pub(super) fn kv_records(&self) -> &[InfoKvRecord] {
        self.kv_records.as_slice()
    }

    pub(super) fn kv_record_by_rev(&self, kv_rev: u64) -> Option<&InfoKvRecord> {
        let idx = self.kv_by_rev.get(&kv_rev).copied()?;
        self.kv_records.get(idx)
    }

    pub(super) fn events(&self) -> &[InfoEvent] {
        self.events.as_slice()
    }

    pub(super) fn event_by_id(&self, event_id: u64) -> Option<&InfoEvent> {
        let idx = self.events_by_id.get(&event_id).copied()?;
        self.events.get(idx)
    }

    pub(super) fn append_event(
        &mut self,
        attempt: u32,
        pass: u32,
        assembly_rev: u32,
        kind: InfoEventKindV1,
        tool_id: Option<String>,
        call_id: Option<String>,
        message: String,
        data: serde_json::Value,
    ) -> Result<InfoEvent, String> {
        let event = InfoEvent {
            event_id: self.next_event_id,
            ts_ms: now_ms(),
            attempt,
            pass,
            assembly_rev,
            kind,
            tool_id,
            call_id,
            message,
            data,
        };
        self.next_event_id = self.next_event_id.saturating_add(1);

        let json = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
        self.append_jsonl(&self.events_path, &json)?;
        self.insert_event(event.clone())?;
        Ok(event)
    }

    pub(super) fn blobs(&self) -> &[InfoBlob] {
        self.blobs.as_slice()
    }

    pub(super) fn blob_by_id(&self, blob_id: &str) -> Option<&InfoBlob> {
        let idx = self.blobs_by_id.get(blob_id).copied()?;
        self.blobs.get(idx)
    }

    pub(super) fn register_blob_file(
        &mut self,
        attempt: u32,
        pass: u32,
        assembly_rev: u32,
        content_type: &str,
        bytes: u64,
        labels: Vec<String>,
        relative_path: String,
    ) -> Result<InfoBlob, String> {
        let blob = InfoBlob {
            blob_id: Uuid::new_v4().to_string(),
            created_at_ms: now_ms(),
            attempt,
            pass,
            assembly_rev,
            content_type: content_type.trim().to_string(),
            bytes,
            labels,
            storage: InfoBlobStorageV1::RunCacheFile { relative_path },
        };

        let json = serde_json::to_value(&blob).unwrap_or(serde_json::Value::Null);
        self.append_jsonl(&self.blobs_path, &json)?;
        self.insert_blob(blob.clone())?;
        Ok(blob)
    }

    pub(super) fn resolve_blob_run_cache_path(&self, blob_id: &str) -> Option<PathBuf> {
        let blob = self.blob_by_id(blob_id)?;
        match &blob.storage {
            InfoBlobStorageV1::RunCacheFile { relative_path } => {
                let rel = relative_path.trim();
                if rel.is_empty() {
                    return None;
                }
                let path = self.run_dir.join(rel);
                Some(path)
            }
        }
    }

    pub(super) fn page_from_args(
        &self,
        kind: &str,
        params_sig: &str,
        page: Option<&InfoPage>,
        default_limit: u32,
        max_limit: u32,
    ) -> Result<(usize, usize), String> {
        let limit = page
            .map(|p| p.limit)
            .filter(|&l| l > 0)
            .unwrap_or(default_limit)
            .min(max_limit) as usize;
        let offset = match page.and_then(|p| p.cursor.as_deref()) {
            Some(cursor) => decode_offset_cursor(kind, params_sig, cursor)?,
            None => 0,
        };
        Ok((limit, offset))
    }

    pub(super) fn page_out<T: Clone>(
        &self,
        items: &[T],
        kind: &str,
        params_sig: &str,
        limit: usize,
        offset: usize,
    ) -> InfoPageOut<T> {
        page_slice(items, kind, params_sig, limit, offset)
    }

    pub(super) fn stable_params_sig(&self, value: &serde_json::Value) -> String {
        stable_params_sig(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("{prefix}_{}", uuid::Uuid::new_v4().to_string()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn cursor_roundtrip_and_mismatch_rejected() {
        let kind = "kv_keys";
        let params_sig = "{\"sort\":\"key_asc\"}";
        let cursor = encode_offset_cursor(kind, params_sig, 123);
        assert_eq!(decode_offset_cursor(kind, params_sig, cursor.as_str()).unwrap(), 123);
        assert!(decode_offset_cursor("events", params_sig, cursor.as_str()).is_err());
        assert!(decode_offset_cursor(kind, "{\"sort\":\"ts_desc\"}", cursor.as_str()).is_err());
    }

    #[test]
    fn kv_rev_monotonic_and_latest_selector() {
        let run_dir = make_temp_dir("gravimera_info_store_test");
        let mut store = Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let rec1 = store
            .kv_put(
                0,
                1,
                10,
                "main",
                "gen3d",
                "ws.main.scene_graph_summary",
                serde_json::json!({"a": 1}),
                "scene graph summary".into(),
                Some(InfoProvenance {
                    tool_id: "get_scene_graph_summary_v1".into(),
                    call_id: "call_1".into(),
                }),
            )
            .expect("kv put 1");
        let rec2 = store
            .kv_put(
                0,
                2,
                11,
                "main",
                "gen3d",
                "ws.main.scene_graph_summary",
                serde_json::json!({"a": 2}),
                "scene graph summary".into(),
                Some(InfoProvenance {
                    tool_id: "get_scene_graph_summary_v1".into(),
                    call_id: "call_2".into(),
                }),
            )
            .expect("kv put 2");

        assert!(rec2.kv_rev > rec1.kv_rev);
        assert_eq!(
            store
                .kv_latest_record("gen3d", "ws.main.scene_graph_summary")
                .unwrap()
                .kv_rev,
            rec2.kv_rev
        );

        // Re-open from disk and ensure latest is preserved.
        let store2 = Gen3dInfoStore::open_or_create(&run_dir).expect("reopen store");
        assert_eq!(
            store2
                .kv_latest_record("gen3d", "ws.main.scene_graph_summary")
                .unwrap()
                .kv_rev,
            rec2.kv_rev
        );

        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn paging_collects_all_items_without_duplicates() {
        let run_dir = make_temp_dir("gravimera_info_store_paging_test");
        let store = Gen3dInfoStore::open_or_create(&run_dir).expect("open store");

        let items: Vec<u32> = (0..7).collect();
        let kind = "info_store_paging_test";
        let params_sig = store.stable_params_sig(&serde_json::json!({"sort": "key_asc"}));

        let mut collected: Vec<u32> = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = InfoPage {
                limit: 2,
                cursor: cursor.clone(),
            };
            let (limit, offset) = store
                .page_from_args(kind, params_sig.as_str(), Some(&page), 50, 200)
                .expect("page_from_args");
            let out = store.page_out(&items, kind, params_sig.as_str(), limit, offset);
            collected.extend(out.items);
            cursor = out.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert_eq!(collected, items);
        let _ = std::fs::remove_dir_all(&run_dir);
    }

    #[test]
    fn gen3d_info_store_fixture_harness() {
        let run_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("test/gen3d_info_store_harness/example_run");
        let store = Gen3dInfoStore::open_or_create(&run_dir).expect("open store fixture");

        let latest = store
            .kv_latest_record("gen3d", "ws.main.scene_graph_summary")
            .expect("expected scene_graph_summary KV");
        assert_eq!(latest.kv_rev, 2);
        assert_eq!(
            latest
                .value
                .get("components_total")
                .and_then(|v| v.as_u64()),
            Some(3)
        );

        assert_eq!(store.events().len(), 2);
        assert_eq!(store.blobs().len(), 1);
        assert!(store
            .blob_by_id("00000000-0000-0000-0000-000000000001")
            .is_some());
    }
}
