use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

pub(crate) struct FloorZipImportReport {
    pub(crate) imported: usize,
    pub(crate) skipped: usize,
    pub(crate) invalid: usize,
}

const ZIP_ROOT_DIR: &str = "floors";
const FLOOR_DEF_FILE_NAME: &str = "floor_def_v1.json";

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
    let mut ids: Vec<u128> = floor_ids.iter().copied().collect();
    ids.sort();
    ids.dedup();

    if ids.contains(&crate::floor_library_ui::DEFAULT_FLOOR_ID) {
        return Err("Default Floor cannot be exported.".to_string());
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
                "Floor package not found in this realm: {}",
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

pub(crate) fn import_floor_packages_from_zip(
    realm_id: &str,
    zip_path: &Path,
) -> Result<FloorZipImportReport, String> {
    let file = File::open(zip_path)
        .map_err(|err| format!("Failed to open {}: {err}", zip_path.display()))?;
    let mut archive = ZipArchive::new(file).map_err(|err| format!("Failed to read zip: {err}"))?;

    struct PackageEntries {
        indices: Vec<usize>,
        has_floor_def: bool,
        uuid_str: String,
    }

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
        if root != ZIP_ROOT_DIR {
            return Err(format!(
                "Zip entry outside {ZIP_ROOT_DIR}/: {}",
                file.name()
            ));
        }

        let Some(Component::Normal(uuid_component)) = components.next() else {
            return Err(format!("Zip entry missing floor UUID: {}", file.name()));
        };
        let uuid_str = uuid_component
            .to_str()
            .ok_or_else(|| format!("Invalid floor UUID path: {}", file.name()))?;
        let uuid = uuid::Uuid::parse_str(uuid_str)
            .map_err(|_| format!("Invalid floor UUID in zip: {uuid_str}"))?;

        if uuid.as_u128() == crate::floor_library_ui::DEFAULT_FLOOR_ID {
            return Err("Zip contains Default Floor UUID, which is not supported.".to_string());
        }

        let rel: PathBuf = components.collect();
        let entry = packages
            .entry(uuid.as_u128())
            .or_insert_with(|| PackageEntries {
                indices: Vec::new(),
                has_floor_def: false,
                uuid_str: uuid_str.to_string(),
            });
        entry.indices.push(idx);

        if !file.is_dir() {
            if let Some(name) = rel.file_name().and_then(|v| v.to_str()) {
                if name == FLOOR_DEF_FILE_NAME {
                    entry.has_floor_def = true;
                }
            }
        }
    }

    if packages.is_empty() {
        return Err("Zip contains no floor packages.".to_string());
    }

    let mut imported = 0;
    let mut skipped = 0;
    let mut invalid = 0;

    for (floor_id, pkg) in packages {
        if !pkg.has_floor_def {
            invalid += 1;
            continue;
        }

        let dest_root = crate::realm_floor_packages::realm_floor_package_dir(realm_id, floor_id);
        if dest_root.exists() {
            skipped += 1;
            continue;
        }

        for idx in pkg.indices {
            let mut file = archive
                .by_index(idx)
                .map_err(|err| format!("Failed to read zip entry: {err}"))?;
            let Some(path) = file.enclosed_name().map(|p| p.to_path_buf()) else {
                return Err("Zip contains invalid path (path traversal).".to_string());
            };
            let rel = path
                .strip_prefix(ZIP_ROOT_DIR)
                .and_then(|path| path.strip_prefix(pkg.uuid_str.as_str()))
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

        imported += 1;
    }

    Ok(FloorZipImportReport {
        imported,
        skipped,
        invalid,
    })
}
