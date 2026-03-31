use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::import_conflicts::ImportConflictPolicy;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) struct SceneZipExportReport {
    pub(crate) exported_scenes: usize,
    pub(crate) exported_prefabs: usize,
    pub(crate) exported_terrains: usize,
}

pub(crate) struct SceneZipImportReport {
    pub(crate) imported_scenes: usize,
    pub(crate) replaced_scenes: usize,
    pub(crate) renamed_scenes: usize,
    pub(crate) invalid_scenes: usize,
    pub(crate) imported_prefabs: usize,
    pub(crate) replaced_prefabs: usize,
    pub(crate) renamed_prefabs: usize,
    pub(crate) invalid_prefabs: usize,
    pub(crate) imported_terrains: usize,
    pub(crate) replaced_terrains: usize,
    pub(crate) renamed_terrains: usize,
    pub(crate) invalid_terrains: usize,
}

pub(crate) struct SceneZipConflictSummary {
    pub(crate) conflicting_scene_ids: Vec<String>,
    pub(crate) conflicting_prefab_ids: Vec<String>,
    pub(crate) conflicting_terrain_ids: Vec<String>,
}

impl SceneZipConflictSummary {
    pub(crate) fn has_conflicts(&self) -> bool {
        !self.conflicting_scene_ids.is_empty()
            || !self.conflicting_prefab_ids.is_empty()
            || !self.conflicting_terrain_ids.is_empty()
    }
}

const SCENES_ZIP_ROOT_DIR: &str = "scenes";
const PREFABS_ZIP_ROOT_DIR: &str = "prefabs";
const TERRAIN_ZIP_ROOT_DIR: &str = "terrain";
const LEGACY_TERRAIN_ZIP_ROOT_DIR: &str = "floors";
const TERRAIN_DEF_FILE_NAME: &str = "terrain_def_v1.json";
const LEGACY_TERRAIN_DEF_FILE_NAME: &str = "floor_def_v1.json";

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
    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;
    let floors_root = crate::paths::realm_floors_dir(realm_id);
    export_scene_packages_to_zip_from_roots(
        &scenes_root,
        &prefabs_root,
        &floors_root,
        scene_ids,
        zip_path,
    )
}

fn export_scene_packages_to_zip_from_roots(
    scenes_root: &Path,
    prefabs_root: &Path,
    floors_root: &Path,
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
    let mut terrain_ids = BTreeSet::new();

    for scene_id in &ids {
        let scene_dir = scenes_root.join(scene_id);
        if !scene_dir.is_dir() {
            return Err(format!("Scene folder not found: {}", scene_dir.display()));
        }

        let zip_root = Path::new(SCENES_ZIP_ROOT_DIR).join(scene_id);
        add_dir_to_zip(&mut writer, &scene_dir, &zip_root)?;
        prefab_ids.extend(collect_referenced_prefab_ids_for_scene_dir(&scene_dir)?);
        terrain_ids.extend(collect_referenced_terrain_ids_for_scene_dir(&scene_dir)?);
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

    let mut exported_terrains = 0usize;
    for terrain_id in terrain_ids {
        if terrain_id == crate::floor_library_ui::DEFAULT_FLOOR_ID {
            continue;
        }
        let uuid = uuid::Uuid::from_u128(terrain_id).to_string();
        let package_dir = floors_root.join(&uuid);
        if !package_dir.is_dir() {
            continue;
        }
        let zip_root = Path::new(TERRAIN_ZIP_ROOT_DIR).join(&uuid);
        add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
        exported_terrains += 1;
    }

    writer
        .finish()
        .map_err(|err| format!("Failed to finalize zip: {err}"))?;

    Ok(SceneZipExportReport {
        exported_scenes: ids.len(),
        exported_prefabs,
        exported_terrains,
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

fn collect_referenced_terrain_ids_for_scene_dir(
    scene_dir: &Path,
) -> Result<BTreeSet<u128>, String> {
    let mut ids = BTreeSet::new();
    let build_dir = scene_dir.join("build");
    if let Some(terrain_id) =
        crate::scene_floor_selection::read_scene_floor_selection_from_build_dir(&build_dir)?
    {
        ids.insert(terrain_id);
    }
    Ok(ids)
}

pub(crate) fn summarize_scene_zip_conflicts(
    realm_id: &str,
    zip_path: &Path,
) -> Result<SceneZipConflictSummary, String> {
    let scenes_root = crate::paths::realm_dir(realm_id).join("scenes");
    let prefabs_root = crate::paths::realm_prefabs_dir(realm_id);
    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;
    let floors_root = crate::paths::realm_floors_dir(realm_id);
    summarize_scene_zip_conflicts_in_roots(&scenes_root, &prefabs_root, &floors_root, zip_path)
}

pub(crate) fn import_scene_packages_from_zip_with_policy(
    realm_id: &str,
    zip_path: &Path,
    policy: ImportConflictPolicy,
) -> Result<SceneZipImportReport, String> {
    let scenes_root = crate::paths::realm_dir(realm_id).join("scenes");
    let prefabs_root = crate::paths::realm_prefabs_dir(realm_id);
    crate::realm_floor_packages::migrate_legacy_floor_storage_for_realm(realm_id)?;
    let floors_root = crate::paths::realm_floors_dir(realm_id);
    import_scene_packages_from_zip_to_roots_with_policy(
        &scenes_root,
        &prefabs_root,
        &floors_root,
        zip_path,
        policy,
    )
}

fn summarize_scene_zip_conflicts_in_roots(
    scenes_root: &Path,
    prefabs_root: &Path,
    floors_root: &Path,
    zip_path: &Path,
) -> Result<SceneZipConflictSummary, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;
    let (scenes, prefabs, terrains) = scan_scene_zip_packages(&mut archive)?;

    let mut conflicting_scene_ids = Vec::new();
    for scene in scenes.values() {
        if !scene.has_scene_content {
            continue;
        }
        let dest_root = scenes_root.join(&scene.scene_id);
        if dest_root.exists() {
            conflicting_scene_ids.push(scene.scene_id.clone());
        }
    }

    let mut conflicting_prefab_ids = Vec::new();
    for (prefab_id, prefab) in prefabs {
        if !prefab.has_prefab_json {
            continue;
        }
        let dest_root = prefabs_root.join(uuid::Uuid::from_u128(prefab_id).to_string());
        if dest_root.exists() {
            conflicting_prefab_ids.push(prefab.uuid_str);
        }
    }

    let mut conflicting_terrain_ids = Vec::new();
    for (_terrain_id, terrain) in terrains {
        if !terrain.has_terrain_def {
            continue;
        }
        let dest_root = floors_root.join(&terrain.uuid_str);
        if dest_root.exists() {
            conflicting_terrain_ids.push(terrain.uuid_str);
        }
    }

    Ok(SceneZipConflictSummary {
        conflicting_scene_ids,
        conflicting_prefab_ids,
        conflicting_terrain_ids,
    })
}

fn import_scene_packages_from_zip_to_roots_with_policy(
    scenes_root: &Path,
    prefabs_root: &Path,
    floors_root: &Path,
    zip_path: &Path,
    policy: ImportConflictPolicy,
) -> Result<SceneZipImportReport, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;
    let (scenes, prefabs, terrains) = scan_scene_zip_packages(&mut archive)?;

    std::fs::create_dir_all(scenes_root)
        .map_err(|err| format!("Failed to create {}: {err}", scenes_root.display()))?;
    std::fs::create_dir_all(prefabs_root)
        .map_err(|err| format!("Failed to create {}: {err}", prefabs_root.display()))?;
    std::fs::create_dir_all(floors_root)
        .map_err(|err| format!("Failed to create {}: {err}", floors_root.display()))?;

    let scene_id_map = resolve_scene_import_ids(scenes_root, &scenes, policy)?;
    let prefab_id_map = resolve_prefab_import_ids(prefabs_root, &prefabs, policy)?;
    let terrain_id_map = resolve_terrain_import_ids(floors_root, &terrains, policy)?;
    let any_scene_renamed = scene_id_map.iter().any(|(old_id, new_id)| old_id != new_id);
    let any_prefab_renamed = prefab_id_map
        .iter()
        .any(|(old_id, new_id)| old_id != new_id);
    let any_terrain_renamed = terrain_id_map
        .iter()
        .any(|(old_id, new_id)| old_id != new_id);

    let mut report = SceneZipImportReport {
        imported_scenes: 0,
        replaced_scenes: 0,
        renamed_scenes: 0,
        invalid_scenes: 0,
        imported_prefabs: 0,
        replaced_prefabs: 0,
        renamed_prefabs: 0,
        invalid_prefabs: 0,
        imported_terrains: 0,
        replaced_terrains: 0,
        renamed_terrains: 0,
        invalid_terrains: 0,
    };

    for (terrain_id, terrain) in terrains {
        if !terrain.has_terrain_def {
            report.invalid_terrains += 1;
            continue;
        }

        let resolved_terrain_id = terrain_id_map
            .get(&terrain_id)
            .copied()
            .unwrap_or(terrain_id);
        let existing_root = floors_root.join(uuid::Uuid::from_u128(terrain_id).to_string());
        let conflict = existing_root.exists();

        match (resolved_terrain_id == terrain_id, conflict, policy) {
            (true, false, _) => {
                extract_terrain_entries_into_root(
                    &mut archive,
                    &terrain.indices,
                    &terrain.uuid_str,
                    &existing_root,
                )?;
                report.imported_terrains += 1;
            }
            (true, true, ImportConflictPolicy::Replace) => {
                std::fs::remove_dir_all(&existing_root).map_err(|err| {
                    format!(
                        "Failed to replace existing terrain package {}: {err}",
                        existing_root.display()
                    )
                })?;
                extract_terrain_entries_into_root(
                    &mut archive,
                    &terrain.indices,
                    &terrain.uuid_str,
                    &existing_root,
                )?;
                report.imported_terrains += 1;
                report.replaced_terrains += 1;
            }
            (false, true, ImportConflictPolicy::KeepBoth) => {
                let dest_root =
                    floors_root.join(uuid::Uuid::from_u128(resolved_terrain_id).to_string());
                extract_terrain_entries_into_root(
                    &mut archive,
                    &terrain.indices,
                    &terrain.uuid_str,
                    &dest_root,
                )?;
                report.imported_terrains += 1;
                report.renamed_terrains += 1;
            }
            _ => {
                return Err(format!(
                    "Unsupported scene terrain import state for {}.",
                    terrain.uuid_str
                ));
            }
        }
    }

    for (prefab_id, prefab) in prefabs {
        if !prefab.has_prefab_json {
            report.invalid_prefabs += 1;
            continue;
        }

        let resolved_prefab_id = prefab_id_map.get(&prefab_id).copied().unwrap_or(prefab_id);
        let existing_root = prefabs_root.join(uuid::Uuid::from_u128(prefab_id).to_string());
        let conflict = existing_root.exists();

        match (resolved_prefab_id == prefab_id, conflict, policy) {
            (true, false, _) => {
                extract_entries_into_root(
                    &mut archive,
                    &prefab.indices,
                    PREFABS_ZIP_ROOT_DIR,
                    &prefab.uuid_str,
                    &existing_root,
                )?;
                report.imported_prefabs += 1;
            }
            (true, true, ImportConflictPolicy::Replace) => {
                std::fs::remove_dir_all(&existing_root).map_err(|err| {
                    format!(
                        "Failed to replace existing prefab package {}: {err}",
                        existing_root.display()
                    )
                })?;
                extract_entries_into_root(
                    &mut archive,
                    &prefab.indices,
                    PREFABS_ZIP_ROOT_DIR,
                    &prefab.uuid_str,
                    &existing_root,
                )?;
                report.imported_prefabs += 1;
                report.replaced_prefabs += 1;
            }
            (false, true, ImportConflictPolicy::KeepBoth) => {
                let stage_root = make_temp_import_dir("gravimera_scene_zip_prefab")?;
                let stage_package_dir = stage_root.join(&prefab.uuid_str);
                let result = (|| {
                    extract_entries_into_root(
                        &mut archive,
                        &prefab.indices,
                        PREFABS_ZIP_ROOT_DIR,
                        &prefab.uuid_str,
                        &stage_package_dir,
                    )?;
                    let new_dest_root =
                        prefabs_root.join(uuid::Uuid::from_u128(resolved_prefab_id).to_string());
                    crate::prefab_zip::remap_staged_prefab_package_into_dest(
                        &stage_package_dir,
                        prefab_id,
                        resolved_prefab_id,
                        &new_dest_root,
                    )?;
                    Ok::<(), String>(())
                })();
                let cleanup_err = remove_dir_if_exists(&stage_root);
                result?;
                cleanup_err?;
                report.imported_prefabs += 1;
                report.renamed_prefabs += 1;
            }
            _ => {
                return Err(format!(
                    "Unsupported scene prefab import state for {}.",
                    prefab.uuid_str
                ));
            }
        }
    }

    for (_scene_id, scene) in scenes {
        if !scene.has_scene_content {
            report.invalid_scenes += 1;
            continue;
        }

        let resolved_scene_id = scene_id_map
            .get(&scene.scene_id)
            .cloned()
            .unwrap_or_else(|| scene.scene_id.clone());
        let existing_root = scenes_root.join(&scene.scene_id);
        let dest_root = scenes_root.join(&resolved_scene_id);
        let conflict = existing_root.exists();

        let needs_rewrite = any_scene_renamed || any_prefab_renamed || any_terrain_renamed;
        match (
            needs_rewrite,
            resolved_scene_id == scene.scene_id,
            conflict,
            policy,
        ) {
            (false, true, false, _) => {
                extract_entries_into_root(
                    &mut archive,
                    &scene.indices,
                    SCENES_ZIP_ROOT_DIR,
                    &scene.scene_id,
                    &dest_root,
                )?;
                report.imported_scenes += 1;
            }
            (false, true, true, ImportConflictPolicy::Replace) => {
                std::fs::remove_dir_all(&existing_root).map_err(|err| {
                    format!(
                        "Failed to replace existing scene {}: {err}",
                        existing_root.display()
                    )
                })?;
                extract_entries_into_root(
                    &mut archive,
                    &scene.indices,
                    SCENES_ZIP_ROOT_DIR,
                    &scene.scene_id,
                    &dest_root,
                )?;
                report.imported_scenes += 1;
                report.replaced_scenes += 1;
            }
            (_, _, _, _) => {
                if conflict && matches!(policy, ImportConflictPolicy::Replace) {
                    std::fs::remove_dir_all(&existing_root).map_err(|err| {
                        format!(
                            "Failed to replace existing scene {}: {err}",
                            existing_root.display()
                        )
                    })?;
                }
                let stage_root = make_temp_import_dir("gravimera_scene_zip_scene")?;
                let stage_scene_dir = stage_root.join(&scene.scene_id);
                let result = (|| {
                    extract_entries_into_root(
                        &mut archive,
                        &scene.indices,
                        SCENES_ZIP_ROOT_DIR,
                        &scene.scene_id,
                        &stage_scene_dir,
                    )?;
                    rewrite_staged_scene_package_in_place(
                        &stage_scene_dir,
                        &scene.scene_id,
                        &resolved_scene_id,
                        &scene_id_map,
                        &prefab_id_map,
                        &terrain_id_map,
                    )?;
                    if dest_root.exists() {
                        return Err(format!(
                            "Destination scene already exists after resolution: {}",
                            dest_root.display()
                        ));
                    }
                    copy_dir_recursive(&stage_scene_dir, &dest_root)?;
                    Ok::<(), String>(())
                })();
                let cleanup_err = remove_dir_if_exists(&stage_root);
                result?;
                cleanup_err?;
                report.imported_scenes += 1;
                if resolved_scene_id != scene.scene_id {
                    report.renamed_scenes += 1;
                } else if conflict && matches!(policy, ImportConflictPolicy::Replace) {
                    report.replaced_scenes += 1;
                }
            }
        }
    }

    Ok(report)
}

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

struct TerrainEntries {
    indices: Vec<usize>,
    has_terrain_def: bool,
    uuid_str: String,
}

fn scan_scene_zip_packages(
    archive: &mut ZipArchive<File>,
) -> Result<
    (
        BTreeMap<String, SceneEntries>,
        BTreeMap<u128, PrefabEntries>,
        BTreeMap<u128, TerrainEntries>,
    ),
    String,
> {
    let mut scenes: BTreeMap<String, SceneEntries> = BTreeMap::new();
    let mut prefabs: BTreeMap<u128, PrefabEntries> = BTreeMap::new();
    let mut terrains: BTreeMap<u128, TerrainEntries> = BTreeMap::new();

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

        if root == TERRAIN_ZIP_ROOT_DIR || root == LEGACY_TERRAIN_ZIP_ROOT_DIR {
            let Some(Component::Normal(uuid_component)) = components.next() else {
                return Err(format!("Zip entry missing terrain UUID: {}", file.name()));
            };
            let uuid_str = uuid_component
                .to_str()
                .ok_or_else(|| format!("Invalid terrain UUID path: {}", file.name()))?;
            let uuid = uuid::Uuid::parse_str(uuid_str)
                .map_err(|_| format!("Invalid terrain UUID in zip: {uuid_str}"))?;
            if uuid.as_u128() == crate::floor_library_ui::DEFAULT_FLOOR_ID {
                return Err(
                    "Zip contains Default Terrain UUID, which is not supported.".to_string()
                );
            }

            let rel: PathBuf = components.collect();
            let entry = terrains
                .entry(uuid.as_u128())
                .or_insert_with(|| TerrainEntries {
                    indices: Vec::new(),
                    has_terrain_def: false,
                    uuid_str: uuid_str.to_string(),
                });
            entry.indices.push(idx);

            if !file.is_dir() {
                if let Some(name) = rel.file_name().and_then(|value| value.to_str()) {
                    if name == TERRAIN_DEF_FILE_NAME || name == LEGACY_TERRAIN_DEF_FILE_NAME {
                        entry.has_terrain_def = true;
                    }
                }
            }
            continue;
        }

        return Err(format!(
            "Zip entry outside {SCENES_ZIP_ROOT_DIR}/, {PREFABS_ZIP_ROOT_DIR}/, or {TERRAIN_ZIP_ROOT_DIR}/: {}",
            file.name()
        ));
    }

    if scenes.is_empty() && prefabs.is_empty() && terrains.is_empty() {
        return Err("Zip contains no scene, prefab, or terrain packages.".to_string());
    }

    Ok((scenes, prefabs, terrains))
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

fn extract_terrain_entries_into_root(
    archive: &mut ZipArchive<File>,
    indices: &[usize],
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
        let rel_from_root = path
            .strip_prefix(TERRAIN_ZIP_ROOT_DIR)
            .or_else(|_| path.strip_prefix(LEGACY_TERRAIN_ZIP_ROOT_DIR))
            .map_err(|_| format!("Zip entry has invalid layout: {}", file.name()))?;
        let rel = rel_from_root
            .strip_prefix(package_name)
            .map_err(|_| format!("Zip entry has invalid layout: {}", file.name()))?;

        let mut out_path = dest_root.join(rel);
        if out_path.file_name().and_then(|value| value.to_str())
            == Some(LEGACY_TERRAIN_DEF_FILE_NAME)
        {
            out_path = out_path.with_file_name(TERRAIN_DEF_FILE_NAME);
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

    Ok(())
}

fn resolve_scene_import_ids(
    scenes_root: &Path,
    scenes: &BTreeMap<String, SceneEntries>,
    policy: ImportConflictPolicy,
) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    let mut reserved = std::collections::HashSet::new();
    for scene in scenes.values() {
        if !scene.has_scene_content {
            continue;
        }
        let existing_root = scenes_root.join(&scene.scene_id);
        let dest_scene_id =
            if existing_root.exists() && matches!(policy, ImportConflictPolicy::KeepBoth) {
                generate_unique_scene_id(scenes_root, &scene.scene_id, &mut reserved)?
            } else {
                scene.scene_id.clone()
            };
        reserved.insert(dest_scene_id.clone());
        out.insert(scene.scene_id.clone(), dest_scene_id);
    }
    Ok(out)
}

fn resolve_prefab_import_ids(
    prefabs_root: &Path,
    prefabs: &BTreeMap<u128, PrefabEntries>,
    policy: ImportConflictPolicy,
) -> Result<BTreeMap<u128, u128>, String> {
    let mut out = BTreeMap::new();
    let mut reserved = std::collections::HashSet::new();
    for (prefab_id, prefab) in prefabs {
        if !prefab.has_prefab_json {
            continue;
        }
        let existing_root = prefabs_root.join(uuid::Uuid::from_u128(*prefab_id).to_string());
        let dest_prefab_id =
            if existing_root.exists() && matches!(policy, ImportConflictPolicy::KeepBoth) {
                generate_unique_prefab_id_from_root(prefabs_root, &mut reserved)?
            } else {
                *prefab_id
            };
        reserved.insert(dest_prefab_id);
        out.insert(*prefab_id, dest_prefab_id);
    }
    Ok(out)
}

fn resolve_terrain_import_ids(
    terrains_root: &Path,
    terrains: &BTreeMap<u128, TerrainEntries>,
    policy: ImportConflictPolicy,
) -> Result<BTreeMap<u128, u128>, String> {
    let mut out = BTreeMap::new();
    let mut reserved = std::collections::HashSet::new();
    for (terrain_id, terrain) in terrains {
        if !terrain.has_terrain_def {
            continue;
        }
        let existing_root = terrains_root.join(uuid::Uuid::from_u128(*terrain_id).to_string());
        let dest_terrain_id =
            if existing_root.exists() && matches!(policy, ImportConflictPolicy::KeepBoth) {
                generate_unique_terrain_id_from_root(terrains_root, &mut reserved)?
            } else {
                *terrain_id
            };
        reserved.insert(dest_terrain_id);
        out.insert(*terrain_id, dest_terrain_id);
    }
    Ok(out)
}

fn generate_unique_scene_id(
    scenes_root: &Path,
    base_scene_id: &str,
    reserved: &mut std::collections::HashSet<String>,
) -> Result<String, String> {
    for _ in 0..1024 {
        let candidate = format!(
            "{}_import_{}",
            base_scene_id,
            &uuid::Uuid::new_v4().simple().to_string()[..8]
        );
        if !reserved.insert(candidate.clone()) {
            continue;
        }
        if !scenes_root.join(&candidate).exists() {
            return Ok(candidate);
        }
    }

    Err(format!(
        "Failed to generate a unique scene id for `{base_scene_id}`."
    ))
}

fn generate_unique_prefab_id_from_root(
    prefabs_root: &Path,
    reserved: &mut std::collections::HashSet<u128>,
) -> Result<u128, String> {
    for _ in 0..1024 {
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        if !reserved.insert(prefab_id) {
            continue;
        }
        if !prefabs_root
            .join(uuid::Uuid::from_u128(prefab_id).to_string())
            .exists()
        {
            return Ok(prefab_id);
        }
    }

    Err("Failed to generate a unique prefab id for scene keep-both import.".to_string())
}

fn generate_unique_terrain_id_from_root(
    terrains_root: &Path,
    reserved: &mut std::collections::HashSet<u128>,
) -> Result<u128, String> {
    for _ in 0..1024 {
        let terrain_id = uuid::Uuid::new_v4().as_u128();
        if terrain_id == crate::floor_library_ui::DEFAULT_FLOOR_ID {
            continue;
        }
        if !reserved.insert(terrain_id) {
            continue;
        }
        if !terrains_root
            .join(uuid::Uuid::from_u128(terrain_id).to_string())
            .exists()
        {
            return Ok(terrain_id);
        }
    }

    Err("Failed to generate a unique terrain id for scene keep-both import.".to_string())
}

fn rewrite_staged_scene_package_in_place(
    staged_scene_dir: &Path,
    _original_scene_id: &str,
    resolved_scene_id: &str,
    scene_id_map: &BTreeMap<String, String>,
    prefab_id_map: &BTreeMap<u128, u128>,
    terrain_id_map: &BTreeMap<u128, u128>,
) -> Result<(), String> {
    for rel_path in ["build/scene.grav", "build/scene.build.grav"] {
        let path = staged_scene_dir.join(rel_path);
        if !path.exists() {
            continue;
        }
        let bytes = std::fs::read(&path)
            .map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
        let remapped =
            crate::scene_store::remap_prefab_ids_in_scene_dat_bytes(&bytes, prefab_id_map)
                .map_err(|err| format!("{}: {err}", path.display()))?;
        std::fs::write(&path, remapped)
            .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    }

    crate::scene_floor_selection::remap_scene_floor_selection_in_build_dir(
        &staged_scene_dir.join("build"),
        terrain_id_map,
    )?;

    let src_dir = staged_scene_dir.join("src");
    let index_path = src_dir.join(crate::scene_sources::SCENE_SOURCES_INDEX_FILE_NAME);
    if !index_path.exists() {
        return Ok(());
    }

    let mut sources = crate::scene_sources::SceneSourcesV1::load_from_dir(&src_dir)
        .map_err(|err| err.to_string())?;
    remap_scene_sources_json_value(&mut sources.index_json, scene_id_map, prefab_id_map);
    remap_scene_sources_json_value(&mut sources.meta_json, scene_id_map, prefab_id_map);
    remap_scene_sources_json_value(&mut sources.markers_json, scene_id_map, prefab_id_map);
    remap_scene_sources_json_value(
        &mut sources.style_pack_ref_json,
        scene_id_map,
        prefab_id_map,
    );
    for value in sources.extra_json_files.values_mut() {
        remap_scene_sources_json_value(value, scene_id_map, prefab_id_map);
    }
    if let Some(obj) = sources.meta_json.as_object_mut() {
        obj.insert(
            "scene_id".to_string(),
            serde_json::Value::String(resolved_scene_id.to_string()),
        );
    }
    sources
        .write_to_dir(&src_dir)
        .map_err(|err| err.to_string())
}

fn remap_scene_sources_json_value(
    value: &mut serde_json::Value,
    scene_id_map: &BTreeMap<String, String>,
    prefab_id_map: &BTreeMap<u128, u128>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map.iter_mut() {
                match key.as_str() {
                    "destination_scene_id" => {
                        if let Some(raw) = child.as_str() {
                            if let Some(mapped) = scene_id_map.get(raw.trim()) {
                                *child = serde_json::Value::String(mapped.clone());
                                continue;
                            }
                        }
                    }
                    "prefab_id" => {
                        if let Some(raw) = child.as_str() {
                            if let Ok(uuid) = uuid::Uuid::parse_str(raw.trim()) {
                                if let Some(mapped) = prefab_id_map.get(&uuid.as_u128()) {
                                    *child = serde_json::Value::String(
                                        uuid::Uuid::from_u128(*mapped).to_string(),
                                    );
                                    continue;
                                }
                            }
                        }
                    }
                    "forms" => {
                        if let Some(items) = child.as_array_mut() {
                            for item in items {
                                let Some(raw) = item.as_str() else {
                                    continue;
                                };
                                let Ok(uuid) = uuid::Uuid::parse_str(raw.trim()) else {
                                    continue;
                                };
                                if let Some(mapped) = prefab_id_map.get(&uuid.as_u128()) {
                                    *item = serde_json::Value::String(
                                        uuid::Uuid::from_u128(*mapped).to_string(),
                                    );
                                }
                            }
                            continue;
                        }
                    }
                    _ => {}
                }
                remap_scene_sources_json_value(child, scene_id_map, prefab_id_map);
            }
        }
        serde_json::Value::Array(items) => {
            for child in items {
                remap_scene_sources_json_value(child, scene_id_map, prefab_id_map);
            }
        }
        _ => {}
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }
    std::fs::create_dir_all(to)
        .map_err(|err| format!("Failed to create {}: {err}", to.display()))?;
    let entries = std::fs::read_dir(from)
        .map_err(|err| format!("Failed to list {}: {err}", from.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let src_path = entry.path();
        let dst_path = to.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
            continue;
        }
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
    Ok(())
}

fn make_temp_import_dir(prefix: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!(
        "{}_{}_{}",
        prefix,
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create temp dir {}: {err}", dir.display()))?;
    Ok(dir)
}

fn remove_dir_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!("Failed to remove {}: {err}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_scene_sources(src_dir: &Path, scene_id: &str, prefab_id: u128) {
        let mut extra = BTreeMap::new();
        extra.insert(
            PathBuf::from("pinned_instances/root.json"),
            json!({
                "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
                "instance_id": uuid::Uuid::new_v4().to_string(),
                "prefab_id": uuid::Uuid::from_u128(prefab_id).to_string(),
                "transform": {
                    "translation": [0.0, 0.0, 0.0]
                },
                "forms": [uuid::Uuid::from_u128(prefab_id).to_string()],
                "active_form": 0
            }),
        );
        extra.insert(
            PathBuf::from("portals/self.json"),
            json!({
                "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
                "portal_id": "self",
                "destination_scene_id": scene_id
            }),
        );

        let sources = crate::scene_sources::SceneSourcesV1 {
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
                "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION
            }),
            style_pack_ref_json: json!({
                "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION
            }),
            extra_json_files: extra,
        };
        sources.write_to_dir(src_dir).expect("write scene sources");
    }

    fn write_prefab_package(root: &Path, prefab_id: u128) {
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let prefab_dir = root.join(&prefab_uuid);
        std::fs::create_dir_all(prefab_dir.join("prefabs")).expect("create prefab dir");
        std::fs::write(
            prefab_dir
                .join("prefabs")
                .join(format!("{prefab_uuid}.json")),
            format!(
                "{{\"format_version\":1,\"prefab_id\":\"{}\",\"role\":\"root\",\"label\":\"Thing\",\"size\":{{\"x\":1.0,\"y\":1.0,\"z\":1.0}},\"collider\":{{\"kind\":\"aabb_xz\",\"half_extents\":{{\"x\":0.5,\"y\":0.5}}}},\"interaction\":{{\"blocks_bullets\":false,\"blocks_laser\":false,\"supports_standing\":false}},\"anchors\":[],\"parts\":[]}}",
                prefab_uuid
            ),
        )
        .expect("write prefab json");
    }

    fn write_terrain_package(root: &Path, terrain_id: u128, label: &str) {
        let uuid = uuid::Uuid::from_u128(terrain_id).to_string();
        let package_dir = root.join(uuid);
        std::fs::create_dir_all(&package_dir).expect("create terrain package dir");
        std::fs::write(
            package_dir.join(TERRAIN_DEF_FILE_NAME),
            format!(
                "{{\"format_version\":1,\"label\":\"{}\",\"mesh\":{{\"kind\":\"grid\",\"size_m\":[10.0,10.0],\"subdiv\":[1,1],\"thickness_m\":0.1,\"uv_tiling\":[1.0,1.0]}},\"material\":{{\"base_color_rgba\":[0.1,0.1,0.1,1.0],\"metallic\":0.0,\"roughness\":1.0,\"unlit\":false}},\"coloring\":{{\"mode\":\"solid\",\"palette\":[],\"scale\":[1.0,1.0],\"angle_deg\":0.0,\"noise\":{{\"seed\":1,\"frequency\":0.1,\"octaves\":1,\"lacunarity\":2.0,\"gain\":0.5}}}},\"relief\":{{\"mode\":\"none\",\"amplitude\":0.0,\"noise\":{{\"seed\":1,\"frequency\":0.1,\"octaves\":1,\"lacunarity\":2.0,\"gain\":0.5}}}},\"animation\":{{\"mode\":\"none\",\"waves\":[],\"normal_strength\":1.0}}}}",
                label
            ),
        )
        .expect("write terrain def");
    }

    fn write_legacy_scene_floor_selection(build_dir: &Path, terrain_id: u128) {
        let uuid = uuid::Uuid::from_u128(terrain_id).to_string();
        std::fs::write(
            build_dir.join("floor_selection.json"),
            serde_json::to_vec_pretty(&json!({
                "format_version": 1,
                "floor_id": uuid,
            }))
            .expect("encode legacy terrain selection"),
        )
        .expect("write legacy terrain selection");
    }

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
        let src_floors_root = temp_root.join("src_terrain");
        let dst_scenes_root = temp_root.join("dst_scenes");
        let dst_prefabs_root = temp_root.join("dst_prefabs");
        let dst_floors_root = temp_root.join("dst_terrain");
        let zip_path = temp_root.join("scenes.zip");

        let scene_id = "alpha".to_string();
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let terrain_id = uuid::Uuid::new_v4().as_u128();
        let terrain_uuid = uuid::Uuid::from_u128(terrain_id).to_string();

        let scene_dir = src_scenes_root.join(&scene_id);
        std::fs::create_dir_all(scene_dir.join("build")).expect("create build dir");
        std::fs::write(
            scene_dir.join("build").join("scene.grav"),
            crate::scene_store::test_encode_scene_dat_with_prefab_ids(&[prefab_id]),
        )
        .expect("write scene.grav");
        write_legacy_scene_floor_selection(&scene_dir.join("build"), terrain_id);
        write_scene_sources(&scene_dir.join("src"), &scene_id, prefab_id);
        write_prefab_package(&src_prefabs_root, prefab_id);
        write_terrain_package(&src_floors_root, terrain_id, "Test Terrain");

        let export_report = export_scene_packages_to_zip_from_roots(
            &src_scenes_root,
            &src_prefabs_root,
            &src_floors_root,
            std::slice::from_ref(&scene_id),
            &zip_path,
        )
        .expect("export scene zip");
        assert_eq!(export_report.exported_scenes, 1);
        assert_eq!(export_report.exported_prefabs, 1);
        assert_eq!(export_report.exported_terrains, 1);

        let import_report = import_scene_packages_from_zip_to_roots_with_policy(
            &dst_scenes_root,
            &dst_prefabs_root,
            &dst_floors_root,
            &zip_path,
            ImportConflictPolicy::Replace,
        )
        .expect("import scene zip");
        assert_eq!(import_report.imported_scenes, 1);
        assert_eq!(import_report.imported_prefabs, 1);
        assert_eq!(import_report.imported_terrains, 1);
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
        assert!(dst_floors_root
            .join(&terrain_uuid)
            .join(TERRAIN_DEF_FILE_NAME)
            .exists());

        let imported_floor =
            crate::scene_floor_selection::read_scene_floor_selection_from_build_dir(
                &dst_scenes_root.join(&scene_id).join("build"),
            )
            .expect("read imported terrain selection");
        assert_eq!(imported_floor, Some(terrain_id));

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn import_replaces_existing_scene_and_prefab() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_zip_skip_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_scenes_root = temp_root.join("src_scenes");
        let src_prefabs_root = temp_root.join("src_prefabs");
        let src_floors_root = temp_root.join("src_terrain");
        let dst_scenes_root = temp_root.join("dst_scenes");
        let dst_prefabs_root = temp_root.join("dst_prefabs");
        let dst_floors_root = temp_root.join("dst_terrain");
        let zip_path = temp_root.join("scenes.zip");

        let scene_id = "alpha".to_string();
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let terrain_id = uuid::Uuid::new_v4().as_u128();
        let terrain_uuid = uuid::Uuid::from_u128(terrain_id).to_string();
        let scene_dir = src_scenes_root.join(&scene_id);
        std::fs::create_dir_all(scene_dir.join("build")).expect("create build dir");
        std::fs::write(
            scene_dir.join("build").join("scene.grav"),
            crate::scene_store::test_encode_scene_dat_with_prefab_ids(&[prefab_id]),
        )
        .expect("write scene.grav");
        write_legacy_scene_floor_selection(&scene_dir.join("build"), terrain_id);
        write_scene_sources(&scene_dir.join("src"), &scene_id, prefab_id);
        write_prefab_package(&src_prefabs_root, prefab_id);
        write_terrain_package(&src_floors_root, terrain_id, "New Terrain");

        export_scene_packages_to_zip_from_roots(
            &src_scenes_root,
            &src_prefabs_root,
            &src_floors_root,
            std::slice::from_ref(&scene_id),
            &zip_path,
        )
        .expect("export scene zip");

        std::fs::create_dir_all(dst_scenes_root.join(&scene_id)).expect("precreate scene");
        std::fs::create_dir_all(dst_prefabs_root.join(&prefab_uuid)).expect("precreate prefab");
        write_terrain_package(&dst_floors_root, terrain_id, "Old Terrain");

        let report = import_scene_packages_from_zip_to_roots_with_policy(
            &dst_scenes_root,
            &dst_prefabs_root,
            &dst_floors_root,
            &zip_path,
            ImportConflictPolicy::Replace,
        )
        .expect("import scene zip");
        assert_eq!(report.imported_scenes, 1);
        assert_eq!(report.replaced_scenes, 1);
        assert_eq!(report.imported_prefabs, 1);
        assert_eq!(report.replaced_prefabs, 1);
        assert_eq!(report.imported_terrains, 1);
        assert_eq!(report.replaced_terrains, 1);

        assert!(dst_floors_root
            .join(&terrain_uuid)
            .join(TERRAIN_DEF_FILE_NAME)
            .exists());

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn keep_both_renames_conflicting_scene_and_prefab_and_rewrites_refs() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_zip_keep_both_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_scenes_root = temp_root.join("src_scenes");
        let src_prefabs_root = temp_root.join("src_prefabs");
        let src_floors_root = temp_root.join("src_terrain");
        let dst_scenes_root = temp_root.join("dst_scenes");
        let dst_prefabs_root = temp_root.join("dst_prefabs");
        let dst_floors_root = temp_root.join("dst_terrain");
        let zip_path = temp_root.join("scenes.zip");

        let scene_id = "alpha".to_string();
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let terrain_id = uuid::Uuid::new_v4().as_u128();

        let scene_dir = src_scenes_root.join(&scene_id);
        std::fs::create_dir_all(scene_dir.join("build")).expect("create build dir");
        std::fs::write(
            scene_dir.join("build").join("scene.grav"),
            crate::scene_store::test_encode_scene_dat_with_prefab_ids(&[prefab_id]),
        )
        .expect("write scene.grav");
        write_legacy_scene_floor_selection(&scene_dir.join("build"), terrain_id);
        write_scene_sources(&scene_dir.join("src"), &scene_id, prefab_id);
        write_prefab_package(&src_prefabs_root, prefab_id);
        write_terrain_package(&src_floors_root, terrain_id, "Copy Terrain");

        export_scene_packages_to_zip_from_roots(
            &src_scenes_root,
            &src_prefabs_root,
            &src_floors_root,
            std::slice::from_ref(&scene_id),
            &zip_path,
        )
        .expect("export scene zip");

        std::fs::create_dir_all(dst_scenes_root.join(&scene_id)).expect("precreate scene");
        write_prefab_package(&dst_prefabs_root, prefab_id);
        write_terrain_package(&dst_floors_root, terrain_id, "Existing Terrain");

        let report = import_scene_packages_from_zip_to_roots_with_policy(
            &dst_scenes_root,
            &dst_prefabs_root,
            &dst_floors_root,
            &zip_path,
            ImportConflictPolicy::KeepBoth,
        )
        .expect("import keep-both");
        assert_eq!(report.imported_scenes, 1);
        assert_eq!(report.renamed_scenes, 1);
        assert_eq!(report.imported_prefabs, 1);
        assert_eq!(report.renamed_prefabs, 1);
        assert_eq!(report.imported_terrains, 1);
        assert_eq!(report.renamed_terrains, 1);

        let imported_scene_ids: Vec<String> = std::fs::read_dir(&dst_scenes_root)
            .expect("list scenes")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect();
        let new_scene_id = imported_scene_ids
            .iter()
            .find(|id| id.as_str() != scene_id)
            .cloned()
            .expect("new scene id");

        let imported_prefab_ids: Vec<u128> = std::fs::read_dir(&dst_prefabs_root)
            .expect("list prefabs")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| uuid::Uuid::parse_str(&name).ok().map(|uuid| uuid.as_u128()))
            .collect();
        let new_prefab_id = imported_prefab_ids
            .iter()
            .copied()
            .find(|id| *id != prefab_id)
            .expect("new prefab id");

        let imported_terrain_ids: Vec<u128> = std::fs::read_dir(&dst_floors_root)
            .expect("list terrains")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| uuid::Uuid::parse_str(&name).ok().map(|uuid| uuid.as_u128()))
            .collect();
        let new_terrain_id = imported_terrain_ids
            .iter()
            .copied()
            .find(|id| *id != terrain_id)
            .expect("new terrain id");

        let remapped_scene_dat = dst_scenes_root.join(&new_scene_id).join("build/scene.grav");
        let referenced =
            crate::scene_store::referenced_prefab_ids_in_scene_dat_path(&remapped_scene_dat)
                .expect("read remapped scene ids");
        assert!(referenced.contains(&new_prefab_id));
        assert!(!referenced.contains(&prefab_id));

        let imported_floor =
            crate::scene_floor_selection::read_scene_floor_selection_from_build_dir(
                &dst_scenes_root.join(&new_scene_id).join("build"),
            )
            .expect("read remapped terrain selection");
        assert_eq!(imported_floor, Some(new_terrain_id));

        let remapped_sources = crate::scene_sources::SceneSourcesV1::load_from_dir(
            &dst_scenes_root.join(&new_scene_id).join("src"),
        )
        .expect("load remapped scene sources");
        assert_eq!(remapped_sources.meta_json["scene_id"], json!(new_scene_id));
        let portal = remapped_sources
            .extra_json_files
            .get(&PathBuf::from("portals/self.json"))
            .expect("portal file");
        assert_eq!(portal["destination_scene_id"], json!(new_scene_id));
        let pinned = remapped_sources
            .extra_json_files
            .get(&PathBuf::from("pinned_instances/root.json"))
            .expect("pinned instance file");
        assert_eq!(
            pinned["prefab_id"],
            json!(uuid::Uuid::from_u128(new_prefab_id).to_string())
        );
        assert_eq!(
            pinned["forms"][0],
            json!(uuid::Uuid::from_u128(new_prefab_id).to_string())
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
