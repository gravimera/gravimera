use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use crate::import_conflicts::ImportConflictPolicy;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) struct PrefabZipImportReport {
    pub(crate) imported: usize,
    pub(crate) replaced: usize,
    pub(crate) renamed: usize,
    pub(crate) invalid: usize,
}

pub(crate) struct PrefabZipConflictSummary {
    pub(crate) conflicting_prefab_ids: Vec<String>,
}

impl PrefabZipConflictSummary {
    pub(crate) fn has_conflicts(&self) -> bool {
        !self.conflicting_prefab_ids.is_empty()
    }
}

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

pub(crate) fn export_prefab_packages_to_zip(
    realm_id: &str,
    prefab_ids: &[u128],
    zip_path: &Path,
) -> Result<usize, String> {
    let mut ids: Vec<u128> = prefab_ids.iter().copied().collect();
    ids.sort();
    ids.dedup();

    if let Some(parent) = zip_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let file = File::create(zip_path)
        .map_err(|err| format!("Failed to create {}: {err}", zip_path.display()))?;
    let mut writer = ZipWriter::new(file);

    for prefab_id in &ids {
        let package_dir =
            crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, *prefab_id);
        if !package_dir.exists() {
            return Err(format!(
                "Prefab package not found in this realm: {}",
                uuid::Uuid::from_u128(*prefab_id)
            ));
        }
        let uuid = uuid::Uuid::from_u128(*prefab_id).to_string();
        let zip_root = Path::new("prefabs").join(uuid);
        add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
    }

    writer
        .finish()
        .map_err(|err| format!("Failed to finalize zip: {err}"))?;
    Ok(ids.len())
}

pub(crate) fn summarize_prefab_zip_conflicts(
    realm_id: &str,
    zip_path: &Path,
) -> Result<PrefabZipConflictSummary, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;
    let packages = scan_prefab_packages(&mut archive)?;

    let mut conflicting_prefab_ids = Vec::new();
    for (prefab_id, pkg) in packages {
        if !pkg.has_prefab_json {
            continue;
        }
        let dest_root = crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, prefab_id);
        if dest_root.exists() {
            conflicting_prefab_ids.push(pkg.uuid_str);
        }
    }

    Ok(PrefabZipConflictSummary {
        conflicting_prefab_ids,
    })
}

pub(crate) fn import_prefab_packages_from_zip_with_policy(
    realm_id: &str,
    zip_path: &Path,
    policy: ImportConflictPolicy,
) -> Result<PrefabZipImportReport, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;
    let packages = scan_prefab_packages(&mut archive)?;

    let mut imported = 0;
    let mut replaced = 0;
    let mut renamed = 0;
    let mut invalid = 0;
    let mut reserved_new_ids = std::collections::HashSet::new();

    for (prefab_id, pkg) in packages {
        if !pkg.has_prefab_json {
            invalid += 1;
            continue;
        }

        let dest_root =
            crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, prefab_id);
        let conflict = dest_root.exists();
        match (conflict, policy) {
            (false, _) => {
                extract_prefab_entries_into_root(
                    &mut archive,
                    &pkg.indices,
                    &pkg.uuid_str,
                    &dest_root,
                )?;
                imported += 1;
            }
            (true, ImportConflictPolicy::Replace) => {
                std::fs::remove_dir_all(&dest_root).map_err(|err| {
                    format!(
                        "Failed to replace existing prefab package {}: {err}",
                        dest_root.display()
                    )
                })?;
                extract_prefab_entries_into_root(
                    &mut archive,
                    &pkg.indices,
                    &pkg.uuid_str,
                    &dest_root,
                )?;
                imported += 1;
                replaced += 1;
            }
            (true, ImportConflictPolicy::KeepBoth) => {
                let stage_root = make_temp_import_dir("gravimera_prefab_import")?;
                let stage_package_dir = stage_root.join(&pkg.uuid_str);
                let result = (|| {
                    extract_prefab_entries_into_root(
                        &mut archive,
                        &pkg.indices,
                        &pkg.uuid_str,
                        &stage_package_dir,
                    )?;
                    let new_root_id =
                        generate_unique_prefab_id(realm_id, &mut reserved_new_ids)?;
                    let new_dest_root =
                        crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, new_root_id);
                    remap_staged_prefab_package_into_dest(
                        &stage_package_dir,
                        prefab_id,
                        new_root_id,
                        &new_dest_root,
                    )?;
                    Ok::<(), String>(())
                })();
                let cleanup_err = remove_dir_if_exists(&stage_root);
                result?;
                cleanup_err?;
                imported += 1;
                renamed += 1;
            }
        }
    }

    Ok(PrefabZipImportReport {
        imported,
        replaced,
        renamed,
        invalid,
    })
}

struct PackageEntries {
    indices: Vec<usize>,
    has_prefab_json: bool,
    uuid_str: String,
}

fn scan_prefab_packages(
    archive: &mut ZipArchive<File>,
) -> Result<BTreeMap<u128, PackageEntries>, String> {
    let mut packages: BTreeMap<u128, PackageEntries> = BTreeMap::new();

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
        if root != "prefabs" {
            return Err(format!("Zip entry outside prefabs/: {}", file.name()));
        }

        let Some(Component::Normal(uuid_component)) = components.next() else {
            return Err(format!("Zip entry missing prefab UUID: {}", file.name()));
        };
        let uuid_str = uuid_component
            .to_str()
            .ok_or_else(|| format!("Invalid prefab UUID path: {}", file.name()))?;
        let uuid = uuid::Uuid::parse_str(uuid_str)
            .map_err(|_| format!("Invalid prefab UUID in zip: {uuid_str}"))?;

        let rel: PathBuf = components.collect();
        let entry = packages
            .entry(uuid.as_u128())
            .or_insert_with(|| PackageEntries {
                indices: Vec::new(),
                has_prefab_json: false,
                uuid_str: uuid_str.to_string(),
            });
        entry.indices.push(idx);

        if !file.is_dir() {
            if let Some(Component::Normal(folder)) = rel.components().next() {
                if folder == "prefabs" {
                    if let Some(name) = rel.file_name().and_then(|v| v.to_str()) {
                        if name.ends_with(".json") && !name.ends_with(".desc.json") {
                            entry.has_prefab_json = true;
                        }
                    }
                }
            }
        }
    }

    if packages.is_empty() {
        return Err("Zip contains no prefab packages.".to_string());
    }

    Ok(packages)
}

fn extract_prefab_entries_into_root(
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
        let rel = path
            .strip_prefix("prefabs")
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

pub(crate) fn remap_staged_prefab_package_into_dest(
    staged_package_dir: &Path,
    src_root_prefab_id: u128,
    new_root_prefab_id: u128,
    dest_root: &Path,
) -> Result<(), String> {
    if dest_root.exists() {
        return Err(format!(
            "Destination prefab package already exists: {}",
            dest_root.display()
        ));
    }

    copy_dir_recursive(staged_package_dir, dest_root)?;
    let prefabs_dir = dest_root.join("prefabs");
    let def_paths = collect_prefab_def_paths(&prefabs_dir)?;
    let id_map = build_prefab_id_map(&def_paths, src_root_prefab_id, new_root_prefab_id)?;
    rewrite_prefab_defs_in_place(&prefabs_dir, &def_paths, &id_map)?;
    rewrite_prefab_descriptors_in_place(&prefabs_dir, &id_map)?;
    rewrite_gen3d_edit_bundle_root_id(
        &dest_root.join("gen3d_edit_bundle_v1.json"),
        new_root_prefab_id,
    )?;
    Ok(())
}

fn collect_prefab_def_paths(prefabs_dir: &Path) -> Result<Vec<PathBuf>, String> {
    if !prefabs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    let mut stack = vec![prefabs_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|err| format!("Failed to list {}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            if !file_name.ends_with(".json") || file_name.ends_with(".desc.json") {
                continue;
            }
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}

fn build_prefab_id_map(
    def_paths: &[PathBuf],
    src_root_prefab_id: u128,
    new_root_prefab_id: u128,
) -> Result<BTreeMap<u128, u128>, String> {
    let mut id_map = BTreeMap::new();
    id_map.insert(src_root_prefab_id, new_root_prefab_id);

    for path in def_paths {
        let Some(stem) = path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        let Ok(old_uuid) = uuid::Uuid::parse_str(stem.trim()) else {
            continue;
        };
        let old_id = old_uuid.as_u128();
        id_map.entry(old_id).or_insert_with(|| {
            if old_id == src_root_prefab_id {
                new_root_prefab_id
            } else {
                uuid::Uuid::new_v4().as_u128()
            }
        });
    }

    if id_map.is_empty() {
        return Err(format!(
            "Prefab package {} contains no remappable prefab ids.",
            uuid::Uuid::from_u128(src_root_prefab_id)
        ));
    }

    Ok(id_map)
}

fn rewrite_prefab_defs_in_place(
    prefabs_dir: &Path,
    def_paths: &[PathBuf],
    id_map: &BTreeMap<u128, u128>,
) -> Result<(), String> {
    for old_path in def_paths {
        let bytes = std::fs::read(old_path)
            .map_err(|err| format!("Failed to read {}: {err}", old_path.display()))?;
        let mut value: serde_json::Value = serde_json::from_slice(&bytes)
            .map_err(|err| format!("Invalid prefab JSON {}: {err}", old_path.display()))?;
        remap_prefab_ids_in_json_value(&mut value, id_map);

        let Some(stem) = old_path.file_stem().and_then(|v| v.to_str()) else {
            continue;
        };
        let Ok(old_uuid) = uuid::Uuid::parse_str(stem.trim()) else {
            continue;
        };
        let mapped_id = id_map
            .get(&old_uuid.as_u128())
            .copied()
            .unwrap_or_else(|| old_uuid.as_u128());
        let new_path = old_path.with_file_name(format!(
            "{}.json",
            uuid::Uuid::from_u128(mapped_id)
        ));
        write_json_file_pretty(&new_path, &value)?;
        if new_path != *old_path {
            std::fs::remove_file(old_path).map_err(|err| {
                format!(
                    "Failed to remove old prefab JSON {}: {err}",
                    old_path.display()
                )
            })?;
        }
    }

    if !prefabs_dir.exists() {
        return Ok(());
    }

    let entries = std::fs::read_dir(prefabs_dir)
        .map_err(|err| format!("Failed to list {}: {err}", prefabs_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".json") || file_name.ends_with(".desc.json") {
            continue;
        }
        let Some(stem) = file_name.strip_suffix(".json") else {
            continue;
        };
        if uuid::Uuid::parse_str(stem.trim()).is_err() {
            continue;
        }
    }

    Ok(())
}

fn rewrite_prefab_descriptors_in_place(
    prefabs_dir: &Path,
    id_map: &BTreeMap<u128, u128>,
) -> Result<(), String> {
    if !prefabs_dir.exists() {
        return Ok(());
    }

    let mut stack = vec![prefabs_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|err| format!("Failed to list {}: {err}", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|v| v.to_str()) else {
                continue;
            };
            let Some(stem) = file_name.strip_suffix(".desc.json") else {
                continue;
            };
            let Ok(old_uuid) = uuid::Uuid::parse_str(stem.trim()) else {
                continue;
            };
            let mapped_id = id_map
                .get(&old_uuid.as_u128())
                .copied()
                .unwrap_or_else(|| old_uuid.as_u128());
            let new_path = path.with_file_name(format!(
                "{}.desc.json",
                uuid::Uuid::from_u128(mapped_id)
            ));

            let bytes =
                std::fs::read(&path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
            match serde_json::from_slice::<serde_json::Value>(&bytes) {
                Ok(mut value) => {
                    if let Some(prefab_id_str) =
                        value.get_mut("prefab_id").and_then(|v| v.as_str().map(str::to_string))
                    {
                        if let Ok(uuid) = uuid::Uuid::parse_str(prefab_id_str.trim()) {
                            if let Some(mapped) = id_map.get(&uuid.as_u128()) {
                                value["prefab_id"] = serde_json::Value::String(
                                    uuid::Uuid::from_u128(*mapped).to_string(),
                                );
                            }
                        }
                    }
                    write_json_file_pretty(&new_path, &value)?;
                }
                Err(_) => {
                    if let Some(parent) = new_path.parent() {
                        std::fs::create_dir_all(parent).map_err(|err| {
                            format!("Failed to create {}: {err}", parent.display())
                        })?;
                    }
                    std::fs::write(&new_path, &bytes)
                        .map_err(|err| format!("Failed to write {}: {err}", new_path.display()))?;
                }
            }

            if new_path != path {
                std::fs::remove_file(&path).map_err(|err| {
                    format!(
                        "Failed to remove old prefab descriptor {}: {err}",
                        path.display()
                    )
                })?;
            }
        }
    }

    Ok(())
}

fn rewrite_gen3d_edit_bundle_root_id(path: &Path, new_root_prefab_id: u128) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    let bytes =
        std::fs::read(path).map_err(|err| format!("Failed to read {}: {err}", path.display()))?;
    let Ok(mut value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Ok(());
    };
    if let Some(field) = value.get_mut("root_prefab_id_uuid") {
        *field =
            serde_json::Value::String(uuid::Uuid::from_u128(new_root_prefab_id).to_string());
    } else if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "root_prefab_id_uuid".to_string(),
            serde_json::Value::String(uuid::Uuid::from_u128(new_root_prefab_id).to_string()),
        );
    }
    write_json_file_pretty(path, &value)
}

fn remap_prefab_ids_in_json_value(
    value: &mut serde_json::Value,
    id_map: &BTreeMap<u128, u128>,
) {
    match value {
        serde_json::Value::Object(map) => {
            for child in map.values_mut() {
                remap_prefab_ids_in_json_value(child, id_map);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                remap_prefab_ids_in_json_value(item, id_map);
            }
        }
        serde_json::Value::String(text) => {
            let Ok(uuid) = uuid::Uuid::parse_str(text.trim()) else {
                return;
            };
            if let Some(mapped) = id_map.get(&uuid.as_u128()) {
                *text = uuid::Uuid::from_u128(*mapped).to_string();
            }
        }
        _ => {}
    }
}

fn write_json_file_pretty(path: &Path, value: &serde_json::Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(value).map_err(|err| err.to_string())?;
    std::fs::write(path, bytes)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    if !from.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(to).map_err(|err| format!("Failed to create {}: {err}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|err| format!("Failed to list {}: {err}", from.display()))?;
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

fn generate_unique_prefab_id(
    realm_id: &str,
    reserved_new_ids: &mut std::collections::HashSet<u128>,
) -> Result<u128, String> {
    for _ in 0..1024 {
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        if !reserved_new_ids.insert(prefab_id) {
            continue;
        }
        let dest_root = crate::realm_prefab_packages::realm_prefab_package_dir(realm_id, prefab_id);
        if !dest_root.exists() {
            return Ok(prefab_id);
        }
    }

    Err("Failed to generate a unique prefab id for keep-both import.".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_prefab_package(root: &Path, prefab_id: u128, label: &str, with_meta: bool) {
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();
        let package_dir = root.join(&prefab_uuid);
        std::fs::create_dir_all(package_dir.join("prefabs")).expect("create prefab dir");
        std::fs::write(
            package_dir
                .join("prefabs")
                .join(format!("{prefab_uuid}.json")),
            format!(
                "{{\"format_version\":1,\"prefab_id\":\"{}\",\"role\":\"root\",\"label\":\"{}\",\"size\":{{\"x\":1.0,\"y\":1.0,\"z\":1.0}},\"collider\":{{\"kind\":\"aabb_xz\",\"half_extents\":{{\"x\":0.5,\"y\":0.5}}}},\"interaction\":{{\"blocks_bullets\":false,\"blocks_laser\":false,\"supports_standing\":false}},\"anchors\":[],\"parts\":[]}}",
                prefab_uuid, label
            ),
        )
        .expect("write prefab json");

        if with_meta {
            std::fs::write(
                package_dir
                    .join("prefabs")
                    .join(format!("{prefab_uuid}.desc.json")),
                format!(
                    "{{\"format_version\":1,\"prefab_id\":\"{}\",\"label\":\"{}\"}}",
                    prefab_uuid, label
                ),
            )
            .expect("write descriptor");
            std::fs::write(
                package_dir.join("gen3d_edit_bundle_v1.json"),
                format!(
                    "{{\"version\":1,\"root_prefab_id_uuid\":\"{}\"}}",
                    prefab_uuid
                ),
            )
            .expect("write edit bundle");
        }
    }

    #[test]
    fn replace_overwrites_conflicting_prefab_package() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_prefab_zip_replace_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_root = temp_root.join("src");
        let dst_realm = "dst_replace";
        let zip_path = temp_root.join("prefabs.zip");
        let prefab_id = uuid::Uuid::new_v4().as_u128();
        let prefab_uuid = uuid::Uuid::from_u128(prefab_id).to_string();

        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
        write_prefab_package(&src_root, prefab_id, "New Label", false);
        write_prefab_package(
            &crate::paths::realm_prefabs_dir(dst_realm),
            prefab_id,
            "Old Label",
            false,
        );
        export_prefab_packages_to_zip_from_root(&src_root, &[prefab_id], &zip_path)
            .expect("export prefab zip");

        let report =
            import_prefab_packages_from_zip_with_policy(dst_realm, &zip_path, ImportConflictPolicy::Replace)
                .expect("replace import");
        assert_eq!(report.imported, 1);
        assert_eq!(report.replaced, 1);
        assert_eq!(report.renamed, 0);

        let text = std::fs::read_to_string(
            crate::realm_prefab_packages::realm_prefab_package_dir(dst_realm, prefab_id)
                .join("prefabs")
                .join(format!("{prefab_uuid}.json")),
        )
        .expect("read replaced prefab");
        assert!(text.contains("New Label"));

        let _ = std::fs::remove_dir_all(&temp_root);
        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
    }

    #[test]
    fn keep_both_renames_conflicting_prefab_package_and_updates_metadata() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_prefab_zip_keep_both_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let src_root = temp_root.join("src");
        let dst_realm = "dst_keep_both";
        let zip_path = temp_root.join("prefabs.zip");
        let prefab_id = uuid::Uuid::new_v4().as_u128();

        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
        write_prefab_package(&src_root, prefab_id, "Copy Me", true);
        write_prefab_package(
            &crate::paths::realm_prefabs_dir(dst_realm),
            prefab_id,
            "Existing",
            true,
        );

        export_prefab_packages_to_zip_from_root(&src_root, &[prefab_id], &zip_path)
            .expect("export prefab zip");

        let report = import_prefab_packages_from_zip_with_policy(
            dst_realm,
            &zip_path,
            ImportConflictPolicy::KeepBoth,
        )
        .expect("keep-both import");
        assert_eq!(report.imported, 1);
        assert_eq!(report.replaced, 0);
        assert_eq!(report.renamed, 1);

        let imported_ids: Vec<u128> = std::fs::read_dir(crate::paths::realm_prefabs_dir(dst_realm))
            .expect("list imported ids")
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter_map(|name| uuid::Uuid::parse_str(&name).ok().map(|uuid| uuid.as_u128()))
            .collect();
        let new_prefab_id = imported_ids
            .iter()
            .copied()
            .find(|id| *id != prefab_id)
            .expect("new prefab id");
        let new_uuid = uuid::Uuid::from_u128(new_prefab_id).to_string();

        let new_root = crate::realm_prefab_packages::realm_prefab_package_dir(dst_realm, new_prefab_id);
        let prefab_json = std::fs::read_to_string(new_root.join("prefabs").join(format!("{new_uuid}.json")))
            .expect("read new prefab json");
        assert!(prefab_json.contains(&new_uuid));
        let descriptor = std::fs::read_to_string(
            new_root
                .join("prefabs")
                .join(format!("{new_uuid}.desc.json")),
        )
        .expect("read descriptor");
        assert!(descriptor.contains(&new_uuid));
        let edit_bundle =
            std::fs::read_to_string(new_root.join("gen3d_edit_bundle_v1.json")).expect("read edit bundle");
        assert!(edit_bundle.contains(&new_uuid));

        let _ = std::fs::remove_dir_all(&temp_root);
        let _ = std::fs::remove_dir_all(crate::paths::realm_dir(dst_realm));
    }

    fn export_prefab_packages_to_zip_from_root(
        prefabs_root: &Path,
        prefab_ids: &[u128],
        zip_path: &Path,
    ) -> Result<usize, String> {
        let mut ids: Vec<u128> = prefab_ids.iter().copied().collect();
        ids.sort();
        ids.dedup();

        if let Some(parent) = zip_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
        }

        let file = File::create(zip_path)
            .map_err(|err| format!("Failed to create {}: {err}", zip_path.display()))?;
        let mut writer = ZipWriter::new(file);
        for prefab_id in &ids {
            let package_dir = prefabs_root.join(uuid::Uuid::from_u128(*prefab_id).to_string());
            let zip_root = Path::new("prefabs").join(uuid::Uuid::from_u128(*prefab_id).to_string());
            add_dir_to_zip(&mut writer, &package_dir, &zip_root)?;
        }
        writer
            .finish()
            .map_err(|err| format!("Failed to finalize zip: {err}"))?;
        Ok(ids.len())
    }
}
