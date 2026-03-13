#![allow(dead_code)]

use bevy::prelude::*;
use std::path::{Path, PathBuf};

use crate::object::registry::{ObjectDef, ObjectLibrary};

const PACKAGE_PREFABS_DIR_NAME: &str = "prefabs";
const PACKAGE_MATERIALS_DIR_NAME: &str = "materials";
const PACKAGE_GEN3D_SOURCE_DIR_NAME: &str = "gen3d_source_v1";
const PACKAGE_GEN3D_EDIT_BUNDLE_FILE_NAME: &str = "gen3d_edit_bundle_v1.json";

pub(crate) fn scene_prefabs_root_dir(realm_id: &str, scene_id: &str) -> PathBuf {
    crate::paths::scene_dir(realm_id, scene_id).join("prefabs")
}

pub(crate) fn scene_prefab_package_dir(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    crate::paths::scene_prefab_package_dir(realm_id, scene_id, root_prefab_id)
}

pub(crate) fn scene_prefab_package_prefabs_dir(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    scene_prefab_package_dir(realm_id, scene_id, root_prefab_id).join(PACKAGE_PREFABS_DIR_NAME)
}

pub(crate) fn scene_prefab_package_materials_dir(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    scene_prefab_package_dir(realm_id, scene_id, root_prefab_id).join(PACKAGE_MATERIALS_DIR_NAME)
}

pub(crate) fn scene_prefab_package_gen3d_source_dir(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    scene_prefab_package_dir(realm_id, scene_id, root_prefab_id).join(PACKAGE_GEN3D_SOURCE_DIR_NAME)
}

pub(crate) fn scene_prefab_package_gen3d_edit_bundle_path(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> PathBuf {
    scene_prefab_package_dir(realm_id, scene_id, root_prefab_id)
        .join(PACKAGE_GEN3D_EDIT_BUNDLE_FILE_NAME)
}

pub(crate) fn list_scene_prefab_packages(
    realm_id: &str,
    scene_id: &str,
) -> Result<Vec<u128>, String> {
    list_scene_prefab_packages_in_dir(&scene_prefabs_root_dir(realm_id, scene_id))
}

fn list_scene_prefab_packages_in_dir(root: &Path) -> Result<Vec<u128>, String> {
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

pub(crate) fn ensure_scene_prefab_package_dirs(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) -> Result<(PathBuf, PathBuf), String> {
    let prefabs_dir = scene_prefab_package_prefabs_dir(realm_id, scene_id, root_prefab_id);
    std::fs::create_dir_all(&prefabs_dir)
        .map_err(|err| format!("Failed to create {}: {err}", prefabs_dir.display()))?;

    let materials_dir = scene_prefab_package_materials_dir(realm_id, scene_id, root_prefab_id);
    std::fs::create_dir_all(&materials_dir)
        .map_err(|err| format!("Failed to create {}: {err}", materials_dir.display()))?;

    Ok((prefabs_dir, materials_dir))
}

pub(crate) fn save_scene_prefab_package_defs(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
    defs: &[ObjectDef],
) -> Result<PathBuf, String> {
    let (prefabs_dir, _materials_dir) =
        ensure_scene_prefab_package_dirs(realm_id, scene_id, root_prefab_id)?;
    crate::realm_prefabs::save_prefab_defs_to_dir(&prefabs_dir, root_prefab_id, defs)?;
    prune_stale_prefab_def_json_files(&prefabs_dir, defs)?;
    Ok(prefabs_dir)
}

pub(crate) fn load_scene_prefab_package_defs_into_library(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
    library: &mut ObjectLibrary,
) -> Result<usize, String> {
    let prefabs_dir = scene_prefab_package_prefabs_dir(realm_id, scene_id, root_prefab_id);
    crate::realm_prefabs::load_prefabs_into_library_from_dir(&prefabs_dir, library)
}

fn prune_stale_prefab_def_json_files(dir: &Path, defs: &[ObjectDef]) -> Result<(), String> {
    if !dir.exists() {
        return Ok(());
    }

    let mut keep: std::collections::HashSet<u128> = std::collections::HashSet::new();
    for def in defs {
        keep.insert(def.object_id);
    }

    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("Failed to list {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
        if !file_name.ends_with(".json") || file_name.ends_with(".desc.json") {
            continue;
        }
        let Some(stem) = file_name.strip_suffix(".json") else {
            continue;
        };
        let Ok(uuid) = uuid::Uuid::parse_str(stem.trim()) else {
            continue;
        };
        let id = uuid.as_u128();
        if keep.contains(&id) {
            continue;
        }
        std::fs::remove_file(&path).map_err(|err| {
            format!(
                "Failed to delete stale prefab def {}: {err}",
                path.display()
            )
        })?;
    }

    Ok(())
}

pub(crate) fn debug_log_missing_prefab_package(
    realm_id: &str,
    scene_id: &str,
    root_prefab_id: u128,
) {
    let root = scene_prefab_package_dir(realm_id, scene_id, root_prefab_id);
    if !root.exists() {
        debug!(
            "Scene prefabs: missing package dir for {} (expected {}).",
            uuid::Uuid::from_u128(root_prefab_id),
            root.display()
        );
        return;
    }
    let prefabs_dir = root.join(PACKAGE_PREFABS_DIR_NAME);
    if !prefabs_dir.exists() {
        debug!(
            "Scene prefabs: missing prefabs dir for {} (expected {}).",
            uuid::Uuid::from_u128(root_prefab_id),
            prefabs_dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_scene_prefab_packages_ignores_non_uuid_folders() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_prefabs_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        std::fs::create_dir_all(temp_root.join("not-a-uuid")).expect("create junk folder");
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        std::fs::create_dir_all(temp_root.join(uuid::Uuid::from_u128(prefab_id).to_string()))
            .expect("create uuid folder");

        let models = list_scene_prefab_packages_in_dir(&temp_root).expect("list prefabs");
        assert_eq!(models, vec![prefab_id]);

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
