use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::import_conflicts::ImportConflictPolicy;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) struct FloorZipImportReport {
    pub(crate) imported: usize,
    pub(crate) replaced: usize,
    pub(crate) renamed: usize,
    pub(crate) invalid: usize,
}

pub(crate) struct FloorZipConflictSummary {
    pub(crate) conflicting_floor_ids: Vec<String>,
}

impl FloorZipConflictSummary {
    pub(crate) fn has_conflicts(&self) -> bool {
        !self.conflicting_floor_ids.is_empty()
    }
}

const ZIP_ROOT_DIR: &str = "terrain";
const LEGACY_ZIP_ROOT_DIR: &str = "floors";
const FLOOR_DEF_FILE_NAME: &str = "terrain_def_v1.json";
const LEGACY_FLOOR_DEF_FILE_NAME: &str = "floor_def_v1.json";

fn zip_path_string(path: &Path) -> Result<String, String> {
    let Some(path_str) = path.to_str() else {
        return Err(format!("Invalid path: {}", path.display()));
    };
    Ok(path_str.replace('\\', "/"))
}

fn add_dir_to_zip(
    writer: &mut ZipWriter<File>,
    src_dir: &Path,
    zip_root: &Path,
) -> Result<(), String> {
    let options = FileOptions::default().compression_method(CompressionMethod::Deflated);

    let mut stack = vec![src_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rel = dir.strip_prefix(src_dir).unwrap_or(Path::new(""));
        let zip_dir = zip_root.join(rel);
        let mut zip_dir_name = zip_path_string(&zip_dir)?;
        if !zip_dir_name.is_empty() {
            if !zip_dir_name.ends_with('/') {
                zip_dir_name.push('/');
            }
            writer
                .add_directory(zip_dir_name, options)
                .map_err(|err| format!("Failed to add zip dir: {err}"))?;
        }

        let entries = std::fs::read_dir(&dir)
            .map_err(|err| format!("Failed to list {}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let rel = path.strip_prefix(src_dir).map_err(|err| {
                format!(
                    "Failed to compute relative path for {}: {err}",
                    path.display()
                )
            })?;
            let zip_path = zip_root.join(rel);
            let zip_name = zip_path_string(&zip_path)?;
            writer
                .start_file(zip_name, options)
                .map_err(|err| format!("Failed to add zip file: {err}"))?;
            let mut file = File::open(&path)
                .map_err(|err| format!("Failed to open {}: {err}", path.display()))?;
            io::copy(&mut file, writer)
                .map_err(|err| format!("Failed to write zip file: {err}"))?;
        }
    }

    Ok(())
}

pub(crate) fn export_floor_packages_to_zip(
    realm_id: &str,
    floor_ids: &[u128],
    zip_path: &Path,
) -> Result<usize, String> {
    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;

    let mut ids: Vec<u128> = floor_ids.iter().copied().collect();
    ids.sort();
    ids.dedup();

    if ids.contains(&crate::floor_library_ui::DEFAULT_FLOOR_ID) {
        return Err("Default Terrain cannot be exported.".to_string());
    }

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let file = File::create(zip_path)
        .map_err(|err| format!("Failed to create {}: {err}", zip_path.display()))?;
    let mut writer = ZipWriter::new(file);

    for floor_id in &ids {
        let package_dir = crate::realm_floor_packages::realm_floor_package_dir(realm_id, *floor_id);
        if !package_dir.exists() {
            return Err(format!(
                "Terrain package not found in this realm: {}",
                uuid::Uuid::from_u128(*floor_id)
            ));
        }
        let uuid = uuid::Uuid::from_u128(*floor_id).to_string();
        let zip_root = Path::new(ZIP_ROOT_DIR).join(uuid);
        add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
    }

    writer
        .finish()
        .map_err(|err| format!("Failed to finalize zip: {err}"))?;
    Ok(ids.len())
}

pub(crate) fn summarize_floor_zip_conflicts(
    realm_id: &str,
    zip_path: &Path,
) -> Result<FloorZipConflictSummary, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;
    let packages = scan_floor_packages(&mut archive)?;

    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;

    let mut conflicting_floor_ids = Vec::new();
    for (floor_id, pkg) in packages {
        if !pkg.has_floor_def {
            continue;
        }
        let dest_root = crate::realm_floor_packages::realm_floor_package_dir(realm_id, floor_id);
        if dest_root.exists() {
            conflicting_floor_ids.push(pkg.uuid_str);
        }
    }

    Ok(FloorZipConflictSummary {
        conflicting_floor_ids,
    })
}

pub(crate) fn import_floor_packages_from_zip_with_policy(
    realm_id: &str,
    zip_path: &Path,
    policy: ImportConflictPolicy,
) -> Result<FloorZipImportReport, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;

    let packages = scan_floor_packages(&mut archive)?;

    let mut imported = 0;
    let mut replaced = 0;
    let mut renamed = 0;
    let mut invalid = 0;

    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;
    let mut reserved_new_ids = std::collections::HashSet::new();

    for (floor_id, pkg) in packages {
        if !pkg.has_floor_def {
            invalid += 1;
            continue;
        }

        let existing_root = crate::realm_floor_packages::realm_floor_package_dir(realm_id, floor_id);
        let conflict = existing_root.exists();
        let (dest_floor_id, replaced_this, renamed_this) = if !conflict {
            (floor_id, false, false)
        } else {
            match policy {
                ImportConflictPolicy::Replace => {
                    std::fs::remove_dir_all(&existing_root).map_err(|err| {
                        format!(
                            "Failed to replace existing terrain package {}: {err}",
                            existing_root.display()
                        )
                    })?;
                    (floor_id, true, false)
                }
                ImportConflictPolicy::KeepBoth => {
                    let new_id =
                        generate_unique_floor_id(realm_id, &mut reserved_new_ids)?;
                    (new_id, false, true)
                }
            }
        };

        let dest_root =
            crate::realm_floor_packages::realm_floor_package_dir(realm_id, dest_floor_id);

        for idx in pkg.indices {
            let mut file = archive
                .by_index(idx)
                .map_err(|err| format!("Failed to read zip entry: {err}"))?;
            let Some(path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
                return Err("Zip contains invalid path (path traversal).".to_string());
            };
            let rel_from_root = path
                .strip_prefix(ZIP_ROOT_DIR)
                .or_else(|_| path.strip_prefix(LEGACY_ZIP_ROOT_DIR))
                .map_err(|_| format!("Zip entry has invalid layout: {}", file.name()))?;
            let rel = rel_from_root
                .strip_prefix(pkg.uuid_str.as_str())
                .map_err(|_| format!("Zip entry has invalid layout: {}", file.name()))?;

            let mut out_path = dest_root.join(rel);
            if out_path.file_name().and_then(|v| v.to_str()) == Some(LEGACY_FLOOR_DEF_FILE_NAME) {
                out_path = out_path.with_file_name(FLOOR_DEF_FILE_NAME);
            }
            if file.is_dir() {
                std::fs::create_dir_all(&out_path)
                    .map_err(|err| format!("Failed to create {}: {err}", out_path.display()))?;
                continue;
            }

            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
            }
            let mut out = File::create(&out_path)
                .map_err(|err| format!("Failed to write {}: {err}", out_path.display()))?;
            io::copy(&mut file, &mut out)
                .map_err(|err| format!("Failed to extract {}: {err}", out_path.display()))?;
        }

        imported += 1;
        if replaced_this {
            replaced += 1;
        }
        if renamed_this {
            renamed += 1;
        }
    }

    Ok(FloorZipImportReport {
        imported,
        replaced,
        renamed,
        invalid,
    })
}

struct FloorPackageEntries {
    indices: Vec<usize>,
    has_floor_def: bool,
    uuid_str: String,
}

fn scan_floor_packages(
    archive: &mut ZipArchive<File>,
) -> Result<BTreeMap<u128, FloorPackageEntries>, String> {
    let mut packages: BTreeMap<u128, FloorPackageEntries> = BTreeMap::new();

    for idx in 0..archive.len() {
        let file = archive
            .by_index(idx)
            .map_err(|err| format!("Failed to read zip entry: {err}"))?;
        let Some(path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err("Zip contains invalid path (path traversal).".to_string());
        };

        let mut components = path.components();
        let Some(Component::Normal(root)) = components.next() else {
            return Err("Zip contains invalid entry path.".to_string());
        };
        if root != ZIP_ROOT_DIR && root != LEGACY_ZIP_ROOT_DIR {
            return Err(format!(
                "Zip entry outside {ZIP_ROOT_DIR}/ or {LEGACY_ZIP_ROOT_DIR}/: {}",
                file.name()
            ));
        }

        let Some(Component::Normal(uuid_component)) = components.next() else {
            return Err(format!("Zip entry missing terrain UUID: {}", file.name()));
        };
        let uuid_str = uuid_component
            .to_str()
            .ok_or_else(|| format!("Invalid terrain UUID path: {}", file.name()))?;
        let uuid = uuid::Uuid::parse_str(uuid_str)
            .map_err(|_| format!("Invalid terrain UUID in zip: {uuid_str}"))?;

        if uuid.as_u128() == crate::floor_library_ui::DEFAULT_FLOOR_ID {
            return Err("Zip contains Default Terrain UUID, which is not supported.".to_string());
        }

        let rel: PathBuf = components.collect();
        let entry = packages
            .entry(uuid.as_u128())
            .or_insert_with(|| FloorPackageEntries {
                indices: Vec::new(),
                has_floor_def: false,
                uuid_str: uuid_str.to_string(),
            });
        entry.indices.push(idx);

        if !file.is_dir() {
            if let Some(name) = rel.file_name().and_then(|v| v.to_str()) {
                if name == FLOOR_DEF_FILE_NAME || name == LEGACY_FLOOR_DEF_FILE_NAME {
                    entry.has_floor_def = true;
                }
            }
        }
    }

    if packages.is_empty() {
        return Err("Zip contains no terrain packages.".to_string());
    }

    Ok(packages)
}

fn generate_unique_floor_id(
    realm_id: &str,
    reserved_new_ids: &mut std::collections::HashSet<u128>,
) -> Result<u128, String> {
    for _ in 0..1024 {
        let floor_id = uuid::Uuid::new_v4().as_u128();
        if floor_id == crate::floor_library_ui::DEFAULT_FLOOR_ID {
            continue;
        }
        if !reserved_new_ids.insert(floor_id) {
            continue;
        }
        let dest_root = crate::realm_floor_packages::realm_floor_package_dir(realm_id, floor_id);
        if !dest_root.exists() {
            return Ok(floor_id);
        }
    }

    Err("Failed to generate a unique terrain id for keep-both import.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_floor_package(root: &Path, floor_id: u128, label: &str) {
        let uuid = uuid::Uuid::from_u128(floor_id).to_string();
        let package_dir = root.join(uuid);
        std::fs::create_dir_all(&package_dir).expect("create floor package dir");
        std::fs::write(
            package_dir.join(FLOOR_DEF_FILE_NAME),
            format!(
                "{{\"format_version\":1,\"label\":\"{}\",\"mesh\":{{\"kind\":\"grid\",\"size_m\":[10.0,10.0],\"subdiv\":[1,1],\"thickness_m\":0.1,\"uv_tiling\":[1.0,1.0]}},\"material\":{{\"base_color_rgba\":[0.1,0.1,0.1,1.0],\"metallic\":0.0,\"roughness\":1.0,\"unlit\":false}},\"coloring\":{{\"mode\":\"solid\",\"palette\":[],\"scale\":[1.0,1.0],\"angle_deg\":0.0,\"noise\":{{\"seed\":1,\"frequency\":0.1,\"octaves\":1,\"lacunarity\":2.0,\"gain\":0.5}}}},\"relief\":{{\"mode\":\"none\",\"amplitude\":0.0,\"noise\":{{\"seed\":1,\"frequency\":0.1,\"octaves\":1,\"lacunarity\":2.0,\"gain\":0.5}}}},\"animation\":{{\"mode\":\"none\",\"waves\":[],\"normal_strength\":1.0}}}}",
                label
            ),
        )
        .expect("write floor def");
    }

    #[test]
    fn replace_overwrites_conflicting_floor_package() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_floor_zip_replace_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_root = temp_root.join("src");
        let dst_realm = "floor_replace";
        let zip_path = temp_root.join("terrain.zip");
        let floor_id = uuid::Uuid::new_v4().as_u128();

        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
        write_floor_package(&src_root, floor_id, "New Floor");
        write_floor_package(
            &crate::paths::realm_floors_dir(dst_realm),
            floor_id,
            "Old Floor",
        );
        export_floor_packages_to_zip_from_root(&src_root, &[floor_id], &zip_path)
            .expect("export floor zip");

        let report =
            import_floor_packages_from_zip_with_policy(dst_realm, &zip_path, ImportConflictPolicy::Replace)
                .expect("replace import");
        assert_eq!(report.imported, 1);
        assert_eq!(report.replaced, 1);
        assert_eq!(report.renamed, 0);

        let def = std::fs::read_to_string(
            crate::realm_floor_packages::realm_floor_package_dir(dst_realm, floor_id)
                .join(FLOOR_DEF_FILE_NAME),
        )
        .expect("read replaced floor");
        assert!(def.contains("New Floor"));

        let _ = std::fs::remove_dir_all(&temp_root);
        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
    }

    #[test]
    fn keep_both_renames_conflicting_floor_package() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_floor_zip_keep_both_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_root = temp_root.join("src");
        let dst_realm = "floor_keep_both";
        let zip_path = temp_root.join("terrain.zip");
        let floor_id = uuid::Uuid::new_v4().as_u128();

        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
        write_floor_package(&src_root, floor_id, "Copy Floor");
        write_floor_package(
            &crate::paths::realm_floors_dir(dst_realm),
            floor_id,
            "Existing Floor",
        );
        export_floor_packages_to_zip_from_root(&src_root, &[floor_id], &zip_path)
            .expect("export floor zip");

        let report = import_floor_packages_from_zip_with_policy(
            dst_realm,
            &zip_path,
            ImportConflictPolicy::KeepBoth,
        )
        .expect("keep-both import");
        assert_eq!(report.imported, 1);
        assert_eq!(report.replaced, 0);
        assert_eq!(report.renamed, 1);

        let imported_ids: Vec<u128> = std::fs::read_dir(crate::paths::realm_floors_dir(dst_realm))
            .expect("list imported floor ids")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| uuid::Uuid::parse_str(&name).ok().map(|uuid| uuid.as_u128()))
            .collect();
        let new_floor_id = imported_ids
            .iter()
            .copied()
            .find(|id| *id != floor_id)
            .expect("new floor id");
        let def = std::fs::read_to_string(
            crate::realm_floor_packages::realm_floor_package_dir(dst_realm, new_floor_id)
                .join(FLOOR_DEF_FILE_NAME),
        )
        .expect("read copied floor");
        assert!(def.contains("Copy Floor"));

        let _ = std::fs::remove_dir_all(&temp_root);
        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
    }

    fn export_floor_packages_to_zip_from_root(
        floors_root: &Path,
        floor_ids: &[u128],
        zip_path: &Path,
    ) -> Result<usize, String> {
        let mut ids: Vec<u128> = floor_ids.iter().copied().collect();
        ids.sort();
        ids.dedup();

        if let Some(parent) = zip_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
        }

        let file = File::create(zip_path)
            .map_err(|err| format!("Failed to create {}: {err}", zip_path.display()))?;
        let mut writer = ZipWriter::new(file);
        for floor_id in &ids {
            let package_dir = floors_root.join(uuid::Uuid::from_u128(*floor_id).to_string());
            let zip_root = Path::new(ZIP_ROOT_DIR).join(uuid::Uuid::from_u128(*floor_id).to_string());
            add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
        }
        writer
            .finish()
            .map_err(|err| format!("Failed to finalize zip: {err}"))?;
        Ok(ids.len())
    }
}
