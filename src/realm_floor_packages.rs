use std::path::{Path, PathBuf};

use bevy::prelude::debug;

use crate::genfloor::defs::FloorDefV1;

const PACKAGE_MATERIALS_DIR_NAME: &str = "materials";
const PACKAGE_GENFLOOR_SOURCE_DIR_NAME: &str = "genfloor_source_v1";
const PACKAGE_FLOOR_DEF_FILE_NAME: &str = "floor_def_v1.json";
const PACKAGE_THUMBNAIL_FILE_NAME: &str = "thumbnail.png";

pub(crate) fn realm_floors_root_dir(realm_id: &str) -> PathBuf {
    crate::paths::realm_floors_dir(realm_id)
}

pub(crate) fn realm_floor_package_dir(realm_id: &str, floor_id: u128) -> PathBuf {
    crate::paths::realm_floor_package_dir(realm_id, floor_id)
}

#[allow(dead_code)]
pub(crate) fn realm_floor_package_materials_dir(realm_id: &str, floor_id: u128) -> PathBuf {
    realm_floor_package_dir(realm_id, floor_id).join(PACKAGE_MATERIALS_DIR_NAME)
}

pub(crate) fn realm_floor_package_genfloor_source_dir(realm_id: &str, floor_id: u128) -> PathBuf {
    realm_floor_package_dir(realm_id, floor_id).join(PACKAGE_GENFLOOR_SOURCE_DIR_NAME)
}

pub(crate) fn realm_floor_package_floor_def_path(realm_id: &str, floor_id: u128) -> PathBuf {
    realm_floor_package_dir(realm_id, floor_id).join(PACKAGE_FLOOR_DEF_FILE_NAME)
}

pub(crate) fn realm_floor_package_thumbnail_path(realm_id: &str, floor_id: u128) -> PathBuf {
    realm_floor_package_dir(realm_id, floor_id).join(PACKAGE_THUMBNAIL_FILE_NAME)
}

pub(crate) fn list_realm_floor_packages(realm_id: &str) -> Result<Vec<u128>, String> {
    list_floor_packages_in_dir(&realm_floors_root_dir(realm_id))
}

fn list_floor_packages_in_dir(root: &Path) -> Result<Vec<u128>, String> {
    if !root.exists() {
        return Ok(Vec::new());
    }

    let entries = std::fs::read_dir(root)
        .map_err(|err| format!("Failed to list {}: {err}", root.display()))?;

    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        let Ok(uuid) = uuid::Uuid::parse_str(name.trim()) else {
            continue;
        };
        out.push(uuid.as_u128());
    }

    out.sort();
    Ok(out)
}

pub(crate) fn ensure_realm_floor_package_dirs(
    realm_id: &str,
    floor_id: u128,
) -> Result<(PathBuf, PathBuf, PathBuf), String> {
    let package_dir = realm_floor_package_dir(realm_id, floor_id);
    std::fs::create_dir_all(&package_dir)
        .map_err(|err| format!("Failed to create {}: {err}", package_dir.display()))?;

    let materials_dir = package_dir.join(PACKAGE_MATERIALS_DIR_NAME);
    std::fs::create_dir_all(&materials_dir)
        .map_err(|err| format!("Failed to create {}: {err}", materials_dir.display()))?;

    let source_dir = package_dir.join(PACKAGE_GENFLOOR_SOURCE_DIR_NAME);
    std::fs::create_dir_all(&source_dir)
        .map_err(|err| format!("Failed to create {}: {err}", source_dir.display()))?;

    Ok((package_dir, materials_dir, source_dir))
}

pub(crate) fn save_realm_floor_def(
    realm_id: &str,
    floor_id: u128,
    def: &FloorDefV1,
) -> Result<PathBuf, String> {
    let (package_dir, _materials_dir, _source_dir) =
        ensure_realm_floor_package_dirs(realm_id, floor_id)?;
    let path = package_dir.join(PACKAGE_FLOOR_DEF_FILE_NAME);
    let json = serde_json::to_string_pretty(def)
        .map_err(|err| format!("Failed to encode floor def JSON: {err}"))?;
    std::fs::write(&path, json)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    Ok(path)
}

pub(crate) fn load_realm_floor_def(realm_id: &str, floor_id: u128) -> Result<FloorDefV1, String> {
    let path = realm_floor_package_floor_def_path(realm_id, floor_id);
    let text = std::fs::read_to_string(&path)
        .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let mut def: FloorDefV1 =
        serde_json::from_str(&text).map_err(|err| format!("Invalid floor def JSON: {err}"))?;
    def.canonicalize_in_place();
    Ok(def)
}

#[allow(dead_code)]
pub(crate) fn delete_realm_floor_package(realm_id: &str, floor_id: u128) -> Result<bool, String> {
    let root = realm_floor_package_dir(realm_id, floor_id);
    if !root.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&root)
        .map_err(|err| format!("Failed to delete floor package {}: {err}", root.display()))?;
    Ok(true)
}

#[allow(dead_code)]
pub(crate) fn debug_log_missing_realm_floor_package(realm_id: &str, floor_id: u128) {
    let root = realm_floor_package_dir(realm_id, floor_id);
    if !root.exists() {
        debug!(
            "Realm floors: missing package dir for {} (expected {}).",
            uuid::Uuid::from_u128(floor_id),
            root.display()
        );
        return;
    }
    let def_path = root.join(PACKAGE_FLOOR_DEF_FILE_NAME);
    if !def_path.exists() {
        debug!(
            "Realm floors: missing floor def for {} (expected {}).",
            uuid::Uuid::from_u128(floor_id),
            def_path.display()
        );
    }
}
