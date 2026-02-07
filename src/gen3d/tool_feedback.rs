use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::config::AppConfig;

#[derive(Clone, Debug, Default, Resource)]
pub(crate) struct Gen3dToolFeedbackHistory {
    pub(crate) entries: Vec<Gen3dToolFeedbackEntry>,
}

impl Gen3dToolFeedbackHistory {
    pub(crate) fn last_run_id(&self) -> Option<&str> {
        self.entries.last().and_then(|e| e.run_id.as_deref())
    }

    pub(crate) fn entries_for_run<'a>(
        &'a self,
        run_id: &'a str,
    ) -> impl Iterator<Item = &'a Gen3dToolFeedbackEntry> + 'a {
        self.entries
            .iter()
            .filter(move |e| e.run_id.as_deref() == Some(run_id))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Gen3dToolFeedbackEntry {
    pub(crate) version: u32,
    pub(crate) entry_id: String,
    pub(crate) created_at_ms: u64,
    pub(crate) run_id: Option<String>,
    pub(crate) attempt: Option<u32>,
    pub(crate) pass: Option<u32>,
    pub(crate) priority: String,
    pub(crate) title: String,
    pub(crate) summary: String,
    #[serde(default)]
    pub(crate) feedback: serde_json::Value,
    #[serde(default)]
    pub(crate) evidence_paths: Vec<String>,
}

pub(crate) fn gen3d_tool_feedback_history_path(config: &AppConfig) -> PathBuf {
    gen3d_cache_base_dir(config).join("tool_feedback_history.jsonl")
}

pub(crate) fn gen3d_cache_base_dir(config: &AppConfig) -> PathBuf {
    config
        .gen3d_cache_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(default_gen3d_cache_dir)
}

fn default_gen3d_cache_dir() -> PathBuf {
    crate::paths::default_gen3d_cache_dir()
}

pub(crate) fn gen3d_load_tool_feedback_history(mut commands: Commands, config: Res<AppConfig>) {
    let history = read_gen3d_tool_feedback_history(&config);
    commands.insert_resource(history);
}

fn read_gen3d_tool_feedback_history(config: &AppConfig) -> Gen3dToolFeedbackHistory {
    let path = gen3d_tool_feedback_history_path(config);
    let data = match std::fs::read_to_string(&path) {
        Ok(data) => data,
        Err(err) => {
            debug!(
                "Gen3D: tool feedback history not loaded ({}): {err}",
                path.display()
            );
            return Gen3dToolFeedbackHistory::default();
        }
    };

    let mut history = Gen3dToolFeedbackHistory::default();
    for (idx, line) in data.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Gen3dToolFeedbackEntry>(line) {
            Ok(entry) => history.entries.push(entry),
            Err(err) => {
                warn!(
                    "Gen3D: skipping invalid tool feedback history line {} ({}): {err}",
                    idx + 1,
                    path.display()
                );
            }
        }
    }

    history
}

pub(crate) fn append_gen3d_tool_feedback_entry(
    config: &AppConfig,
    run_dir: Option<&Path>,
    entry: &Gen3dToolFeedbackEntry,
) {
    let json = match serde_json::to_string(entry) {
        Ok(json) => json,
        Err(err) => {
            warn!("Gen3D: failed to serialize tool feedback entry: {err}");
            return;
        }
    };
    let mut line = json;
    line.push('\n');

    let history_path = gen3d_tool_feedback_history_path(config);
    if let Some(parent) = history_path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            warn!(
                "Gen3D: failed to create tool feedback history dir {}: {err}",
                parent.display()
            );
        }
    }
    if let Err(err) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&history_path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()))
    {
        warn!(
            "Gen3D: failed to append tool feedback history {}: {err}",
            history_path.display()
        );
    }

    if let Some(run_dir) = run_dir {
        let run_path = run_dir.join("tool_feedback.jsonl");
        if let Err(err) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&run_path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()))
        {
            warn!(
                "Gen3D: failed to append per-run tool feedback {}: {err}",
                run_path.display()
            );
        }
    }
}
