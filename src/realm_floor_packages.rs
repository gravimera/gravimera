use std::path::{Path, PathBuf};

use bevy::prelude::debug;

use crate::genfloor::defs::FloorDefV1;

const PACKAGE_MATERIALS_DIR_NAME: &str = "materials";
const PACKAGE_GENFLOOR_SOURCE_DIR_NAME: &str = "genfloor_source_v1";
const PACKAGE_FLOOR_DEF_FILE_NAME: &str = "terrain_def_v1.json";
const LEGACY_PACKAGE_FLOOR_DEF_FILE_NAME: &str = "floor_def_v1.json";
const PACKAGE_THUMBNAIL_FILE_NAME: &str = "thumbnail.png";

pub(crate) fn realm_floors_root_dir(realm_id: &str) -> PathBuf {
    crate::paths::realm_floors_dir(realm_id)
}

fn legacy_realm_floors_root_dir(realm_id: &str) -> PathBuf {
    crate::paths::legacy_realm_floors_dir(realm_id)
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
    migrate_legacy_floor_storage_for_realm(realm_id)?;
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
    migrate_legacy_floor_storage_for_realm(realm_id)?;

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

    let legacy = package_dir.join(LEGACY_PACKAGE_FLOOR_DEF_FILE_NAME);
    if legacy.exists() {
        let _ = std::fs::remove_file(&legacy);
    }

    Ok(path)
}

pub(crate) fn load_realm_floor_def(realm_id: &str, floor_id: u128) -> Result<FloorDefV1, String> {
    migrate_legacy_floor_storage_for_realm(realm_id)?;

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
    migrate_legacy_floor_storage_for_realm(realm_id)?;

    let root = realm_floor_package_dir(realm_id, floor_id);
    if !root.exists() {
        return Ok(false);
    }
    std::fs::remove_dir_all(&root)
        .map_err(|err| format!("Failed to delete floor package {}: {err}", root.display()))?;
    Ok(true)
}

pub(crate) fn duplicate_realm_floor_package(realm_id: &str, floor_id: u128) -> Result<u128, String> {
    migrate_legacy_floor_storage_for_realm(realm_id)?;

    let src = realm_floor_package_dir(realm_id, floor_id);
    if !src.exists() {
        return Err(format!(
            "Failed to duplicate terrain: missing package dir {}.",
            src.display()
        ));
    }

    let mut new_id = uuid::Uuid::new_v4().as_u128();
    let mut dst = realm_floor_package_dir(realm_id, new_id);
    let mut attempts = 0;
    while dst.exists() && attempts < 5 {
        new_id = uuid::Uuid::new_v4().as_u128();
        dst = realm_floor_package_dir(realm_id, new_id);
        attempts += 1;
    }
    if dst.exists() {
        return Err(format!(
            "Failed to duplicate terrain: destination already exists {}.",
            dst.display()
        ));
    }

    copy_dir_recursive(&src, &dst)?;
    Ok(new_id)
}

#[allow(dead_code)]
pub(crate) fn debug_log_missing_realm_floor_package(realm_id: &str, floor_id: u128) {
    let root = realm_floor_package_dir(realm_id, floor_id);
    if !root.exists() {
        debug!(
            "Realm terrain: missing package dir for {} (expected {}).",
            uuid::Uuid::from_u128(floor_id),
            root.display()
        );
        return;
    }
    let def_path = root.join(PACKAGE_FLOOR_DEF_FILE_NAME);
    if !def_path.exists() {
        debug!(
            "Realm terrain: missing terrain def for {} (expected {}).",
            uuid::Uuid::from_u128(floor_id),
            def_path.display()
        );
    }
}

pub(crate) fn migrate_legacy_floor_storage_for_realm(realm_id: &str) -> Result<(), String> {
    migrate_floor_storage_roots(
        &legacy_realm_floors_root_dir(realm_id),
        &realm_floors_root_dir(realm_id),
    )
}

fn migrate_floor_storage_roots(old_root: &Path, new_root: &Path) -> Result<(), String> {
    if old_root.exists() && !new_root.exists() {
        move_path_without_replacing(old_root, new_root)?;
    }

    if old_root.exists() {
        std::fs::create_dir_all(new_root)
            .map_err(|err| format!("Failed to create {}: {err}", new_root.display()))?;

        let entries = std::fs::read_dir(old_root)
            .map_err(|err| format!("Failed to list {}: {err}", old_root.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let src = entry.path();
            if !src.is_dir() {
                continue;
            }
            let Some(name) = src.file_name() else {
                continue;
            };
            let dst = new_root.join(name);
            if dst.exists() {
                continue;
            }
            move_path_without_replacing(&src, &dst)?;
        }

        let is_empty = std::fs::read_dir(old_root)
            .map(|mut entries| entries.next().is_none())
            .unwrap_or(false);
        if is_empty {
            let _ = std::fs::remove_dir(old_root);
        }
    }

    migrate_floor_package_layouts_in_root(new_root)
}

fn migrate_floor_package_layouts_in_root(root: &Path) -> Result<(), String> {
    if !root.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(root)
        .map_err(|err| format!("Failed to list {}: {err}", root.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let package_dir = entry.path();
        if !package_dir.is_dir() {
            continue;
        }
        migrate_floor_package_layout(&package_dir)?;
    }
    Ok(())
}

fn migrate_floor_package_layout(package_dir: &Path) -> Result<(), String> {
    let legacy = package_dir.join(LEGACY_PACKAGE_FLOOR_DEF_FILE_NAME);
    let current = package_dir.join(PACKAGE_FLOOR_DEF_FILE_NAME);
    if legacy.exists() && !current.exists() {
        move_path_without_replacing(&legacy, &current)?;
    } else if legacy.exists() && current.exists() {
        let _ = std::fs::remove_file(&legacy);
    }
    Ok(())
}

fn move_path_without_replacing(src: &Path, dst: &Path) -> Result<(), String> {
    if !src.exists() || dst.exists() {
        return Ok(());
    }
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    match std::fs::rename(src, dst) {
        Ok(_) => Ok(()),
        Err(rename_err) => {
            if src.is_dir() {
                copy_dir_recursive(src, dst)?;
                std::fs::remove_dir_all(src).map_err(|err| {
                    format!(
                        "Failed to remove migrated legacy dir {} after rename error {rename_err}: {err}",
                        src.display()
                    )
                })?;
                return Ok(());
            }

            std::fs::copy(src, dst).map_err(|err| {
                format!(
                    "Failed to migrate {} to {} after rename error {rename_err}: {err}",
                    src.display(),
                    dst.display()
                )
            })?;
            std::fs::remove_file(src).map_err(|err| {
                format!(
                    "Failed to remove migrated legacy file {} after rename error {rename_err}: {err}",
                    src.display()
                )
            })?;
            Ok(())
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst)
        .map_err(|err| format!("Failed to create {}: {err}", dst.display()))?;

    let entries =
        std::fs::read_dir(src).map_err(|err| format!("Failed to list {}: {err}", src.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            if let Some(parent) = dst_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
            }
            std::fs::copy(&src_path, &dst_path).map_err(|err| {
                format!(
                    "Failed to copy {} to {}: {err}",
                    src_path.display(),
                    dst_path.display()
                )
            })?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_floor_packages_ignores_non_uuid_folders() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_realm_terrain_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");
        std::fs::create_dir_all(temp_root.join("not-a-uuid")).expect("create junk folder");
        let floor_id = uuid::Uuid::new_v4().as_u128();
        std::fs::create_dir_all(temp_root.join(uuid::Uuid::from_u128(floor_id).to_string()))
            .expect("create uuid folder");

        let floors = list_floor_packages_in_dir(&temp_root).expect("list terrain packages");
        assert_eq!(floors, vec![floor_id]);

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn migrates_legacy_floor_root_and_def_name() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_realm_terrain_migration_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let old_root = temp_root.join("realm").join("default").join("floors");
        let new_root = temp_root.join("realm").join("default").join("terrain");
        let floor_id = uuid::Uuid::new_v4().as_u128();
        let package_dir = old_root.join(uuid::Uuid::from_u128(floor_id).to_string());
        std::fs::create_dir_all(&package_dir).expect("create legacy package dir");
        std::fs::write(
            package_dir.join(LEGACY_PACKAGE_FLOOR_DEF_FILE_NAME),
            serde_json::to_string(&FloorDefV1::default_world()).expect("encode floor def"),
        )
        .expect("write legacy floor def");

        migrate_floor_storage_roots(&old_root, &new_root).expect("migrate terrain roots");

        assert!(!old_root.exists(), "legacy floors root should be removed");
        assert!(
            new_root
                .join(uuid::Uuid::from_u128(floor_id).to_string())
                .join(PACKAGE_FLOOR_DEF_FILE_NAME)
                .exists(),
            "terrain_def_v1.json should exist after migration"
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
