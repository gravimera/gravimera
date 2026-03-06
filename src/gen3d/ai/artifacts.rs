use bevy::prelude::*;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

use super::Gen3dPlannedComponent;

pub(super) fn write_gen3d_text_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    text: &str,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    if let Err(err) = std::fs::write(&path, text) {
        debug!("Gen3D: failed to write {}: {err}", path.display());
    }
}

pub(super) fn append_gen3d_run_log(run_dir: Option<&Path>, message: impl AsRef<str>) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join("gen3d_run.log");
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut line = format!("[{ts_ms}] {}", message.as_ref());
    if !line.ends_with('\n') {
        line.push('\n');
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}

pub(super) fn write_gen3d_json_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    json: &serde_json::Value,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    let data = serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string());
    if let Err(err) = std::fs::write(&path, data) {
        debug!("Gen3D: failed to write {}: {err}", path.display());
    }
}

pub(super) fn append_gen3d_jsonl_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    json: &serde_json::Value,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    let line = match serde_json::to_string(json) {
        Ok(mut line) => {
            line.push('\n');
            line
        }
        Err(err) => {
            debug!("Gen3D: failed to serialize JSONL {}: {err}", path.display());
            return;
        }
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}

pub(super) fn write_gen3d_assembly_snapshot(
    run_dir: Option<&Path>,
    components: &[Gen3dPlannedComponent],
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let components: Vec<serde_json::Value> = components
        .iter()
        .map(|c| {
            let forward = c.rot * Vec3::Z;
            let up = c.rot * Vec3::Y;
            let size = c.actual_size.unwrap_or(c.planned_size);
            serde_json::json!({
                "name": c.name.as_str(),
                "generated": c.actual_size.is_some(),
                "pos": [c.pos.x, c.pos.y, c.pos.z],
                "forward": [forward.x, forward.y, forward.z],
                "up": [up.x, up.y, up.z],
                "size": [size.x, size.y, size.z],
            })
        })
        .collect();
    let snapshot = serde_json::json!({
        "version": 2,
        "components": components,
    });
    write_gen3d_json_artifact(Some(run_dir), "assembly_transforms.json", &snapshot);
}

fn normalize_artifact_ref(ref_str: &str) -> Result<PathBuf, String> {
    let trimmed = ref_str.trim();
    if trimmed.is_empty() {
        return Err("Missing args.artifact_ref".into());
    }
    if trimmed.len() > 1024 {
        return Err("args.artifact_ref is too long.".into());
    }
    if trimmed.len() >= 2 {
        let bytes = trimmed.as_bytes();
        if bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
            return Err("Invalid args.artifact_ref (looks like an absolute Windows path).".into());
        }
    }

    let normalized = trimmed.replace('\\', "/");
    let mut out = PathBuf::new();
    for comp in Path::new(&normalized).components() {
        match comp {
            Component::Normal(part) => out.push(part),
            _ => {
                return Err(
                    "Invalid args.artifact_ref (must be a relative path; no '.', '..', or absolute paths)."
                        .into(),
                );
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err("Invalid args.artifact_ref (empty after normalization).".into());
    }
    Ok(out)
}

fn artifact_ref_from_path(run_dir: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(run_dir).ok()?;
    let mut parts = Vec::new();
    for comp in rel.components() {
        match comp {
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}

fn artifact_kind_from_path(path: &Path) -> &'static str {
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    match ext.to_ascii_lowercase().as_str() {
        "json" => "json",
        "jsonl" => "jsonl",
        "log" => "log",
        "txt" => "txt",
        "png" => "png",
        "jpg" | "jpeg" => "jpeg",
        "webp" => "webp",
        "glb" => "glb",
        _ => "other",
    }
}

fn modified_at_ms(meta: &std::fs::Metadata) -> Option<u64> {
    let modified = meta.modified().ok()?;
    let dur = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
    Some(dur.as_millis().min(u64::MAX as u128) as u64)
}

fn read_dir_sorted(dir: &Path) -> Result<Vec<std::fs::DirEntry>, String> {
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(dir)
        .map_err(|err| format!("Failed to read dir {}: {err}", dir.display()))?
        .filter_map(|e| e.ok())
        .collect();
    entries.sort_by_key(|e| e.file_name().to_string_lossy().to_string());
    Ok(entries)
}

fn list_files_recursive_sorted(
    run_dir: &Path,
    dir: &Path,
    max_items: usize,
    out: &mut Vec<serde_json::Value>,
    truncated: &mut bool,
) -> Result<(), String> {
    if *truncated || out.len() >= max_items {
        *truncated = true;
        return Ok(());
    }

    for entry in read_dir_sorted(dir)? {
        if *truncated || out.len() >= max_items {
            *truncated = true;
            break;
        }
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(err) => {
                debug!("Gen3D artifacts: failed to stat {}: {err}", path.display());
                continue;
            }
        };

        if meta.is_dir() {
            list_files_recursive_sorted(run_dir, &path, max_items, out, truncated)?;
            continue;
        }
        if !meta.is_file() {
            continue;
        }

        let Some(artifact_ref) = artifact_ref_from_path(run_dir, &path) else {
            continue;
        };
        out.push(serde_json::json!({
            "artifact_ref": artifact_ref,
            "kind": artifact_kind_from_path(&path),
            "size_bytes": meta.len(),
            "modified_at_ms": modified_at_ms(&meta),
        }));
    }

    Ok(())
}

pub(super) fn list_run_artifacts_v1(
    run_dir: &Path,
    path_prefix: Option<&str>,
    max_items: usize,
) -> Result<(Vec<serde_json::Value>, bool), String> {
    if !run_dir.is_dir() {
        return Err(format!(
            "Gen3D run dir does not exist or is not a directory: {}",
            run_dir.display()
        ));
    }
    if max_items == 0 {
        return Ok((Vec::new(), false));
    }

    let base_dir = if let Some(prefix) = path_prefix {
        let rel = normalize_artifact_ref(prefix)?;
        let base = run_dir.join(rel);
        if !base.exists() {
            return Err(format!(
                "path_prefix does not exist under the current run dir: {}",
                prefix.trim()
            ));
        }
        base
    } else {
        run_dir.to_path_buf()
    };

    let mut out = Vec::new();
    let mut truncated = false;
    if base_dir.is_file() {
        let meta = std::fs::metadata(&base_dir)
            .map_err(|err| format!("Failed to stat {}: {err}", base_dir.display()))?;
        let Some(artifact_ref) = artifact_ref_from_path(run_dir, &base_dir) else {
            return Ok((Vec::new(), false));
        };
        out.push(serde_json::json!({
            "artifact_ref": artifact_ref,
            "kind": artifact_kind_from_path(&base_dir),
            "size_bytes": meta.len(),
            "modified_at_ms": modified_at_ms(&meta),
        }));
        return Ok((out, false));
    }

    list_files_recursive_sorted(run_dir, &base_dir, max_items, &mut out, &mut truncated)?;
    Ok((out, truncated))
}

fn read_file_head_bytes(path: &Path, max_bytes: usize) -> Result<(Vec<u8>, bool), String> {
    let meta = std::fs::metadata(path)
        .map_err(|err| format!("Failed to stat {}: {err}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("Artifact is not a file: {}", path.display()));
    }

    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    let limit = meta.len().min(max_bytes as u64) as usize;
    let mut buf = vec![0u8; limit];
    file.read_exact(&mut buf)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let truncated = meta.len() > limit as u64;
    Ok((buf, truncated))
}

fn read_file_tail_bytes(path: &Path, max_bytes: usize) -> Result<(Vec<u8>, bool), String> {
    let meta = std::fs::metadata(path)
        .map_err(|err| format!("Failed to stat {}: {err}", path.display()))?;
    if !meta.is_file() {
        return Err(format!("Artifact is not a file: {}", path.display()));
    }
    let size = meta.len();
    let read_len = (max_bytes as u64).min(size) as usize;
    let truncated = size > read_len as u64;

    let mut file = std::fs::File::open(path)
        .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
    if size > read_len as u64 {
        file.seek(SeekFrom::End(-(read_len as i64)))
            .map_err(|err| format!("Failed to seek {}: {err}", path.display()))?;
    }
    let mut buf = vec![0u8; read_len];
    file.read_exact(&mut buf)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    Ok((buf, truncated))
}

fn tail_lines_from_text(text: &str, tail_lines: usize) -> String {
    if tail_lines == 0 {
        return String::new();
    }
    let mut lines: Vec<&str> = text.lines().collect();
    if lines.len() > tail_lines {
        lines.drain(0..(lines.len() - tail_lines));
    }
    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

pub(super) fn read_artifact_v1(
    run_dir: &Path,
    artifact_ref: &str,
    max_bytes: usize,
    tail_lines: Option<usize>,
    json_pointer: Option<&str>,
) -> Result<serde_json::Value, String> {
    if !run_dir.is_dir() {
        return Err(format!(
            "Gen3D run dir does not exist or is not a directory: {}",
            run_dir.display()
        ));
    }
    let rel = normalize_artifact_ref(artifact_ref)?;
    let path = run_dir.join(&rel);
    if !path.exists() {
        return Err(format!("Artifact not found: {}", artifact_ref.trim()));
    }
    if !path.is_file() {
        return Err(format!("Artifact is not a file: {}", artifact_ref.trim()));
    }

    let meta = std::fs::metadata(&path)
        .map_err(|err| format!("Failed to stat {}: {err}", path.display()))?;
    let size_bytes = meta.len();
    let content_type = match artifact_kind_from_path(&path) {
        "json" => "application/json",
        "jsonl" => "application/jsonl",
        _ => "text/plain",
    };

    let max_bytes = max_bytes.clamp(1024, 64 * 1024);
    let tail_lines = tail_lines.unwrap_or(0).min(2000);

    if tail_lines > 0 {
        let (buf, truncated) = read_file_tail_bytes(&path, max_bytes)?;
        let text = String::from_utf8_lossy(&buf);
        let text = tail_lines_from_text(&text, tail_lines);
        return Ok(serde_json::json!({
            "ok": true,
            "artifact_ref": artifact_ref.trim(),
            "content_type": content_type,
            "size_bytes": size_bytes,
            "truncated": truncated,
            "text": text,
        }));
    }

    let (buf, truncated) = read_file_head_bytes(&path, max_bytes)?;
    let kind = artifact_kind_from_path(&path);
    if kind == "json" {
        let text = String::from_utf8_lossy(&buf);
        let json = serde_json::from_str::<serde_json::Value>(&text).map_err(|err| {
            format!("Failed to parse JSON (use tail_lines for logs or adjust max_bytes): {err}")
        })?;
        if let Some(ptr) = json_pointer.map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let Some(selected) = json.pointer(ptr) else {
                return Err(format!("JSON pointer not found: {ptr}"));
            };
            return Ok(serde_json::json!({
                "ok": true,
                "artifact_ref": artifact_ref.trim(),
                "content_type": content_type,
                "size_bytes": size_bytes,
                "truncated": truncated,
                "json_pointer": ptr,
                "json": selected,
            }));
        }
        return Ok(serde_json::json!({
            "ok": true,
            "artifact_ref": artifact_ref.trim(),
            "content_type": content_type,
            "size_bytes": size_bytes,
            "truncated": truncated,
            "json": json,
        }));
    }

    let text = String::from_utf8_lossy(&buf).to_string();
    Ok(serde_json::json!({
        "ok": true,
        "artifact_ref": artifact_ref.trim(),
        "content_type": content_type,
        "size_bytes": size_bytes,
        "truncated": truncated,
        "text": text,
    }))
}

fn is_searchable_artifact(path: &Path) -> bool {
    matches!(
        artifact_kind_from_path(path),
        "json" | "jsonl" | "log" | "txt"
    )
}

fn truncate_line_for_search(line: &str) -> String {
    const MAX_CHARS: usize = 400;
    if line.chars().count() <= MAX_CHARS {
        return line.to_string();
    }
    let mut out = String::with_capacity(MAX_CHARS + 24);
    for ch in line.chars().take(MAX_CHARS) {
        out.push(ch);
    }
    out.push_str("…(truncated)");
    out
}

pub(super) fn search_artifacts_v1(
    run_dir: &Path,
    query: &str,
    path_prefix: Option<&str>,
    max_matches: usize,
    max_bytes_per_file: usize,
) -> Result<(Vec<serde_json::Value>, bool), String> {
    if !run_dir.is_dir() {
        return Err(format!(
            "Gen3D run dir does not exist or is not a directory: {}",
            run_dir.display()
        ));
    }
    let query = query.trim();
    if query.is_empty() {
        return Err("Missing args.query".into());
    }

    let base_dir = if let Some(prefix) = path_prefix {
        let rel = normalize_artifact_ref(prefix)?;
        let base = run_dir.join(rel);
        if !base.exists() {
            return Err(format!(
                "path_prefix does not exist under the current run dir: {}",
                prefix.trim()
            ));
        }
        base
    } else {
        run_dir.to_path_buf()
    };

    let max_matches = max_matches.clamp(1, 200);
    let max_bytes_per_file = max_bytes_per_file.clamp(1024, 128 * 1024);

    let mut matches_out = Vec::new();
    let mut truncated = false;

    fn walk(
        run_dir: &Path,
        dir: &Path,
        query: &str,
        max_matches: usize,
        max_bytes_per_file: usize,
        matches_out: &mut Vec<serde_json::Value>,
        truncated: &mut bool,
    ) -> Result<(), String> {
        if *truncated || matches_out.len() >= max_matches {
            *truncated = true;
            return Ok(());
        }

        for entry in read_dir_sorted(dir)? {
            if *truncated || matches_out.len() >= max_matches {
                *truncated = true;
                break;
            }
            let path = entry.path();
            let meta = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if meta.is_dir() {
                walk(
                    run_dir,
                    &path,
                    query,
                    max_matches,
                    max_bytes_per_file,
                    matches_out,
                    truncated,
                )?;
                continue;
            }
            if !meta.is_file() {
                continue;
            }
            if !is_searchable_artifact(&path) {
                continue;
            }
            let Some(artifact_ref) = artifact_ref_from_path(run_dir, &path) else {
                continue;
            };

            let (head, _head_truncated) = read_file_head_bytes(&path, max_bytes_per_file)?;
            let head_text = String::from_utf8_lossy(&head);
            for line in head_text.lines() {
                if line.contains(query) {
                    matches_out.push(serde_json::json!({
                        "artifact_ref": artifact_ref,
                        "where": "head",
                        "line": truncate_line_for_search(line),
                    }));
                    if matches_out.len() >= max_matches {
                        *truncated = true;
                        return Ok(());
                    }
                }
            }

            if meta.len() as usize > max_bytes_per_file {
                let (tail, _tail_truncated) = read_file_tail_bytes(&path, max_bytes_per_file)?;
                let tail_text = String::from_utf8_lossy(&tail);
                for line in tail_text.lines() {
                    if line.contains(query) {
                        matches_out.push(serde_json::json!({
                            "artifact_ref": artifact_ref,
                            "where": "tail",
                            "line": truncate_line_for_search(line),
                        }));
                        if matches_out.len() >= max_matches {
                            *truncated = true;
                            return Ok(());
                        }
                    }
                }
            }
        }

        Ok(())
    }

    if base_dir.is_file() {
        if !is_searchable_artifact(&base_dir) {
            return Ok((Vec::new(), false));
        }
        let Some(artifact_ref) = artifact_ref_from_path(run_dir, &base_dir) else {
            return Ok((Vec::new(), false));
        };
        let (head, _head_truncated) = read_file_head_bytes(&base_dir, max_bytes_per_file)?;
        let head_text = String::from_utf8_lossy(&head);
        for line in head_text.lines() {
            if line.contains(query) {
                matches_out.push(serde_json::json!({
                    "artifact_ref": artifact_ref,
                    "where": "head",
                    "line": truncate_line_for_search(line),
                }));
                if matches_out.len() >= max_matches {
                    truncated = true;
                    break;
                }
            }
        }
        return Ok((matches_out, truncated));
    }

    walk(
        run_dir,
        &base_dir,
        query,
        max_matches,
        max_bytes_per_file,
        &mut matches_out,
        &mut truncated,
    )?;
    Ok((matches_out, truncated))
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
    fn normalize_artifact_ref_rejects_traversal_and_absolute() {
        assert!(normalize_artifact_ref("").is_err());
        assert!(normalize_artifact_ref("..").is_err());
        assert!(normalize_artifact_ref("../x").is_err());
        assert!(normalize_artifact_ref("./x").is_err());
        assert!(normalize_artifact_ref("/abs/path").is_err());
        assert!(normalize_artifact_ref("C:\\abs\\path").is_err());
        assert!(normalize_artifact_ref("attempt_0/pass_0/validate.json").is_ok());
    }

    #[test]
    fn list_read_and_search_artifacts_smoke() {
        let run_dir = make_temp_dir("gravimera_gen3d_run_artifacts_test");
        let pass0 = run_dir.join("attempt_0").join("pass_0");
        let pass1 = run_dir.join("attempt_0").join("pass_1");
        std::fs::create_dir_all(&pass0).expect("create pass0 dir");
        std::fs::create_dir_all(&pass1).expect("create pass1 dir");

        std::fs::write(
            pass0.join("gen3d_run.log"),
            "line 1\nERROR: something bad\nline 3\n",
        )
        .expect("write log");
        std::fs::write(
            pass1.join("validate.json"),
            serde_json::json!({"ok":true,"issues":[]}).to_string(),
        )
        .expect("write json");

        let (items, truncated) = list_run_artifacts_v1(&run_dir, None, 500).expect("list");
        assert!(!items.is_empty());
        assert!(!truncated);

        let read = read_artifact_v1(
            &run_dir,
            "attempt_0/pass_1/validate.json",
            64 * 1024,
            None,
            Some("/ok"),
        )
        .expect("read json pointer");
        assert_eq!(read.get("json").and_then(|v| v.as_bool()), Some(true));

        let (matches_out, search_truncated) =
            search_artifacts_v1(&run_dir, "ERROR", None, 200, 64 * 1024).expect("search");
        assert!(!matches_out.is_empty());
        assert!(!search_truncated);

        // Best-effort cleanup.
        let _ = std::fs::remove_dir_all(&run_dir);
    }
}
