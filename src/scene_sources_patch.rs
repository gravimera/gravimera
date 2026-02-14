use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::scene_sources::{SceneSourcesIndexPaths, SceneSourcesV1, SCENE_SOURCES_FORMAT_VERSION};

pub(crate) const SCENE_SOURCES_PATCH_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct SceneSourcesPatchV1 {
    pub(crate) format_version: u32,
    pub(crate) request_id: String,
    #[serde(default)]
    pub(crate) ops: Vec<SceneSourcesPatchOpV1>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind")]
pub(crate) enum SceneSourcesPatchOpV1 {
    #[serde(rename = "upsert_pinned_instance")]
    UpsertPinnedInstance {
        #[serde(default)]
        instance_id: Option<String>,
        #[serde(default)]
        local_ref: Option<String>,
        prefab_id: String,
        transform: Value,
        #[serde(default)]
        tint_rgba: Option<Value>,
    },
    #[serde(rename = "delete_pinned_instance")]
    DeletePinnedInstance { instance_id: String },

    #[serde(rename = "upsert_layer")]
    UpsertLayer { layer_id: String, doc: Value },
    #[serde(rename = "delete_layer")]
    DeleteLayer { layer_id: String },

    #[serde(rename = "upsert_portal")]
    UpsertPortal {
        portal_id: String,
        destination_scene_id: String,
        #[serde(default)]
        from_marker_id: Option<String>,
    },
    #[serde(rename = "delete_portal")]
    DeletePortal { portal_id: String },
}

#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct SceneSourcesPatchSummaryV1 {
    pub(crate) changed_paths: Vec<String>,
    pub(crate) derived_instance_ids: BTreeMap<String, String>,
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

fn require_non_empty(label: &str, value: &str) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} must be a non-empty string"));
    }
    Ok(trimmed.to_string())
}

fn derive_instance_uuid(scene_id: &str, request_id: &str, local_ref: &str) -> uuid::Uuid {
    let key = format!(
        "gravimera/scene_sources_patch/v1/scene/{scene_id}/request/{request_id}/local/{local_ref}"
    );
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes())
}

fn resolve_scene_id_for_patch(sources: &SceneSourcesV1) -> Result<String, String> {
    sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "meta.json missing scene_id".to_string())
}

fn upsert_extra_json(
    sources: &mut SceneSourcesV1,
    rel_path: PathBuf,
    mut doc: Value,
    changed: &mut BTreeSet<String>,
) {
    canonicalize_json_value(&mut doc);
    match sources.extra_json_files.get(&rel_path) {
        Some(existing) if existing == &doc => {}
        _ => {
            sources.extra_json_files.insert(rel_path.clone(), doc);
            changed.insert(rel_path.display().to_string());
        }
    }
}

fn remove_extra_json(sources: &mut SceneSourcesV1, rel_path: PathBuf, changed: &mut BTreeSet<String>) {
    if sources.extra_json_files.remove(&rel_path).is_some() {
        changed.insert(rel_path.display().to_string());
    }
}

fn rel_path_join(dir: &Path, file_name: &str) -> PathBuf {
    let mut out = PathBuf::from(dir);
    out.push(file_name);
    out
}

pub(crate) fn apply_patch_to_sources(
    sources: &mut SceneSourcesV1,
    patch: &SceneSourcesPatchV1,
) -> Result<SceneSourcesPatchSummaryV1, String> {
    if patch.format_version != SCENE_SOURCES_PATCH_FORMAT_VERSION {
        return Err(format!(
            "Unsupported patch format_version {} (expected {})",
            patch.format_version, SCENE_SOURCES_PATCH_FORMAT_VERSION
        ));
    }

    let request_id = require_non_empty("request_id", &patch.request_id)?;
    let scene_id = resolve_scene_id_for_patch(sources)?;

    let index_paths =
        SceneSourcesIndexPaths::from_index_json_value(&sources.index_json).map_err(|err| {
            format!("Invalid scene sources index.json: {err}")
        })?;
    let pinned_dir = index_paths.pinned_instances_dir;
    let layers_dir = index_paths.layers_dir;
    let portals_dir = index_paths.portals_dir;

    let mut derived_instance_ids = BTreeMap::new();
    let mut changed: BTreeSet<String> = BTreeSet::new();

    for op in &patch.ops {
        match op {
            SceneSourcesPatchOpV1::UpsertPinnedInstance {
                instance_id,
                local_ref,
                prefab_id,
                transform,
                tint_rgba,
            } => {
                let (instance_uuid, local_ref_key) = match (instance_id, local_ref) {
                    (Some(id), None) => {
                        let id = require_non_empty("instance_id", id)?;
                        (
                            uuid::Uuid::parse_str(&id)
                                .map_err(|err| format!("invalid instance_id UUID: {err}"))?,
                            None,
                        )
                    }
                    (None, Some(local)) => {
                        let local = require_non_empty("local_ref", local)?;
                        let derived = derive_instance_uuid(&scene_id, &request_id, &local);
                        derived_instance_ids.insert(local.clone(), derived.to_string());
                        (derived, Some(local))
                    }
                    (Some(_), Some(_)) => {
                        return Err(
                            "upsert_pinned_instance must provide exactly one of instance_id or local_ref"
                                .to_string(),
                        );
                    }
                    (None, None) => {
                        return Err(
                            "upsert_pinned_instance must provide instance_id or local_ref".to_string(),
                        );
                    }
                };

                let prefab_id = require_non_empty("prefab_id", prefab_id)?;
                let prefab_uuid = uuid::Uuid::parse_str(&prefab_id)
                    .map_err(|err| format!("invalid prefab_id UUID: {err}"))?;

                let mut doc = serde_json::json!({
                    "format_version": SCENE_SOURCES_FORMAT_VERSION,
                    "instance_id": instance_uuid.to_string(),
                    "prefab_id": prefab_uuid.to_string(),
                    "transform": transform,
                });

                if let Some(tint) = tint_rgba {
                    if let Some(obj) = doc.as_object_mut() {
                        obj.insert("tint_rgba".to_string(), tint.clone());
                    }
                }

                // Ensure deterministic doc bytes even when `transform` is injected from JSON.
                canonicalize_json_value(&mut doc);

                let rel_path = rel_path_join(
                    &pinned_dir,
                    &format!("{}.json", instance_uuid.to_string()),
                );
                upsert_extra_json(sources, rel_path, doc, &mut changed);

                let _ = local_ref_key;
            }
            SceneSourcesPatchOpV1::DeletePinnedInstance { instance_id } => {
                let instance_id = require_non_empty("instance_id", instance_id)?;
                let instance_uuid = uuid::Uuid::parse_str(&instance_id)
                    .map_err(|err| format!("invalid instance_id UUID: {err}"))?;
                let rel_path = rel_path_join(
                    &pinned_dir,
                    &format!("{}.json", instance_uuid.to_string()),
                );
                remove_extra_json(sources, rel_path, &mut changed);
            }
            SceneSourcesPatchOpV1::UpsertLayer { layer_id, doc } => {
                let layer_id = require_non_empty("layer_id", layer_id)?;
                let mut doc = doc.clone();
                if let Some(obj) = doc.as_object_mut() {
                    obj.insert(
                        "format_version".to_string(),
                        Value::from(SCENE_SOURCES_FORMAT_VERSION),
                    );
                    obj.insert("layer_id".to_string(), Value::from(layer_id.clone()));
                } else {
                    return Err("upsert_layer.doc must be a JSON object".to_string());
                }
                canonicalize_json_value(&mut doc);

                let rel_path = rel_path_join(&layers_dir, &format!("{layer_id}.json"));
                upsert_extra_json(sources, rel_path, doc, &mut changed);
            }
            SceneSourcesPatchOpV1::DeleteLayer { layer_id } => {
                let layer_id = require_non_empty("layer_id", layer_id)?;
                let rel_path = rel_path_join(&layers_dir, &format!("{layer_id}.json"));
                remove_extra_json(sources, rel_path, &mut changed);
            }
            SceneSourcesPatchOpV1::UpsertPortal {
                portal_id,
                destination_scene_id,
                from_marker_id,
            } => {
                let portal_id = require_non_empty("portal_id", portal_id)?;
                let destination_scene_id =
                    require_non_empty("destination_scene_id", destination_scene_id)?;
                let from_marker_id = from_marker_id
                    .as_deref()
                    .map(|v| require_non_empty("from_marker_id", v))
                    .transpose()?;

                let mut doc = serde_json::json!({
                    "format_version": SCENE_SOURCES_FORMAT_VERSION,
                    "portal_id": portal_id,
                    "destination_scene_id": destination_scene_id,
                });
                if let Some(from_marker_id) = from_marker_id {
                    if let Some(obj) = doc.as_object_mut() {
                        obj.insert("from_marker_id".to_string(), Value::from(from_marker_id));
                    }
                }
                canonicalize_json_value(&mut doc);

                let rel_path = rel_path_join(&portals_dir, &format!("{portal_id}.json"));
                upsert_extra_json(sources, rel_path, doc, &mut changed);
            }
            SceneSourcesPatchOpV1::DeletePortal { portal_id } => {
                let portal_id = require_non_empty("portal_id", portal_id)?;
                let rel_path = rel_path_join(&portals_dir, &format!("{portal_id}.json"));
                remove_extra_json(sources, rel_path, &mut changed);
            }
        }
    }

    Ok(SceneSourcesPatchSummaryV1 {
        changed_paths: changed.into_iter().collect(),
        derived_instance_ids,
    })
}

