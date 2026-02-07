use bevy::prelude::*;
use std::io::Write;
use std::path::Path;

use super::Gen3dPlannedComponent;

pub(super) fn write_gen3d_text_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    text: &str,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    if let Err(err) = std::fs::write(&path, text) {
        debug!("Gen3D: failed to write {}: {err}", path.display());
    }
}

pub(super) fn append_gen3d_run_log(run_dir: Option<&Path>, message: impl AsRef<str>) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join("gen3d_run.log");
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut line = format!("[{ts_ms}] {}", message.as_ref());
    if !line.ends_with('\n') {
        line.push('\n');
    }
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}

pub(super) fn write_gen3d_json_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    json: &serde_json::Value,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    let data = serde_json::to_string_pretty(json).unwrap_or_else(|_| json.to_string());
    if let Err(err) = std::fs::write(&path, data) {
        debug!("Gen3D: failed to write {}: {err}", path.display());
    }
}

pub(super) fn append_gen3d_jsonl_artifact(
    run_dir: Option<&Path>,
    filename: impl AsRef<str>,
    json: &serde_json::Value,
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let path = run_dir.join(filename.as_ref());
    let line = match serde_json::to_string(json) {
        Ok(mut line) => {
            line.push('\n');
            line
        }
        Err(err) => {
            debug!("Gen3D: failed to serialize JSONL {}: {err}", path.display());
            return;
        }
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    let _ = file.write_all(line.as_bytes());
}

pub(super) fn write_gen3d_assembly_snapshot(
    run_dir: Option<&Path>,
    components: &[Gen3dPlannedComponent],
) {
    let Some(run_dir) = run_dir else {
        return;
    };
    let components: Vec<serde_json::Value> = components
        .iter()
        .map(|c| {
            let forward = c.rot * Vec3::Z;
            let up = c.rot * Vec3::Y;
            let size = c.actual_size.unwrap_or(c.planned_size);
            serde_json::json!({
                "name": c.name.as_str(),
                "generated": c.actual_size.is_some(),
                "pos": [c.pos.x, c.pos.y, c.pos.z],
                "forward": [forward.x, forward.y, forward.z],
                "up": [up.x, up.y, up.z],
                "size": [size.x, size.y, size.z],
            })
        })
        .collect();
    let snapshot = serde_json::json!({
        "version": 2,
        "components": components,
    });
    write_gen3d_json_artifact(Some(run_dir), "assembly_transforms.json", &snapshot);
}
