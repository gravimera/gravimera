use bevy::prelude::*;
use std::path::{Path, PathBuf};

use crate::object::registry::{ObjectDef, ObjectLibrary};
use crate::prefab_descriptors::PrefabDescriptorLibrary;

const MODEL_PREFABS_DIR_NAME: &str = "prefabs";

pub(crate) fn depot_models_dir() -> PathBuf {
    crate::paths::depot_models_dir()
}

pub(crate) fn depot_model_dir(model_id: u128) -> PathBuf {
    let uuid = uuid::Uuid::from_u128(model_id).to_string();
    depot_models_dir().join(uuid)
}

pub(crate) fn depot_model_prefabs_dir(model_id: u128) -> PathBuf {
    depot_model_dir(model_id).join(MODEL_PREFABS_DIR_NAME)
}

pub(crate) fn ensure_depot_model_prefabs_dir(model_id: u128) -> Result<PathBuf, String> {
    let dir = depot_model_prefabs_dir(model_id);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create {}: {err}", dir.display()))?;
    Ok(dir)
}

pub(crate) fn list_depot_models() -> Result<Vec<u128>, String> {
    list_depot_models_in_dir(&depot_models_dir())
}

pub(crate) fn save_model_prefab_defs_to_depot(
    model_id: u128,
    root_prefab_id: u128,
    defs: &[ObjectDef],
) -> Result<PathBuf, String> {
    let dir = ensure_depot_model_prefabs_dir(model_id)?;
    crate::realm_prefabs::save_prefab_defs_to_dir(&dir, root_prefab_id, defs)?;
    Ok(dir)
}

pub(crate) fn load_depot_prefabs_into_library(
    library: &mut ObjectLibrary,
) -> Result<usize, String> {
    let models = list_depot_models()?;
    let mut loaded = 0usize;
    for model_id in models {
        let dir = depot_model_prefabs_dir(model_id);
        match crate::realm_prefabs::load_prefabs_into_library_from_dir(&dir, library) {
            Ok(count) => loaded += count,
            Err(err) => warn!("Depot: {err}"),
        }
    }
    Ok(loaded)
}

pub(crate) fn load_depot_prefab_descriptors_into_library(
    library: &mut PrefabDescriptorLibrary,
) -> Result<usize, String> {
    let models = list_depot_models()?;
    let mut loaded = 0usize;
    for model_id in models {
        let dir = depot_model_prefabs_dir(model_id);
        match crate::prefab_descriptors::load_prefab_descriptors_from_dir(&dir, library) {
            Ok(count) => loaded += count,
            Err(err) => warn!("Depot: {err}"),
        }
    }
    Ok(loaded)
}

fn list_depot_models_in_dir(root: &Path) -> Result<Vec<u128>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_depot_models_ignores_non_uuid_folders() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_model_depot_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp depot root");
        std::fs::create_dir_all(temp_root.join("not-a-uuid")).expect("create junk folder");
        let model_id = uuid::Uuid::new_v4().as_u128();
        std::fs::create_dir_all(temp_root.join(uuid::Uuid::from_u128(model_id).to_string()))
            .expect("create uuid folder");

        let models = list_depot_models_in_dir(&temp_root).expect("list models");
        assert_eq!(models, vec![model_id]);

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
