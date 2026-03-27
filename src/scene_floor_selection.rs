use serde::{Deserialize, Serialize};
use std::io;

const SCENE_FLOOR_SELECTION_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SceneFloorSelectionFileV1 {
    format_version: u32,
    floor_id: Option<String>,
}

pub(crate) fn load_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
) -> Result<Option<u128>, String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene floor selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene floor selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);

    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(format!(
                "scene floor selection: failed to read {}: {err}",
                path.display()
            ))
        }
    };

    let parsed: SceneFloorSelectionFileV1 = serde_json::from_slice(&bytes).map_err(|err| {
        format!(
            "scene floor selection: invalid JSON in {}: {err}",
            path.display()
        )
    })?;

    if parsed.format_version != SCENE_FLOOR_SELECTION_FORMAT_VERSION {
        return Ok(None);
    }

    let Some(raw_id) = parsed.floor_id else {
        return Ok(None);
    };
    let id = uuid::Uuid::parse_str(raw_id.trim())
        .map_err(|err| format!("scene floor selection: invalid floor id: {err}"))?;
    Ok(Some(id.as_u128()))
}

pub(crate) fn save_scene_floor_selection(
    realm_id: &str,
    scene_id: &str,
    floor_id: Option<u128>,
) -> Result<(), String> {
    let realm_id = crate::realm::sanitize_id(realm_id)
        .ok_or_else(|| "scene floor selection: invalid realm id".to_string())?;
    let scene_id = crate::realm::sanitize_id(scene_id)
        .ok_or_else(|| "scene floor selection: invalid scene id".to_string())?;
    let path = crate::paths::scene_floor_selection_path(&realm_id, &scene_id);

    if floor_id.is_none() {
        if let Err(err) = std::fs::remove_file(&path) {
            if err.kind() != io::ErrorKind::NotFound {
                return Err(format!(
                    "scene floor selection: failed to remove {}: {err}",
                    path.display()
                ));
            }
        }
        return Ok(());
    }

    let floor_uuid = uuid::Uuid::from_u128(floor_id.unwrap()).to_string();
    let doc = SceneFloorSelectionFileV1 {
        format_version: SCENE_FLOOR_SELECTION_FORMAT_VERSION,
        floor_id: Some(floor_uuid),
    };
    let bytes = serde_json::to_vec_pretty(&doc)
        .map_err(|err| format!("scene floor selection: encode JSON failed: {err}"))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| {
            format!(
                "scene floor selection: failed to create {}: {err}",
                parent.display()
            )
        })?;
    }
    std::fs::write(&path, format!("{}\n", String::from_utf8_lossy(&bytes))).map_err(|err| {
        format!(
            "scene floor selection: failed to write {}: {err}",
            path.display()
        )
    })
}
