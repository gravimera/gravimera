use prost::Message;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

const SCENE_TERRAIN_SELECTION_FORMAT_VERSION: u32 = 1;
const LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION: u32 = 1;

#[derive(Clone, PartialEq, Message)]
struct SceneTerrainSelectionFileV1 {
    #[prost(uint32, tag = "1")]
    format_version: u32,
    #[prost(message, optional, tag = "2")]
    terrain_id: Option<Uuid128Dat>,
}

#[derive(Clone, PartialEq, Message)]
struct Uuid128Dat {
    #[prost(fixed64, tag = "1")]
    hi: u64,
    #[prost(fixed64, tag = "2")]
    lo: u64,
}

#[derive(Debug, Serialize, Deserialize)]
struct LegacySceneFloorSelectionFileV1 {
    format_version: u32,
    floor_id: Option<String>,
}

pub(crate) fn load_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
) -> Result<Option<u128>, String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene terrain selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene terrain selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
    let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
    load_scene_floor_selection_from_paths(&path, &legacy_path)
}

pub(crate) fn save_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
    floor_id: Option<u128>,
) -> Result<(), String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene terrain selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene terrain selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
    let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
    save_scene_floor_selection_to_paths(&path, &legacy_path, floor_id)
}

pub(crate) fn migrate_legacy_scene_floor_selection_files() -> Result<(), String> {
    let realms_dir = crate::paths::realms_dir();
    if !realms_dir.exists() {
        return Ok(());
    }

    let mut errors = Vec::new();
    let Ok(realm_entries) = std::fs::read_dir(&realms_dir) else {
        return Ok(());
    };

    for realm_entry in realm_entries.flatten() {
        let realm_path = realm_entry.path();
        if !realm_path.is_dir() {
            continue;
        }
        let Some(realm_name) = realm_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Some(realm_id) = crate::realm::sanitize_id(realm_name) else {
            continue;
        };

        let scenes_dir = crate::paths::realm_dir(&realm_id).join("scenes");
        let Ok(scene_entries) = std::fs::read_dir(&scenes_dir) else {
            continue;
        };

        for scene_entry in scene_entries.flatten() {
            let scene_path = scene_entry.path();
            if !scene_path.is_dir() {
                continue;
            }
            let Some(scene_name) = scene_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(scene_id) = crate::realm::sanitize_id(scene_name) else {
                continue;
            };

            let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);
            let legacy_path = crate::paths::legacy_scene_floor_selection_path(&realm_id, &scene_id);
            if !legacy_path.exists() {
                continue;
            }

            if let Err(err) = load_scene_floor_selection_from_paths(&path, &legacy_path) {
                errors.push(err);
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join(" | "))
    }
}

pub(crate) fn read_scene_floor_selection_from_build_dir(
    build_dir: &Path,
) -> Result<Option<u128>, String> {
    let path = build_dir.join("terrain.grav");
    match std::fs::read(&path) {
        Ok(bytes) => {
            let parsed = SceneTerrainSelectionFileV1::decode(bytes.as_slice()).map_err(|err| {
                format!(
                    "scene terrain selection: failed to decode {}: {err}",
                    path.display()
                )
            })?;
            if parsed.format_version != SCENE_TERRAIN_SELECTION_FORMAT_VERSION {
                return Ok(None);
            }
            return Ok(parsed.terrain_id.map(uuid128_to_u128));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                path.display()
            ))
        }
    }

    let legacy_path = build_dir.join("floor_selection.json");
    let legacy_bytes = match std::fs::read(&legacy_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                legacy_path.display()
            ))
        }
    };

    parse_legacy_scene_floor_selection(&legacy_bytes, &legacy_path)
}

pub(crate) fn remap_scene_floor_selection_in_build_dir(
    build_dir: &Path,
    floor_id_map: &BTreeMap<u128, u128>,
) -> Result<(), String> {
    if floor_id_map.is_empty() {
        return Ok(());
    }

    let Some(floor_id) = read_scene_floor_selection_from_build_dir(build_dir)? else {
        return Ok(());
    };
    let Some(new_id) = floor_id_map.get(&floor_id).copied() else {
        return Ok(());
    };
    if new_id == floor_id {
        return Ok(());
    }

    let path = build_dir.join("terrain.grav");
    let legacy_path = build_dir.join("floor_selection.json");
    save_scene_floor_selection_to_paths(&path, &legacy_path, Some(new_id))
}

fn load_scene_floor_selection_from_paths(
    path: &Path,
    legacy_path: &Path,
) -> Result<Option<u128>, String> {
    match std::fs::read(path) {
        Ok(bytes) => {
            let parsed = SceneTerrainSelectionFileV1::decode(bytes.as_slice()).map_err(|err| {
                format!(
                    "scene terrain selection: failed to decode {}: {err}",
                    path.display()
                )
            })?;
            if parsed.format_version != SCENE_TERRAIN_SELECTION_FORMAT_VERSION {
                return Ok(None);
            }
            if legacy_path.exists() {
                let _ = std::fs::remove_file(legacy_path);
            }
            return Ok(parsed.terrain_id.map(uuid128_to_u128));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                path.display()
            ))
        }
    }

    let legacy_bytes = match std::fs::read(legacy_path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "scene terrain selection: failed to read {}: {err}",
                legacy_path.display()
            ))
        }
    };

    let parsed = parse_legacy_scene_floor_selection(&legacy_bytes, legacy_path)?;
    save_scene_floor_selection_to_paths(path, legacy_path, parsed)?;
    Ok(parsed)
}

fn save_scene_floor_selection_to_paths(
    path: &Path,
    legacy_path: &Path,
    floor_id: Option<u128>,
) -> Result<(), String> {
    if floor_id.is_none() {
        remove_if_exists(path)?;
        remove_if_exists(legacy_path)?;
        return Ok(());
    }

    let doc = SceneTerrainSelectionFileV1 {
        format_version: SCENE_TERRAIN_SELECTION_FORMAT_VERSION,
        terrain_id: floor_id.map(u128_to_uuid128),
    };
    let bytes = doc.encode_to_vec();
    write_atomic(path, &bytes)?;
    remove_if_exists(legacy_path)?;
    Ok(())
}

fn parse_legacy_scene_floor_selection(bytes: &[u8], path: &Path) -> Result<Option<u128>, String> {
    let parsed: LegacySceneFloorSelectionFileV1 = serde_json::from_slice(bytes).map_err(|err| {
        format!(
            "scene terrain selection: invalid legacy JSON in {}: {err}",
            path.display()
        )
    })?;

    if parsed.format_version != LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION {
        return Ok(None);
    }

    let Some(raw_id) = parsed.floor_id else {
        return Ok(None);
    };
    let id = uuid::Uuid::parse_str(raw_id.trim()).map_err(|err| {
        format!(
            "scene terrain selection: invalid legacy floor id in {}: {err}",
            path.display()
        )
    })?;
    Ok(Some(id.as_u128()))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "scene terrain selection: failed to create {}: {err}",
                parent.display()
            )
        })?;
    }

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, bytes).map_err(|err| {
        format!(
            "scene terrain selection: failed to write {}: {err}",
            tmp_path.display()
        )
    })?;

    if let Err(err) = std::fs::rename(&tmp_path, path) {
        if path.exists() {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp_path, path).map_err(|rename_err| {
                format!(
                    "scene terrain selection: failed to replace {} after rename error {err}: {rename_err}",
                    path.display()
                )
            })?;
        } else {
            return Err(format!(
                "scene terrain selection: failed to rename {} to {}: {err}",
                tmp_path.display(),
                path.display()
            ));
        }
    }

    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<(), String> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(format!(
            "scene terrain selection: failed to remove {}: {err}",
            path.display()
        )),
    }
}

fn u128_to_uuid128(value: u128) -> Uuid128Dat {
    Uuid128Dat {
        hi: (value >> 64) as u64,
        lo: value as u64,
    }
}

fn uuid128_to_u128(value: Uuid128Dat) -> u128 {
    ((value.hi as u128) << 64) | (value.lo as u128)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protobuf_roundtrip_uses_terrain_grav() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_terrain_selection_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let path = temp_root.join("terrain.grav");
        let legacy_path = temp_root.join("floor_selection.json");
        let floor_id = uuid::Uuid::new_v4().as_u128();

        save_scene_floor_selection_to_paths(&path, &legacy_path, Some(floor_id))
            .expect("save terrain selection");
        let loaded =
            load_scene_floor_selection_from_paths(&path, &legacy_path).expect("load selection");

        assert_eq!(loaded, Some(floor_id));
        assert!(path.exists(), "terrain.grav should be written");
        assert!(
            !legacy_path.exists(),
            "legacy floor_selection.json should not exist after save"
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }

    #[test]
    fn migrates_legacy_json_into_protobuf() {
        let temp_root = std::env::temp_dir().join(format!(
            "gravimera_scene_terrain_selection_legacy_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        std::fs::create_dir_all(&temp_root).expect("create temp root");

        let path = temp_root.join("terrain.grav");
        let legacy_path = temp_root.join("floor_selection.json");
        let floor_id = uuid::Uuid::new_v4();
        std::fs::write(
            &legacy_path,
            serde_json::to_vec_pretty(&LegacySceneFloorSelectionFileV1 {
                format_version: LEGACY_SCENE_FLOOR_SELECTION_FORMAT_VERSION,
                floor_id: Some(floor_id.to_string()),
            })
            .expect("encode legacy json"),
        )
        .expect("write legacy json");

        let loaded =
            load_scene_floor_selection_from_paths(&path, &legacy_path).expect("migrate selection");

        assert_eq!(loaded, Some(floor_id.as_u128()));
        assert!(path.exists(), "terrain.grav should exist after migration");
        assert!(
            !legacy_path.exists(),
            "legacy floor_selection.json should be removed after migration"
        );

        let _ = std::fs::remove_dir_all(&temp_root);
    }
}
