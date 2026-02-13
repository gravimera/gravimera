#![cfg_attr(not(test), allow(dead_code))]

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

pub(crate) const SCENE_SOURCES_FORMAT_VERSION: u32 = 1;
pub(crate) const SCENE_SOURCES_INDEX_FILE_NAME: &str = "index.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SceneSourcesIndexPaths {
    pub(crate) meta_path: PathBuf,
    pub(crate) markers_path: PathBuf,
    pub(crate) style_pack_ref_path: PathBuf,
    pub(crate) portals_dir: PathBuf,
    pub(crate) layers_dir: PathBuf,
    pub(crate) pinned_instances_dir: PathBuf,
}

impl SceneSourcesIndexPaths {
    pub(crate) fn from_index_json_value(index: &Value) -> Result<Self, SceneSourcesError> {
        let meta_path = get_required_rel_path(index, "meta_path")?;
        let markers_path = get_required_rel_path(index, "markers_path")?;
        let style_pack_ref_path = get_required_rel_path(index, "style_pack_ref_path")?;
        let portals_dir = get_required_rel_path(index, "portals_dir")?;
        let layers_dir = get_required_rel_path(index, "layers_dir")?;
        let pinned_instances_dir = get_required_rel_path(index, "pinned_instances_dir")?;

        Ok(Self {
            meta_path,
            markers_path,
            style_pack_ref_path,
            portals_dir,
            layers_dir,
            pinned_instances_dir,
        })
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SceneSourcesV1 {
    pub(crate) index_json: Value,
    pub(crate) meta_json: Value,
    pub(crate) markers_json: Value,
    pub(crate) style_pack_ref_json: Value,
    pub(crate) extra_json_files: BTreeMap<PathBuf, Value>,
}

impl SceneSourcesV1 {
    pub(crate) fn load_from_dir(src_dir: &Path) -> Result<Self, SceneSourcesError> {
        let index_path = src_dir.join(SCENE_SOURCES_INDEX_FILE_NAME);
        let mut index_json = read_json_file(&index_path)?;
        canonicalize_json_value(&mut index_json);
        validate_format_version(&index_path, &index_json)?;

        let paths = SceneSourcesIndexPaths::from_index_json_value(&index_json)?;
        let meta_path = src_dir.join(&paths.meta_path);
        let markers_path = src_dir.join(&paths.markers_path);
        let style_pack_ref_path = src_dir.join(&paths.style_pack_ref_path);

        let mut meta_json = read_json_file(&meta_path)?;
        canonicalize_json_value(&mut meta_json);
        validate_format_version(&meta_path, &meta_json)?;
        canonicalize_known_lists_in_meta(&mut meta_json);

        let mut markers_json = read_json_file(&markers_path)?;
        canonicalize_json_value(&mut markers_json);
        validate_format_version(&markers_path, &markers_json)?;

        let mut style_pack_ref_json = read_json_file(&style_pack_ref_path)?;
        canonicalize_json_value(&mut style_pack_ref_json);
        validate_format_version(&style_pack_ref_path, &style_pack_ref_json)?;

        let mut extra_json_files: BTreeMap<PathBuf, Value> = BTreeMap::new();
        for rel_dir in [
            paths.portals_dir,
            paths.layers_dir,
            paths.pinned_instances_dir,
        ] {
            let dir_path = src_dir.join(&rel_dir);
            if !dir_path.exists() {
                continue;
            }
            let mut found = find_json_files(&dir_path)?;
            found.sort();
            for abs_path in found {
                let rel_path = abs_path.strip_prefix(src_dir).map_err(|_| {
                    SceneSourcesError::InvalidPath {
                        message: format!(
                            "JSON file is not under src dir: src_dir={} file={}",
                            src_dir.display(),
                            abs_path.display()
                        ),
                    }
                })?;
                let mut value = read_json_file(&abs_path)?;
                canonicalize_json_value(&mut value);
                extra_json_files.insert(rel_path.to_path_buf(), value);
            }
        }

        Ok(Self {
            index_json,
            meta_json,
            markers_json,
            style_pack_ref_json,
            extra_json_files,
        })
    }

    pub(crate) fn write_to_dir(&self, src_dir: &Path) -> Result<(), SceneSourcesError> {
        let index_path = src_dir.join(SCENE_SOURCES_INDEX_FILE_NAME);
        write_json_file_canonical(&index_path, &self.index_json)?;

        let paths = SceneSourcesIndexPaths::from_index_json_value(&self.index_json)?;
        write_json_file_canonical(&src_dir.join(paths.meta_path), &self.meta_json)?;
        write_json_file_canonical(&src_dir.join(paths.markers_path), &self.markers_json)?;
        write_json_file_canonical(
            &src_dir.join(paths.style_pack_ref_path),
            &self.style_pack_ref_json,
        )?;

        for (rel_path, value) in &self.extra_json_files {
            write_json_file_canonical(&src_dir.join(rel_path), value)?;
        }

        Ok(())
    }

    pub(crate) fn canonicalize_dir_in_place(src_dir: &Path) -> Result<(), SceneSourcesError> {
        let sources = Self::load_from_dir(src_dir)?;
        sources.write_to_dir(src_dir)?;
        Ok(())
    }
}

#[derive(Debug)]
pub(crate) enum SceneSourcesError {
    Io { path: PathBuf, error: io::Error },
    Json { path: PathBuf, error: serde_json::Error },
    InvalidFormatVersion { path: PathBuf, message: String },
    InvalidPath { message: String },
    MissingRequiredField { path: PathBuf, field: &'static str },
}

impl std::fmt::Display for SceneSourcesError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { path, error } => write!(f, "{}: io error: {}", path.display(), error),
            Self::Json { path, error } => write!(f, "{}: json error: {}", path.display(), error),
            Self::InvalidFormatVersion { path, message } => {
                write!(f, "{}: invalid format_version: {}", path.display(), message)
            }
            Self::InvalidPath { message } => write!(f, "invalid path: {}", message),
            Self::MissingRequiredField { path, field } => {
                write!(f, "{}: missing required field {}", path.display(), field)
            }
        }
    }
}

impl std::error::Error for SceneSourcesError {}

fn read_json_file(path: &Path) -> Result<Value, SceneSourcesError> {
    let bytes = fs::read(path).map_err(|error| SceneSourcesError::Io {
        path: path.to_path_buf(),
        error,
    })?;
    serde_json::from_slice(&bytes).map_err(|error| SceneSourcesError::Json {
        path: path.to_path_buf(),
        error,
    })
}

fn write_json_file_canonical(path: &Path, value: &Value) -> Result<(), SceneSourcesError> {
    let Some(parent) = path.parent() else {
        return Err(SceneSourcesError::InvalidPath {
            message: format!("no parent for path {}", path.display()),
        });
    };
    fs::create_dir_all(parent).map_err(|error| SceneSourcesError::Io {
        path: parent.to_path_buf(),
        error,
    })?;

    let bytes = canonical_json_bytes(value)?;

    // Avoid partial writes (important for future run/checkpoint semantics).
    let tmp_path = path.with_extension("json.tmp");
    fs::write(&tmp_path, &bytes).map_err(|error| SceneSourcesError::Io {
        path: tmp_path.clone(),
        error,
    })?;
    fs::rename(&tmp_path, path).map_err(|error| SceneSourcesError::Io {
        path: path.to_path_buf(),
        error,
    })?;

    Ok(())
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, SceneSourcesError> {
    let mut value = value.clone();
    canonicalize_json_value(&mut value);
    let text = serde_json::to_string_pretty(&value).map_err(|error| SceneSourcesError::Json {
        path: PathBuf::from("<serialize>"),
        error,
    })?;
    Ok(format!("{text}\n").into_bytes())
}

fn validate_format_version(path: &Path, doc: &Value) -> Result<(), SceneSourcesError> {
    let Some(v) = doc.get("format_version") else {
        return Err(SceneSourcesError::MissingRequiredField {
            path: path.to_path_buf(),
            field: "format_version",
        });
    };

    let Some(v) = v.as_u64() else {
        return Err(SceneSourcesError::InvalidFormatVersion {
            path: path.to_path_buf(),
            message: "format_version must be an integer".to_string(),
        });
    };

    if v != SCENE_SOURCES_FORMAT_VERSION as u64 {
        return Err(SceneSourcesError::InvalidFormatVersion {
            path: path.to_path_buf(),
            message: format!(
                "expected {}, got {}",
                SCENE_SOURCES_FORMAT_VERSION, v
            ),
        });
    }

    Ok(())
}

fn get_required_rel_path(doc: &Value, field: &'static str) -> Result<PathBuf, SceneSourcesError> {
    let Value::Object(map) = doc else {
        return Err(SceneSourcesError::InvalidPath {
            message: "index.json must be a JSON object".to_string(),
        });
    };
    let Some(value) = map.get(field) else {
        return Err(SceneSourcesError::MissingRequiredField {
            path: PathBuf::from(SCENE_SOURCES_INDEX_FILE_NAME),
            field,
        });
    };
    let Some(text) = value.as_str() else {
        return Err(SceneSourcesError::InvalidPath {
            message: format!("index.{field} must be a string"),
        });
    };

    let rel = PathBuf::from(text);
    if rel.is_absolute() {
        return Err(SceneSourcesError::InvalidPath {
            message: format!("index.{field} must be a relative path, got {}", rel.display()),
        });
    }
    for component in rel.components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(SceneSourcesError::InvalidPath {
                    message: format!("index.{field} must not contain '..'"),
                })
            }
            Component::Prefix(_) | Component::RootDir => {
                return Err(SceneSourcesError::InvalidPath {
                    message: format!("index.{field} must be a normal relative path"),
                })
            }
        }
    }

    Ok(rel)
}

fn find_json_files(dir: &Path) -> Result<Vec<PathBuf>, SceneSourcesError> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = fs::read_dir(&next).map_err(|error| SceneSourcesError::Io {
            path: next.clone(),
            error,
        })?;
        for entry in entries {
            let entry = entry.map_err(|error| SceneSourcesError::Io {
                path: next.clone(),
                error,
            })?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|v| v.to_str()) == Some("json") {
                out.push(path);
            }
        }
    }
    Ok(out)
}

fn canonicalize_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Recursively canonicalize children first.
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in &keys {
                if let Some(child) = map.get_mut(key) {
                    canonicalize_json_value(child);
                }
            }

            // Then sort keys deterministically (works even if serde_json switches to preserve_order).
            let mut sorted_keys = keys;
            sorted_keys.sort();
            let mut new_map = serde_json::Map::new();
            for key in sorted_keys {
                if let Some(value) = map.remove(&key) {
                    new_map.insert(key, value);
                }
            }
            *map = new_map;
        }
        Value::Array(items) => {
            for item in items {
                canonicalize_json_value(item);
            }
        }
        _ => {}
    }
}

fn canonicalize_known_lists_in_meta(meta_json: &mut Value) {
    let Value::Object(map) = meta_json else {
        return;
    };
    let Some(Value::Array(tags)) = map.get_mut("tags") else {
        return;
    };
    let mut out: Vec<String> = tags
        .iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    out.sort();
    out.dedup();
    *tags = out.into_iter().map(Value::String).collect();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::borrow::Cow;

    fn fixture_src_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/scene_generation/fixtures/minimal/src")
    }

    #[test]
    fn canonicalize_fixture_minimal_no_changes() {
        let fixture_src = fixture_src_dir();
        let temp_root = make_temp_dir("gravimera_scene_sources_fixture").unwrap();
        let temp_src = temp_root.join("src");
        copy_dir_recursive(&fixture_src, &temp_src).unwrap();

        SceneSourcesV1::canonicalize_dir_in_place(&temp_src).unwrap();

        let rel_files = collect_relative_json_files(&fixture_src).unwrap();
        for rel in rel_files {
            let expected = read_file_bytes(&fixture_src.join(&rel)).unwrap();
            let got = read_file_bytes(&temp_src.join(&rel)).unwrap();
            assert_eq!(
                expected,
                got,
                "file changed after canonicalize: {} diff:\n{}",
                rel.display(),
                diff_bytes(&expected, &got),
            );
        }
    }

    #[test]
    fn canonicalize_is_idempotent_and_preserves_unknown_fields() {
        let temp_root = make_temp_dir("gravimera_scene_sources_unknown").unwrap();
        let src_dir = temp_root.join("src");
        fs::create_dir_all(&src_dir).unwrap();

        // Intentionally non-canonical key order and an unknown nested object.
        let index = r#"{
  "markers_path": "markers.json",
  "pinned_instances_dir": "pinned_instances",
  "format_version": 1,
  "meta_path": "meta.json",
  "layers_dir": "layers",
  "portals_dir": "portals",
  "style_pack_ref_path": "style/style_pack_ref.json",
  "unknown_index": { "b": 2, "a": 1 }
}"#;
        write_file_bytes(&src_dir.join("index.json"), index.as_bytes()).unwrap();

        let meta = r#"{
  "tags": ["b", "a", "a"],
  "scene_id": "tmp",
  "format_version": 1,
  "unknown_meta": { "z": [3,2,1], "y": { "k2": "v2", "k1": "v1" } }
}"#;
        write_file_bytes(&src_dir.join("meta.json"), meta.as_bytes()).unwrap();

        let markers = r#"{
  "format_version": 1,
  "markers": {}
}"#;
        write_file_bytes(&src_dir.join("markers.json"), markers.as_bytes()).unwrap();

        fs::create_dir_all(src_dir.join("style")).unwrap();
        let style_pack_ref = r#"{
  "style_pack_id": "default",
  "format_version": 1,
  "kind": "builtin",
  "unknown_style": true
}"#;
        write_file_bytes(
            &src_dir.join("style/style_pack_ref.json"),
            style_pack_ref.as_bytes(),
        )
        .unwrap();

        // First canonicalize.
        SceneSourcesV1::canonicalize_dir_in_place(&src_dir).unwrap();
        let bytes_1 = read_file_bytes(&src_dir.join("meta.json")).unwrap();

        // Second canonicalize (must not change any bytes).
        SceneSourcesV1::canonicalize_dir_in_place(&src_dir).unwrap();
        let bytes_2 = read_file_bytes(&src_dir.join("meta.json")).unwrap();
        assert_eq!(bytes_1, bytes_2);

        // Unknown fields should still exist with the same value after round-trip.
        let sources = SceneSourcesV1::load_from_dir(&src_dir).unwrap();
        assert_eq!(sources.index_json["unknown_index"]["a"], Value::from(1));
        assert_eq!(sources.index_json["unknown_index"]["b"], Value::from(2));
        assert_eq!(sources.meta_json["unknown_meta"]["y"]["k1"], Value::from("v1"));
        assert_eq!(sources.meta_json["unknown_meta"]["y"]["k2"], Value::from("v2"));
        assert_eq!(sources.style_pack_ref_json["unknown_style"], Value::from(true));

        // Canonicalization sorts and dedups meta.tags.
        assert_eq!(sources.meta_json["tags"], Value::from(vec!["a", "b"]));
    }

    fn read_file_bytes(path: &Path) -> Result<Vec<u8>, SceneSourcesError> {
        fs::read(path).map_err(|error| SceneSourcesError::Io {
            path: path.to_path_buf(),
            error,
        })
    }

    fn write_file_bytes(path: &Path, bytes: &[u8]) -> Result<(), SceneSourcesError> {
        let Some(parent) = path.parent() else {
            return Err(SceneSourcesError::InvalidPath {
                message: format!("no parent for path {}", path.display()),
            });
        };
        fs::create_dir_all(parent).map_err(|error| SceneSourcesError::Io {
            path: parent.to_path_buf(),
            error,
        })?;
        fs::write(path, bytes).map_err(|error| SceneSourcesError::Io {
            path: path.to_path_buf(),
            error,
        })
    }

    fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), SceneSourcesError> {
        fs::create_dir_all(to).map_err(|error| SceneSourcesError::Io {
            path: to.to_path_buf(),
            error,
        })?;
        for entry in fs::read_dir(from).map_err(|error| SceneSourcesError::Io {
            path: from.to_path_buf(),
            error,
        })? {
            let entry = entry.map_err(|error| SceneSourcesError::Io {
                path: from.to_path_buf(),
                error,
            })?;
            let src_path = entry.path();
            let dst_path = to.join(entry.file_name());
            if src_path.is_dir() {
                copy_dir_recursive(&src_path, &dst_path)?;
            } else {
                let bytes = read_file_bytes(&src_path)?;
                write_file_bytes(&dst_path, &bytes)?;
            }
        }
        Ok(())
    }

    fn make_temp_dir(prefix: &str) -> Result<PathBuf, SceneSourcesError> {
        let mut dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("{prefix}_{pid}_{nanos}"));
        fs::create_dir_all(&dir).map_err(|error| SceneSourcesError::Io {
            path: dir.clone(),
            error,
        })?;
        Ok(dir)
    }

    fn collect_relative_json_files(root: &Path) -> Result<Vec<PathBuf>, SceneSourcesError> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(next) = stack.pop() {
            let entries = fs::read_dir(&next).map_err(|error| SceneSourcesError::Io {
                path: next.clone(),
                error,
            })?;
            for entry in entries {
                let entry = entry.map_err(|error| SceneSourcesError::Io {
                    path: next.clone(),
                    error,
                })?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(|v| v.to_str()) == Some("json") {
                    let rel =
                        path.strip_prefix(root).map_err(|_| SceneSourcesError::InvalidPath {
                            message: "strip_prefix failed".to_string(),
                        })?;
                    out.push(rel.to_path_buf());
                }
            }
        }
        out.sort();
        Ok(out)
    }

    fn diff_bytes<'a>(expected: &'a [u8], got: &'a [u8]) -> Cow<'a, str> {
        if expected == got {
            return Cow::Borrowed("");
        }
        let expected = String::from_utf8_lossy(expected);
        let got = String::from_utf8_lossy(got);
        Cow::Owned(format!("--- expected\n{expected}\n--- got\n{got}\n"))
    }
}
