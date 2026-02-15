use bevy::prelude::*;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::scene_sources::SceneSourcesV1;
use crate::scene_sources_patch::SceneSourcesPatchV1;
use crate::scene_sources_runtime::{
    apply_scene_sources_patch, scene_signature_summary_from_sources, validate_scene_sources_patch,
    SceneSourcesPatchApplyReport, SceneSourcesPatchValidateReport, SceneSourcesWorkspace,
};
use crate::scene_validation::ScorecardSpecV1;

pub(crate) const SCENE_RUN_FORMAT_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SceneRunStatusV1 {
    pub(crate) format_version: u32,
    pub(crate) run_id: String,
    pub(crate) last_complete_step: u32,
    pub(crate) next_step: u32,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct SceneRunStepResponseV1 {
    pub(crate) format_version: u32,
    pub(crate) run_id: String,
    pub(crate) step: u32,
    pub(crate) mode: String,
    pub(crate) result: Value,
}

fn require_safe_id(label: &str, value: &str) -> Result<String, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{label} must be a non-empty string"));
    }
    if v.contains('/') || v.contains('\\') || v.contains("..") {
        return Err(format!("{label} must not contain path separators or '..'"));
    }
    Ok(v.to_string())
}

fn write_json_value_atomic(path: &Path, value: &Value) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("no parent for path {}", path.display()));
    };
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("create_dir_all failed ({}): {err}", parent.display()))?;

    let text = serde_json::to_string_pretty(value)
        .map_err(|err| format!("json serialize failed ({}): {err}", path.display()))?;
    let bytes = format!("{text}\n").into_bytes();

    let tmp_path = path.with_extension("json.tmp");
    std::fs::write(&tmp_path, &bytes)
        .map_err(|err| format!("write failed ({}): {err}", tmp_path.display()))?;
    std::fs::rename(&tmp_path, path)
        .map_err(|err| format!("rename failed ({}): {err}", path.display()))?;
    Ok(())
}

fn read_json_value(path: &Path) -> Result<Value, String> {
    let bytes =
        std::fs::read(path).map_err(|err| format!("read failed ({}): {err}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|err| format!("json parse failed ({}): {err}", path.display()))
}

fn run_base_dir(src_dir: &Path, run_id: &str) -> Result<PathBuf, String> {
    let run_id = require_safe_id("run_id", run_id)?;
    let scene_dir = src_dir
        .parent()
        .ok_or_else(|| format!("invalid src dir (no parent): {}", src_dir.display()))?;
    Ok(scene_dir.join("runs").join(run_id))
}

fn step_dir(run_base: &Path, step: u32) -> PathBuf {
    run_base.join("steps").join(format!("{:04}", step.max(1)))
}

fn current_scene_id(sources: &SceneSourcesV1) -> Result<String, String> {
    sources
        .meta_json
        .get("scene_id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .ok_or_else(|| "meta.json missing scene_id".to_string())
}

fn ensure_run_manifest(run_base: &Path, run_id: &str, scene_id: &str) -> Result<(), String> {
    let manifest_path = run_base.join("run.json");
    if manifest_path.exists() {
        let doc = read_json_value(&manifest_path)?;
        let got_run_id = doc
            .get("run_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        let got_scene_id = doc
            .get("scene_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if got_run_id != run_id || got_scene_id != scene_id {
            return Err(format!(
                "run.json mismatch: expected run_id={run_id} scene_id={scene_id}, got run_id={got_run_id} scene_id={got_scene_id}"
            ));
        }
        return Ok(());
    }

    let doc = serde_json::json!({
        "format_version": SCENE_RUN_FORMAT_VERSION,
        "run_id": run_id,
        "scene_id": scene_id,
    });
    write_json_value_atomic(&manifest_path, &doc)?;
    Ok(())
}

fn last_complete_step(run_base: &Path) -> Result<u32, String> {
    let steps_dir = run_base.join("steps");
    if !steps_dir.exists() {
        return Ok(0);
    }
    let mut max_step = 0u32;
    let entries = std::fs::read_dir(&steps_dir)
        .map_err(|err| format!("read_dir failed ({}): {err}", steps_dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("read_dir entry failed: {err}"))?;
        let ty = entry
            .file_type()
            .map_err(|err| format!("stat failed ({}): {err}", entry.path().display()))?;
        if !ty.is_dir() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(|s| s.to_string()) else {
            continue;
        };
        let Ok(step) = name.parse::<u32>() else {
            continue;
        };
        let complete_path = entry.path().join("complete.json");
        if complete_path.exists() {
            max_step = max_step.max(step);
        }
    }
    Ok(max_step)
}

pub(crate) fn scene_run_status(
    workspace: &SceneSourcesWorkspace,
    run_id: &str,
) -> Result<SceneRunStatusV1, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory has been imported in this session.".to_string());
    };
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };
    let run_id = require_safe_id("run_id", run_id)?;
    let scene_id = current_scene_id(sources)?;

    let run_base = run_base_dir(src_dir, &run_id)?;
    ensure_run_manifest(&run_base, &run_id, &scene_id)?;

    let last = last_complete_step(&run_base)?;
    Ok(SceneRunStatusV1 {
        format_version: SCENE_RUN_FORMAT_VERSION,
        run_id,
        last_complete_step: last,
        next_step: last.saturating_add(1).max(1),
    })
}

pub(crate) fn scene_run_apply_patch_step(
    commands: &mut Commands,
    workspace: &mut SceneSourcesWorkspace,
    library: &crate::object::registry::ObjectLibrary,
    existing_instances: impl Iterator<Item = crate::scene_sources_runtime::SceneWorldInstance>,
    run_id: &str,
    step: u32,
    scorecard: &ScorecardSpecV1,
    patch: &SceneSourcesPatchV1,
) -> Result<SceneRunStepResponseV1, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory has been imported in this session.".to_string());
    };
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources have been imported in this session.".to_string());
    };

    let run_id = require_safe_id("run_id", run_id)?;
    let step = step.max(1);
    let scene_id = current_scene_id(sources)?;

    let run_base = run_base_dir(src_dir, &run_id)?;
    ensure_run_manifest(&run_base, &run_id, &scene_id)?;

    let last = last_complete_step(&run_base)?;
    let step_dir = step_dir(&run_base, step);
    let complete_path = step_dir.join("complete.json");

    if complete_path.exists() {
        let apply_path = step_dir.join("apply_result.json");
        let result = read_json_value(&apply_path)?;
        return Ok(SceneRunStepResponseV1 {
            format_version: SCENE_RUN_FORMAT_VERSION,
            run_id,
            step,
            mode: "replayed".to_string(),
            result,
        });
    }

    if step != last.saturating_add(1).max(1) {
        return Err(format!(
            "Step out of order: requested step {step}, but last_complete_step is {last}"
        ));
    }

    std::fs::create_dir_all(&step_dir)
        .map_err(|err| format!("create_dir_all failed ({}): {err}", step_dir.display()))?;

    let scorecard_doc = serde_json::to_value(scorecard)
        .map_err(|err| format!("scorecard to_value failed: {err}"))?;
    let patch_doc =
        serde_json::to_value(patch).map_err(|err| format!("patch to_value failed: {err}"))?;
    write_json_value_atomic(&step_dir.join("scorecard.json"), &scorecard_doc)?;
    write_json_value_atomic(&step_dir.join("patch.json"), &patch_doc)?;

    let pre: SceneSourcesPatchValidateReport =
        validate_scene_sources_patch(workspace, library, scorecard, patch)?;
    let pre_doc =
        serde_json::to_value(&pre).map_err(|err| format!("pre to_value failed: {err}"))?;
    write_json_value_atomic(&step_dir.join("pre_validation_report.json"), &pre_doc)?;

    let apply: SceneSourcesPatchApplyReport = apply_scene_sources_patch(
        commands,
        workspace,
        library,
        existing_instances,
        scorecard,
        patch,
    )?;
    let apply_doc =
        serde_json::to_value(&apply).map_err(|err| format!("apply to_value failed: {err}"))?;
    write_json_value_atomic(&step_dir.join("apply_result.json"), &apply_doc)?;

    if apply.applied {
        let Some(patched_sources) = workspace.sources.as_ref() else {
            return Err("missing workspace.sources after apply".to_string());
        };
        let sig = scene_signature_summary_from_sources(patched_sources)?;
        let sig_doc =
            serde_json::to_value(sig).map_err(|err| format!("signature to_value failed: {err}"))?;
        write_json_value_atomic(&step_dir.join("post_signature.json"), &sig_doc)?;
    }

    let status = if apply.applied { "applied" } else { "rejected" };
    let complete_doc = serde_json::json!({
        "format_version": SCENE_RUN_FORMAT_VERSION,
        "status": status,
    });
    write_json_value_atomic(&complete_path, &complete_doc)?;

    Ok(SceneRunStepResponseV1 {
        format_version: SCENE_RUN_FORMAT_VERSION,
        run_id,
        step,
        mode: "executed".to_string(),
        result: apply_doc,
    })
}
