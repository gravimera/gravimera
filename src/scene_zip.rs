use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) struct SceneZipExportReport {
    pub(crate) exported_scenes: usize,
    pub(crate) exported_prefabs: usize,
}

pub(crate) struct SceneZipImportReport {
    pub(crate) imported_scenes: usize,
    pub(crate) skipped_scenes: usize,
    pub(crate) invalid_scenes: usize,
    pub(crate) imported_prefabs: usize,
    pub(crate) skipped_prefabs: usize,
    pub(crate) invalid_prefabs: usize,
}

const SCENES_ZIP_ROOT_DIR: &str = "scenes";
const PREFABS_ZIP_ROOT_DIR: &str = "prefabs";

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

pub(crate) fn export_scene_packages_to_zip(
    realm_id: &str,
    scene_ids: &[String],
    zip_path: &Path,
) -> Result<SceneZipExportReport, String> {
    let scenes_root = crate::paths::realm_dir(realm_id).join("scenes");
    let prefabs_root = crate::paths::realm_prefabs_dir(realm_id);
    export_scene_packages_to_zip_from_roots(&scenes_root, &prefabs_root, scene_ids, zip_path)
}

fn export_scene_packages_to_zip_from_roots(
    scenes_root: &Path,
    prefabs_root: &Path,
    scene_ids: &[String],
    zip_path: &Path,
) -> Result<SceneZipExportReport, String> {
    let mut ids: Vec<String> = scene_ids
        .iter()
        .filter_map(|scene_id| crate::realm::sanitize_id(scene_id))
        .collect();
    ids.sort();
    ids.dedup();

    if ids.is_empty() {
        return Err("No scene ids provided.".to_string());
    }

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let file = File::create(zip_path)
        .map_err(|err| format!("Failed to create {}: {err}", zip_path.display()))?;
    let mut writer = ZipWriter::new(file);
    let mut prefab_ids = BTreeSet::new();

    for scene_id in &ids {
        let scene_dir = scenes_root.join(scene_id);
        if !scene_dir.is_dir() {
            return Err(format!("Scene folder not found: {}", scene_dir.display()));
        }

        let zip_root = Path::new(SCENES_ZIP_ROOT_DIR).join(scene_id);
        add_dir_to_zip(&mut writer, &scene_dir, &zip_root)?;
        prefab_ids.extend(collect_referenced_prefab_ids_for_scene_dir(&scene_dir)?);
    }

    let mut exported_prefabs = 0usize;
    for prefab_id in prefab_ids {
        let uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let package_dir = prefabs_root.join(&uuid);
        if !package_dir.is_dir() {
            continue;
        }
        let zip_root = Path::new(PREFABS_ZIP_ROOT_DIR).join(&uuid);
        add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
        exported_prefabs += 1;
    }

    writer
        .finish()
        .map_err(|err| format!("Failed to finalize zip: {err}"))?;

    Ok(SceneZipExportReport {
        exported_scenes: ids.len(),
        exported_prefabs,
    })
}

fn collect_referenced_prefab_ids_for_scene_dir(scene_dir: &Path) -> Result<BTreeSet<u128>, String> {
    let mut ids = BTreeSet::new();
    let build_dir = scene_dir.join("build");
    for file_name in ["scene.grav", "scene.build.grav"] {
        let path = build_dir.join(file_name);
        for prefab_id in crate::scene_store::referenced_prefab_ids_in_scene_dat_path(&path)? {
            ids.insert(prefab_id);
        }
    }
    Ok(ids)
}

pub(crate) fn import_scene_packages_from_zip(
    realm_id: &str,
    zip_path: &Path,
) -> Result<SceneZipImportReport, String> {
    let scenes_root = crate::paths::realm_dir(realm_id).join("scenes");
    let prefabs_root = crate::paths::realm_prefabs_dir(realm_id);
    import_scene_packages_from_zip_to_roots(&scenes_root, &prefabs_root, zip_path)
}

fn import_scene_packages_from_zip_to_roots(
    scenes_root: &Path,
    prefabs_root: &Path,
    zip_path: &Path,
) -> Result<SceneZipImportReport, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;

    struct SceneEntries {
        indices: Vec<usize>,
        has_scene_content: bool,
        scene_id: String,
    }

    struct PrefabEntries {
        indices: Vec<usize>,
        has_prefab_json: bool,
        uuid_str: String,
    }

    let mut scenes: BTreeMap<String, SceneEntries> = BTreeMap::new();
    let mut prefabs: BTreeMap<u128, PrefabEntries> = BTreeMap::new();

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

        if root == SCENES_ZIP_ROOT_DIR {
            let Some(Component::Normal(scene_component)) = components.next() else {
                return Err(format!("Zip entry missing scene id: {}", file.name()));
            };
            let scene_raw = scene_component
                .to_str()
                .ok_or_else(|| format!("Invalid scene id path: {}", file.name()))?;
            let scene_id = crate::realm::sanitize_id(scene_raw)
                .ok_or_else(|| format!("Invalid scene id in zip: {scene_raw}"))?;
            let rel: PathBuf = components.collect();
            let entry = scenes
                .entry(scene_id.clone())
                .or_insert_with(|| SceneEntries {
                    indices: Vec::new(),
                    has_scene_content: false,
                    scene_id,
                });
            entry.indices.push(idx);

            if !file.is_dir() {
                let rel_str = rel.to_string_lossy().replace('\\', "/");
                if matches!(
                    rel_str.as_str(),
                    "build/scene.grav" | "build/scene.build.grav" | "src/index.json"
                ) {
                    entry.has_scene_content = true;
                }
            }
            continue;
        }

        if root == PREFABS_ZIP_ROOT_DIR {
            let Some(Component::Normal(uuid_component)) = components.next() else {
                return Err(format!("Zip entry missing prefab UUID: {}", file.name()));
            };
            let uuid_str = uuid_component
                .to_str()
                .ok_or_else(|| format!("Invalid prefab UUID path: {}", file.name()))?;
            let uuid = uuid::Uuid::parse_str(uuid_str)
                .map_err(|_| format!("Invalid prefab UUID in zip: {uuid_str}"))?;
            let rel: PathBuf = components.collect();
            let entry = prefabs
                .entry(uuid.as_u128())
                .or_insert_with(|| PrefabEntries {
                    indices: Vec::new(),
                    has_prefab_json: false,
                    uuid_str: uuid_str.to_string(),
                });
            entry.indices.push(idx);

            if !file.is_dir() {
                if let Some(Component::Normal(folder)) = rel.components().next() {
                    if folder == "prefabs" {
                        if let Some(name) = rel.file_name().and_then(|value| value.to_str()) {
                            if name.ends_with(".json") && !name.ends_with(".desc.json") {
                                entry.has_prefab_json = true;
                            }
                        }
                    }
                }
            }
            continue;
        }

        return Err(format!(
            "Zip entry outside {SCENES_ZIP_ROOT_DIR}/ or {PREFABS_ZIP_ROOT_DIR}/: {}",
            file.name()
        ));
    }

    if scenes.is_empty() && prefabs.is_empty() {
        return Err("Zip contains no scene packages.".to_string());
    }

    std::fs::create_dir_all(scenes_root)
        .map_err(|err| format!("Failed to create {}: {err}", scenes_root.display()))?;
    std::fs::create_dir_all(prefabs_root)
        .map_err(|err| format!("Failed to create {}: {err}", prefabs_root.display()))?;

    let mut report = SceneZipImportReport {
        imported_scenes: 0,
        skipped_scenes: 0,
        invalid_scenes: 0,
        imported_prefabs: 0,
        skipped_prefabs: 0,
        invalid_prefabs: 0,
    };

    for (_scene_id, scene) in scenes {
        if !scene.has_scene_content {
            report.invalid_scenes += 1;
            continue;
        }

        let dest_root = scenes_root.join(&scene.scene_id);
        if dest_root.exists() {
            report.skipped_scenes += 1;
            continue;
        }

        extract_entries_into_root(
            &mut archive,
            &scene.indices,
            SCENES_ZIP_ROOT_DIR,
            &scene.scene_id,
            &dest_root,
        )?;
        report.imported_scenes += 1;
    }

    for (prefab_id, prefab) in prefabs {
        if !prefab.has_prefab_json {
            report.invalid_prefabs += 1;
            continue;
        }

        let dest_root = prefabs_root.join(uuid::Uuid::from_u128(prefab_id).to_string());
        if dest_root.exists() {
            report.skipped_prefabs += 1;
            continue;
        }

        extract_entries_into_root(
            &mut archive,
            &prefab.indices,
            PREFABS_ZIP_ROOT_DIR,
            &prefab.uuid_str,
            &dest_root,
        )?;
        report.imported_prefabs += 1;
    }

    Ok(report)
}

fn extract_entries_into_root(
    archive: &mut ZipArchive<File>,
    indices: &[usize],
    root_dir_name: &str,
    package_name: &str,
    dest_root: &Path,
) -> Result<(), String> {
    for idx in indices {
        let mut file = archive
            .by_index(*idx)
            .map_err(|err| format!("Failed to read zip entry: {err}"))?;
        let Some(path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
            return Err("Zip contains invalid path (path traversal).".to_string());
        };
        let rel = path
            .strip_prefix(root_dir_name)
            .and_then(|path| path.strip_prefix(package_name))
            .map_err(|_| format!("Zip entry has invalid layout: {}", file.name()))?;

        let out_path = dest_root.join(rel);
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

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_and_import_scene_zip_roundtrips_scene_and_prefab_package() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_zip_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_scenes_root = temp_root.join("src_scenes");
        let src_prefabs_root = temp_root.join("src_prefabs");
        let dst_scenes_root = temp_root.join("dst_scenes");
        let dst_prefabs_root = temp_root.join("dst_prefabs");
        let zip_path = temp_root.join("scenes.zip");

        let scene_id = "alpha".to_string();
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();

        let scene_dir = src_scenes_root.join(&scene_id);
        std::fs::create_dir_all(scene_dir.join("build")).expect("create build dir");
        std::fs::create_dir_all(scene_dir.join("src")).expect("create src dir");
        std::fs::write(
            scene_dir.join("build").join("scene.grav"),
            crate::scene_store::test_encode_scene_dat_with_prefab_ids(&[prefab_id]),
        )
        .expect("write scene.grav");
        std::fs::write(scene_dir.join("src").join("index.json"), "{}\n").expect("write index");

        let prefab_dir = src_prefabs_root.join(&prefab_uuid);
        std::fs::create_dir_all(prefab_dir.join("prefabs")).expect("create prefab dir");
        std::fs::write(
            prefab_dir
                .join("prefabs")
                .join(format!("{prefab_uuid}.json")),
            "{}\n",
        )
        .expect("write prefab json");

        let export_report = export_scene_packages_to_zip_from_roots(
            &src_scenes_root,
            &src_prefabs_root,
            std::slice::from_ref(&scene_id),
            &zip_path,
        )
        .expect("export scene zip");
        assert_eq!(export_report.exported_scenes, 1);
        assert_eq!(export_report.exported_prefabs, 1);

        let import_report =
            import_scene_packages_from_zip_to_roots(&dst_scenes_root, &dst_prefabs_root, &zip_path)
                .expect("import scene zip");
        assert_eq!(import_report.imported_scenes, 1);
        assert_eq!(import_report.imported_prefabs, 1);
        assert!(dst_scenes_root
            .join(&scene_id)
            .join("build")
            .join("scene.grav")
            .exists());
        assert!(dst_prefabs_root
            .join(&prefab_uuid)
            .join("prefabs")
            .join(format!("{prefab_uuid}.json"))
            .exists());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn import_skips_existing_scene_and_prefab() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_zip_skip_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_scenes_root = temp_root.join("src_scenes");
        let src_prefabs_root = temp_root.join("src_prefabs");
        let dst_scenes_root = temp_root.join("dst_scenes");
        let dst_prefabs_root = temp_root.join("dst_prefabs");
        let zip_path = temp_root.join("scenes.zip");

        let scene_id = "alpha".to_string();
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();

        let scene_dir = src_scenes_root.join(&scene_id);
        std::fs::create_dir_all(scene_dir.join("build")).expect("create build dir");
        std::fs::create_dir_all(scene_dir.join("src")).expect("create src dir");
        std::fs::write(
            scene_dir.join("build").join("scene.grav"),
            crate::scene_store::test_encode_scene_dat_with_prefab_ids(&[prefab_id]),
        )
        .expect("write scene.grav");
        std::fs::write(scene_dir.join("src").join("index.json"), "{}\n").expect("write index");

        let prefab_dir = src_prefabs_root.join(&prefab_uuid);
        std::fs::create_dir_all(prefab_dir.join("prefabs")).expect("create prefab dir");
        std::fs::write(
            prefab_dir
                .join("prefabs")
                .join(format!("{prefab_uuid}.json")),
            "{}\n",
        )
        .expect("write prefab json");

        export_scene_packages_to_zip_from_roots(
            &src_scenes_root,
            &src_prefabs_root,
            std::slice::from_ref(&scene_id),
            &zip_path,
        )
        .expect("export scene zip");

        std::fs::create_dir_all(dst_scenes_root.join(&scene_id)).expect("precreate scene");
        std::fs::create_dir_all(dst_prefabs_root.join(&prefab_uuid)).expect("precreate prefab");

        let report =
            import_scene_packages_from_zip_to_roots(&dst_scenes_root, &dst_prefabs_root, &zip_path)
                .expect("import scene zip");
        assert_eq!(report.skipped_scenes, 1);
        assert_eq!(report.skipped_prefabs, 1);

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
