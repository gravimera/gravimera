use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ai::Gen3dAiJob;

use crate::model_library_ui::ModelLibraryUiState;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Gen3dInFlightFileV1 {
    version: u32,
    entries: Vec<Gen3dInFlightEntry>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Gen3dInFlightStatus {
    Running,
    Queued,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Gen3dInFlightEntry {
    pub(crate) run_id: String,
    pub(crate) label: String,
    pub(crate) status: Gen3dInFlightStatus,
    pub(crate) created_at_ms: u128,
    pub(crate) updated_at_ms: u128,
    pub(crate) error: Option<String>,
}

pub(crate) fn gen3d_in_flight_label(prompt: &str, image_count: usize) -> String {
    let trimmed = prompt.trim();
    let base = if trimmed.is_empty() {
        if image_count > 0 {
            "Image-based run".to_string()
        } else {
            "Untitled run".to_string()
        }
    } else {
        let first_line = trimmed.lines().next().unwrap_or("").trim();
        if first_line.is_empty() {
            "Untitled run".to_string()
        } else {
            first_line.to_string()
        }
    };
    const MAX_CHARS: usize = 48;
    if base.chars().count() > MAX_CHARS {
        let mut out: String = base.chars().take(MAX_CHARS).collect();
        out.push_str("...");
        out
    } else {
        base
    }
}

pub(crate) fn load_gen3d_in_flight_entries(realm_id: &str) -> Vec<Gen3dInFlightEntry> {
    let path = crate::realm_prefab_packages::realm_gen3d_in_flight_path(realm_id);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!("Failed to read {}: {err}", path.display());
            }
            return Vec::new();
        }
    };
    let parsed: Gen3dInFlightFileV1 = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(err) => {
            warn!("Failed to parse {}: {err}", path.display());
            return Vec::new();
        }
    };
    if parsed.version != 1 {
        warn!(
            "Unsupported Gen3D in-flight file version {} at {}",
            parsed.version,
            path.display()
        );
        return Vec::new();
    }
    parsed.entries
}

pub(crate) fn upsert_gen3d_in_flight_entry(
    realm_id: &str,
    run_id: Uuid,
    label: String,
    status: Gen3dInFlightStatus,
    error: Option<String>,
) -> Result<(), String> {
    let mut entries = load_gen3d_in_flight_entries(realm_id);
    let now = now_ms();
    let run_id_str = run_id.to_string();
    if let Some(entry) = entries.iter_mut().find(|e| e.run_id == run_id_str) {
        entry.label = label;
        entry.status = status;
        entry.updated_at_ms = now;
        entry.error = error;
    } else {
        entries.push(Gen3dInFlightEntry {
            run_id: run_id_str,
            label,
            status,
            created_at_ms: now,
            updated_at_ms: now,
            error,
        });
    }
    write_gen3d_in_flight_entries(realm_id, &entries)
}

pub(crate) fn mark_gen3d_in_flight_failed(
    realm_id: &str,
    run_id: Uuid,
    error: String,
) -> Result<(), String> {
    let mut entries = load_gen3d_in_flight_entries(realm_id);
    let run_id_str = run_id.to_string();
    let now = now_ms();
    if let Some(entry) = entries.iter_mut().find(|e| e.run_id == run_id_str) {
        entry.status = Gen3dInFlightStatus::Failed;
        entry.updated_at_ms = now;
        entry.error = Some(error);
        return write_gen3d_in_flight_entries(realm_id, &entries);
    }
    upsert_gen3d_in_flight_entry(
        realm_id,
        run_id,
        "Untitled run".to_string(),
        Gen3dInFlightStatus::Failed,
        Some(error),
    )
}

pub(crate) fn remove_gen3d_in_flight_entry(realm_id: &str, run_id: Uuid) -> Result<(), String> {
    let mut entries = load_gen3d_in_flight_entries(realm_id);
    let run_id_str = run_id.to_string();
    let before = entries.len();
    entries.retain(|entry| entry.run_id != run_id_str);
    if entries.len() == before {
        return Ok(());
    }
    write_gen3d_in_flight_entries(realm_id, &entries)
}

fn write_gen3d_in_flight_entries(
    realm_id: &str,
    entries: &[Gen3dInFlightEntry],
) -> Result<(), String> {
    let path = crate::realm_prefab_packages::realm_gen3d_in_flight_path(realm_id);
    if entries.is_empty() {
        if path.exists() {
            std::fs::remove_file(&path)
                .map_err(|err| format!("Failed to delete {}: {err}", path.display()))?;
        }
        return Ok(());
    }

    let payload = Gen3dInFlightFileV1 {
        version: 1,
        entries: entries.to_vec(),
    };
    let bytes = serde_json::to_vec_pretty(&payload)
        .map_err(|err| format!("Failed to serialize {}: {err}", path.display()))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }

    let tmp_path = path.with_extension("tmp");
    std::fs::write(&tmp_path, bytes)
        .map_err(|err| format!("Failed to write {}: {err}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, &path)
        .map_err(|err| format!("Failed to move {}: {err}", path.display()))?;
    Ok(())
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0)
}

pub(crate) fn gen3d_flush_in_flight_dirty(
    mut job: ResMut<Gen3dAiJob>,
    mut model_library: ResMut<ModelLibraryUiState>,
) {
    if !job.take_in_flight_dirty() {
        return;
    }
    model_library.mark_models_dirty();
}
