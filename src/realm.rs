use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::scene_sources::SceneSourcesV1;

const ACTIVE_SELECTION_FILE_NAME: &str = "active.json";
const ACTIVE_SELECTION_FORMAT_VERSION: u32 = 1;

#[derive(Resource, Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActiveRealmScene {
    pub(crate) realm_id: String,
    pub(crate) scene_id: String,
}

impl Default for ActiveRealmScene {
    fn default() -> Self {
        Self {
            realm_id: crate::paths::default_realm_id().to_string(),
            scene_id: crate::paths::default_scene_id().to_string(),
        }
    }
}

#[derive(Resource, Debug, Default)]
pub(crate) struct PendingRealmSceneSwitch {
    pub(crate) target: Option<ActiveRealmScene>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ActiveSelectionFileV1 {
    format_version: u32,
    realm_id: String,
    scene_id: String,
}

pub(crate) fn active_selection_path() -> PathBuf {
    crate::paths::realms_dir().join(ACTIVE_SELECTION_FILE_NAME)
}

pub(crate) fn realm_startup_init(mut active: ResMut<ActiveRealmScene>) {
    if let Err(err) = migrate_legacy_scene_dat_to_default_realm() {
        warn!("{err}");
    }
    if let Err(err) = migrate_legacy_floor_storage_all_realms() {
        warn!("{err}");
    }
    if let Err(err) = crate::scene_floor_selection::migrate_legacy_scene_floor_selection_files() {
        warn!("{err}");
    }

    if let Some(loaded) = load_active_selection_from_disk() {
        *active = loaded;
    }

    if let Err(err) = ensure_scene_dirs(&active.realm_id, &active.scene_id) {
        warn!("{err}");
    }

    if let Err(err) = std::fs::create_dir_all(crate::paths::realm_prefabs_dir(&active.realm_id)) {
        warn!(
            "Failed to create realm prefabs dir {}: {err}",
            crate::paths::realm_prefabs_dir(&active.realm_id).display()
        );
    }
    if let Err(err) =
        crate::realm_prefab_packages::migrate_scene_prefab_packages_to_realm(&active.realm_id)
    {
        warn!("{err}");
    }

    if let Err(err) = ensure_scene_sources_scaffold(&active.realm_id, &active.scene_id) {
        warn!("{err}");
    }

    if let Err(err) = persist_active_selection_to_disk(&active.realm_id, &active.scene_id) {
        warn!("{err}");
    }
}

pub(crate) fn ensure_realm_scene_scaffold(realm_id: &str, scene_id: &str) -> Result<(), String> {
    let realm_id = sanitize_id(realm_id).ok_or_else(|| {
        "realm id contains invalid characters (allowed: [A-Za-z0-9._-])".to_string()
    })?;
    let scene_id = sanitize_id(scene_id).ok_or_else(|| {
        "scene id contains invalid characters (allowed: [A-Za-z0-9._-])".to_string()
    })?;

    ensure_scene_dirs(&realm_id, &scene_id)?;
    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(&realm_id)?;
    std::fs::create_dir_all(crate::paths::realm_prefabs_dir(&realm_id)).map_err(|err| {
        format!(
            "Failed to create realm prefabs dir {}: {err}",
            crate::paths::realm_prefabs_dir(&realm_id).display()
        )
    })?;
    crate::realm_prefab_packages::migrate_scene_prefab_packages_to_realm(&realm_id)?;
    ensure_scene_sources_scaffold(&realm_id, &scene_id)?;
    Ok(())
}

pub(crate) fn persist_active_selection(realm_id: &str, scene_id: &str) -> Result<(), String> {
    persist_active_selection_to_disk(realm_id, scene_id)
}

pub(crate) fn list_realms() -> Vec<String> {
    let dir = crate::paths::realms_dir();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![crate::paths::default_realm_id().to_string()];
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(name) = sanitize_id(name) {
            out.push(name);
        }
    }
    out.sort();
    if out.is_empty() {
        out.push(crate::paths::default_realm_id().to_string());
    }
    out
}

pub(crate) fn list_scenes(realm_id: &str) -> Vec<String> {
    let mut out = Vec::new();
    let dir = crate::paths::realm_dir(realm_id).join("scenes");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return vec![crate::paths::default_scene_id().to_string()];
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        if let Some(name) = sanitize_id(name) {
            out.push(name);
        }
    }
    out.sort();
    if out.is_empty() {
        out.push(crate::paths::default_scene_id().to_string());
    }
    out
}

pub(crate) fn scene_src_dir(active: &ActiveRealmScene) -> PathBuf {
    crate::paths::scene_src_dir(&active.realm_id, &active.scene_id)
}

pub(crate) fn scene_dat_path(active: &ActiveRealmScene) -> PathBuf {
    crate::paths::scene_dat_path(&active.realm_id, &active.scene_id)
}

pub(crate) fn load_scene_description(src_dir: &Path) -> Result<String, String> {
    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    Ok(sources
        .meta_json
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string())
}

pub(crate) fn save_scene_description(src_dir: &Path, description: &str) -> Result<(), String> {
    let mut sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    sources.meta_json["description"] = Value::from(description);
    sources
        .write_to_dir(src_dir)
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn ensure_scene_dirs(realm_id: &str, scene_id: &str) -> Result<(), String> {
    let realm_id = sanitize_id(realm_id).ok_or_else(|| {
        "realm id contains invalid characters (allowed: [A-Za-z0-9._-])".to_string()
    })?;
    let scene_id = sanitize_id(scene_id).ok_or_else(|| {
        "scene id contains invalid characters (allowed: [A-Za-z0-9._-])".to_string()
    })?;

    let realm_dir = crate::paths::realm_dir(&realm_id);
    std::fs::create_dir_all(realm_dir.join("scenes")).map_err(|err| {
        format!(
            "Failed to create realm scenes dir {}: {err}",
            realm_dir.display()
        )
    })?;

    let scene_dir = crate::paths::scene_dir(&realm_id, &scene_id);
    std::fs::create_dir_all(crate::paths::scene_build_dir(&realm_id, &scene_id)).map_err(
        |err| {
            format!(
                "Failed to create scene build dir {}: {err}",
                scene_dir.display()
            )
        },
    )?;
    std::fs::create_dir_all(crate::paths::scene_src_dir(&realm_id, &scene_id)).map_err(|err| {
        format!(
            "Failed to create scene src dir {}: {err}",
            scene_dir.display()
        )
    })?;
    Ok(())
}

fn ensure_scene_sources_scaffold(realm_id: &str, scene_id: &str) -> Result<(), String> {
    let src_dir = crate::paths::scene_src_dir(realm_id, scene_id);
    if src_dir
        .join(crate::scene_sources::SCENE_SOURCES_INDEX_FILE_NAME)
        .exists()
    {
        return Ok(());
    }

    std::fs::create_dir_all(src_dir.join("style"))
        .map_err(|err| format!("Failed to create {}: {err}", src_dir.display()))?;
    std::fs::create_dir_all(src_dir.join("portals"))
        .map_err(|err| format!("Failed to create {}: {err}", src_dir.display()))?;
    std::fs::create_dir_all(src_dir.join("layers"))
        .map_err(|err| format!("Failed to create {}: {err}", src_dir.display()))?;
    std::fs::create_dir_all(src_dir.join("pinned_instances"))
        .map_err(|err| format!("Failed to create {}: {err}", src_dir.display()))?;

    let sources = SceneSourcesV1 {
        index_json: json!({
            "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
            "meta_path": "meta.json",
            "markers_path": "markers.json",
            "style_pack_ref_path": "style/style_pack_ref.json",
            "portals_dir": "portals",
            "layers_dir": "layers",
            "pinned_instances_dir": "pinned_instances"
        }),
        meta_json: json!({
            "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
            "scene_id": scene_id,
            "label": scene_id,
            "description": "",
            "tags": []
        }),
        markers_json: json!({
            "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
            "markers": {
                "spawn": {
                    "pos": { "x": 0.0, "y": 0.0, "z": 0.0 },
                    "yaw": 0.0
                }
            }
        }),
        style_pack_ref_json: json!({
            "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
            "kind": "builtin",
            "style_pack_id": "default"
        }),
        extra_json_files: BTreeMap::new(),
    };

    sources
        .write_to_dir(&src_dir)
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn migrate_legacy_scene_dat_to_default_realm() -> Result<(), String> {
    let dst = crate::paths::scene_dat_path(
        crate::paths::default_realm_id(),
        crate::paths::default_scene_id(),
    );
    if dst.exists() {
        return Ok(());
    }

    let src = crate::paths::legacy_scene_dat_path();
    if src.exists() {
        return migrate_scene_dat(&src, &dst);
    }

    let Some(legacy_exe) = crate::paths::legacy_path_next_to_exe("scene.dat") else {
        return Ok(());
    };
    if !legacy_exe.exists() {
        return Ok(());
    }
    migrate_scene_dat_copy(&legacy_exe, &dst)
}

fn migrate_scene_dat(src: &Path, dst: &Path) -> Result<(), String> {
    let Some(parent) = dst.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|err| {
        format!(
            "Failed to create realm scene dir {}: {err}",
            parent.display()
        )
    })?;

    match std::fs::rename(&src, &dst) {
        Ok(_) => {
            info!(
                "Migrated legacy scene file from {} to {}.",
                src.display(),
                dst.display()
            );
            Ok(())
        }
        Err(rename_err) => {
            std::fs::copy(&src, &dst).map_err(|err| {
                format!(
                    "Failed to migrate legacy scene file {} to {}: {err}",
                    src.display(),
                    dst.display()
                )
            })?;
            let _ = std::fs::remove_file(&src);
            info!(
                "Migrated legacy scene file from {} to {} (copy+remove after rename error: {rename_err}).",
                src.display(),
                dst.display()
            );
            Ok(())
        }
    }
}

fn migrate_scene_dat_copy(src: &Path, dst: &Path) -> Result<(), String> {
    let Some(parent) = dst.parent() else {
        return Ok(());
    };
    std::fs::create_dir_all(parent).map_err(|err| {
        format!(
            "Failed to create realm scene dir {}: {err}",
            parent.display()
        )
    })?;

    std::fs::copy(src, dst).map_err(|err| {
        format!(
            "Failed to migrate legacy scene file {} to {}: {err}",
            src.display(),
            dst.display()
        )
    })?;
    info!(
        "Migrated legacy scene file from {} to {} (copied).",
        src.display(),
        dst.display()
    );
    Ok(())
}

fn migrate_legacy_floor_storage_all_realms() -> Result<(), String> {
    let realms_dir = crate::paths::realms_dir();
    if !realms_dir.exists() {
        return Ok(());
    }

    let mut errors = Vec::new();
    let entries = std::fs::read_dir(&realms_dir)
        .map_err(|err| format!("Failed to list {}: {err}", realms_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read realm entry: {err}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(realm_id) = sanitize_id(name) else {
            continue;
        };
        if let Err(err) =
            crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(&realm_id)
        {
            errors.push(err);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

fn load_active_selection_from_disk() -> Option<ActiveRealmScene> {
    let path = active_selection_path();
    let bytes = std::fs::read(&path).ok()?;
    let parsed: ActiveSelectionFileV1 = serde_json::from_slice(&bytes).ok()?;
    if parsed.format_version != ACTIVE_SELECTION_FORMAT_VERSION {
        return None;
    }
    let realm_id = sanitize_id(&parsed.realm_id)?;
    let scene_id = sanitize_id(&parsed.scene_id)?;
    Some(ActiveRealmScene { realm_id, scene_id })
}

fn persist_active_selection_to_disk(realm_id: &str, scene_id: &str) -> Result<(), String> {
    let realm_id = sanitize_id(realm_id)
        .ok_or_else(|| "active realm id contains invalid characters".to_string())?;
    let scene_id = sanitize_id(scene_id)
        .ok_or_else(|| "active scene id contains invalid characters".to_string())?;
    let doc = ActiveSelectionFileV1 {
        format_version: ACTIVE_SELECTION_FORMAT_VERSION,
        realm_id,
        scene_id,
    };
    let bytes = serde_json::to_vec_pretty(&doc).map_err(|err| err.to_string())?;
    let path = active_selection_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "Failed to create active selection dir {}: {err}",
                parent.display()
            )
        })?;
    }
    std::fs::write(&path, format!("{}\n", String::from_utf8_lossy(&bytes)))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

pub(crate) fn sanitize_id(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return None;
    }
    if trimmed == "." || trimmed == ".." || trimmed.contains("..") {
        return None;
    }
    if !trimmed
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    Some(trimmed.to_string())
}
