use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

pub(crate) const PREFAB_DESCRIPTOR_FORMAT_VERSION: u32 = 1;

#[derive(Resource, Default)]
pub(crate) struct PrefabDescriptorLibrary {
    descriptors: HashMap<u128, PrefabDescriptorFileV1>,
}

impl PrefabDescriptorLibrary {
    pub(crate) fn clear(&mut self) {
        self.descriptors.clear();
    }

    pub(crate) fn upsert(&mut self, prefab_id: u128, descriptor: PrefabDescriptorFileV1) {
        self.descriptors.insert(prefab_id, descriptor);
    }

    pub(crate) fn get(&self, prefab_id: u128) -> Option<&PrefabDescriptorFileV1> {
        self.descriptors.get(&prefab_id)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorFileV1 {
    pub(crate) format_version: u32,
    pub(crate) prefab_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<PrefabDescriptorTextV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) roles: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) interfaces: Option<PrefabDescriptorInterfacesV1>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) provenance: Option<PrefabDescriptorProvenanceV1>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorFileV1 {
    pub(crate) fn canonicalize_in_place(&mut self) {
        self.prefab_id = self.prefab_id.trim().to_string();

        self.label = self.label.as_ref().map(|v| v.trim().to_string());
        if self.label.as_ref().is_some_and(|v| v.is_empty()) {
            self.label = None;
        }

        if let Some(text) = self.text.as_mut() {
            text.canonicalize_in_place();
            if text.is_empty() {
                self.text = None;
            }
        }

        canonicalize_string_list(&mut self.tags);
        canonicalize_string_list(&mut self.roles);

        if let Some(interfaces) = self.interfaces.as_mut() {
            interfaces.canonicalize_in_place();
            if interfaces.is_empty() {
                self.interfaces = None;
            }
        }

        if let Some(prov) = self.provenance.as_mut() {
            prov.canonicalize_in_place();
            if prov.is_empty() {
                self.provenance = None;
            }
        }
    }

    pub(crate) fn prefab_id_u128(&self) -> Result<u128, String> {
        let uuid = uuid::Uuid::parse_str(self.prefab_id.trim())
            .map_err(|err| format!("Invalid prefab_id UUID: {err}"))?;
        Ok(uuid.as_u128())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorTextV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) short: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) long: Option<String>,
}

impl PrefabDescriptorTextV1 {
    fn canonicalize_in_place(&mut self) {
        self.short = self.short.as_ref().map(|v| v.trim().to_string());
        if self.short.as_ref().is_some_and(|v| v.is_empty()) {
            self.short = None;
        }
        self.long = self.long.as_ref().map(|v| v.trim().to_string());
        if self.long.as_ref().is_some_and(|v| v.is_empty()) {
            self.long = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.short.as_ref().is_none_or(|v| v.trim().is_empty())
            && self.long.as_ref().is_none_or(|v| v.trim().is_empty())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorInterfacesV1 {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) anchors: Vec<PrefabDescriptorAnchorV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) animation_channels: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorInterfacesV1 {
    fn canonicalize_in_place(&mut self) {
        for anchor in &mut self.anchors {
            anchor.canonicalize_in_place();
        }
        self.anchors.retain(|a| !a.name.trim().is_empty());
        self.anchors.sort_by(|a, b| a.name.cmp(&b.name));

        canonicalize_string_list(&mut self.animation_channels);

        self.notes = self.notes.as_ref().map(|v| v.trim().to_string());
        if self.notes.as_ref().is_some_and(|v| v.is_empty()) {
            self.notes = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.anchors.is_empty()
            && self.animation_channels.is_empty()
            && self.notes.as_ref().is_none_or(|v| v.trim().is_empty())
            && self.extra.is_empty()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorAnchorV1 {
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) meaning: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) required: Option<bool>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorAnchorV1 {
    fn canonicalize_in_place(&mut self) {
        self.name = self.name.trim().to_string();
        self.meaning = self.meaning.as_ref().map(|v| v.trim().to_string());
        if self.meaning.as_ref().is_some_and(|v| v.is_empty()) {
            self.meaning = None;
        }
        self.notes = self.notes.as_ref().map(|v| v.trim().to_string());
        if self.notes.as_ref().is_some_and(|v| v.is_empty()) {
            self.notes = None;
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorProvenanceV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_at_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) created_duration_ms: Option<u128>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) modified_at_ms: Option<u128>,
    /// Total input tokens consumed by Gen3D across all saved revisions for this prefab (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) total_input_tokens: Option<u64>,
    /// Total output tokens consumed by Gen3D across all saved revisions for this prefab (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) total_output_tokens: Option<u64>,
    /// Total tokens consumed by Gen3D where an input/output breakdown was not available (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) total_unsplit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) gen3d: Option<PrefabDescriptorGen3dV1>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) revisions: Vec<PrefabDescriptorRevisionV1>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorProvenanceV1 {
    fn canonicalize_in_place(&mut self) {
        self.source = self.source.as_ref().map(|v| v.trim().to_string());
        if self.source.as_ref().is_some_and(|v| v.is_empty()) {
            self.source = None;
        }
        if let Some(gen3d) = self.gen3d.as_mut() {
            gen3d.canonicalize_in_place();
            if gen3d.is_empty() {
                self.gen3d = None;
            }
        }
        for rev in &mut self.revisions {
            rev.canonicalize_in_place();
        }
        self.revisions
            .retain(|rev| !rev.actor.trim().is_empty() && !rev.summary.trim().is_empty());
        self.revisions.sort_by_key(|rev| rev.rev);
    }

    fn is_empty(&self) -> bool {
        self.source.as_ref().is_none_or(|v| v.trim().is_empty())
            && self.created_at_ms.is_none()
            && self.created_duration_ms.is_none()
            && self.modified_at_ms.is_none()
            && self.total_input_tokens.is_none()
            && self.total_output_tokens.is_none()
            && self.total_unsplit_tokens.is_none()
            && self.gen3d.is_none()
            && self.revisions.is_empty()
            && self.extra.is_empty()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorGen3dV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) style_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) run_id: Option<String>,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorGen3dV1 {
    fn canonicalize_in_place(&mut self) {
        self.prompt = self.prompt.as_ref().map(|v| v.trim().to_string());
        if self.prompt.as_ref().is_some_and(|v| v.is_empty()) {
            self.prompt = None;
        }
        self.style_prompt = self.style_prompt.as_ref().map(|v| v.trim().to_string());
        if self.style_prompt.as_ref().is_some_and(|v| v.is_empty()) {
            self.style_prompt = None;
        }
        self.run_id = self.run_id.as_ref().map(|v| v.trim().to_string());
        if self.run_id.as_ref().is_some_and(|v| v.is_empty()) {
            self.run_id = None;
        }
    }

    fn is_empty(&self) -> bool {
        self.prompt.as_ref().is_none_or(|v| v.trim().is_empty())
            && self
                .style_prompt
                .as_ref()
                .is_none_or(|v| v.trim().is_empty())
            && self.run_id.as_ref().is_none_or(|v| v.trim().is_empty())
            && self.extra.is_empty()
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct PrefabDescriptorRevisionV1 {
    pub(crate) rev: u32,
    pub(crate) created_at_ms: u128,
    pub(crate) actor: String,
    pub(crate) summary: String,
    #[serde(flatten)]
    pub(crate) extra: BTreeMap<String, Value>,
}

impl PrefabDescriptorRevisionV1 {
    fn canonicalize_in_place(&mut self) {
        self.actor = self.actor.trim().to_string();
        self.summary = self.summary.trim().to_string();
    }
}

fn canonicalize_string_list(list: &mut Vec<String>) {
    let mut out: Vec<String> = list
        .iter()
        .map(|v| v.trim())
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string())
        .collect();
    out.sort();
    out.dedup();
    *list = out;
}

fn write_json_file_canonical(path: &Path, value: &Value) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("no parent for path {}", path.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;

    let bytes = canonical_json_bytes(value).map_err(|err| err.to_string())?;
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    std::fs::write(&tmp_path, &bytes)
        .map_err(|err| format!("Failed to write {}: {err}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|err| format!("Failed to rename {}: {err}", path.display()))?;

    Ok(())
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, serde_json::Error> {
    let mut value = value.clone();
    canonicalize_json_value(&mut value);
    let text = serde_json::to_string_pretty(&value)?;
    Ok(format!(
        "{text}
"
    )
    .into_bytes())
}

fn canonicalize_json_value(value: &mut Value) {
    match value {
        Value::Object(map) => {
            let keys: Vec<String> = map.keys().cloned().collect();
            for key in &keys {
                if let Some(child) = map.get_mut(key) {
                    canonicalize_json_value(child);
                }
            }

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

pub(crate) fn prefab_descriptor_path_for_prefab_json(prefab_json: &Path) -> PathBuf {
    let name = prefab_json
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("");
    if let Some(stem) = name.strip_suffix(".json") {
        return prefab_json.with_file_name(format!("{stem}.desc.json"));
    }
    prefab_json.with_extension("desc.json")
}

pub(crate) fn save_prefab_descriptor_file(
    path: &Path,
    descriptor: &PrefabDescriptorFileV1,
) -> Result<(), String> {
    let mut descriptor = descriptor.clone();
    descriptor.canonicalize_in_place();
    let value = serde_json::to_value(descriptor).map_err(|err| err.to_string())?;
    write_json_file_canonical(path, &value)
}

pub(crate) fn load_prefab_descriptors_from_dir(
    root: &Path,
    library: &mut PrefabDescriptorLibrary,
) -> Result<usize, String> {
    load_prefab_descriptors_from_packs_dir(root, library)
}

fn load_prefab_descriptors_from_packs_dir(
    packs_dir: &Path,
    library: &mut PrefabDescriptorLibrary,
) -> Result<usize, String> {
    if !packs_dir.exists() {
        return Ok(0);
    }

    let mut loaded = 0usize;
    let mut stack = vec![packs_dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next)
            .map_err(|err| format!("Failed to list {}: {err}", next.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
            if !file_name.ends_with(".desc.json") {
                continue;
            }

            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(err) => {
                    warn!(
                        "Prefab descriptors: failed to read {}: {err}",
                        path.display()
                    );
                    continue;
                }
            };
            let json: Value = match serde_json::from_slice(&bytes) {
                Ok(v) => v,
                Err(err) => {
                    warn!("Prefab descriptors: invalid JSON {}: {err}", path.display());
                    continue;
                }
            };
            let mut doc: PrefabDescriptorFileV1 = match serde_json::from_value(json) {
                Ok(v) => v,
                Err(err) => {
                    warn!(
                        "Prefab descriptors: schema mismatch {}: {err}",
                        path.display()
                    );
                    continue;
                }
            };
            if doc.format_version != PREFAB_DESCRIPTOR_FORMAT_VERSION {
                warn!(
                    "Prefab descriptors: ignoring {}: unsupported format_version {} (expected {}).",
                    path.display(),
                    doc.format_version,
                    PREFAB_DESCRIPTOR_FORMAT_VERSION
                );
                continue;
            }
            let prefab_id = match doc.prefab_id_u128() {
                Ok(id) => id,
                Err(err) => {
                    warn!("Prefab descriptors: skipping {}: {err}", path.display());
                    continue;
                }
            };

            doc.canonicalize_in_place();
            library.upsert(prefab_id, doc);
            loaded += 1;
        }
    }

    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_dir(prefix: &str) -> PathBuf {
        let mut dir = std::env::temp_dir();
        let pid = std::process::id();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        dir.push(format!("{prefix}_{pid}_{nanos}"));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn canonicalize_trims_and_sorts_lists() {
        let mut doc = PrefabDescriptorFileV1 {
            format_version: PREFAB_DESCRIPTOR_FORMAT_VERSION,
            prefab_id: " 00000000-0000-0000-0000-000000000000 ".to_string(),
            label: Some("  Test  ".to_string()),
            text: Some(PrefabDescriptorTextV1 {
                short: Some("  hello   world \n".to_string()),
                long: Some("   ".to_string()),
            }),
            tags: vec![
                " b ".to_string(),
                "a".to_string(),
                "".to_string(),
                "a".to_string(),
            ],
            roles: vec![" unit ".to_string(), "unit".to_string(), "  ".to_string()],
            interfaces: Some(PrefabDescriptorInterfacesV1 {
                anchors: vec![
                    PrefabDescriptorAnchorV1 {
                        name: " door ".to_string(),
                        meaning: Some("  entrance ".to_string()),
                        notes: Some("  ".to_string()),
                        required: None,
                        extra: Default::default(),
                    },
                    PrefabDescriptorAnchorV1 {
                        name: " ".to_string(),
                        meaning: None,
                        notes: None,
                        required: None,
                        extra: Default::default(),
                    },
                ],
                animation_channels: vec![" move ".to_string(), "idle".to_string(), "".to_string()],
                notes: Some("   ".to_string()),
                extra: Default::default(),
            }),
            provenance: Some(PrefabDescriptorProvenanceV1 {
                source: Some(" gen3d ".to_string()),
                created_at_ms: Some(123),
                created_duration_ms: None,
                modified_at_ms: None,
                total_input_tokens: None,
                total_output_tokens: None,
                total_unsplit_tokens: None,
                gen3d: Some(PrefabDescriptorGen3dV1 {
                    prompt: Some("  make a tower ".to_string()),
                    style_prompt: Some("  ".to_string()),
                    run_id: Some("  run ".to_string()),
                    extra: Default::default(),
                }),
                revisions: vec![PrefabDescriptorRevisionV1 {
                    rev: 2,
                    created_at_ms: 999,
                    actor: " human ".to_string(),
                    summary: "  edited ".to_string(),
                    extra: Default::default(),
                }],
                extra: Default::default(),
            }),
            extra: Default::default(),
        };

        doc.canonicalize_in_place();

        assert_eq!(
            doc.prefab_id,
            "00000000-0000-0000-0000-000000000000".to_string()
        );
        assert_eq!(doc.label.as_deref(), Some("Test"));
        assert_eq!(doc.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(doc.roles, vec!["unit".to_string()]);

        let text = doc.text.expect("text");
        assert_eq!(text.short.as_deref(), Some("hello   world"));
        assert!(text.long.is_none());

        let interfaces = doc.interfaces.expect("interfaces");
        assert_eq!(interfaces.anchors.len(), 1);
        assert_eq!(interfaces.anchors[0].name, "door".to_string());
        assert_eq!(interfaces.anchors[0].meaning.as_deref(), Some("entrance"));
        assert!(interfaces.anchors[0].notes.is_none());
        assert_eq!(
            interfaces.animation_channels,
            vec!["idle".to_string(), "move".to_string()]
        );
        assert!(interfaces.notes.is_none());

        let prov = doc.provenance.expect("provenance");
        assert_eq!(prov.source.as_deref(), Some("gen3d"));
        let gen3d = prov.gen3d.expect("gen3d");
        assert_eq!(gen3d.prompt.as_deref(), Some("make a tower"));
        assert!(gen3d.style_prompt.is_none());
        assert_eq!(gen3d.run_id.as_deref(), Some("run"));
        assert_eq!(prov.revisions.len(), 1);
        assert_eq!(prov.revisions[0].actor, "human".to_string());
        assert_eq!(prov.revisions[0].summary, "edited".to_string());
    }

    #[test]
    fn save_writes_canonical_json_with_newline() {
        let tmp = make_temp_dir("gravimera_prefab_desc_save");
        let path = tmp.join("thing.desc.json");

        let doc = PrefabDescriptorFileV1 {
            format_version: PREFAB_DESCRIPTOR_FORMAT_VERSION,
            prefab_id: "00000000-0000-0000-0000-000000000000".to_string(),
            label: Some("Thing".to_string()),
            text: None,
            tags: vec!["b".to_string(), "a".to_string(), "a".to_string()],
            roles: vec!["unit".to_string(), "unit".to_string()],
            interfaces: None,
            provenance: None,
            extra: Default::default(),
        };

        save_prefab_descriptor_file(&path, &doc).expect("save descriptor");
        let text = std::fs::read_to_string(&path).expect("read saved descriptor");
        assert!(
            text.ends_with('\n'),
            "descriptor file should end with newline"
        );

        let json: serde_json::Value = serde_json::from_str(&text).expect("parse saved JSON");
        let tags = json
            .get("tags")
            .and_then(|v| v.as_array())
            .expect("tags array");
        assert_eq!(tags[0].as_str(), Some("a"));
        assert_eq!(tags[1].as_str(), Some("b"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_from_packs_dir_skips_invalid_and_wrong_versions() {
        let tmp = make_temp_dir("gravimera_prefab_desc_load");
        let packs_dir = tmp.join("packs");
        let pack = packs_dir.join("generated").join("prefabs");
        std::fs::create_dir_all(&pack).expect("create pack dir");

        let good_id = uuid::Uuid::new_v4();
        let good_path = pack.join(format!("{}.desc.json", good_id));
        std::fs::write(
            &good_path,
            serde_json::json!({
                "format_version": 1,
                "prefab_id": good_id.to_string(),
                "label": "Good",
                "tags": ["b", "a"]
            })
            .to_string(),
        )
        .expect("write good descriptor");

        let bad_json_path = pack.join("bad.desc.json");
        std::fs::write(&bad_json_path, "{not json").expect("write bad json");

        let wrong_ver_id = uuid::Uuid::new_v4();
        let wrong_ver_path = pack.join(format!("{}.desc.json", wrong_ver_id));
        std::fs::write(
            &wrong_ver_path,
            serde_json::json!({
                "format_version": 999,
                "prefab_id": wrong_ver_id.to_string(),
            })
            .to_string(),
        )
        .expect("write wrong version");

        let mut lib = PrefabDescriptorLibrary::default();
        let loaded =
            load_prefab_descriptors_from_packs_dir(&packs_dir, &mut lib).expect("load descriptors");
        assert_eq!(loaded, 1);

        let stored = lib
            .get(good_id.as_u128())
            .expect("good descriptor should be loaded");
        assert_eq!(stored.label.as_deref(), Some("Good"));
        assert_eq!(stored.tags, vec!["a".to_string(), "b".to_string()]);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
