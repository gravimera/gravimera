use bevy::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::config::AppConfig;
use crate::object::registry::ObjectLibrary;
use crate::realm::ActiveRealmScene;
use crate::scene_authoring_ui::SceneAuthoringUiState;
use crate::scene_sources::SceneSourcesV1;
use crate::scene_sources_patch::{
    SceneSourcesPatchOpV1, SceneSourcesPatchV1, SCENE_SOURCES_PATCH_FORMAT_VERSION,
};
use crate::scene_sources_runtime::{SceneSourcesWorkspace, SceneWorldInstance};
use crate::scene_validation::{HardGateSpecV1, ScorecardSpecV1};
use crate::types::{
    BuildObject, Commandable, ObjectId, ObjectPrefabId, ObjectTint, Player, SceneLayerOwner,
};

const CURL_CONNECT_TIMEOUT_SECS: u32 = 15;
const CURL_MAX_TIME_SECS: u32 = 600;

#[derive(Resource, Default)]
pub(crate) struct SceneBuildAiRuntime {
    in_flight: Option<SceneBuildAiJob>,
}

struct SceneBuildAiJob {
    run_id: String,
    target_realm_id: String,
    target_scene_id: String,
    run_dir: PathBuf,
    shared_result: Arc<Mutex<Option<Result<String, String>>>>,
}

pub(crate) fn start_scene_build_from_description(
    runtime: &mut SceneBuildAiRuntime,
    config: &AppConfig,
    active: &ActiveRealmScene,
    library: &ObjectLibrary,
    description: &str,
) -> Result<String, String> {
    if runtime.in_flight.is_some() {
        return Err("A build is already running.".to_string());
    }

    let description = description.trim();
    if description.is_empty() {
        return Err("Scene description is empty.".to_string());
    }

    let openai = config
        .openai
        .as_ref()
        .ok_or_else(|| "OpenAI is not configured (missing [openai] in config.toml).".to_string())?;

    let run_id = format!("scene_build_{}", uuid::Uuid::new_v4());
    let scene_dir = crate::paths::scene_dir(&active.realm_id, &active.scene_id);
    let run_dir = scene_dir.join("runs").join(&run_id);
    let llm_dir = run_dir.join("llm");
    std::fs::create_dir_all(&llm_dir)
        .map_err(|err| format!("Failed to create {}: {err}", llm_dir.display()))?;

    let system_text = build_system_prompt();
    let user_text = build_user_prompt(active, library, description);

    write_text_artifact(&llm_dir.join("system.txt"), &system_text)?;
    write_text_artifact(&llm_dir.join("user.txt"), &user_text)?;

    let base_url = openai.base_url.clone();
    let api_key = openai.api_key.clone();
    let model = openai.model.clone();
    let reasoning_effort = openai.model_reasoning_effort.clone();

    let shared_result: Arc<Mutex<Option<Result<String, String>>>> = Arc::new(Mutex::new(None));
    let shared_result_thread = shared_result.clone();
    let llm_dir_thread = llm_dir.clone();

    std::thread::Builder::new()
        .name(format!("gravimera_scene_build_ai_{run_id}"))
        .spawn(move || {
            let res = call_openai_chat_json_object(
                &base_url,
                &api_key,
                &model,
                &reasoning_effort,
                &system_text,
                &user_text,
                &llm_dir_thread,
            );
            if let Ok(mut guard) = shared_result_thread.lock() {
                *guard = Some(res);
            }
        })
        .map_err(|err| format!("Failed to spawn build thread: {err}"))?;

    runtime.in_flight = Some(SceneBuildAiJob {
        run_id: run_id.clone(),
        target_realm_id: active.realm_id.clone(),
        target_scene_id: active.scene_id.clone(),
        run_dir,
        shared_result,
    });

    Ok(run_id)
}

pub(crate) fn scene_build_ai_poll(
    mut commands: Commands,
    mut runtime: ResMut<SceneBuildAiRuntime>,
    mut ui: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    library: Res<ObjectLibrary>,
    mut workspace: ResMut<SceneSourcesWorkspace>,
    scene_instances: Query<
        (
            Entity,
            &Transform,
            &ObjectId,
            &ObjectPrefabId,
            Option<&ObjectTint>,
            Option<&SceneLayerOwner>,
        ),
        (Without<Player>, Or<(With<BuildObject>, With<Commandable>)>),
    >,
) {
    let Some(job) = runtime.in_flight.as_ref() else {
        return;
    };

    let result = {
        let Ok(mut guard) = job.shared_result.lock() else {
            return;
        };
        guard.take()
    };
    let Some(result) = result else {
        return;
    };

    let job = runtime.in_flight.take().expect("checked job exists above");

    if active.realm_id != job.target_realm_id || active.scene_id != job.target_scene_id {
        ui.set_error(format!(
            "Build finished for {}/{} but active scene is now {}/{}; ignoring result (run_id={}).",
            job.target_realm_id, job.target_scene_id, active.realm_id, active.scene_id, job.run_id
        ));
        return;
    }

    ui.clear_error();
    match result {
        Err(err) => {
            ui.set_error(err);
        }
        Ok(text) => {
            let llm_dir = job.run_dir.join("llm");
            let _ = std::fs::create_dir_all(&llm_dir);
            let _ = write_text_artifact(&llm_dir.join("response.txt"), &text);

            let src_dir = crate::realm::scene_src_dir(&active);
            if workspace.loaded_from_dir.as_deref() != Some(src_dir.as_path()) {
                workspace.loaded_from_dir = Some(src_dir.clone());
                workspace.sources = None;
            }

            let scorecard = default_scorecard();
            let patch = match build_patch_from_llm_layers(&src_dir, &job.run_id, &text) {
                Ok(p) => p,
                Err(err) => {
                    ui.set_error(err);
                    return;
                }
            };

            let existing = scene_instances
                .iter()
                .map(|(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                    entity: e,
                    instance_id: *id,
                    prefab_id: *prefab,
                    transform: t.clone(),
                    tint: tint.map(|t| t.0),
                    owner_layer_id: owner.map(|o| o.layer_id.clone()),
                });

            let response = crate::scene_runs::scene_run_apply_patch_step(
                &mut commands,
                &mut workspace,
                &library,
                existing,
                &job.run_id,
                1,
                &scorecard,
                &patch,
            );

            match response {
                Ok(resp) => {
                    let applied = resp
                        .result
                        .get("applied")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    let compile = resp.result.get("compile_report");
                    let spawned = compile
                        .and_then(|c| c.get("spawned"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let updated = compile
                        .and_then(|c| c.get("updated"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    let despawned = compile
                        .and_then(|c| c.get("despawned"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);

                    if applied {
                        ui.set_status(format!(
                            "Built scene (run_id={}): spawned={} updated={} despawned={}.",
                            job.run_id, spawned, updated, despawned
                        ));
                    } else {
                        ui.set_error(format!(
                            "Build rejected by validators (run_id={}).",
                            job.run_id
                        ));
                    }
                }
                Err(err) => {
                    ui.set_error(err);
                }
            }
        }
    }
}

fn default_scorecard() -> ScorecardSpecV1 {
    ScorecardSpecV1 {
        format_version: crate::scene_validation::SCORECARD_FORMAT_VERSION,
        scope: Default::default(),
        hard_gates: vec![
            HardGateSpecV1::Schema {},
            HardGateSpecV1::Budget {
                max_instances: Some(200_000),
                max_portals: Some(10_000),
            },
        ],
        soft_metrics: Vec::new(),
        weights: Default::default(),
    }
}

fn build_patch_from_llm_layers(
    src_dir: &Path,
    request_id: &str,
    raw_text: &str,
) -> Result<SceneSourcesPatchV1, String> {
    let doc = parse_layers_envelope(raw_text)?;
    let layers_val = doc
        .get("layers")
        .ok_or_else(|| "LLM output missing `layers` field".to_string())?;
    let layers = layers_val
        .as_array()
        .ok_or_else(|| "`layers` must be an array".to_string())?;

    let mut desired_layer_docs: Vec<(String, Value)> = Vec::new();
    for (idx, layer_val) in layers.iter().enumerate() {
        let mut layer_doc = layer_val.clone();
        let layer_id = layer_doc
            .get("layer_id")
            .and_then(|v| v.as_str())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("layers[{idx}] missing layer_id"))?
            .to_string();

        if !layer_id.starts_with("ai_") {
            return Err(format!(
                "layers[{idx}].layer_id must start with \"ai_\" (got {layer_id})"
            ));
        }

        layer_doc["format_version"] =
            Value::from(crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION);
        layer_doc["layer_id"] = Value::from(layer_id.clone());

        let kind = layer_doc
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim();
        if !matches!(
            kind,
            "explicit_instances" | "grid_instances" | "polyline_instances"
        ) {
            return Err(format!(
                "layers[{idx}] has unsupported kind {kind:?} (expected explicit_instances|grid_instances|polyline_instances)"
            ));
        }

        desired_layer_docs.push((layer_id, layer_doc));
    }

    let mut desired_layer_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (layer_id, _doc) in &desired_layer_docs {
        if !desired_layer_ids.insert(layer_id.clone()) {
            return Err(format!("Duplicate layer_id in LLM output: {layer_id}"));
        }
    }

    let sources = SceneSourcesV1::load_from_dir(src_dir).map_err(|err| err.to_string())?;
    let existing_ai_layer_ids = existing_layer_ids_with_prefix(&sources, "ai_")?;

    let mut ops: Vec<SceneSourcesPatchOpV1> = Vec::new();
    for layer_id in existing_ai_layer_ids {
        if desired_layer_ids.contains(&layer_id) {
            continue;
        }
        ops.push(SceneSourcesPatchOpV1::DeleteLayer { layer_id });
    }

    for (layer_id, doc) in desired_layer_docs {
        ops.push(SceneSourcesPatchOpV1::UpsertLayer { layer_id, doc });
    }

    Ok(SceneSourcesPatchV1 {
        format_version: SCENE_SOURCES_PATCH_FORMAT_VERSION,
        request_id: request_id.to_string(),
        ops,
    })
}

fn existing_layer_ids_with_prefix(
    sources: &SceneSourcesV1,
    prefix: &str,
) -> Result<Vec<String>, String> {
    let index_paths =
        crate::scene_sources::SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
            .map_err(|err| format!("Invalid scene sources index.json: {err}"))?;
    let layers_dir = index_paths.layers_dir;

    let mut out = Vec::new();
    for (rel_path, doc) in &sources.extra_json_files {
        if !is_under_dir(rel_path, &layers_dir) {
            continue;
        }
        let Some(layer_id) = doc.get("layer_id").and_then(|v| v.as_str()) else {
            continue;
        };
        let layer_id = layer_id.trim();
        if layer_id.starts_with(prefix) {
            out.push(layer_id.to_string());
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

fn is_under_dir(path: &Path, dir: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(dir) else {
        return false;
    };
    !rel.as_os_str().is_empty()
}

fn parse_layers_envelope(raw_text: &str) -> Result<Value, String> {
    let trimmed = raw_text.trim();
    if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
        return Ok(v);
    }
    if let Some(extracted) = extract_json_object(trimmed) {
        if let Ok(v) = serde_json::from_str::<Value>(&extracted) {
            return Ok(v);
        }
    }
    Err("Failed to parse LLM output as JSON.".to_string())
}

fn extract_json_object(text: &str) -> Option<String> {
    let mut depth = 0i32;
    let mut start: Option<usize> = None;
    let mut last: Option<(usize, usize)> = None;

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(idx);
                }
                depth = depth.saturating_add(1);
            }
            '}' => {
                if depth > 0 {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        if let Some(s) = start.take() {
                            last = Some((s, idx + 1));
                        }
                    }
                }
            }
            _ => {}
        }
    }

    last.map(|(s, e)| text[s..e].to_string())
}

fn build_system_prompt() -> String {
    "You are a scene generation assistant for the game Gravimera.\n\
Return ONLY valid JSON.\n\
\n\
Output schema:\n\
{\n\
  \"layers\": [ <layer_doc>, ... ]\n\
}\n\
\n\
Rules:\n\
- Every layer_id MUST start with \"ai_\".\n\
- Supported layer kinds: \"explicit_instances\", \"grid_instances\", \"polyline_instances\".\n\
- Use only prefab_id values from the provided catalog.\n\
- Coordinate system: XZ is ground plane, Y is up. Keep objects near the origin.\n\
- Prefer a small number of layers. Keep instance counts reasonable (hundreds, not tens of thousands).\n\
- For explicit_instances, each instance MUST have unique local_id.\n"
        .to_string()
}

fn build_user_prompt(
    active: &ActiveRealmScene,
    library: &ObjectLibrary,
    description: &str,
) -> String {
    let mut prefabs: Vec<(String, String, &'static str)> = Vec::new();
    for (id, def) in library.iter() {
        let uuid = uuid::Uuid::from_u128(*id).to_string();
        let label = def.label.to_string();
        let kind = if def.mobility.is_some() {
            "unit"
        } else {
            "building"
        };
        prefabs.push((uuid, label, kind));
    }
    prefabs.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));

    let mut catalog = String::new();
    for (uuid, label, kind) in prefabs {
        catalog.push_str(&format!("- {uuid} | {kind} | {label}\n"));
    }

    format!(
        "Target scene: realm_id={}/ scene_id={}\n\n\
Scene description:\n\
{}\n\n\
Prefab catalog (prefab_id | kind | label):\n\
{}\n\n\
Now output JSON with `layers`.\n\
Hint: If you need bespoke placement, use one explicit_instances layer (ai_main).\n",
        active.realm_id, active.scene_id, description, catalog
    )
}

struct TempSecretFile {
    path: PathBuf,
}

impl TempSecretFile {
    fn create(prefix: &str, contents: &str) -> std::io::Result<Self> {
        use std::io::Write;

        let mut path = std::env::temp_dir();
        let pid = std::process::id();
        let nonce = uuid::Uuid::new_v4();
        path.push(format!("gravimera_{prefix}_{pid}_{nonce}.txt"));

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(contents.as_bytes())?;
        file.flush()?;

        Ok(Self { path })
    }

    fn curl_header_arg(&self) -> String {
        format!("@{}", self.path.display())
    }
}

impl Drop for TempSecretFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn curl_auth_header_file(api_key: &str) -> Result<TempSecretFile, String> {
    let api_key = api_key.replace(['\n', '\r'], "");
    let headers = format!("Authorization: Bearer {api_key}\n");
    TempSecretFile::create("openai_auth", &headers).map_err(|err| err.to_string())
}

fn split_curl_http_status<'a>(stdout: &'a str, marker: &str) -> (&'a str, Option<u16>) {
    let Some(pos) = stdout.rfind(marker) else {
        return (stdout, None);
    };
    let (body, rest) = stdout.split_at(pos);
    let code_str = rest[marker.len()..].lines().next().unwrap_or("").trim();
    (body, code_str.parse::<u16>().ok())
}

fn call_openai_chat_json_object(
    base_url: &str,
    api_key: &str,
    model: &str,
    reasoning_effort: &str,
    system_instructions: &str,
    user_text: &str,
    llm_dir: &Path,
) -> Result<String, String> {
    let url = crate::config::join_base_url(base_url, "chat/completions");
    let mut body_json = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [
            { "role": "system", "content": system_instructions },
            { "role": "user", "content": user_text }
        ],
        "response_format": { "type": "json_object" }
    });
    if reasoning_effort.trim() != "none" && !reasoning_effort.trim().is_empty() {
        body_json["reasoning_effort"] = Value::from(reasoning_effort.trim());
    }

    let _ = write_json_artifact(&llm_dir.join("request.json"), &body_json);
    let body = serde_json::to_vec(&body_json).map_err(|err| err.to_string())?;

    let auth_headers = curl_auth_header_file(api_key)?;
    let mut cmd = std::process::Command::new("curl");
    cmd.arg("-sS")
        .arg("--connect-timeout")
        .arg(CURL_CONNECT_TIMEOUT_SECS.to_string())
        .arg("--max-time")
        .arg(CURL_MAX_TIME_SECS.to_string())
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("-H")
        .arg(auth_headers.curl_header_arg())
        .arg("-d")
        .arg("@-")
        .arg(&url)
        .arg("-w")
        .arg("\n__GRAVIMERA_HTTP_STATUS__%{http_code}\n")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to start curl: {err}"))?;

    if let Some(stdin) = child.stdin.as_mut() {
        use std::io::Write;
        stdin
            .write_all(&body)
            .map_err(|err| format!("Failed to write request to curl stdin: {err}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|err| format!("Failed to wait for curl: {err}"))?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if !output.status.success() {
        return Err(format!("curl exited with non-zero status:\n{stderr}"));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let _ = std::fs::write(llm_dir.join("api_response_raw.txt"), &stdout);

    const STATUS_MARKER: &str = "\n__GRAVIMERA_HTTP_STATUS__";
    let (body, status_code) = split_curl_http_status(&stdout, STATUS_MARKER);
    let status_code =
        status_code.ok_or_else(|| "Missing HTTP status marker in curl output.".to_string())?;

    if !(200..=299).contains(&status_code) {
        return Err(format!(
            "OpenAI request failed (HTTP {status_code}). Body (truncated): {}",
            truncate_text(body.trim(), 1200)
        ));
    }

    let json: Value = serde_json::from_str(body.trim()).map_err(|err| {
        format!(
            "Failed to parse OpenAI response JSON: {err}. Body (truncated): {}",
            truncate_text(body.trim(), 1200)
        )
    })?;
    let _ = write_json_artifact(&llm_dir.join("api_response.json"), &json);

    let text = json
        .get("choices")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| "OpenAI response missing choices[0].message.content".to_string())?
        .to_string();

    Ok(text)
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_chars + 16);
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push('…');
    out
}

fn write_text_artifact(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    std::fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}

fn write_json_artifact(path: &Path, json: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("Failed to create {}: {err}", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(json)
        .map_err(|err| format!("json serialize failed: {err}"))?;
    std::fs::write(path, format!("{text}\n"))
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))
}
