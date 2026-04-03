use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::AppConfig;
use crate::floor_library_ui::{FloorLibraryUiState, DEFAULT_FLOOR_ID};
use crate::genfloor::{genfloor_cancel_ai_job, genfloor_start_ai_job, set_active_world_floor, ActiveWorldFloor, GenFloorAiJob, GenFloorWorkshop};
use crate::gen3d::{
    gen3d_cancel_build_from_api, gen3d_generate_text_simple_with_prefix, Gen3dAiJob, Gen3dDraft,
    Gen3dSessionKind, Gen3dSessionState, Gen3dTaskQueue, Gen3dTaskState, Gen3dWorkshop,
};
use crate::object::registry::ObjectLibrary;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::realm::{ActiveRealmScene, PendingRealmSceneSwitch};
use crate::scene_floor_selection;
use crate::scene_runs::scene_run_apply_patch_step;
use crate::scene_sources_patch::{
    SceneSourcesPatchOpV1, SceneSourcesPatchV1, SCENE_SOURCES_PATCH_FORMAT_VERSION,
};
use crate::scene_sources_runtime::{
    reload_scene_sources_in_workspace, SceneSourcesWorkspace, SceneWorldInstance,
};
use crate::scene_validation::{HardGateSpecV1, ScorecardSpecV1};
use crate::threaded_result::{new_shared_result, take_shared_result, SharedResult};
use crate::types::{GameMode, ObjectId, ObjectPrefabId, ObjectTint, SceneLayerOwner};
use crate::workspace_scenes_ui::ScenesPanelUiState;

use super::state::*;

const GEN_SCENE_SYSTEM_PROMPT: &str = "You are a scene planner for a game editor.\n\
Return ONLY a single JSON object. No markdown, no commentary.\n\
The JSON must follow this schema (all fields required unless marked optional):\n\
{\n\
  version: 1,\n\
  terrain: {\n\
    existing_floor_id: string?,\n\
    genfloor_prompt: string?\n\
  },\n\
  assets: [\n\
    { key: string, existing_prefab_id: string?, gen3d_prompt: string? }\n\
  ],\n\
  placements: [\n\
    { asset_key: string, x: f32, z: f32, yaw_deg: f32, scale: f32?, count: u32? }\n\
  ]\n\
}\n\
Rules:\n\
- Prefer existing assets and existing terrain from the catalog when they match the prompt.\n\
- When using existing ids, you MUST use ids that appear in the provided catalog.\n\
- Never invent ids that are not present in the catalog.\n\
- For terrain, set exactly one of existing_floor_id or genfloor_prompt.\n\
- For assets, set exactly one of existing_prefab_id or gen3d_prompt.\n\
- Use stable asset keys (e.g. 'tree_main', 'house_1') and reuse them in placements.\n\
- Output placements with meters in the scene X/Z plane; yaw_deg is degrees around Y.\n\
- If you need multiple copies at distinct positions, output multiple placement entries.\n\
- The output MUST be valid JSON (not JSON5).\n\
";

#[derive(Clone, Debug, Serialize)]
pub(crate) struct GenSceneAutomationStatus {
    pub(crate) running: bool,
    pub(crate) run_id: Option<String>,
    pub(crate) phase: String,
    pub(crate) message: Option<String>,
    pub(crate) scene_id: Option<String>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenSceneCatalogSnapshot {
    floors: Vec<GenSceneCatalogFloor>,
    prefabs: Vec<GenSceneCatalogPrefab>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenSceneCatalogFloor {
    id: String,
    label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct GenSceneCatalogPrefab {
    id: String,
    label: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    short: Option<String>,
}

pub(crate) fn gen_scene_status(
    workshop: &GenSceneWorkshop,
    job: &GenSceneJob,
) -> GenSceneAutomationStatus {
    GenSceneAutomationStatus {
        running: job.running,
        run_id: job.run_id.clone(),
        phase: phase_label(&job.phase).to_string(),
        message: (!workshop.status.trim().is_empty()).then(|| workshop.status.clone()),
        scene_id: job.target_scene_id.clone(),
        error: workshop.error.clone(),
    }
}

pub(crate) fn gen_scene_set_prompt_from_api(workshop: &mut GenSceneWorkshop, prompt: &str) {
    workshop.prompt = prompt.trim().to_string();
    workshop.prompt_focused = false;
}

pub(crate) fn gen_scene_request_build(
    config: &AppConfig,
    mode: Option<&State<GameMode>>,
    active: &ActiveRealmScene,
    workshop: &mut GenSceneWorkshop,
    job: &mut GenSceneJob,
    scenes_state: &mut ScenesPanelUiState,
    pending_switch: &mut PendingRealmSceneSwitch,
    saves: &mut MessageWriter<crate::scene_store::SceneSaveRequest>,
) -> Result<(), String> {
    if let Some(mode) = mode {
        if !matches!(mode.get(), GameMode::Build) {
            return Err("GenScene build is only available in Build mode.".to_string());
        }
    }

    if job.running {
        return Err("GenScene build already running (stop it first).".to_string());
    }

    let prompt = workshop.prompt.trim();
    if prompt.is_empty() {
        return Err("Prompt is empty.".to_string());
    }
    crate::gen3d::validate_gen3d_user_prompt_limits(prompt)?;

    let scene_id = allocate_scene_id(config, &active.realm_id, prompt)?;
    crate::realm::ensure_realm_scene_scaffold(&active.realm_id, &scene_id)?;
    crate::scene_store::ensure_default_scene_dat_exists(&active.realm_id, &scene_id)?;

    scenes_state.scenes_dirty = true;

    pending_switch.target = Some(ActiveRealmScene {
        realm_id: active.realm_id.clone(),
        scene_id: scene_id.clone(),
    });
    saves.write(crate::scene_store::SceneSaveRequest::new("gen scene build"));

    let run_id = format!("gen_scene_{}", uuid::Uuid::new_v4());
    let run_dir = gen_scene_run_dir(&active.realm_id, &scene_id, &run_id)?;

    job.phase = GenScenePhase::Planning;
    job.running = true;
    job.cancel_requested = false;
    job.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    job.run_id = Some(run_id);
    job.run_dir = Some(run_dir);
    job.target_scene_id = Some(scene_id.clone());
    job.plan = None;
    job.plan_shared = None;
    job.resolved_prefabs.clear();
    job.model_tasks.clear();
    job.floor_choice = None;
    job.floor_generation_started = false;
    job.floor_generation_prev_id = None;
    job.placements.clear();
    job.next_run_step = 1;

    workshop.status = "Planning scene…".to_string();
    workshop.error = None;
    workshop.running = true;
    workshop.close_locked = true;
    workshop.run_id = job.run_id.clone();
    workshop.active_scene_id = Some(scene_id);
    workshop.side_panel_open = true;

    Ok(())
}

pub(crate) fn gen_scene_cancel_job(
    job: &mut GenSceneJob,
    workshop: &mut GenSceneWorkshop,
    gen3d_queue: &mut Gen3dTaskQueue,
    gen3d_workshop: &mut Gen3dWorkshop,
    gen3d_job: &mut Gen3dAiJob,
    gen3d_draft: &mut Gen3dDraft,
    genfloor_job: &mut GenFloorAiJob,
) {
    if !job.running {
        return;
    }
    job.cancel_requested = true;
    if let Some(flag) = job.cancel_flag.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }

    if genfloor_job.running {
        genfloor_cancel_ai_job(genfloor_job, gen3d_workshop);
    }

    cancel_gen3d_tasks(job, gen3d_queue, gen3d_workshop, gen3d_job, gen3d_draft);

    workshop.status = "Stop requested…".to_string();
}

#[derive(SystemParam)]
pub(crate) struct GenScenePollJobDeps<'w, 's> {
    commands: Commands<'w, 's>,
    config: Res<'w, AppConfig>,
    active: Res<'w, ActiveRealmScene>,
    job: ResMut<'w, GenSceneJob>,
    workshop: ResMut<'w, GenSceneWorkshop>,
    preview: ResMut<'w, GenScenePreview>,
    scene_workspace: ResMut<'w, SceneSourcesWorkspace>,
    library: ResMut<'w, ObjectLibrary>,
    prefab_descriptors: ResMut<'w, PrefabDescriptorLibrary>,
    floor_job: ResMut<'w, GenFloorAiJob>,
    floor_workshop: ResMut<'w, GenFloorWorkshop>,
    active_floor: ResMut<'w, ActiveWorldFloor>,
    floor_library: ResMut<'w, FloorLibraryUiState>,
    gen3d_queue: ResMut<'w, Gen3dTaskQueue>,
    gen3d_workshop: ResMut<'w, Gen3dWorkshop>,
    gen3d_job: Res<'w, Gen3dAiJob>,
    saves: MessageWriter<'w, crate::scene_store::SceneSaveRequest>,
    scene_instances: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static ObjectId,
            &'static ObjectPrefabId,
            Option<&'static ObjectTint>,
            Option<&'static SceneLayerOwner>,
        ),
        Without<crate::types::Player>,
    >,
}

pub(crate) fn gen_scene_poll_job(
    deps: GenScenePollJobDeps,
) {
    let GenScenePollJobDeps {
        mut commands,
        config,
        active,
        mut job,
        mut workshop,
        mut preview,
        mut scene_workspace,
        mut library,
        mut prefab_descriptors,
        mut floor_job,
        mut floor_workshop,
        mut active_floor,
        mut floor_library,
        mut gen3d_queue,
        mut gen3d_workshop,
        gen3d_job,
        mut saves,
        scene_instances,
    } = deps;
    if !job.running {
        return;
    }

    if !preview.active {
        preview.active = true;
    }

    if job.cancel_requested {
        finish_canceled(&mut job, &mut workshop);
        return;
    }

    match job.phase {
        GenScenePhase::Planning => {
            if job.plan_shared.is_none() {
                let catalog = match build_catalog_snapshot(
                    &active,
                    &mut library,
                    &mut prefab_descriptors,
                ) {
                    Ok(snapshot) => snapshot,
                    Err(err) => {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                };

                let catalog_json = match serde_json::to_string_pretty(&catalog) {
                    Ok(text) => text,
                    Err(err) => {
                        finish_with_error(&mut job, &mut workshop, err.to_string());
                        return;
                    }
                };

                let prompt = workshop.prompt.trim().to_string();
                let shared: SharedResult<GenScenePlanV1, String> = new_shared_result();
                job.plan_shared = Some(shared.clone());

                let cancel_flag = job.cancel_flag.clone();
                let config = config.clone();
                let thread_name = format!("gravimera_gen_scene_plan_{}", uuid::Uuid::new_v4());

                let _ = crate::threaded_result::spawn_worker_thread(
                    thread_name,
                    shared,
                    move || call_gen_scene_plan(&config, &prompt, &catalog_json, cancel_flag),
                    |_| {},
                );
            }

            let Some(shared) = job.plan_shared.as_ref() else {
                return;
            };
            let Some(result) = take_shared_result(shared) else {
                return;
            };
            job.plan_shared = None;

            match result {
                Ok(plan) => {
                    if let Err(err) = write_plan_artifact(&job, &plan) {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                    job.placements = plan.placements.clone();
                    job.plan = Some(plan);
                    job.phase = GenScenePhase::AwaitSceneSwitch;
                    workshop.status = "Waiting for scene switch…".to_string();
                }
                Err(err) => {
                    finish_with_error(&mut job, &mut workshop, err);
                }
            }
        }
        GenScenePhase::AwaitSceneSwitch => {
            let target = match job.target_scene_id.as_ref() {
                Some(id) => id,
                None => {
                    finish_with_error(&mut job, &mut workshop, "Missing target scene.".to_string());
                    return;
                }
            };

            if active.scene_id != *target {
                workshop.status = "Waiting for scene switch…".to_string();
                return;
            }

            job.phase = GenScenePhase::GeneratingFloor;
            workshop.status = "Selecting terrain…".to_string();
        }
        GenScenePhase::GeneratingFloor => {
            let Some(plan) = job.plan.clone() else {
                finish_with_error(&mut job, &mut workshop, "Missing plan.".to_string());
                return;
            };

            if active.scene_id != job.target_scene_id.as_deref().unwrap_or("") {
                workshop.status = "Waiting for scene switch…".to_string();
                return;
            }

            let floor_choice = if let Some(choice) = job.floor_choice.clone() {
                choice
            } else {
                match resolve_floor_choice(&plan) {
                    Ok(choice) => {
                        job.floor_choice = Some(choice.clone());
                        choice
                    }
                    Err(err) => {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                }
            };

            match floor_choice {
                GenSceneFloorChoice::Default => {
                    if let Err(err) = apply_floor_choice(
                        &active,
                        DEFAULT_FLOOR_ID,
                        &mut active_floor,
                        &mut floor_library,
                    ) {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                    preview.focus = Vec3::ZERO;
                    preview.half_extents = floor_half_extents(&active_floor);
                    preview.dirty = true;
                    job.phase = GenScenePhase::GeneratingModels;
                    workshop.status = "Selecting models…".to_string();
                }
                GenSceneFloorChoice::Existing(floor_id) => {
                    if let Err(err) = apply_floor_choice(
                        &active,
                        floor_id,
                        &mut active_floor,
                        &mut floor_library,
                    ) {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                    preview.focus = Vec3::ZERO;
                    preview.half_extents = floor_half_extents(&active_floor);
                    preview.dirty = true;
                    job.phase = GenScenePhase::GeneratingModels;
                    workshop.status = "Selecting models…".to_string();
                }
                GenSceneFloorChoice::GeneratedPrompt(prompt) => {
                    if !job.floor_generation_started {
                        if floor_job.running {
                            workshop.status = "Waiting for terrain generator…".to_string();
                            return;
                        }
                        job.floor_generation_prev_id = floor_job.last_saved_floor_id;
                        job.floor_generation_started = true;
                        genfloor_start_ai_job(
                            &config,
                            &prompt,
                            &mut floor_job,
                            &mut gen3d_workshop,
                            &mut floor_workshop,
                        );
                        workshop.status = "Generating terrain…".to_string();
                        return;
                    }

                    if floor_job.running {
                        workshop.status = "Generating terrain…".to_string();
                        return;
                    }

                    if let Some(err) = floor_workshop.error.clone() {
                        finish_with_error(&mut job, &mut workshop, format!("GenFloor failed: {err}"));
                        return;
                    }

                    let new_floor_id = match floor_job.last_saved_floor_id {
                        Some(id) if Some(id) != job.floor_generation_prev_id => id,
                        Some(id) => id,
                        None => {
                            workshop.status = "Waiting for terrain to finish saving…".to_string();
                            return;
                        }
                    };

                    if let Err(err) = apply_floor_choice(
                        &active,
                        new_floor_id,
                        &mut active_floor,
                        &mut floor_library,
                    ) {
                        finish_with_error(&mut job, &mut workshop, err);
                        return;
                    }
                    job.floor_choice = Some(GenSceneFloorChoice::Existing(new_floor_id));
                    preview.focus = Vec3::ZERO;
                    preview.half_extents = floor_half_extents(&active_floor);
                    preview.dirty = true;
                    job.phase = GenScenePhase::GeneratingModels;
                    workshop.status = "Selecting models…".to_string();
                }
            }
        }
        GenScenePhase::GeneratingModels => {
            let Some(plan) = job.plan.clone() else {
                finish_with_error(&mut job, &mut workshop, "Missing plan.".to_string());
                return;
            };

            if active.scene_id != job.target_scene_id.as_deref().unwrap_or("") {
                workshop.status = "Waiting for scene switch…".to_string();
                return;
            }

            if job.model_tasks.is_empty() && job.resolved_prefabs.is_empty() {
                for asset in plan.assets.iter() {
                    let key = asset.key.trim();
                    if key.is_empty() {
                        finish_with_error(&mut job, &mut workshop, "Asset key is empty.".to_string());
                        return;
                    }
                    if let Some(prefab_id) = asset.existing_prefab_id.as_ref() {
                        match parse_prefab_id(prefab_id) {
                            Ok(id) => {
                                if let Err(err) = ensure_realm_prefab_loaded(&active, id, &mut library) {
                                    finish_with_error(
                                        &mut job,
                                        &mut workshop,
                                        format!(
                                            "Prefab id {prefab_id} not available in the library for asset {key}. {err}"
                                        ),
                                    );
                                    return;
                                }
                                job.resolved_prefabs.insert(key.to_string(), id);
                            }
                            Err(err) => {
                                finish_with_error(&mut job, &mut workshop, err);
                                return;
                            }
                        }
                    } else if let Some(prompt) = asset.gen3d_prompt.as_ref() {
                        let session_id = enqueue_gen3d_task(&mut gen3d_queue, prompt);
                        job.model_tasks.push(GenSceneModelTask {
                            asset_key: key.to_string(),
                            prompt: prompt.clone(),
                            session_id,
                        });
                    } else {
                        finish_with_error(
                            &mut job,
                            &mut workshop,
                            format!("Asset {key} missing prefab id or gen3d prompt."),
                        );
                        return;
                    }
                }
            }

            if job.model_tasks.is_empty() {
                job.phase = GenScenePhase::Applying;
                workshop.status = "Applying placements…".to_string();
                return;
            }

            let mut pending = false;
            let tasks = job.model_tasks.clone();
            for task in tasks.iter() {
                let Some(meta) = gen3d_queue.metas.get(&task.session_id) else {
                    finish_with_error(
                        &mut job,
                        &mut workshop,
                        format!("Missing Gen3D session for {}.", task.asset_key),
                    );
                    return;
                };
                match meta.task_state {
                    Gen3dTaskState::Waiting | Gen3dTaskState::Running | Gen3dTaskState::Idle => {
                        pending = true;
                    }
                    Gen3dTaskState::Failed | Gen3dTaskState::Canceled => {
                        finish_with_error(
                            &mut job,
                            &mut workshop,
                            format!("Gen3D failed for {}.", task.asset_key),
                        );
                        return;
                    }
                    Gen3dTaskState::Done => {
                        let prefab_id = if task.session_id == gen3d_queue.active_session_id {
                            gen3d_job.last_saved_prefab_id()
                        } else {
                            gen3d_queue
                                .inactive_states
                                .get(&task.session_id)
                                .and_then(|state| state.job.last_saved_prefab_id())
                        };
                        if let Some(prefab_id) = prefab_id {
                            job.resolved_prefabs
                                .entry(task.asset_key.clone())
                                .or_insert(prefab_id);
                        } else {
                            pending = true;
                        }
                    }
                }
            }

            if pending {
                workshop.status = "Generating models…".to_string();
                return;
            }

            job.phase = GenScenePhase::Applying;
            workshop.status = "Applying placements…".to_string();
        }
        GenScenePhase::Applying => {
            let Some(plan) = job.plan.clone() else {
                finish_with_error(&mut job, &mut workshop, "Missing plan.".to_string());
                return;
            };

            if active.scene_id != job.target_scene_id.as_deref().unwrap_or("") {
                workshop.status = "Waiting for scene switch…".to_string();
                return;
            }

            if scene_workspace.loaded_from_dir.as_deref()
                != Some(crate::realm::scene_src_dir(&active).as_path())
            {
                scene_workspace.loaded_from_dir =
                    Some(crate::realm::scene_src_dir(&active).to_path_buf());
                scene_workspace.sources = None;
            }
            if scene_workspace.sources.is_none() {
                if let Err(err) = reload_scene_sources_in_workspace(&mut scene_workspace) {
                    finish_with_error(&mut job, &mut workshop, err);
                    return;
                }
            }

            let placements = match build_resolved_placements(
                &plan,
                &job.resolved_prefabs,
                &library,
                &active_floor,
            ) {
                Ok(list) => list,
                Err(err) => {
                    finish_with_error(&mut job, &mut workshop, err);
                    return;
                }
            };

            let patch = match build_scene_patch(&job, &placements) {
                Ok(patch) => patch,
                Err(err) => {
                    finish_with_error(&mut job, &mut workshop, err);
                    return;
                }
            };

            let existing_instances = scene_instances.iter().map(
                |(entity, transform, instance_id, prefab_id, tint, owner)| SceneWorldInstance {
                    entity,
                    instance_id: *instance_id,
                    prefab_id: *prefab_id,
                    transform: *transform,
                    tint: tint.map(|t| t.0),
                    owner_layer_id: owner.map(|o| o.layer_id.clone()),
                },
            );

            let scorecard = default_scorecard();
            let run_id = job.run_id.clone().unwrap_or_else(|| "gen_scene".to_string());

            let resp = scene_run_apply_patch_step(
                &mut commands,
                &mut scene_workspace,
                &library,
                existing_instances,
                &run_id,
                job.next_run_step,
                &scorecard,
                &patch,
            );

            match resp {
                Ok(response) => {
                    let applied = response
                        .result
                        .get("applied")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    if !applied {
                        finish_with_error(
                            &mut job,
                            &mut workshop,
                            format!("Scene patch rejected (run_id={run_id})."),
                        );
                        return;
                    }
                    job.next_run_step = job.next_run_step.saturating_add(1).max(1);

                    let (focus, half_extents) = focus_and_extents_from_placements(&placements, &library, &active_floor);
                    preview.focus = focus;
                    preview.half_extents = half_extents;
                    preview.dirty = true;
                    saves.write(crate::scene_store::SceneSaveRequest::new(
                        "gen scene apply",
                    ));

                    workshop.status = "Scene build complete.".to_string();
                    finish_done(&mut job, &mut workshop);
                }
                Err(err) => {
                    finish_with_error(&mut job, &mut workshop, err);
                }
            }
        }
        GenScenePhase::Done | GenScenePhase::Failed | GenScenePhase::Canceled | GenScenePhase::Idle => {
            return;
        }
    }

    if job.running && !workshop.running {
        workshop.running = true;
        workshop.close_locked = true;
    }
}

fn call_gen_scene_plan(
    config: &AppConfig,
    prompt: &str,
    catalog_json: &str,
    cancel: Option<Arc<AtomicBool>>,
) -> Result<GenScenePlanV1, String> {
    let catalog: GenSceneCatalogSnapshot = serde_json::from_str(catalog_json)
        .map_err(|err| format!("Failed to parse catalog JSON: {err}"))?;
    let user_text = format!(
        "User prompt:\n{prompt}\n\nCatalog (JSON):\n{catalog_json}\n"
    );
    let response = gen3d_generate_text_simple_with_prefix(
        config,
        GEN_SCENE_SYSTEM_PROMPT,
        &user_text,
        cancel,
        "gen_scene",
    )
    .map_err(|err| format!("Plan request failed: {err}"))?;
    let raw = response.text.trim();
    let plan: GenScenePlanV1 = serde_json::from_str(raw)
        .map_err(|err| format!("Failed to parse plan JSON: {err}. Raw: {raw}"))?;
    validate_plan(&plan)?;
    validate_plan_against_catalog(&plan, &catalog)?;
    Ok(plan)
}

fn validate_plan(plan: &GenScenePlanV1) -> Result<(), String> {
    if plan.version != 1 {
        return Err(format!("Unsupported plan version {}", plan.version));
    }
    let terrain = &plan.terrain;
    let has_existing = terrain.existing_floor_id.as_ref().is_some_and(|v| !v.trim().is_empty());
    let has_prompt = terrain.genfloor_prompt.as_ref().is_some_and(|v| !v.trim().is_empty());
    if has_existing == has_prompt {
        return Err(
            "Terrain must set exactly one of existing_floor_id or genfloor_prompt.".to_string(),
        );
    }

    let mut keys = HashSet::new();
    for asset in &plan.assets {
        let key = asset.key.trim();
        if key.is_empty() {
            return Err("Asset key must be non-empty.".to_string());
        }
        if !keys.insert(key.to_string()) {
            return Err(format!("Duplicate asset key: {key}"));
        }
        let has_prefab = asset
            .existing_prefab_id
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty());
        let has_prompt = asset
            .gen3d_prompt
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty());
        if has_prefab == has_prompt {
            return Err(format!(
                "Asset {key} must set exactly one of existing_prefab_id or gen3d_prompt."
            ));
        }
    }

    for placement in &plan.placements {
        if placement.asset_key.trim().is_empty() {
            return Err("Placement asset_key must be non-empty.".to_string());
        }
        if !placement.x.is_finite() || !placement.z.is_finite() || !placement.yaw_deg.is_finite() {
            return Err(format!("Invalid placement values for {}.", placement.asset_key));
        }
        if let Some(scale) = placement.scale {
            if !scale.is_finite() || scale <= 0.0 {
                return Err(format!("Invalid scale for {}.", placement.asset_key));
            }
        }
        if let Some(count) = placement.count {
            if count == 0 {
                return Err(format!("Invalid count for {}.", placement.asset_key));
            }
        }
    }

    Ok(())
}

fn validate_plan_against_catalog(
    plan: &GenScenePlanV1,
    catalog: &GenSceneCatalogSnapshot,
) -> Result<(), String> {
    let mut floor_ids = HashSet::new();
    for floor in &catalog.floors {
        if let Ok(id) = parse_floor_id(&floor.id) {
            floor_ids.insert(id);
        }
    }
    let mut prefab_ids = HashSet::new();
    for prefab in &catalog.prefabs {
        if let Ok(id) = parse_prefab_id(&prefab.id) {
            prefab_ids.insert(id);
        }
    }

    if let Some(existing) = plan.terrain.existing_floor_id.as_ref() {
        let floor_id = parse_floor_id(existing)?;
        if !floor_ids.contains(&floor_id) {
            return Err(format!(
                "Plan references unknown floor id {existing}. Use ids from the catalog only."
            ));
        }
    }

    for asset in &plan.assets {
        if let Some(existing) = asset.existing_prefab_id.as_ref() {
            let prefab_id = parse_prefab_id(existing)?;
            if !prefab_ids.contains(&prefab_id) {
                return Err(format!(
                    "Plan references unknown prefab id {existing} for asset {}. Use ids from the catalog only.",
                    asset.key.trim()
                ));
            }
        }
    }

    Ok(())
}

fn resolve_floor_choice(plan: &GenScenePlanV1) -> Result<GenSceneFloorChoice, String> {
    if let Some(id) = plan.terrain.existing_floor_id.as_ref() {
        let floor_id = parse_floor_id(id)?;
        if floor_id == DEFAULT_FLOOR_ID {
            return Ok(GenSceneFloorChoice::Default);
        }
        return Ok(GenSceneFloorChoice::Existing(floor_id));
    }
    if let Some(prompt) = plan.terrain.genfloor_prompt.as_ref() {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            return Err("GenFloor prompt is empty.".to_string());
        }
        return Ok(GenSceneFloorChoice::GeneratedPrompt(trimmed.to_string()));
    }
    Ok(GenSceneFloorChoice::Default)
}

fn build_catalog_snapshot(
    active: &ActiveRealmScene,
    library: &mut ObjectLibrary,
    prefab_descriptors: &mut PrefabDescriptorLibrary,
) -> Result<GenSceneCatalogSnapshot, String> {
    prefab_descriptors.clear();
    let realm_prefabs_dir = crate::realm_prefab_packages::realm_prefabs_root_dir(&active.realm_id);
    let _ = crate::prefab_descriptors::load_prefab_descriptors_from_dir(
        &realm_prefabs_dir,
        prefab_descriptors,
    );

    if let Ok(prefab_packages) =
        crate::realm_prefab_packages::list_realm_prefab_packages(&active.realm_id)
    {
        for prefab_id in prefab_packages {
            let _ = crate::realm_prefab_packages::load_realm_prefab_package_defs_into_library(
                &active.realm_id,
                prefab_id,
                library,
            );
        }
    }

    let mut floors = Vec::new();
    floors.push(GenSceneCatalogFloor {
        id: DEFAULT_FLOOR_ID.to_string(),
        label: "Default terrain".to_string(),
    });
    if let Ok(ids) = crate::realm_floor_packages::list_realm_floor_packages(&active.realm_id) {
        for floor_id in ids {
            let label = crate::realm_floor_packages::load_realm_floor_def(&active.realm_id, floor_id)
                .ok()
                .and_then(|def| def.label)
                .filter(|label| !label.trim().is_empty())
                .unwrap_or_else(|| {
                    let short = uuid::Uuid::from_u128(floor_id).to_string();
                    format!("Terrain {short}")
                });
            floors.push(GenSceneCatalogFloor {
                id: uuid::Uuid::from_u128(floor_id).to_string(),
                label,
            });
        }
    }

    let mut prefabs = Vec::new();
    for (id, def) in library.iter() {
        let label = def.label.trim().to_string();
        if label.is_empty() {
            continue;
        }
        let mut tags = Vec::new();
        let mut short = None;
        if let Some(desc) = prefab_descriptors.get(*id) {
            tags = desc.tags.clone();
            short = desc.text.as_ref().and_then(|t| t.short.clone());
        }
        prefabs.push(GenSceneCatalogPrefab {
            id: uuid::Uuid::from_u128(*id).to_string(),
            label,
            tags,
            short,
        });
    }

    prefabs.sort_by(|a, b| a.label.cmp(&b.label));

    Ok(GenSceneCatalogSnapshot { floors, prefabs })
}

fn enqueue_gen3d_task(queue: &mut Gen3dTaskQueue, prompt: &str) -> crate::gen3d::Gen3dSessionId {
    let mut state = Gen3dSessionState::default();
    state.workshop.prompt = prompt.trim().to_string();
    state.workshop.status = "Queued Gen3D run (GenScene).".to_string();
    state.workshop.speed_mode = crate::gen3d::Gen3dSpeedMode::Level3;
    let session_id = queue.create_session(Gen3dSessionKind::NewBuild, state);
    queue.queue.push_back(session_id);
    queue.set_task_state(session_id, Gen3dTaskState::Waiting);
    session_id
}

fn parse_floor_id(raw: &str) -> Result<u128, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Floor id is empty.".to_string());
    }
    if trimmed == "default" || trimmed == "0" {
        return Ok(DEFAULT_FLOOR_ID);
    }
    if let Ok(uuid) = uuid::Uuid::parse_str(trimmed) {
        return Ok(uuid.as_u128());
    }
    trimmed
        .parse::<u128>()
        .map_err(|_| format!("Invalid floor id: {trimmed}"))
}

fn parse_prefab_id(raw: &str) -> Result<u128, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Prefab id is empty.".to_string());
    }
    if let Ok(uuid) = uuid::Uuid::parse_str(trimmed) {
        return Ok(uuid.as_u128());
    }
    trimmed
        .parse::<u128>()
        .map_err(|_| format!("Invalid prefab id: {trimmed}"))
}

fn ensure_realm_prefab_loaded(
    active: &ActiveRealmScene,
    prefab_id: u128,
    library: &mut ObjectLibrary,
) -> Result<(), String> {
    if library.get(prefab_id).is_some() {
        return Ok(());
    }

    let loaded = crate::realm_prefab_packages::load_realm_prefab_package_defs_into_library(
        &active.realm_id,
        prefab_id,
        library,
    )?;
    if loaded == 0 {
        return Err(format!(
            "Prefab {} is not loaded and no realm prefab package was found under {}.",
            uuid::Uuid::from_u128(prefab_id),
            active.realm_id
        ));
    }

    Ok(())
}

fn build_resolved_placements(
    plan: &GenScenePlanV1,
    resolved_prefabs: &HashMap<String, u128>,
    library: &ObjectLibrary,
    active_floor: &ActiveWorldFloor,
) -> Result<Vec<(String, u128, Transform)>, String> {
    let mut out = Vec::new();
    for placement in plan.placements.iter() {
        let key = placement.asset_key.trim();
        let prefab_id = resolved_prefabs.get(key).ok_or_else(|| {
            format!("Placement references unknown asset key {key}.")
        })?;

        let count = placement.count.unwrap_or(1).max(1);
        let scale_value = placement.scale.unwrap_or(1.0).max(0.01);
        let yaw = placement.yaw_deg.to_radians();
        let base_y = library.ground_origin_y_or_default(*prefab_id);
        let rotation = Quat::from_rotation_y(yaw);

        for idx in 0..count {
            let floor_sample = crate::genfloor::sample_floor_point(
                active_floor,
                placement.x,
                placement.z,
            );
            let translation = Vec3::new(
                placement.x,
                floor_sample.height + base_y,
                placement.z,
            );
            let transform = Transform::from_translation(translation)
                .with_rotation(rotation)
                .with_scale(Vec3::splat(scale_value));
            let local_ref = format!("{}_{}", key, idx);
            out.push((local_ref, *prefab_id, transform));
        }
    }
    Ok(out)
}

fn build_scene_patch(
    job: &GenSceneJob,
    placements: &[(String, u128, Transform)],
) -> Result<SceneSourcesPatchV1, String> {
    let run_id = job.run_id.clone().unwrap_or_else(|| "gen_scene".to_string());
    let mut ops = Vec::new();
    for (local_ref, prefab_id, transform) in placements {
        ops.push(SceneSourcesPatchOpV1::UpsertPinnedInstance {
            instance_id: None,
            local_ref: Some(local_ref.clone()),
            prefab_id: uuid::Uuid::from_u128(*prefab_id).to_string(),
            transform: transform_json(transform),
            tint_rgba: None,
        });
    }
    Ok(SceneSourcesPatchV1 {
        format_version: SCENE_SOURCES_PATCH_FORMAT_VERSION,
        request_id: run_id,
        ops,
    })
}

fn transform_json(transform: &Transform) -> serde_json::Value {
    serde_json::json!({
        "translation": {"x": transform.translation.x, "y": transform.translation.y, "z": transform.translation.z},
        "rotation": {
            "x": transform.rotation.x,
            "y": transform.rotation.y,
            "z": transform.rotation.z,
            "w": transform.rotation.w
        },
        "scale": {"x": transform.scale.x, "y": transform.scale.y, "z": transform.scale.z}
    })
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

fn focus_and_extents_from_placements(
    placements: &[(String, u128, Transform)],
    library: &ObjectLibrary,
    active_floor: &ActiveWorldFloor,
) -> (Vec3, Vec3) {
    let mut any = false;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for (_local, prefab_id, transform) in placements.iter() {
        if !transform.translation.is_finite() || !transform.scale.is_finite() {
            continue;
        }
        let base_size = library
            .size(*prefab_id)
            .unwrap_or_else(|| Vec3::splat(crate::constants::DEFAULT_OBJECT_SIZE_M));
        let size = base_size.abs() * transform.scale.abs().max(Vec3::splat(0.001));
        let half = (size * 0.5).max(Vec3::splat(0.001));

        let axis_x = (transform.rotation * Vec3::X).abs();
        let axis_y = (transform.rotation * Vec3::Y).abs();
        let axis_z = (transform.rotation * Vec3::Z).abs();
        let world_half = axis_x * half.x + axis_y * half.y + axis_z * half.z;

        let aabb_min = transform.translation - world_half;
        let aabb_max = transform.translation + world_half;

        min = min.min(aabb_min);
        max = max.max(aabb_max);
        any = true;
    }

    if !any {
        let half = floor_half_extents(active_floor);
        return (Vec3::ZERO, half);
    }

    let focus = (min + max) * 0.5;
    let half_extents = ((max - min) * 0.5).max(Vec3::splat(0.5));
    (focus, half_extents)
}

fn floor_half_extents(active_floor: &ActiveWorldFloor) -> Vec3 {
    let size_x = active_floor.def.mesh.size_m[0].max(0.5);
    let size_z = active_floor.def.mesh.size_m[1].max(0.5);
    let thickness = active_floor.def.mesh.thickness_m.max(0.05);
    Vec3::new(size_x, thickness, size_z) * 0.5
}

fn phase_label(phase: &GenScenePhase) -> &'static str {
    match phase {
        GenScenePhase::Idle => "idle",
        GenScenePhase::Planning => "planning",
        GenScenePhase::AwaitSceneSwitch => "await_scene_switch",
        GenScenePhase::GeneratingFloor => "generating_floor",
        GenScenePhase::GeneratingModels => "generating_models",
        GenScenePhase::Applying => "applying",
        GenScenePhase::Done => "done",
        GenScenePhase::Failed => "failed",
        GenScenePhase::Canceled => "canceled",
    }
}

fn apply_floor_choice(
    active: &ActiveRealmScene,
    floor_id: u128,
    active_floor: &mut ActiveWorldFloor,
    floor_library: &mut FloorLibraryUiState,
) -> Result<(), String> {
    let def = if floor_id == DEFAULT_FLOOR_ID {
        crate::genfloor::defs::FloorDefV1::default_world()
    } else {
        crate::realm_floor_packages::load_realm_floor_def(&active.realm_id, floor_id)
            .map_err(|err| format!("Failed to load terrain: {err}"))?
    };
    set_active_world_floor(active_floor, Some(floor_id), def);
    floor_library.set_selected_floor_id(Some(floor_id));
    floor_library.mark_models_dirty();
    scene_floor_selection::save_scene_floor_selection(&active.realm_id, &active.scene_id, Some(floor_id))
        .map_err(|err| format!("Failed to persist terrain selection: {err}"))?;
    Ok(())
}

fn finish_done(job: &mut GenSceneJob, workshop: &mut GenSceneWorkshop) {
    job.running = false;
    job.cancel_requested = false;
    job.cancel_flag = None;
    job.phase = GenScenePhase::Done;
    workshop.running = false;
    workshop.close_locked = false;
}

fn finish_canceled(job: &mut GenSceneJob, workshop: &mut GenSceneWorkshop) {
    job.running = false;
    job.cancel_requested = false;
    job.cancel_flag = None;
    job.phase = GenScenePhase::Canceled;
    workshop.running = false;
    workshop.close_locked = false;
    workshop.status = "Build canceled.".to_string();
}

fn finish_with_error(job: &mut GenSceneJob, workshop: &mut GenSceneWorkshop, err: String) {
    job.running = false;
    job.cancel_requested = false;
    job.cancel_flag = None;
    job.phase = GenScenePhase::Failed;
    workshop.running = false;
    workshop.close_locked = false;
    workshop.error = Some(err.clone());
    workshop.status = format!("Failed: {}", truncate_text(err.trim(), 160));
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

fn write_plan_artifact(job: &GenSceneJob, plan: &GenScenePlanV1) -> Result<(), String> {
    let Some(run_dir) = job.run_dir.as_ref() else {
        return Ok(());
    };
    std::fs::create_dir_all(run_dir)
        .map_err(|err| format!("Failed to create run dir {}: {err}", run_dir.display()))?;
    let path = run_dir.join("plan.json");
    let bytes = serde_json::to_vec_pretty(plan)
        .map_err(|err| format!("Failed to serialize plan: {err}"))?;
    std::fs::write(&path, bytes)
        .map_err(|err| format!("Failed to write {}: {err}", path.display()))?;
    Ok(())
}

fn gen_scene_run_dir(realm_id: &str, scene_id: &str, run_id: &str) -> Result<PathBuf, String> {
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err("run_id must be non-empty".to_string());
    }
    let base = crate::paths::scene_dir(realm_id, scene_id).join("runs");
    let dir = base.join(run_id);
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("Failed to create run dir {}: {err}", dir.display()))?;
    Ok(dir)
}

fn allocate_scene_id(config: &AppConfig, realm_id: &str, prompt: &str) -> Result<String, String> {
    let base = scene_base_id_from_prompt(config, prompt);
    let base = crate::realm::sanitize_id(&base).unwrap_or_else(|| "scene".to_string());

    for idx in 0..100 {
        let candidate = if idx == 0 {
            base.clone()
        } else {
            format!("{base}_{}", idx + 1)
        };
        if crate::realm::sanitize_id(&candidate).is_none() {
            continue;
        }
        let dir = crate::paths::scene_dir(realm_id, &candidate);
        if !dir.exists() {
            return Ok(candidate);
        }
    }

    for _ in 0..50 {
        let short = uuid::Uuid::new_v4().to_string();
        let candidate = format!("{base}_{short}");
        if crate::realm::sanitize_id(&candidate).is_none() {
            continue;
        }
        let dir = crate::paths::scene_dir(realm_id, &candidate);
        if !dir.exists() {
            return Ok(candidate);
        }
    }

    Err("Failed to allocate a unique scene id.".to_string())
}

fn scene_base_id_from_prompt(config: &AppConfig, prompt: &str) -> String {
    let slug = slugify_prompt(prompt, 3, 24);
    if slug.is_empty() {
        if let Some(translated) = translate_prompt_to_english_slug(config, prompt) {
            translated
        } else {
            "untitled".to_string()
        }
    } else {
        slug
    }
}

fn translate_prompt_to_english_slug(config: &AppConfig, prompt: &str) -> Option<String> {
    let system = "You are a naming assistant. Return a short English scene name (2-4 words). \
Return plain text only; no quotes, punctuation, or numbering. Use ASCII letters only.";
    let user = format!(
        "Prompt:\n{prompt}\n\nReturn a short English scene name (2-4 words)."
    );
    let response = gen3d_generate_text_simple_with_prefix(
        config,
        system,
        &user,
        None,
        "gen_scene_name",
    )
    .ok()?;
    let raw = response.text.trim();
    let slug = slugify_prompt(raw, 3, 24);
    if slug.is_empty() {
        None
    } else {
        Some(slug)
    }
}

fn slugify_prompt(prompt: &str, max_words: usize, max_len: usize) -> String {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    for ch in prompt.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
        } else if !current.is_empty() {
            words.push(current.clone());
            current.clear();
        }
    }
    if !current.is_empty() {
        words.push(current);
    }

    let mut unique = Vec::new();
    for word in words {
        if word.len() < 2 {
            continue;
        }
        if unique.contains(&word) {
            continue;
        }
        unique.push(word);
        if unique.len() >= max_words {
            break;
        }
    }

    if unique.is_empty() {
        return String::new();
    }

    let mut slug = unique.join("_");
    if slug.len() > max_len {
        slug.truncate(max_len);
        slug = slug.trim_matches('_').to_string();
    }
    slug
}

fn cancel_gen3d_tasks(
    job: &GenSceneJob,
    queue: &mut Gen3dTaskQueue,
    workshop: &mut Gen3dWorkshop,
    gen3d_job: &mut Gen3dAiJob,
    _gen3d_draft: &mut Gen3dDraft,
) {
    let mut pending_ids: Vec<_> = job.model_tasks.iter().map(|t| t.session_id).collect();
    pending_ids.sort();
    pending_ids.dedup();

    if let Some(running_id) = queue.running_session_id {
        if pending_ids.contains(&running_id) {
            if running_id == queue.active_session_id {
                gen3d_cancel_build_from_api(workshop, gen3d_job);
            } else if let Some(state) = queue.inactive_states.get_mut(&running_id) {
                gen3d_cancel_build_from_api(&mut state.workshop, &mut state.job);
            }
        }
    }

    if !queue.queue.is_empty() {
        queue.queue.retain(|id| !pending_ids.contains(id));
    }

    for id in pending_ids {
        if let Some(meta) = queue.metas.get_mut(&id) {
            meta.task_state = Gen3dTaskState::Canceled;
        }
    }
}
