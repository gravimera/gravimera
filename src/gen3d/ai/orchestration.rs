// Gen3D AI orchestration and helpers.
use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot, ScreenshotCaptured};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::object::registry::{
    builtin_object_id, ObjectDef, ObjectLibrary, ObjectPartDef, ObjectPartKind, PartAnimationDef,
    PartAnimationDriver, UnitAttackKind,
};
use crate::threaded_result::{
    new_shared_result, spawn_worker_thread, take_shared_result, SharedResult,
};
use crate::types::{AnimationChannelsActive, AttackClock, BuildScene, LocomotionClock};

use super::agent_loop;
use super::ai_service::{generate_text_via_ai_service, Gen3dAiServiceConfig};
use super::artifacts::{
    append_gen3d_jsonl_artifact, append_gen3d_run_log, write_gen3d_assembly_snapshot,
    write_gen3d_json_artifact,
};
use super::job::*;
use super::prompts::{
    build_gen3d_component_system_instructions, build_gen3d_component_user_text,
    build_gen3d_plan_system_instructions, build_gen3d_plan_user_text,
    build_gen3d_review_delta_system_instructions, build_gen3d_review_delta_user_text,
};
use super::schema::*;
use super::structured_outputs::Gen3dAiJsonSchemaKind;

use super::convert;
use super::motion_validation;
use super::openai;
use super::parse;
use super::reuse_groups;

use super::super::state::{
    Gen3dContinueButton, Gen3dDraft, Gen3dGenerateButton, Gen3dPendingSeedFromPrefab, Gen3dPreview,
    Gen3dPreviewModelRoot, Gen3dReviewCaptureCamera, Gen3dSeedFromPrefabMode, Gen3dSideTab,
    Gen3dSpeedMode, Gen3dWorkshop,
};
use super::super::tool_feedback::{
    append_gen3d_tool_feedback_entry, gen3d_tool_feedback_history_path, Gen3dToolFeedbackEntry,
    Gen3dToolFeedbackHistory,
};
use super::super::{
    gen3d_draft_object_id, GEN3D_MAX_REQUEST_IMAGES, GEN3D_PREVIEW_DEFAULT_PITCH,
    GEN3D_PREVIEW_DEFAULT_YAW, GEN3D_PREVIEW_LAYER, GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
    GEN3D_REVIEW_CAPTURE_WIDTH_PX, GEN3D_REVIEW_LAYER,
};

pub(crate) fn gen3d_generate_button(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dGenerateButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let log_sinks = log_sinks.map(|sinks| sinks.into_inner().clone());

    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.18, 0.13, 0.92));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.12, 0.20, 0.15, 0.98));
                if job.running {
                    gen3d_cancel_build_from_api(&mut workshop, &mut job);
                    continue;
                }
                match gen3d_start_build_from_api(
                    build_scene.as_ref(),
                    &config,
                    log_sinks.clone(),
                    &mut workshop,
                    &mut job,
                    &mut draft,
                ) {
                    Ok(()) => {}
                    Err(err) => {
                        workshop.error = Some(err);
                    }
                }
            }
        }
    }
}

pub(crate) fn gen3d_continue_button(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dContinueButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let log_sinks = log_sinks.map(|sinks| sinks.into_inner().clone());

    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.06, 0.11, 0.08, 0.80));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.15, 0.10, 0.88));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.18, 0.12, 0.95));
                if job.is_running() {
                    workshop.error =
                        Some("Cannot Continue while building. Click Stop first.".into());
                    continue;
                }
                if !job.can_resume() {
                    workshop.error =
                        Some("No resumable Gen3D session yet. Click Build first.".into());
                    continue;
                }
                if let Err(err) = gen3d_resume_build_from_api(
                    build_scene.as_ref(),
                    &config,
                    log_sinks.clone(),
                    &mut workshop,
                    &mut job,
                ) {
                    workshop.error = Some(err);
                }
            }
        }
    }
}

pub(crate) fn gen3d_apply_pending_seed_from_prefab(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    active: Res<crate::realm::ActiveRealmScene>,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut pending: ResMut<Gen3dPendingSeedFromPrefab>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let Some(req) = pending.request.take() else {
        return;
    };

    if job.is_running() {
        workshop.error = Some("Cannot seed while a Gen3D build is running.".into());
        return;
    }

    let log_sinks = log_sinks.map(|sinks| sinks.into_inner().clone());
    job.set_seed_target_entity(None);
    let result = match req.mode {
        Gen3dSeedFromPrefabMode::EditOverwrite => gen3d_start_edit_session_from_prefab_id_from_api(
            build_scene.as_ref(),
            &config,
            log_sinks,
            &mut workshop,
            &mut job,
            &mut draft,
            &active.realm_id,
            &active.scene_id,
            req.prefab_id,
        ),
        Gen3dSeedFromPrefabMode::Fork => gen3d_start_fork_session_from_prefab_id_from_api(
            build_scene.as_ref(),
            &config,
            log_sinks,
            &mut workshop,
            &mut job,
            &mut draft,
            &active.realm_id,
            &active.scene_id,
            req.prefab_id,
        ),
    };

    match result {
        Ok(()) => {
            job.set_seed_target_entity(req.target_entity);
        }
        Err(err) => {
            workshop.error = Some(err);
            workshop.status = "Failed to seed Gen3D session from prefab.".into();
        }
    }
}

pub(crate) fn gen3d_cancel_build_from_api(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob) {
    if !job.running {
        return;
    }

    // Stop the current build, but keep the session context so it can be resumed.
    // Any in-flight AI request thread stops as soon as it observes the cancel flag; we also ignore
    // its eventual result and stop updating UI state.
    if let Some(flag) = job.cancel_flag.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    job.cancel_flag = None;
    abort_pending_agent_tool_call(job, "Stopped by user.".into());
    job.finish_run_metrics();
    job.running = false;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.capture_previews_only = false;
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;

    // Clear in-flight agent execution state, but keep workspaces + tool history.
    job.agent.step_actions.clear();
    job.agent.step_action_idx = 0;
    job.agent.step_repair_attempt = 0;
    job.agent.step_request_retry_attempt = 0;
    job.agent.pending_tool_call = None;
    job.agent.pending_llm_tool = None;
    job.agent.pending_llm_repair_attempt = 0;
    job.agent.pending_component_batch = None;
    job.agent.pending_render = None;
    job.agent.pending_pass_snapshot = None;
    job.agent.pending_after_pass_snapshot = None;
    job.agent.pending_regen_component_indices.clear();
    job.agent
        .pending_regen_component_indices_skipped_due_to_budget
        .clear();

    workshop.error = None;
    workshop.status =
        "Build stopped. Click Continue to resume, or Build to start a new run.".to_string();
}

pub(crate) fn gen3d_resume_build_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) -> Result<(), String> {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return Err("Gen3D resume requires Build Preview scene.".into());
    }
    if job.running {
        return Err("Gen3D build is already running (stop it first).".into());
    }
    if job.ai.is_none() {
        return Err("Cannot resume: missing AI config (start a new Build).".into());
    }
    if job.run_id.is_none() || job.run_dir.is_none() || job.pass_dir.is_none() {
        return Err("Cannot resume: no prior Gen3D session (start a new Build).".into());
    }

    if !workshop.prompt.trim().is_empty() {
        job.user_prompt_raw = workshop.prompt.clone();
    }

    job.log_sinks = log_sinks;
    job.resume_run_metrics();
    if let Some(flag) = job.cancel_flag.take() {
        flag.store(true, Ordering::Relaxed);
    }
    job.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::AgentWaitingStep;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.capture_previews_only = false;

    // Resume always starts a fresh pass to keep artifacts append-only.
    gen3d_advance_pass(job)?;
    let pass_dir = job
        .pass_dir
        .clone()
        .ok_or_else(|| "Internal error: missing Gen3D pass dir.".to_string())?;

    workshop.error = None;
    workshop.status = format!(
        "Continuing…\nService: {}\nModel: {}\nImages: {}",
        job.ai.as_ref().map(|c| c.service_label()).unwrap_or(""),
        job.ai.as_ref().map(|c| c.model()).unwrap_or(""),
        job.user_images.len()
    );

    if let Err(err) = agent_loop::spawn_agent_step_request(config, workshop, job, pass_dir.clone())
    {
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = false;
        job.phase = Gen3dAiPhase::Idle;
        return Err(err);
    }

    Ok(())
}

fn resolve_gen3d_ai_service_config(config: &AppConfig) -> Result<Gen3dAiServiceConfig, String> {
    match config.gen3d_ai_service {
        crate::config::Gen3dAiService::OpenAi => match config.openai.clone() {
            Some(openai) => Ok(Gen3dAiServiceConfig::OpenAi(openai)),
            None => {
                let details = if config.errors.is_empty() {
                    "Missing config.toml. See gen_3d.md for setup.".to_string()
                } else {
                    config.errors.join("\n")
                };
                Err(details)
            }
        },
        crate::config::Gen3dAiService::Gemini => match config.gemini.clone() {
            Some(gemini) => Ok(Gen3dAiServiceConfig::Gemini(gemini)),
            None => {
                let details = if config.errors.is_empty() {
                    "Missing config.toml. See gen_3d.md for setup.".to_string()
                } else {
                    config.errors.join("\n")
                };
                Err(details)
            }
        },
        crate::config::Gen3dAiService::Claude => match config.claude.clone() {
            Some(claude) => Ok(Gen3dAiServiceConfig::Claude(claude)),
            None => {
                let details = if config.errors.is_empty() {
                    "Missing config.toml. See gen_3d.md for setup.".to_string()
                } else {
                    config.errors.join("\n")
                };
                Err(details)
            }
        },
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gen3dSeededSessionMode {
    EditOverwrite,
    Fork,
}

pub(crate) fn gen3d_start_edit_session_from_prefab_id_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    realm_id: &str,
    scene_id: &str,
    prefab_id: u128,
) -> Result<(), String> {
    gen3d_start_seeded_session_from_prefab_id_from_api(
        build_scene,
        config,
        log_sinks,
        workshop,
        job,
        draft,
        realm_id,
        scene_id,
        prefab_id,
        Gen3dSeededSessionMode::EditOverwrite,
    )
}

pub(crate) fn gen3d_start_fork_session_from_prefab_id_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    realm_id: &str,
    scene_id: &str,
    prefab_id: u128,
) -> Result<(), String> {
    gen3d_start_seeded_session_from_prefab_id_from_api(
        build_scene,
        config,
        log_sinks,
        workshop,
        job,
        draft,
        realm_id,
        scene_id,
        prefab_id,
        Gen3dSeededSessionMode::Fork,
    )
}

fn gen3d_start_seeded_session_from_prefab_id_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    realm_id: &str,
    scene_id: &str,
    prefab_id: u128,
    mode: Gen3dSeededSessionMode,
) -> Result<(), String> {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return Err("Gen3D edit/fork requires Build Preview scene.".into());
    }
    if job.running {
        return Err("Gen3D build is already running (stop it first).".into());
    }

    let ai = resolve_gen3d_ai_service_config(config)?;

    let package_dir = crate::scene_prefabs::scene_prefab_package_dir(realm_id, scene_id, prefab_id);
    if !package_dir.exists() {
        crate::scene_prefabs::debug_log_missing_prefab_package(realm_id, scene_id, prefab_id);
        return Err("Prefab package not found in the active scene. It may have been saved in a different scene.".into());
    }

    let source_dir =
        crate::scene_prefabs::scene_prefab_package_gen3d_source_dir(realm_id, scene_id, prefab_id);
    let has_source_bundle = source_dir.exists();

    let edit_bundle_path = crate::scene_prefabs::scene_prefab_package_gen3d_edit_bundle_path(
        realm_id, scene_id, prefab_id,
    );
    if !edit_bundle_path.exists() {
        return Err("This prefab can’t be edited because it’s missing Gen3D edit metadata (gen3d_edit_bundle_v1.json).".into());
    }

    let edit_bundle = crate::gen3d::ai::gen3d_load_edit_bundle_v1(&edit_bundle_path)?;

    if let Ok(id) = Uuid::parse_str(edit_bundle.root_prefab_id_uuid.trim()) {
        if id.as_u128() != prefab_id {
            return Err(format!(
                "Gen3D edit bundle root id mismatch (bundle={}, requested={}).",
                edit_bundle.root_prefab_id_uuid.trim(),
                Uuid::from_u128(prefab_id),
            ));
        }
    }

    let descriptor =
        load_prefab_descriptor_from_scene_prefab_package(realm_id, scene_id, prefab_id).ok();

    let prompt_from_descriptor = descriptor
        .as_ref()
        .and_then(|d| d.provenance.as_ref())
        .and_then(|p| p.gen3d.as_ref())
        .and_then(|g| g.prompt.as_ref())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    if let Some(prompt) = prompt_from_descriptor.as_ref() {
        workshop.prompt = prompt.clone();
    }

    if descriptor.is_none() && !has_source_bundle {
        return Err("Edit/Fork is supported only for Gen3D-saved prefabs (missing source bundle and descriptor).".into());
    }

    let seeded_defs =
        load_gen3d_draft_defs_from_scene_prefab_package_or_fallback(realm_id, scene_id, prefab_id)?;
    if seeded_defs
        .iter()
        .all(|d| d.object_id != gen3d_draft_object_id())
    {
        return Err("Internal error: seeded draft is missing the Gen3D draft root def.".into());
    }
    draft.defs = seeded_defs;

    // New run dir for the edit/fork session (artifacts append-only).
    let (run_id, run_dir) = gen3d_make_run_dir(config);
    std::fs::create_dir_all(&run_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D cache dir {}: {err}",
            run_dir.display()
        )
    })?;

    // Reset job state but keep the seeded draft.
    job.log_sinks = log_sinks;
    job.metrics = Gen3dRunMetrics::default();
    job.reset_session();
    job.running = false;
    job.cancel_flag = None;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.capture_previews_only = false;
    job.plan_attempt = 0;
    job.max_parallel_components = config.gen3d_max_parallel_components.max(1);
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_appearance = config.gen3d_review_appearance;
    job.require_structured_outputs = config.gen3d_require_structured_outputs;
    job.review_component_idx = None;
    job.auto_refine_passes_done = 0;
    job.auto_refine_passes_remaining = refine_passes_for_speed(config, workshop.speed_mode);
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.replan_attempts = 0;
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.agent = Gen3dAgentState::default();
    job.run_started_at = None;
    job.last_run_elapsed = None;
    job.current_run_tokens = 0;
    job.chat_fallbacks_this_run = 0;

    job.ai = Some(ai);
    job.run_id = Some(run_id);
    job.run_dir = Some(run_dir.clone());

    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    // Seeded Edit/Fork sessions start from an already-generated draft; default to preserve mode so
    // the agent doesn't accidentally regenerate existing components when making small edits.
    job.preserve_existing_components_mode = true;
    job.assembly_rev = 0;
    job.user_prompt_raw = prompt_from_descriptor.unwrap_or_default();
    job.user_images.clear();
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.motion_roles = None;
    job.motion_authoring = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.regen_total = 0;
    job.regen_per_component.clear();
    job.save_seq = 0;

    crate::gen3d::ai::gen3d_hydrate_seeded_job_from_edit_bundle_v1(job, &edit_bundle, &draft.defs)?;

    job.edit_base_prefab_id = Some(prefab_id);
    job.save_overwrite_prefab_id = match mode {
        Gen3dSeededSessionMode::EditOverwrite => Some(prefab_id),
        Gen3dSeededSessionMode::Fork => None,
    };
    job.seed_target_entity = None;

    // Create pass_0 to record the seed action and to enable Continue.
    gen3d_set_current_attempt_pass(job, &run_dir, 0, 0)?;
    let pass_dir = job
        .pass_dir
        .clone()
        .ok_or_else(|| "Internal error: missing Gen3D pass dir.".to_string())?;

    write_gen3d_json_artifact(
        Some(&run_dir),
        "run.json",
        &serde_json::json!({
            "version": 1,
            "run_id": run_id.to_string(),
            "created_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            "seed": {
                "kind": match mode { Gen3dSeededSessionMode::EditOverwrite => "edit_overwrite", Gen3dSeededSessionMode::Fork => "fork" },
                "prefab_id": uuid::Uuid::from_u128(prefab_id).to_string(),
            },
            "ai": {
                "service": job.ai.as_ref().map(|c| c.service_label()).unwrap_or(""),
                "model": job.ai.as_ref().map(|c| c.model()).unwrap_or(""),
                "base_url": job.ai.as_ref().map(|c| c.base_url()).unwrap_or(""),
            },
        }),
    );
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "seed_from_prefab kind={} prefab_id={}",
            match mode {
                Gen3dSeededSessionMode::EditOverwrite => "edit_overwrite",
                Gen3dSeededSessionMode::Fork => "fork",
            },
            uuid::Uuid::from_u128(prefab_id),
        ),
    );

    workshop.error = None;
    workshop.status = match mode {
        Gen3dSeededSessionMode::EditOverwrite => {
            "Edit session loaded. Click Continue to resume generation; Save overwrites the same prefab id.".into()
        }
        Gen3dSeededSessionMode::Fork => {
            "Fork session loaded. Click Continue to resume generation; Save writes a new prefab id.".into()
        }
    };

    Ok(())
}

fn load_prefab_descriptor_from_scene_prefab_package(
    realm_id: &str,
    scene_id: &str,
    prefab_id: u128,
) -> Result<crate::prefab_descriptors::PrefabDescriptorFileV1, String> {
    let prefabs_dir =
        crate::scene_prefabs::scene_prefab_package_prefabs_dir(realm_id, scene_id, prefab_id);
    let uuid = uuid::Uuid::from_u128(prefab_id).to_string();
    let prefab_json = prefabs_dir.join(format!("{uuid}.json"));
    let descriptor_path =
        crate::prefab_descriptors::prefab_descriptor_path_for_prefab_json(&prefab_json);
    let bytes = std::fs::read(&descriptor_path).map_err(|err| {
        format!(
            "Failed to read prefab descriptor {}: {err}",
            descriptor_path.display()
        )
    })?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|err| format!("Invalid JSON: {err}"))?;
    let descriptor: crate::prefab_descriptors::PrefabDescriptorFileV1 =
        serde_json::from_value(value)
            .map_err(|err| format!("Descriptor schema mismatch: {err}"))?;
    Ok(descriptor)
}

fn load_gen3d_draft_defs_from_scene_prefab_package_or_fallback(
    realm_id: &str,
    scene_id: &str,
    prefab_id: u128,
) -> Result<Vec<ObjectDef>, String> {
    let source_dir =
        crate::scene_prefabs::scene_prefab_package_gen3d_source_dir(realm_id, scene_id, prefab_id);
    if source_dir.exists() {
        return load_prefab_defs_from_dir(&source_dir, true);
    }

    let prefabs_dir =
        crate::scene_prefabs::scene_prefab_package_prefabs_dir(realm_id, scene_id, prefab_id);
    reconstruct_gen3d_draft_defs_from_saved_prefabs(&prefabs_dir, prefab_id)
}

fn load_prefab_defs_from_dir(
    dir: &Path,
    expect_gen3d_root: bool,
) -> Result<Vec<ObjectDef>, String> {
    if !dir.exists() {
        return Err(format!("Missing prefab dir: {}", dir.display()));
    }

    let mut ids: Vec<u128> = Vec::new();
    let entries =
        std::fs::read_dir(dir).map_err(|err| format!("Failed to list {}: {err}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|err| format!("Failed to read dir entry: {err}"))?;
        let path = entry.path();
        if path.is_dir() {
            continue;
        }
        let file_name = path.file_name().and_then(|v| v.to_str()).unwrap_or("");
        if !file_name.ends_with(".json") {
            continue;
        }
        if file_name.ends_with(".desc.json") {
            continue;
        }
        let Some(stem) = file_name.strip_suffix(".json") else {
            continue;
        };
        if let Ok(uuid) = uuid::Uuid::parse_str(stem.trim()) {
            ids.push(uuid.as_u128());
        }
    }
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return Err(format!("No prefab defs found under {}", dir.display()));
    }

    let mut library = ObjectLibrary::default();
    crate::realm_prefabs::load_prefabs_into_library_from_dir(dir, &mut library)?;

    let mut defs: Vec<ObjectDef> = Vec::with_capacity(ids.len());
    for id in &ids {
        let Some(def) = library.get(*id) else {
            return Err(format!(
                "Internal error: missing loaded prefab def for id {} from {}",
                uuid::Uuid::from_u128(*id),
                dir.display()
            ));
        };
        defs.push(def.clone());
    }

    if expect_gen3d_root && defs.iter().all(|d| d.object_id != gen3d_draft_object_id()) {
        return Err("Missing Gen3D draft root in source bundle (expected gen3d_source_v1).".into());
    }

    Ok(defs)
}

fn reconstruct_gen3d_draft_defs_from_saved_prefabs(
    prefabs_dir: &Path,
    saved_root_prefab_id: u128,
) -> Result<Vec<ObjectDef>, String> {
    let mut library = ObjectLibrary::default();
    crate::realm_prefabs::load_prefabs_into_library_from_dir(prefabs_dir, &mut library)?;

    let Some(root_def) = library.get(saved_root_prefab_id).cloned() else {
        return Err(format!(
            "Missing root prefab def {} in {}",
            uuid::Uuid::from_u128(saved_root_prefab_id),
            prefabs_dir.display()
        ));
    };

    let mut stack: Vec<u128> = vec![saved_root_prefab_id];
    if let Some(attack) = root_def.attack.as_ref() {
        if matches!(attack.kind, UnitAttackKind::RangedProjectile) {
            if let Some(ranged) = attack.ranged.as_ref() {
                if ranged.projectile_prefab != 0 {
                    stack.push(ranged.projectile_prefab);
                }
                if ranged.muzzle.object_id != 0 {
                    stack.push(ranged.muzzle.object_id);
                }
            }
        }
    }
    if let Some(aim) = root_def.aim.as_ref() {
        for id in &aim.components {
            if *id != 0 {
                stack.push(*id);
            }
        }
    }

    let mut keep: std::collections::BTreeSet<u128> = std::collections::BTreeSet::new();
    while let Some(next) = stack.pop() {
        if keep.contains(&next) {
            continue;
        }
        let Some(def) = library.get(next) else {
            return Err(format!(
                "Missing prefab def {} referenced by {}",
                uuid::Uuid::from_u128(next),
                prefabs_dir.display()
            ));
        };
        keep.insert(next);
        for part in def.parts.iter() {
            if let ObjectPartKind::ObjectRef { object_id } = part.kind {
                if object_id != 0 {
                    stack.push(object_id);
                }
            }
        }
    }

    let mut defs: Vec<ObjectDef> = Vec::with_capacity(keep.len());
    for id in &keep {
        let Some(def) = library.get(*id) else {
            return Err(format!(
                "Missing prefab def {} referenced by {}",
                uuid::Uuid::from_u128(*id),
                prefabs_dir.display()
            ));
        };
        defs.push(def.clone());
    }

    let draft_root_id = gen3d_draft_object_id();
    let saved_projectile_id = root_def
        .attack
        .as_ref()
        .and_then(|a| a.ranged.as_ref())
        .map(|r| r.projectile_prefab);
    let draft_projectile_id =
        saved_projectile_id.map(|_| super::super::gen3d_draft_projectile_object_id());

    let mut remap: std::collections::HashMap<u128, u128> = std::collections::HashMap::new();
    remap.insert(saved_root_prefab_id, draft_root_id);
    if let (Some(saved_projectile_id), Some(draft_projectile_id)) =
        (saved_projectile_id, draft_projectile_id)
    {
        remap.insert(saved_projectile_id, draft_projectile_id);
    }

    for def in &mut defs {
        if let Some(mapped) = remap.get(&def.object_id).copied() {
            def.object_id = mapped;
        }

        for part in &mut def.parts {
            if let ObjectPartKind::ObjectRef { object_id } = &mut part.kind {
                if let Some(mapped) = remap.get(object_id).copied() {
                    *object_id = mapped;
                }
            }
        }

        if let Some(attack) = def.attack.as_mut() {
            if matches!(attack.kind, UnitAttackKind::RangedProjectile) {
                if let Some(ranged) = attack.ranged.as_mut() {
                    if let Some(mapped) = remap.get(&ranged.projectile_prefab).copied() {
                        ranged.projectile_prefab = mapped;
                    }
                    if let Some(mapped) = remap.get(&ranged.muzzle.object_id).copied() {
                        ranged.muzzle.object_id = mapped;
                    }
                }
            }
        }

        if let Some(aim) = def.aim.as_mut() {
            for component_id in aim.components.iter_mut() {
                if let Some(mapped) = remap.get(component_id).copied() {
                    *component_id = mapped;
                }
            }
        }
    }

    if defs.iter().all(|d| d.object_id != draft_root_id) {
        return Err("Internal error: reconstructed draft is missing root def.".into());
    }
    Ok(defs)
}

pub(crate) fn gen3d_start_build_from_api(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
) -> Result<(), String> {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return Err("Gen3D build requires Build Preview scene.".into());
    }
    if job.running {
        return Err("Gen3D build is already running (stop it first).".into());
    }
    if workshop.images.is_empty() && workshop.prompt.trim().is_empty() {
        return Err("Provide at least 1 image or a text prompt.".into());
    }

    let ai = resolve_gen3d_ai_service_config(config)?;

    job.log_sinks = log_sinks;
    job.metrics = Gen3dRunMetrics::default();

    let image_paths: Vec<PathBuf> = workshop.images.iter().map(|i| i.path.clone()).collect();
    let (run_id, run_dir) = gen3d_make_run_dir(config);
    std::fs::create_dir_all(&run_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D cache dir {}: {err}",
            run_dir.display()
        )
    })?;

    write_gen3d_json_artifact(
        Some(&run_dir),
        "run.json",
        &serde_json::json!({
            "version": 1,
            "run_id": run_id.to_string(),
            "created_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
            "ai": {
                "service": ai.service_label(),
                "model": ai.model(),
                "reasoning_effort": ai.model_reasoning_effort(),
                "base_url": ai.base_url(),
            }
        }),
    );

    gen3d_set_current_attempt_pass(job, &run_dir, 0, 0)?;
    let attempt_dir = gen3d_attempt_dir(&run_dir, 0);
    let Some(pass_dir) = job.pass_dir.clone() else {
        return Err("Internal error: missing Gen3D pass dir.".into());
    };

    let cached_inputs = cache_gen3d_inputs(&attempt_dir, &workshop.prompt, &image_paths);
    let cached_image_paths = cached_inputs.cached_image_paths;
    append_gen3d_run_log(
        Some(&pass_dir),
        format!(
            "run_start speed={} max_parallel={} service={} model={} reasoning_effort={} base_url={} review_appearance={} images={} prompt_chars={}",
            workshop.speed_mode.short_label(),
            config.gen3d_max_parallel_components.max(1),
            ai.service_label(),
            ai.model(),
            ai.model_reasoning_effort(),
            ai.base_url(),
            config.gen3d_review_appearance,
            cached_image_paths.len(),
            workshop.prompt.chars().count()
        ),
    );

    workshop.error = None;
    workshop.status = format!(
        "Planning components…\nService: {}\nModel: {}\nImages: {}",
        ai.service_label(),
        ai.model(),
        cached_image_paths.len()
    );

    // Each Build is a fresh run (new cache dir + fresh AI session).
    job.reset_session();
    job.start_run_metrics();
    if let Some(flag) = job.cancel_flag.take() {
        flag.store(true, Ordering::Relaxed);
    }
    job.cancel_flag = Some(Arc::new(AtomicBool::new(false)));
    job.running = true;
    job.build_complete = false;
    job.mode = Gen3dAiMode::Agent;
    job.phase = Gen3dAiPhase::AgentWaitingStep;
    job.capture_previews_only = false;
    job.plan_attempt = 0;
    job.max_parallel_components = config.gen3d_max_parallel_components.max(1);
    job.ai = Some(ai.clone());
    job.run_id = Some(run_id);
    job.attempt = 0;
    job.pass = 0;
    job.plan_hash.clear();
    job.preserve_existing_components_mode = false;
    job.assembly_rev = 0;
    job.user_prompt_raw = workshop.prompt.clone();
    job.user_images = cached_image_paths.clone();
    job.run_dir = Some(run_dir.clone());
    job.pass_dir = Some(pass_dir.clone());
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_appearance = config.gen3d_review_appearance;
    job.require_structured_outputs = config.gen3d_require_structured_outputs;
    job.review_component_idx = None;
    job.auto_refine_passes_done = 0;
    job.auto_refine_passes_remaining = refine_passes_for_speed(config, workshop.speed_mode);
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.replan_attempts = 0;
    job.regen_total = 0;
    job.regen_per_component.clear();
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.motion_roles = None;
    job.motion_authoring = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.agent = Gen3dAgentState::default();
    job.save_seq = 0;
    job.edit_base_prefab_id = None;
    job.save_overwrite_prefab_id = None;
    job.seed_target_entity = None;
    draft.defs.clear();

    workshop.status = format!(
        "Building…\nService: {}\nModel: {}\nImages: {}",
        job.ai.as_ref().map(|c| c.service_label()).unwrap_or(""),
        job.ai.as_ref().map(|c| c.model()).unwrap_or(""),
        job.user_images.len()
    );

    if let Err(err) = agent_loop::spawn_agent_step_request(config, workshop, job, pass_dir.clone())
    {
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = false;
        job.phase = Gen3dAiPhase::Idle;
        return Err(err);
    }

    Ok(())
}

fn refine_passes_for_speed(config: &AppConfig, _speed: Gen3dSpeedMode) -> u32 {
    config.refine_iterations
}

fn component_refine_cycles_for_speed(_config: &AppConfig, _speed: Gen3dSpeedMode) -> u32 {
    0
}

pub(super) fn max_components_for_speed(speed: Gen3dSpeedMode) -> usize {
    let _ = speed;
    24
}

pub(crate) fn gen3d_poll_ai_job(
    config: Res<AppConfig>,
    time: Res<Time>,
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut feedback_history: ResMut<Gen3dToolFeedbackHistory>,
    mut job: ResMut<Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
    mut preview: ResMut<Gen3dPreview>,
    mut preview_model: Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
    review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>,
) {
    if !job.running {
        return;
    }
    if matches!(job.phase, Gen3dAiPhase::Idle) {
        return;
    }

    // Hard budgets: stop the run when exceeded (best-effort draft stays in the preview).
    if config.gen3d_max_seconds > 0 {
        if let Some(elapsed) = job.run_elapsed() {
            if elapsed >= std::time::Duration::from_secs(config.gen3d_max_seconds) {
                let secs = elapsed.as_secs_f32();
                let mins = secs / 60.0;
                let max_mins = config.gen3d_max_seconds as f32 / 60.0;
                finish_job_best_effort(
                    &mut commands,
                    &review_cameras,
                    &mut workshop,
                    &mut job,
                    format!(
                        "Time budget exhausted ({secs:.1}s / {mins:.1}min >= {}s / {max_mins:.1}min).",
                        config.gen3d_max_seconds,
                    ),
                );
                return;
            }
        }
    }
    let max_tokens = config.gen3d_max_tokens;
    if max_tokens > 0 && job.current_run_tokens >= max_tokens {
        let current_tokens = job.current_run_tokens;
        finish_job_best_effort(
            &mut commands,
            &review_cameras,
            &mut workshop,
            &mut job,
            format!("Token budget exhausted ({current_tokens} >= {max_tokens})."),
        );
        return;
    }

    if matches!(job.mode, Gen3dAiMode::Agent) {
        agent_loop::poll_gen3d_agent(
            &config,
            &time,
            &mut commands,
            &mut images,
            &review_cameras,
            &mut workshop,
            &mut feedback_history,
            &mut job,
            &mut draft,
            &mut preview,
            &mut preview_model,
        );
        return;
    }

    let speed_mode = workshop.speed_mode;

    // Apply speed changes to the current run when possible.
    let desired_total_passes = refine_passes_for_speed(&config, workshop.speed_mode);
    let current_total_passes = job.auto_refine_passes_done + job.auto_refine_passes_remaining;
    if desired_total_passes == 0
        && matches!(
            job.phase,
            Gen3dAiPhase::CapturingReview | Gen3dAiPhase::WaitingReview
        )
        && matches!(job.review_kind, Gen3dAutoReviewKind::EndOfRun)
        && !job.capture_previews_only
    {
        debug!("Gen3D: auto-refine disabled mid-run; skipping review phase.");
        for entity in &review_cameras {
            commands.entity(entity).try_despawn();
        }
        job.review_capture = None;
        job.shared_result = None;
        job.shared_progress = None;
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = true;
        job.phase = Gen3dAiPhase::Idle;
        workshop.status =
            "Build finished. (Auto-review skipped due to speed mode change.) Orbit/zoom the preview. Click Build to start a new run."
                .into();
        return;
    }
    if desired_total_passes != current_total_passes {
        job.auto_refine_passes_remaining =
            desired_total_passes.saturating_sub(job.auto_refine_passes_done);
    }

    let per_component_enabled = component_refine_cycles_for_speed(&config, workshop.speed_mode) > 0;
    if !per_component_enabled
        && matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent)
        && matches!(
            job.phase,
            Gen3dAiPhase::CapturingReview | Gen3dAiPhase::WaitingReview
        )
    {
        debug!("Gen3D: per-component review disabled mid-run; resuming build.");
        for entity in &review_cameras {
            commands.entity(entity).try_despawn();
        }
        job.review_capture = None;
        job.shared_result = None;
        workshop.status = "Auto-review skipped due to speed mode change (continuing build).".into();
        resume_after_per_component_review(&mut workshop, &mut job);
        return;
    }

    // Parallel component generation: we don't use `shared_result` during this phase.
    // (Keep the legacy single-request path working if `shared_result` is still set.)
    if matches!(job.phase, Gen3dAiPhase::WaitingComponent) && job.shared_result.is_none() {
        poll_gen3d_parallel_components(&mut workshop, &mut job, &mut draft, speed_mode);
        return;
    }

    // Phase without an AI thread: capture review views as PNGs.
    if matches!(job.phase, Gen3dAiPhase::CapturingReview) {
        // 1) Poll static capture (7 views).
        if let Some(state) = &job.review_capture {
            let (done, expected) = state
                .progress
                .lock()
                .map(|g| (g.completed, g.expected))
                .unwrap_or((0, 7));
            if done < expected {
                if let Some(progress) = job.shared_progress.as_ref() {
                    if job.capture_previews_only {
                        set_progress(
                            progress,
                            format!("Capturing preview renders… ({done}/{expected})"),
                        );
                    } else {
                        set_progress(
                            progress,
                            format!("Capturing review views… ({done}/{expected})"),
                        );
                    }
                }
                return;
            }

            // Clean up capture cameras.
            for cam in state.cameras.iter().copied() {
                commands.entity(cam).try_despawn();
            }
            let review_paths = state.image_paths.clone();
            job.review_capture = None;

            for path in &review_paths {
                if std::fs::metadata(path).is_err() {
                    let label = if job.capture_previews_only {
                        "preview"
                    } else {
                        "review"
                    };
                    workshop.error = Some(format!(
                        "Failed to capture {label} image: {}",
                        path.display()
                    ));
                    if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                        debug!("Gen3D: per-component review image missing; continuing build.");
                        workshop.status = "Auto-review failed (continuing build).".into();
                        resume_after_per_component_review(&mut workshop, &mut job);
                        return;
                    }

                    job.finish_run_metrics();
                    job.running = false;
                    job.build_complete = true;
                    job.phase = Gen3dAiPhase::Idle;
                    job.shared_progress = None;
                    workshop.status =
                        "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                            .into();
                    return;
                }
            }

            if job.capture_previews_only {
                append_gen3d_run_log(job.run_dir.as_deref(), "capture_previews_done");
                job.capture_previews_only = false;
                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished. (Preview renders saved.) Orbit/zoom the preview. Click Build to start a new run."
                        .into();
                return;
            }

            // After static views, capture motion sprite sheets (move + attack), then start the review request.
            job.review_static_paths = review_paths;
            job.motion_capture = Some(Gen3dMotionCaptureState::new());
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Capturing move animation…");
            }
            return;
        }

        // 2) Poll motion capture (2× 2x2 sprite sheets), which appends to `review_static_paths`.
        if job.motion_capture.is_some() {
            poll_gen3d_motion_capture(
                &time,
                &mut commands,
                &mut images,
                &mut workshop,
                &mut job,
                &draft,
                &mut preview_model,
            );
            return;
        }

        // 3) If we have review images ready, start the AI review request.
        if !job.review_static_paths.is_empty() {
            let review_paths = job.review_static_paths.clone();
            job.review_static_paths.clear();

            let Some(ai) = job.ai.clone() else {
                workshop.error = Some("Internal error: missing AI config.".into());
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
                return;
            };
            let Some(run_dir) = job.pass_dir.clone() else {
                workshop.error = Some("Internal error: missing Gen3D pass dir.".into());
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
                return;
            };

            let mut review_inputs = job.user_images.clone();
            review_inputs.extend(review_paths.clone());
            if review_inputs.len() > GEN3D_MAX_REQUEST_IMAGES {
                debug!(
                    "Gen3D: review inputs exceed max images ({} > {}), truncating extra reference photos",
                    review_inputs.len(),
                    GEN3D_MAX_REQUEST_IMAGES
                );
                review_inputs.truncate(GEN3D_MAX_REQUEST_IMAGES);
            }
            if !job.review_appearance {
                review_inputs.clear();
            }

            job.phase = Gen3dAiPhase::WaitingReview;
            job.last_review_inputs = review_inputs.clone();
            job.review_delta_repair_attempt = 0;
            let (status, prefix) = match job.review_kind {
                Gen3dAutoReviewKind::EndOfRun => (
                    format!(
                        "Auto-reviewing assembly… (pass {})",
                        job.auto_refine_passes_done.max(1)
                    ),
                    format!("review{:02}", job.auto_refine_passes_done.max(1)),
                ),
                Gen3dAutoReviewKind::PerComponent => {
                    let pass = job.per_component_refine_passes_done.max(1);
                    let total = pass + job.per_component_refine_passes_remaining;
                    let component = job
                        .review_component_idx
                        .and_then(|idx| job.planned_components.get(idx))
                        .map(|c| c.display_name.clone())
                        .unwrap_or_else(|| "unknown".into());
                    (
                        format!(
                            "Auto-reviewing assembly… (after {})\n(pass {pass}/{total})",
                            component
                        ),
                        format!(
                            "percomp_component{:02}_review{:02}",
                            job.review_component_idx.map(|idx| idx + 1).unwrap_or(0),
                            pass
                        ),
                    )
                }
            };
            workshop.status = status;
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Requesting AI auto-review…");
            }

            let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
            job.shared_result = Some(shared.clone());
            let progress = job
                .shared_progress
                .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                .clone();

            let run_id = job
                .run_id
                .map(|id| id.to_string())
                .unwrap_or_else(|| "unknown".into());
            let plan_hash = compute_gen3d_plan_hash(
                &job.assembly_notes,
                job.rig_move_cycle_m,
                &job.planned_components,
            );
            job.plan_hash = plan_hash.clone();

            let scene_graph_summary = build_gen3d_scene_graph_summary(
                &run_id,
                job.attempt,
                job.pass,
                &plan_hash,
                job.assembly_rev,
                &job.planned_components,
                &draft,
            );
            let smoke_results = build_gen3d_smoke_results(
                &job.user_prompt_raw,
                !job.user_images.is_empty(),
                job.rig_move_cycle_m,
                &job.planned_components,
                &draft,
            );

            write_gen3d_json_artifact(
                job.artifact_dir(),
                "scene_graph_summary.json",
                &scene_graph_summary,
            );
            write_gen3d_json_artifact(job.artifact_dir(), "smoke_results.json", &smoke_results);

            let system = build_gen3d_review_delta_system_instructions(job.review_appearance);
            let user_text = build_gen3d_review_delta_user_text(
                &run_id,
                job.attempt,
                &plan_hash,
                job.assembly_rev,
                &job.user_prompt_raw,
                job.review_appearance && !job.user_images.is_empty(),
                &scene_graph_summary,
                &smoke_results,
            );
            job.last_review_user_text = user_text.clone();
            let reasoning_effort = ai.model_reasoning_effort().to_string();
            spawn_gen3d_ai_text_thread(
                shared,
                progress,
                job.cancel_flag.clone(),
                job.session.clone(),
                Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1),
                config.gen3d_require_structured_outputs,
                ai,
                reasoning_effort,
                system,
                user_text,
                review_inputs,
                run_dir,
                prefix,
            );
            return;
        }

        // 4) Start a new static capture.
        let Some(run_dir) = job.pass_dir.clone() else {
            workshop.error = Some("Internal error: missing Gen3D cache dir.".into());
            if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                workshop.status = "Auto-review failed (continuing build).".into();
                resume_after_per_component_review(&mut workshop, &mut job);
                return;
            }

            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            job.shared_progress = None;
            workshop.status =
                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                    .into();
            return;
        };
        let prefix = if job.capture_previews_only {
            "preview"
        } else {
            "review"
        };
        let include_overlay = !job.capture_previews_only;
        let views = [
            Gen3dReviewView::Front,
            Gen3dReviewView::FrontLeft,
            Gen3dReviewView::LeftBack,
            Gen3dReviewView::Back,
            Gen3dReviewView::RightBack,
            Gen3dReviewView::FrontRight,
            Gen3dReviewView::Top,
        ];
        match start_gen3d_review_capture(
            &mut commands,
            &mut images,
            &run_dir,
            &draft,
            include_overlay,
            prefix,
            &views,
            GEN3D_REVIEW_CAPTURE_WIDTH_PX,
            GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
        ) {
            Ok(state) => {
                job.review_capture = Some(state);
                if let Some(progress) = job.shared_progress.as_ref() {
                    if job.capture_previews_only {
                        set_progress(progress, "Capturing preview renders… (0/7)");
                    } else {
                        set_progress(progress, "Capturing review views… (0/7)");
                    }
                }
            }
            Err(err) => {
                workshop.error = Some(err);
                if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                    debug!("Gen3D: per-component review capture failed; continuing build.");
                    job.review_capture = None;
                    workshop.status = "Auto-review failed (continuing build).".into();
                    resume_after_per_component_review(&mut workshop, &mut job);
                    return;
                }

                job.finish_run_metrics();
                job.running = false;
                job.build_complete = true;
                job.phase = Gen3dAiPhase::Idle;
                job.shared_progress = None;
                job.review_capture = None;
                workshop.status =
                    "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                        .into();
            }
        }
        return;
    }

    // Phases that wait for an AI response.
    let Some(shared) = job.shared_result.as_ref() else {
        fail_job(
            &mut workshop,
            &mut job,
            "Internal error: missing AI job handle.",
        );
        return;
    };

    let Some(result) = take_shared_result(shared) else {
        return;
    };

    job.shared_result = None;

    match result {
        Ok(resp) => {
            debug!("Gen3D: OpenAI response via {:?}", resp.api);
            job.note_api_used(resp.api);
            job.session = resp.session;
            if let Some(tokens) = resp.total_tokens {
                debug!("Gen3D: OpenAI usage total_tokens={tokens}");
                job.add_tokens(tokens);
            }
            let max_tokens = config.gen3d_max_tokens;
            if max_tokens > 0 && job.current_run_tokens >= max_tokens {
                let current_tokens = job.current_run_tokens;
                finish_job_best_effort(
                    &mut commands,
                    &review_cameras,
                    &mut workshop,
                    &mut job,
                    format!("Token budget exhausted ({current_tokens} >= {max_tokens})."),
                );
                return;
            }
            let text = resp.text;

            match job.phase {
                Gen3dAiPhase::AgentWaitingStep
                | Gen3dAiPhase::AgentExecutingActions
                | Gen3dAiPhase::AgentWaitingTool
                | Gen3dAiPhase::AgentCapturingRender
                | Gen3dAiPhase::AgentCapturingPassSnapshot => {
                    // Agent mode is polled via `agent_loop::poll_gen3d_agent`. If we end up here,
                    // just ignore this legacy response path.
                    debug!("Gen3D: ignoring legacy AI result while in agent phase.");
                }
                Gen3dAiPhase::WaitingPlan => {
                    debug!("Gen3D: plan request finished.");
                    let mut plan = match parse::parse_ai_plan_from_text(&text) {
                        Ok(plan) => plan,
                        Err(err) => {
                            debug!("Gen3D: failed to parse AI plan: {err}");
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    let max_components = max_components_for_speed(workshop.speed_mode);
                    if plan.components.len() > max_components {
                        debug!(
                            "Gen3D: truncating plan components from {} to {} due to speed mode",
                            plan.components.len(),
                            max_components
                        );
                        plan.components.truncate(max_components);
                    }

                    job.plan_collider = plan.collider.clone();
                    job.rig_move_cycle_m = plan
                        .rig
                        .as_ref()
                        .and_then(|r| r.move_cycle_m)
                        .filter(|v| v.is_finite())
                        .map(|v| v.abs())
                        .filter(|v| *v > 1e-3);
                    let plan_reuse_groups = plan.reuse_groups.clone();
                    match convert::ai_plan_to_initial_draft_defs(plan) {
                        Ok((planned, assembly_notes, defs)) => {
                            job.planned_components = planned;
                            job.assembly_notes = assembly_notes;
                            let (validated, warnings) = reuse_groups::validate_reuse_groups(
                                &plan_reuse_groups,
                                &job.planned_components,
                            );
                            job.reuse_groups = validated;
                            job.reuse_group_warnings = warnings;
                            job.component_queue = (0..job.planned_components.len()).collect();
                            job.component_queue_pos = 0;
                            job.generation_kind = Gen3dComponentGenerationKind::Initial;
                            job.regen_per_component = vec![0; job.planned_components.len()];
                            draft.defs = defs;
                            workshop.error = None;

                            if let Some(run_dir) = job.artifact_dir() {
                                let components: Vec<serde_json::Value> = job
                                    .planned_components
                                    .iter()
                                    .map(|c| {
                                        let forward = c.rot * Vec3::Z;
                                        let up = c.rot * Vec3::Y;
                                        serde_json::json!({
                                            "name": c.name.as_str(),
                                            "purpose": c.purpose.as_str(),
                                            "modeling_notes": c.modeling_notes.as_str(),
                                            "pos": [c.pos.x, c.pos.y, c.pos.z],
                                            "forward": [forward.x, forward.y, forward.z],
                                            "up": [up.x, up.y, up.z],
                                            "size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                                        })
                                    })
                                    .collect();
                                let extracted = serde_json::json!({
                                    "version": 2,
                                    "assembly_notes": job.assembly_notes.as_str(),
                                    "components": components,
                                });
                                write_gen3d_json_artifact(
                                    Some(run_dir),
                                    "plan_extracted.json",
                                    &extracted,
                                );
                            }
                            write_gen3d_assembly_snapshot(
                                job.artifact_dir(),
                                &job.planned_components,
                            );

                            if let Some(def) = draft.root_def() {
                                let max_dim = def.size.x.max(def.size.y).max(def.size.z).max(0.5);
                                preview.distance = (max_dim * 2.8 + 0.8).clamp(2.0, 250.0);
                                preview.pitch = GEN3D_PREVIEW_DEFAULT_PITCH;
                                preview.yaw = GEN3D_PREVIEW_DEFAULT_YAW;
                                preview.last_cursor = None;
                            }

                            if job.component_queue.is_empty() {
                                let err = "AI plan did not include any components.".to_string();
                                if retry_gen3d_plan(
                                    &mut workshop,
                                    &mut job,
                                    &mut draft,
                                    speed_mode,
                                    &err,
                                ) {
                                    return;
                                }
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }

                            job.phase = Gen3dAiPhase::WaitingComponent;
                            job.component_attempts = vec![0; job.planned_components.len()];
                            job.component_in_flight.clear();
                            job.shared_progress = None;

                            workshop.status = format!(
                                "Building components… (0/{})\nParallel: {}",
                                job.planned_components.len(),
                                job.max_parallel_components.max(1),
                            );
                        }
                        Err(err) => {
                            debug!("Gen3D: failed to build draft from plan: {err}");
                            if retry_gen3d_plan(
                                &mut workshop,
                                &mut job,
                                &mut draft,
                                speed_mode,
                                &err,
                            ) {
                                return;
                            }
                            fail_job(&mut workshop, &mut job, err);
                        }
                    }
                }
                Gen3dAiPhase::WaitingComponent => {
                    if job.component_queue_pos >= job.component_queue.len() {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: component queue out of range.",
                        );
                        return;
                    }
                    let idx = job.component_queue[job.component_queue_pos];
                    if idx >= job.planned_components.len() {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: component index out of range.",
                        );
                        return;
                    }

                    let component_name = job.planned_components[idx].name.clone();
                    debug!(
                        "Gen3D: component generation finished ({}/{}, name={})",
                        job.component_queue_pos + 1,
                        job.component_queue.len(),
                        component_name
                    );

                    let ai = match parse::parse_ai_draft_from_text(&text) {
                        Ok(ai) => ai,
                        Err(err) => {
                            debug!("Gen3D: failed to parse component draft: {err}");
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    let component_def = match convert::ai_to_component_def(
                        &job.planned_components[idx],
                        ai,
                        job.artifact_dir(),
                    ) {
                        Ok(def) => def,
                        Err(err) => {
                            debug!("Gen3D: failed to convert component draft: {err}");
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    };

                    job.planned_components[idx].actual_size = Some(component_def.size);
                    job.planned_components[idx].anchors = component_def.anchors.clone();

                    // Replace component def in-place.
                    let target_id = component_def.object_id;
                    if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id)
                    {
                        let preserved_refs: Vec<ObjectPartDef> = existing
                            .parts
                            .iter()
                            .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
                            .cloned()
                            .collect();
                        let mut merged = component_def;
                        merged.parts.extend(preserved_refs);
                        *existing = merged;
                    } else {
                        draft.defs.push(component_def);
                    }

                    if let Some(root_idx) = job
                        .planned_components
                        .iter()
                        .position(|c| c.attach_to.is_none())
                    {
                        if let Err(err) = convert::resolve_planned_component_transforms(
                            &mut job.planned_components,
                            root_idx,
                        ) {
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    }
                    convert::update_root_def_from_planned_components(
                        &job.planned_components,
                        &job.plan_collider,
                        &mut draft,
                    );
                    write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                    job.assembly_rev = job.assembly_rev.saturating_add(1);

                    let next_pos = job.component_queue_pos + 1;
                    let per_component_refine_total =
                        component_refine_cycles_for_speed(&config, workshop.speed_mode);
                    if per_component_refine_total > 0
                        && matches!(job.generation_kind, Gen3dComponentGenerationKind::Initial)
                        && job.per_component_resume.is_none()
                    {
                        job.review_kind = Gen3dAutoReviewKind::PerComponent;
                        job.review_component_idx = Some(idx);
                        job.per_component_resume = Some(Gen3dComponentBatchResume {
                            generation_kind: job.generation_kind,
                            component_queue: job.component_queue.clone(),
                            component_queue_pos: next_pos,
                        });
                        job.component_queue_pos = next_pos;
                        job.per_component_refine_passes_done = 1;
                        job.per_component_refine_passes_remaining =
                            per_component_refine_total.saturating_sub(1);
                        job.phase = Gen3dAiPhase::CapturingReview;
                        job.review_capture = None;
                        job.review_static_paths.clear();
                        job.motion_capture = None;
                        workshop.status = format!(
                            "Auto-reviewing assembly… (after {})\n(pass {}/{})",
                            job.planned_components[idx].display_name,
                            job.per_component_refine_passes_done,
                            job.per_component_refine_passes_done
                                + job.per_component_refine_passes_remaining
                        );
                        if let Some(progress) = job.shared_progress.as_ref() {
                            set_progress(progress, "Preparing review capture…");
                        }
                        return;
                    }
                    if next_pos >= job.component_queue.len() {
                        if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent)
                            && matches!(
                                job.generation_kind,
                                Gen3dComponentGenerationKind::Regenerate
                            )
                            && job.per_component_resume.is_some()
                        {
                            if job.per_component_refine_passes_remaining > 0 {
                                job.per_component_refine_passes_remaining -= 1;
                                job.per_component_refine_passes_done += 1;
                                job.phase = Gen3dAiPhase::CapturingReview;
                                job.review_capture = None;
                                job.review_static_paths.clear();
                                job.motion_capture = None;
                                workshop.status = format!(
                                    "Auto-reviewing assembly… (component pass {}/{})",
                                    job.per_component_refine_passes_done,
                                    job.per_component_refine_passes_done
                                        + job.per_component_refine_passes_remaining
                                );
                                if let Some(progress) = job.shared_progress.as_ref() {
                                    set_progress(progress, "Preparing review capture…");
                                }
                                return;
                            }

                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }

                        // Finished this generation batch.
                        match job.generation_kind {
                            Gen3dComponentGenerationKind::Initial
                            | Gen3dComponentGenerationKind::Regenerate => {
                                if job.auto_refine_passes_remaining > 0 {
                                    job.auto_refine_passes_remaining -= 1;
                                    job.auto_refine_passes_done += 1;
                                    job.phase = Gen3dAiPhase::CapturingReview;
                                    job.review_capture = None;
                                    job.review_static_paths.clear();
                                    job.motion_capture = None;
                                    workshop.status = format!(
                                        "Auto-reviewing assembly… (pass {}/{})",
                                        job.auto_refine_passes_done,
                                        job.auto_refine_passes_done
                                            + job.auto_refine_passes_remaining
                                    );
                                    if let Some(progress) = job.shared_progress.as_ref() {
                                        set_progress(progress, "Preparing review capture…");
                                    }
                                    return;
                                }
                            }
                        }

                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    job.component_queue_pos = next_pos;
                    let next_idx = job.component_queue[next_pos];

                    let Some(ai) = job.ai.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing AI config.",
                        );
                        return;
                    };
                    let Some(run_dir) = job.pass_dir.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing Gen3D pass dir.",
                        );
                        return;
                    };

                    let phase_label = match job.generation_kind {
                        Gen3dComponentGenerationKind::Initial => "Building components…",
                        Gen3dComponentGenerationKind::Regenerate => "Regenerating components…",
                    };
                    let comp = &job.planned_components[next_idx];
                    let forward = comp.rot * Vec3::Z;
                    let up = comp.rot * Vec3::Y;
                    workshop.status = format!(
                        "{phase_label} ({}/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
                        next_pos + 1,
                        job.component_queue.len(),
                        comp.display_name,
                        comp.pos.x,
                        comp.pos.y,
                        comp.pos.z,
                        forward.x,
                        forward.y,
                        forward.z,
                        up.x,
                        up.y,
                        up.z,
                    );
                    if let Some(progress) = job.shared_progress.as_ref() {
                        set_progress(progress, "Starting next component…");
                    }

                    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
                    job.shared_result = Some(shared.clone());

                    let progress = job
                        .shared_progress
                        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                        .clone();

                    let system = build_gen3d_component_system_instructions();
                    let user_text = build_gen3d_component_user_text(
                        &job.user_prompt_raw,
                        !job.user_images.is_empty(),
                        workshop.speed_mode,
                        &job.assembly_notes,
                        &job.planned_components,
                        next_idx,
                    );
                    let prefix = match job.generation_kind {
                        Gen3dComponentGenerationKind::Initial => format!(
                            "component{:02}_{}",
                            next_idx + 1,
                            job.planned_components[next_idx].name
                        ),
                        Gen3dComponentGenerationKind::Regenerate => match job.review_kind {
                            Gen3dAutoReviewKind::EndOfRun => format!(
                                "review{:02}_regen_component{:02}_{}",
                                job.auto_refine_passes_done.max(1),
                                next_idx + 1,
                                job.planned_components[next_idx].name
                            ),
                            Gen3dAutoReviewKind::PerComponent => format!(
                                "percomp_component{:02}_pass{:02}_regen_component{:02}_{}",
                                job.review_component_idx
                                    .map(|idx| idx + 1)
                                    .unwrap_or(next_idx + 1),
                                job.per_component_refine_passes_done.max(1),
                                next_idx + 1,
                                job.planned_components[next_idx].name
                            ),
                        },
                    };
                    let reasoning_effort = ai.model_reasoning_effort().to_string();
                    spawn_gen3d_ai_text_thread(
                        shared,
                        progress,
                        job.cancel_flag.clone(),
                        job.session.clone(),
                        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
                        config.gen3d_require_structured_outputs,
                        ai,
                        reasoning_effort,
                        system,
                        user_text,
                        job.user_images.clone(),
                        run_dir,
                        prefix,
                    );
                }
                Gen3dAiPhase::WaitingReview => {
                    debug!("Gen3D: auto-review delta request finished.");
                    let delta = match parse::parse_ai_review_delta_from_text(&text) {
                        Ok(delta) => delta,
                        Err(err) => {
                            warn!("Gen3D: failed to parse AI review-delta: {err}");
                            if retry_gen3d_review_delta(
                                &mut workshop,
                                &mut job,
                                &config,
                                speed_mode,
                                &format!("Parse error: {err}"),
                            ) {
                                return;
                            }
                            workshop.error = Some(err);
                            if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                                debug!("Gen3D: per-component review failed; continuing build.");
                                workshop.status = "Auto-review failed (continuing build).".into();
                                resume_after_per_component_review(&mut workshop, &mut job);
                                return;
                            }

                            job.finish_run_metrics();
                            job.running = false;
                            job.build_complete = true;
                            job.phase = Gen3dAiPhase::Idle;
                            job.shared_progress = None;
                            workshop.status =
                                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                    .into();
                            return;
                        }
                    };

                    if let Some(summary) = delta.summary.as_deref() {
                        if !summary.trim().is_empty() {
                            debug!(
                                "Gen3D: review-delta summary: {}",
                                truncate_for_ui(summary.trim(), 800)
                            );
                        }
                    }
                    if let Some(notes) = delta.notes.as_deref() {
                        if !notes.trim().is_empty() {
                            debug!(
                                "Gen3D: review-delta notes: {}",
                                truncate_for_ui(notes.trim(), 800)
                            );
                        }
                    }

                    let expected_run_id = job
                        .run_id
                        .map(|id| id.to_string())
                        .unwrap_or_else(|| "unknown".into());
                    if delta.applies_to.run_id != expected_run_id
                        || delta.applies_to.attempt != job.attempt
                        || delta.applies_to.plan_hash != job.plan_hash
                        || delta.applies_to.assembly_rev != job.assembly_rev
                    {
                        let msg = format!(
                            "applies_to mismatch (expected run_id={}, attempt={}, plan_hash={}, assembly_rev={})",
                            expected_run_id, job.attempt, job.plan_hash, job.assembly_rev
                        );
                        warn!("Gen3D: review-delta rejected: {msg}");
                        if retry_gen3d_review_delta(
                            &mut workshop,
                            &mut job,
                            &config,
                            speed_mode,
                            &msg,
                        ) {
                            return;
                        }
                        workshop.error = Some(msg);
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                .into();
                        return;
                    }

                    let plan_collider = job.plan_collider.clone();
                    let artifact_dir = job.pass_dir.clone();
                    let apply = match convert::apply_ai_review_delta_actions(
                        delta,
                        &mut job.planned_components,
                        &plan_collider,
                        &mut draft,
                        artifact_dir.as_deref(),
                    ) {
                        Ok(apply) => apply,
                        Err(err) => {
                            warn!("Gen3D: failed to apply review-delta actions: {err}");
                            if retry_gen3d_review_delta(
                                &mut workshop,
                                &mut job,
                                &config,
                                speed_mode,
                                &format!("Apply error: {err}"),
                            ) {
                                return;
                            }
                            workshop.error = Some(err);
                            job.finish_run_metrics();
                            job.running = false;
                            job.build_complete = true;
                            job.phase = Gen3dAiPhase::Idle;
                            job.shared_progress = None;
                            workshop.status =
                                "Build finished (auto-review failed). Orbit/zoom the preview, then click Build to start a new run."
                                    .into();
                            return;
                        }
                    };

                    if !apply.tooling_feedback.is_empty() {
                        record_gen3d_tooling_feedback(
                            &config,
                            &mut workshop,
                            &mut feedback_history,
                            &job,
                            &apply.tooling_feedback,
                        );
                    }

                    if apply.accepted {
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. (Reviewer accepted.) Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    if apply.had_actions {
                        job.assembly_rev = job.assembly_rev.saturating_add(1);
                        write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                    }

                    if let Some(reason) = apply.replan_reason {
                        if try_start_gen3d_replan(
                            &config,
                            &mut workshop,
                            &mut job,
                            &mut draft,
                            reason,
                        ) {
                            return;
                        }
                        finish_job_best_effort(
                            &mut commands,
                            &review_cameras,
                            &mut workshop,
                            &mut job,
                            format!(
                                "Replan budget exhausted (max_replans={}).",
                                config.gen3d_max_replans
                            ),
                        );
                        return;
                    }

                    if matches!(job.review_kind, Gen3dAutoReviewKind::PerComponent) {
                        // Keep the old per-component review behavior for now.
                        if !apply.had_actions {
                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }
                        if apply.regen_indices.is_empty() {
                            resume_after_per_component_review(&mut workshop, &mut job);
                            return;
                        }
                    } else if !apply.had_actions {
                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished. (Auto-review made no changes.) Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    if matches!(job.review_kind, Gen3dAutoReviewKind::EndOfRun)
                        && apply.regen_indices.is_empty()
                    {
                        if job.auto_refine_passes_remaining > 0 {
                            if let Err(err) = gen3d_advance_pass(&mut job) {
                                fail_job(&mut workshop, &mut job, err);
                                return;
                            }
                            job.auto_refine_passes_remaining -= 1;
                            job.auto_refine_passes_done += 1;
                            job.phase = Gen3dAiPhase::CapturingReview;
                            job.review_capture = None;
                            job.review_static_paths.clear();
                            job.motion_capture = None;
                            workshop.status = format!(
                                "Auto-reviewing assembly… (pass {}/{})",
                                job.auto_refine_passes_done,
                                job.auto_refine_passes_done + job.auto_refine_passes_remaining
                            );
                            if let Some(progress) = job.shared_progress.as_ref() {
                                set_progress(progress, "Preparing review capture…");
                            }
                            return;
                        }

                        job.finish_run_metrics();
                        job.running = false;
                        job.build_complete = true;
                        job.phase = Gen3dAiPhase::Idle;
                        job.shared_progress = None;
                        workshop.status =
                            "Build finished (auto-review applied tweaks). Orbit/zoom the preview. Click Build to start a new run."
                                .into();
                        return;
                    }

                    let requested_regen = apply.regen_indices;
                    let mut regen_allowed = Vec::new();
                    let mut regen_skipped = Vec::new();
                    if !requested_regen.is_empty() {
                        let max_total = config.gen3d_max_regen_total;
                        let max_per_component = config.gen3d_max_regen_per_component;
                        let planned_len = job.planned_components.len();
                        if job.regen_per_component.len() != planned_len {
                            job.regen_per_component.resize(planned_len, 0);
                        }

                        for idx in requested_regen {
                            if idx >= planned_len {
                                continue;
                            }
                            if max_total > 0 && job.regen_total >= max_total {
                                regen_skipped.push(idx);
                                continue;
                            }
                            if max_per_component > 0
                                && job.regen_per_component[idx] >= max_per_component
                            {
                                regen_skipped.push(idx);
                                continue;
                            }

                            job.regen_total = job.regen_total.saturating_add(1);
                            job.regen_per_component[idx] =
                                job.regen_per_component[idx].saturating_add(1);
                            regen_allowed.push(idx);
                        }

                        if regen_allowed.is_empty() {
                            finish_job_best_effort(
                                &mut commands,
                                &review_cameras,
                                &mut workshop,
                                &mut job,
                                format!(
                                    "Regen budget exhausted (max_regen_total={}, max_regen_per_component={}).",
                                    config.gen3d_max_regen_total, config.gen3d_max_regen_per_component
                                ),
                            );
                            return;
                        }
                        if !regen_skipped.is_empty() {
                            regen_skipped.sort();
                            regen_skipped.dedup();
                            warn!(
                                "Gen3D: regen budget reached; skipping regen for {} component(s): {:?}",
                                regen_skipped.len(),
                                regen_skipped
                            );
                            append_gen3d_run_log(
                                job.artifact_dir(),
                                format!(
                                    "regen_budget_skip skipped={} max_total={} max_per_component={}",
                                    regen_skipped.len(),
                                    config.gen3d_max_regen_total,
                                    config.gen3d_max_regen_per_component
                                ),
                            );
                        }
                    }

                    if !regen_allowed.is_empty() {
                        if let Err(err) = gen3d_advance_pass(&mut job) {
                            fail_job(&mut workshop, &mut job, err);
                            return;
                        }
                    }

                    job.generation_kind = Gen3dComponentGenerationKind::Regenerate;
                    job.component_queue = regen_allowed;
                    job.component_queue_pos = 0;

                    let Some(ai) = job.ai.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing AI config.",
                        );
                        return;
                    };
                    let Some(run_dir) = job.pass_dir.clone() else {
                        fail_job(
                            &mut workshop,
                            &mut job,
                            "Internal error: missing Gen3D pass dir.",
                        );
                        return;
                    };

                    let idx = job.component_queue[0];
                    job.phase = Gen3dAiPhase::WaitingComponent;
                    let comp = &job.planned_components[idx];
                    let forward = comp.rot * Vec3::Z;
                    let up = comp.rot * Vec3::Y;
                    workshop.status = format!(
                        "Regenerating components… (1/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
                        job.component_queue.len(),
                        comp.display_name,
                        comp.pos.x,
                        comp.pos.y,
                        comp.pos.z,
                        forward.x,
                        forward.y,
                        forward.z,
                        up.x,
                        up.y,
                        up.z,
                    );
                    if let Some(progress) = job.shared_progress.as_ref() {
                        set_progress(progress, "Starting component regeneration…");
                    }

                    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
                    job.shared_result = Some(shared.clone());
                    let progress = job
                        .shared_progress
                        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
                        .clone();
                    let system = build_gen3d_component_system_instructions();
                    let user_text = build_gen3d_component_user_text(
                        &job.user_prompt_raw,
                        !job.user_images.is_empty(),
                        workshop.speed_mode,
                        &job.assembly_notes,
                        &job.planned_components,
                        idx,
                    );
                    let prefix = match job.review_kind {
                        Gen3dAutoReviewKind::EndOfRun => format!(
                            "review{:02}_regen_component{:02}_{}",
                            job.auto_refine_passes_done.max(1),
                            idx + 1,
                            job.planned_components[idx].name
                        ),
                        Gen3dAutoReviewKind::PerComponent => format!(
                            "percomp_component{:02}_pass{:02}_regen_component{:02}_{}",
                            job.review_component_idx
                                .map(|idx| idx + 1)
                                .unwrap_or(idx + 1),
                            job.per_component_refine_passes_done.max(1),
                            idx + 1,
                            job.planned_components[idx].name
                        ),
                    };
                    let reasoning_effort = ai.model_reasoning_effort().to_string();
                    spawn_gen3d_ai_text_thread(
                        shared,
                        progress,
                        job.cancel_flag.clone(),
                        job.session.clone(),
                        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
                        config.gen3d_require_structured_outputs,
                        ai,
                        reasoning_effort,
                        system,
                        user_text,
                        job.user_images.clone(),
                        run_dir,
                        prefix,
                    );
                }
                Gen3dAiPhase::CapturingReview | Gen3dAiPhase::Idle => {}
            }
        }
        Err(err) => {
            debug!("Gen3D: AI job failed: {err}");
            if matches!(job.phase, Gen3dAiPhase::WaitingPlan)
                && retry_gen3d_plan(&mut workshop, &mut job, &mut draft, speed_mode, &err)
            {
                return;
            }

            fail_job(&mut workshop, &mut job, err);
        }
    }
}

const GEN3D_MAX_PLAN_RETRIES: u8 = 1;
const GEN3D_MAX_COMPONENT_RETRIES: u8 = 1;
const GEN3D_MAX_REVIEW_DELTA_REPAIRS: u8 = 1;

fn retry_gen3d_review_delta(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    _config: &AppConfig,
    _speed: Gen3dSpeedMode,
    reason: &str,
) -> bool {
    if job.review_delta_repair_attempt >= GEN3D_MAX_REVIEW_DELTA_REPAIRS {
        return false;
    }
    job.review_delta_repair_attempt += 1;

    let Some(ai) = job.ai.clone() else {
        return false;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        return false;
    };
    if job.last_review_inputs.is_empty() || job.last_review_user_text.trim().is_empty() {
        return false;
    }

    warn!(
        "Gen3D: retrying review-delta (repair {}/{}) reason={}",
        job.review_delta_repair_attempt,
        GEN3D_MAX_REVIEW_DELTA_REPAIRS,
        truncate_for_ui(reason, 800)
    );

    workshop.error = None;
    workshop.status = format!(
        "Auto-reviewing assembly… (repair {}/{})",
        job.review_delta_repair_attempt, GEN3D_MAX_REVIEW_DELTA_REPAIRS
    );

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();
    set_progress(&progress, "Repairing review-delta JSON…");
    job.phase = Gen3dAiPhase::WaitingReview;

    let system = build_gen3d_review_delta_system_instructions(job.review_appearance);
    let mut user_text = job.last_review_user_text.clone();
    user_text.push_str("\n\nYour previous response was invalid.\nError:\n");
    user_text.push_str(reason.trim());
    user_text.push_str("\n\nReturn corrected JSON ONLY. No markdown.\n");
    job.last_review_user_text = user_text.clone();

    let prefix = format!(
        "review{:02}_repair{:02}",
        job.auto_refine_passes_done.max(1),
        job.review_delta_repair_attempt
    );
    let images = job.last_review_inputs.clone();
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::ReviewDeltaV1),
        job.require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        images,
        run_dir,
        prefix,
    );

    true
}

fn retry_gen3d_plan(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: Gen3dSpeedMode,
    reason: &str,
) -> bool {
    if job.plan_attempt >= GEN3D_MAX_PLAN_RETRIES {
        return false;
    }

    job.plan_attempt += 1;
    warn!(
        "Gen3D: plan failed; retrying (attempt {}/{}) reason={}",
        job.plan_attempt + 1,
        GEN3D_MAX_PLAN_RETRIES + 1,
        truncate_for_ui(reason, 600)
    );

    job.reset_session();
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.motion_roles = None;
    job.motion_authoring = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    draft.defs.clear();

    workshop.error = None;
    workshop.status = format!(
        "Planning components… (attempt {}/{})\nImages: {}",
        job.plan_attempt + 1,
        GEN3D_MAX_PLAN_RETRIES + 1,
        job.user_images.len()
    );

    let Some(ai) = job.ai.clone() else {
        fail_job(workshop, job, "Internal error: missing AI config.");
        return true;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return true;
    };

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
        message: "Starting…".into(),
    }));
    job.shared_progress = Some(progress.clone());
    job.phase = Gen3dAiPhase::WaitingPlan;

    let system = build_gen3d_plan_system_instructions();
    let user_text =
        build_gen3d_plan_user_text(&job.user_prompt_raw, !job.user_images.is_empty(), speed);
    let prefix = format!("plan_retry{}", job.plan_attempt);
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::PlanV1),
        job.require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        run_dir,
        prefix,
    );

    true
}

fn poll_gen3d_parallel_components(
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    speed: Gen3dSpeedMode,
) {
    let total = job.planned_components.len();
    if total == 0 {
        fail_job(workshop, job, "Internal error: no planned components.");
        return;
    }

    // 1) Apply any completed component results.
    let mut i = 0usize;
    while i < job.component_in_flight.len() {
        let Some(result) = take_shared_result(&job.component_in_flight[i].shared_result) else {
            i += 1;
            continue;
        };

        let task = job.component_in_flight.swap_remove(i);
        let idx = task.idx;
        let component_name = job
            .planned_components
            .get(idx)
            .map(|c| c.name.clone())
            .unwrap_or_else(|| format!("component_{idx}"));

        match result {
            Ok(resp) => {
                debug!(
                    "Gen3D: component generation finished (idx={}, name={}, api={:?}, sent_images={})",
                    idx,
                    component_name,
                    resp.api,
                    task.sent_images
                );
                job.note_api_used(resp.api);
                if let Some(tokens) = resp.total_tokens {
                    job.add_tokens(tokens);
                }
                if let Some(flag) = resp.session.responses_supported {
                    job.session.responses_supported = Some(flag);
                }
                if let Some(flag) = resp.session.responses_continuation_supported {
                    job.session.responses_continuation_supported = Some(flag);
                }

                let ai = match parse::parse_ai_draft_from_text(&resp.text) {
                    Ok(ai) => ai,
                    Err(err) => {
                        if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D: component draft parse failed; retrying component {} (idx={}, attempt {}/{}) err={}",
                                component_name,
                                idx,
                                next + 1,
                                GEN3D_MAX_COMPONENT_RETRIES + 1,
                                truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        fail_job(workshop, job, err);
                        return;
                    }
                };

                let component_def = match job
                    .planned_components
                    .get(idx)
                    .ok_or_else(|| {
                        format!("Internal error: missing planned component for idx={idx}")
                    })
                    .and_then(|planned| {
                        convert::ai_to_component_def(planned, ai, job.artifact_dir())
                    }) {
                    Ok(def) => def,
                    Err(err) => {
                        if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                            let next = task.attempt + 1;
                            warn!(
                                "Gen3D: component draft conversion failed; retrying component {} (idx={}, attempt {}/{}) err={}",
                                component_name,
                                idx,
                                next + 1,
                                GEN3D_MAX_COMPONENT_RETRIES + 1,
                                truncate_for_ui(&err, 600),
                            );
                            if idx >= job.component_attempts.len() {
                                job.component_attempts
                                    .resize(job.planned_components.len(), 0);
                            }
                            job.component_attempts[idx] = next;
                            job.component_queue.insert(0, idx);
                            continue;
                        }
                        fail_job(workshop, job, err);
                        return;
                    }
                };

                if let Some(comp) = job.planned_components.get_mut(idx) {
                    comp.actual_size = Some(component_def.size);
                    comp.anchors = component_def.anchors.clone();
                }

                // Replace component def in-place.
                let target_id = component_def.object_id;
                if let Some(existing) = draft.defs.iter_mut().find(|d| d.object_id == target_id) {
                    let preserved_refs: Vec<ObjectPartDef> = existing
                        .parts
                        .iter()
                        .filter(|p| matches!(p.kind, ObjectPartKind::ObjectRef { .. }))
                        .cloned()
                        .collect();
                    let mut merged = component_def;
                    merged.parts.extend(preserved_refs);
                    *existing = merged;
                } else {
                    draft.defs.push(component_def);
                }

                if let Some(root_idx) = job
                    .planned_components
                    .iter()
                    .position(|c| c.attach_to.is_none())
                {
                    if let Err(err) = convert::resolve_planned_component_transforms(
                        &mut job.planned_components,
                        root_idx,
                    ) {
                        fail_job(workshop, job, err);
                        return;
                    }
                }
                convert::update_root_def_from_planned_components(
                    &job.planned_components,
                    &job.plan_collider,
                    draft,
                );
                write_gen3d_assembly_snapshot(job.artifact_dir(), &job.planned_components);
                job.assembly_rev = job.assembly_rev.saturating_add(1);
            }
            Err(err) => {
                if task.attempt < GEN3D_MAX_COMPONENT_RETRIES {
                    let next = task.attempt + 1;
                    warn!(
                        "Gen3D: component request failed; retrying component {} (idx={}, attempt {}/{}, sent_images={}) err={}",
                        component_name,
                        idx,
                        next + 1,
                        GEN3D_MAX_COMPONENT_RETRIES + 1,
                        task.sent_images,
                        truncate_for_ui(&err, 600),
                    );
                    if idx >= job.component_attempts.len() {
                        job.component_attempts
                            .resize(job.planned_components.len(), 0);
                    }
                    job.component_attempts[idx] = next;
                    job.component_queue.insert(0, idx);
                    continue;
                }
                fail_job(workshop, job, err);
                return;
            }
        }
    }

    // 2) Start new component requests up to the parallel limit.
    let mut parallel = job.max_parallel_components.max(1).min(total);
    // Some providers support `/responses` but do not support `previous_response_id` continuation.
    // When that support is unknown, "probe" with a single request first to avoid spamming 400s.
    if job.session.responses_previous_id.is_some()
        && job.session.responses_continuation_supported.is_none()
    {
        parallel = parallel.min(1);
    }
    while job.component_in_flight.len() < parallel && !job.component_queue.is_empty() {
        let idx = job.component_queue.remove(0);
        if idx >= job.planned_components.len() {
            continue;
        }

        let Some(ai) = job.ai.clone() else {
            fail_job(workshop, job, "Internal error: missing AI config.");
            return;
        };
        let Some(run_dir) = job.pass_dir.clone() else {
            fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
            return;
        };

        let attempt = *job.component_attempts.get(idx).unwrap_or(&0);
        let sent_images = !job.user_images.is_empty();
        let image_paths = if sent_images {
            job.user_images.clone()
        } else {
            Vec::new()
        };

        let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Starting…".into(),
        }));

        let system = build_gen3d_component_system_instructions();
        let user_text = build_gen3d_component_user_text(
            &job.user_prompt_raw,
            !job.user_images.is_empty(),
            speed,
            &job.assembly_notes,
            &job.planned_components,
            idx,
        );
        let prefix = if attempt == 0 {
            format!(
                "component{:02}_{}",
                idx + 1,
                job.planned_components[idx].name
            )
        } else {
            format!(
                "component{:02}_{}_retry{}",
                idx + 1,
                job.planned_components[idx].name,
                attempt
            )
        };
        let reasoning_effort = ai.model_reasoning_effort().to_string();
        spawn_gen3d_ai_text_thread(
            shared.clone(),
            progress.clone(),
            job.cancel_flag.clone(),
            job.session.clone(),
            Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
            job.require_structured_outputs,
            ai,
            reasoning_effort,
            system,
            user_text,
            image_paths,
            run_dir,
            prefix,
        );

        job.component_in_flight.push(Gen3dInFlightComponent {
            idx,
            attempt,
            sent_images,
            shared_result: shared,
            _progress: progress,
        });
    }

    // 3) Update status and complete if finished.
    let done = job
        .planned_components
        .iter()
        .filter(|c| c.actual_size.is_some())
        .count();
    let in_flight = job.component_in_flight.len();
    let pending = job.component_queue.len();
    workshop.status = format!(
        "Building components… ({done}/{total})\nIn flight: {in_flight} | pending: {pending}\nParallel: {parallel}"
    );

    if done == total && in_flight == 0 && pending == 0 {
        if job.auto_refine_passes_remaining > 0 {
            job.auto_refine_passes_remaining -= 1;
            job.auto_refine_passes_done += 1;
            job.phase = Gen3dAiPhase::CapturingReview;
            job.review_capture = None;
            job.review_static_paths.clear();
            job.motion_capture = None;
            job.capture_previews_only = false;
            workshop.status = format!(
                "Auto-reviewing assembly… (pass {}/{})",
                job.auto_refine_passes_done,
                job.auto_refine_passes_done + job.auto_refine_passes_remaining
            );
            if let Some(progress) = job.shared_progress.as_ref() {
                set_progress(progress, "Preparing review capture…");
            }
            append_gen3d_run_log(job.artifact_dir(), "auto_review_start");
        } else if job.pass_dir.is_some() {
            job.phase = Gen3dAiPhase::CapturingReview;
            job.review_capture = None;
            job.review_static_paths.clear();
            job.motion_capture = None;
            job.capture_previews_only = true;
            workshop.status = "Capturing preview renders…".into();
            append_gen3d_run_log(job.artifact_dir(), "capture_previews_start");
        } else {
            job.finish_run_metrics();
            job.running = false;
            job.build_complete = true;
            job.phase = Gen3dAiPhase::Idle;
            workshop.status =
                "Build finished. Orbit/zoom the preview. Click Build to start a new run.".into();
        }
    }
}

pub(super) fn fail_job(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob, err: impl Into<String>) {
    let err = err.into();
    error!("Gen3D: build failed: {}", truncate_for_ui(&err, 1200));
    if let Some(flag) = job.cancel_flag.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    job.cancel_flag = None;
    abort_pending_agent_tool_call(job, format!("Run failed: {err}"));
    workshop.error = Some(err);
    workshop.status = "Build failed.".into();
    job.finish_run_metrics();
    job.running = false;
    job.build_complete = false;
    job.phase = Gen3dAiPhase::Idle;
    job.plan_attempt = 0;
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.shared_progress = None;
    job.shared_result = None;
    job.review_capture = None;
    job.capture_previews_only = false;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.replan_attempts = 0;
    job.regen_total = 0;
    job.regen_per_component.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
}

fn abort_pending_agent_tool_call(job: &mut Gen3dAiJob, reason: String) {
    let Some(call) = job.agent.pending_tool_call.take() else {
        return;
    };
    let result = crate::gen3d::agent::Gen3dToolResultJsonV1::err(
        call.call_id.clone(),
        call.tool_id.clone(),
        reason.clone(),
    );
    append_gen3d_jsonl_artifact(
        job.artifact_dir(),
        "tool_results.jsonl",
        &serde_json::to_value(&result).unwrap_or(serde_json::Value::Null),
    );
    append_gen3d_run_log(
        job.artifact_dir(),
        format!(
            "tool_call_aborted call_id={} tool_id={} reason={}",
            call.call_id,
            call.tool_id,
            truncate_for_ui(reason.trim(), 360)
        ),
    );
    job.agent.step_tool_results.push(result);
    job.agent.pending_llm_tool = None;
    job.agent.pending_component_batch = None;
    job.agent.pending_render = None;
    job.agent.pending_pass_snapshot = None;
    job.agent.pending_after_pass_snapshot = None;
}

pub(super) fn finish_job_best_effort(
    commands: &mut Commands,
    review_cameras: &Query<Entity, With<Gen3dReviewCaptureCamera>>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    reason: String,
) {
    warn!(
        "Gen3D: stopping run due to budget: {}",
        truncate_for_ui(&reason, 800)
    );
    if let Some(flag) = job.cancel_flag.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    job.cancel_flag = None;
    append_gen3d_run_log(
        job.artifact_dir(),
        format!("budget_stop reason={}", truncate_for_ui(&reason, 600)),
    );
    abort_pending_agent_tool_call(job, format!("Run stopped (best effort): {reason}"));

    workshop.error = None;
    workshop.status = format!(
        "Build finished (best effort).\nReason: {}\nYou can Save this draft or click Build to start a new run.",
        truncate_for_ui(&reason, 600)
    );

    for entity in review_cameras {
        commands.entity(entity).try_despawn();
    }
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.capture_previews_only = false;

    job.finish_run_metrics();
    job.running = false;
    job.build_complete = true;
    job.phase = Gen3dAiPhase::Idle;
    job.shared_progress = None;
    job.shared_result = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.component_attempts.clear();
    job.component_in_flight.clear();
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
}

fn resume_after_per_component_review(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob) {
    let Some(resume) = job.per_component_resume.take() else {
        return;
    };

    job.component_queue = resume.component_queue;
    job.component_queue_pos = resume.component_queue_pos;
    job.generation_kind = resume.generation_kind;
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;

    let next_pos = job.component_queue_pos;
    if next_pos >= job.component_queue.len() {
        job.finish_run_metrics();
        job.running = false;
        job.build_complete = true;
        job.phase = Gen3dAiPhase::Idle;
        job.shared_progress = None;
        workshop.status =
            "Build finished. Orbit/zoom the preview. Click Build to start a new run.".into();
        return;
    }

    let Some(ai) = job.ai.clone() else {
        fail_job(workshop, job, "Internal error: missing AI config.");
        return;
    };
    let Some(run_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return;
    };

    let next_idx = job.component_queue[next_pos];
    job.phase = Gen3dAiPhase::WaitingComponent;
    let comp = &job.planned_components[next_idx];
    let forward = comp.rot * Vec3::Z;
    let up = comp.rot * Vec3::Y;
    workshop.status = format!(
        "Building components… ({}/{})\nComponent: {}\nPlacement: pos=[{:.2},{:.2},{:.2}], forward=[{:.2},{:.2},{:.2}], up=[{:.2},{:.2},{:.2}]",
        next_pos + 1,
        job.component_queue.len(),
        comp.display_name,
        comp.pos.x,
        comp.pos.y,
        comp.pos.z,
        forward.x,
        forward.y,
        forward.z,
        up.x,
        up.y,
        up.z,
    );

    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();

    let system = build_gen3d_component_system_instructions();
    let user_text = build_gen3d_component_user_text(
        &job.user_prompt_raw,
        !job.user_images.is_empty(),
        workshop.speed_mode,
        &job.assembly_notes,
        &job.planned_components,
        next_idx,
    );
    let prefix = format!(
        "component{:02}_{}",
        next_idx + 1,
        job.planned_components[next_idx].name
    );
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::ComponentDraftV1),
        job.require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        run_dir,
        prefix,
    );
}

fn try_start_gen3d_replan(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    reason: String,
) -> bool {
    let max_replans = config.gen3d_max_replans;
    if job.replan_attempts >= max_replans {
        debug!("Gen3D: replan requested, but max attempts reached; ignoring.");
        return false;
    }
    job.replan_attempts += 1;

    let Some(ai) = job.ai.clone() else {
        fail_job(workshop, job, "Internal error: missing AI config.");
        return true;
    };
    let Some(run_dir) = job.run_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D cache dir.");
        return true;
    };

    debug!(
        "Gen3D: starting replan attempt {}: {}",
        job.replan_attempts, reason
    );

    let next_attempt = job.replan_attempts;
    if let Err(err) = gen3d_set_current_attempt_pass(job, &run_dir, next_attempt, 0) {
        fail_job(workshop, job, err);
        return true;
    }
    let attempt_dir = gen3d_attempt_dir(&run_dir, next_attempt);
    let cached_inputs = cache_gen3d_inputs(&attempt_dir, &job.user_prompt_raw, &job.user_images);
    job.user_images = cached_inputs.cached_image_paths;

    // Reset build state but keep the same run/session/tokens.
    job.review_kind = Gen3dAutoReviewKind::EndOfRun;
    job.review_component_idx = None;
    job.auto_refine_passes_done = 0;
    job.auto_refine_passes_remaining = refine_passes_for_speed(config, workshop.speed_mode);
    job.per_component_refine_passes_remaining = 0;
    job.per_component_refine_passes_done = 0;
    job.per_component_resume = None;
    job.planned_components.clear();
    job.assembly_notes.clear();
    job.plan_collider = None;
    job.rig_move_cycle_m = None;
    job.motion_roles = None;
    job.motion_authoring = None;
    job.reuse_groups.clear();
    job.reuse_group_warnings.clear();
    job.pending_plan = None;
    job.component_queue.clear();
    job.component_queue_pos = 0;
    job.generation_kind = Gen3dComponentGenerationKind::Initial;
    job.review_capture = None;
    job.review_static_paths.clear();
    job.motion_capture = None;
    job.plan_hash.clear();
    job.preserve_existing_components_mode = false;
    job.assembly_rev = 0;
    job.last_review_inputs.clear();
    job.last_review_user_text.clear();
    job.review_delta_repair_attempt = 0;
    job.regen_per_component.clear();
    draft.defs.clear();

    workshop.error = None;
    workshop.status = format!(
        "Re-planning components…\nReason: {}\nService: {}\nModel: {}\nImages: {}",
        reason,
        ai.service_label(),
        ai.model(),
        job.user_images.len()
    );

    job.phase = Gen3dAiPhase::WaitingPlan;
    let shared: SharedResult<Gen3dAiTextResponse, String> = new_shared_result();
    job.shared_result = Some(shared.clone());
    let progress = job
        .shared_progress
        .get_or_insert_with(|| Arc::new(Mutex::new(Gen3dAiProgress::default())))
        .clone();
    set_progress(&progress, "Starting re-plan…");

    let system = build_gen3d_plan_system_instructions();
    let mut user_text = build_gen3d_plan_user_text(
        &job.user_prompt_raw,
        !job.user_images.is_empty(),
        workshop.speed_mode,
    );
    user_text.push_str("\n\nReplan requested by reviewer.\nReason:\n");
    user_text.push_str(reason.trim());
    user_text.push('\n');
    let prefix = format!("attempt{:02}_plan", job.attempt);
    let Some(pass_dir) = job.pass_dir.clone() else {
        fail_job(workshop, job, "Internal error: missing Gen3D pass dir.");
        return true;
    };
    let reasoning_effort = ai.model_reasoning_effort().to_string();
    spawn_gen3d_ai_text_thread(
        shared,
        progress,
        job.cancel_flag.clone(),
        job.session.clone(),
        Some(Gen3dAiJsonSchemaKind::PlanV1),
        job.require_structured_outputs,
        ai,
        reasoning_effort,
        system,
        user_text,
        job.user_images.clone(),
        pass_dir,
        prefix,
    );

    true
}

fn default_gen3d_cache_dir() -> PathBuf {
    crate::paths::default_gen3d_cache_dir()
}

fn gen3d_make_run_dir(config: &AppConfig) -> (Uuid, PathBuf) {
    let base = config
        .gen3d_cache_dir
        .as_ref()
        .cloned()
        .unwrap_or_else(default_gen3d_cache_dir);
    let run_id = Uuid::new_v4();
    (run_id, base.join(run_id.to_string()))
}

fn gen3d_attempt_dir(run_dir: &Path, attempt: u32) -> PathBuf {
    run_dir.join(format!("attempt_{attempt}"))
}

fn gen3d_set_current_attempt_pass(
    job: &mut Gen3dAiJob,
    run_dir: &Path,
    attempt: u32,
    pass: u32,
) -> Result<(), String> {
    let attempt_dir = gen3d_attempt_dir(run_dir, attempt);
    std::fs::create_dir_all(&attempt_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D attempt dir {}: {err}",
            attempt_dir.display()
        )
    })?;
    let pass_dir = attempt_dir.join(format!("pass_{pass}"));
    std::fs::create_dir_all(&pass_dir).map_err(|err| {
        format!(
            "Failed to create Gen3D pass dir {}: {err}",
            pass_dir.display()
        )
    })?;

    job.attempt = attempt;
    job.pass = pass;
    job.pass_dir = Some(pass_dir.clone());

    if let Some(sinks) = job.log_sinks.as_ref() {
        if let Err(err) = sinks.start_gen3d_pass_log(pass_dir.join("gravimera.log")) {
            warn!("Gen3D: failed to start per-pass log capture: {err}");
        }
    }

    write_gen3d_json_artifact(
        Some(&pass_dir),
        "pass.json",
        &serde_json::json!({
            "version": 1,
            "attempt": attempt,
            "pass": pass,
            "created_at_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0),
        }),
    );

    job.metrics.note_pass_started(pass);

    Ok(())
}

pub(super) fn gen3d_advance_pass(job: &mut Gen3dAiJob) -> Result<(), String> {
    let run_dir = job
        .run_dir
        .clone()
        .ok_or_else(|| "Internal error: missing Gen3D run dir.".to_string())?;
    let next = job.pass.saturating_add(1);
    gen3d_set_current_attempt_pass(job, &run_dir, job.attempt, next)
}

#[derive(Clone, Debug)]
struct Gen3dCachedInputs {
    cached_image_paths: Vec<PathBuf>,
}

fn cache_gen3d_inputs(
    attempt_dir: &Path,
    prompt_raw: &str,
    image_paths: &[PathBuf],
) -> Gen3dCachedInputs {
    let inputs_dir = attempt_dir.join("inputs");
    let images_dir = inputs_dir.join("images");
    if let Err(err) = std::fs::create_dir_all(&images_dir) {
        debug!(
            "Gen3D: failed to create inputs dir {}: {err}",
            images_dir.display()
        );
    }

    let prompt_path = inputs_dir.join("user_prompt.txt");
    if let Err(err) = std::fs::write(&prompt_path, prompt_raw) {
        debug!(
            "Gen3D: failed to write prompt {}: {err}",
            prompt_path.display()
        );
    }

    let mut cached_image_paths = Vec::with_capacity(image_paths.len());
    let mut manifest_images: Vec<serde_json::Value> = Vec::with_capacity(image_paths.len());

    for (idx, src) in image_paths.iter().enumerate() {
        let file_name = src.file_name().and_then(|s| s.to_str()).unwrap_or("image");
        let sanitized = file_name
            .chars()
            .map(|ch| if ch == '/' || ch == '\\' { '_' } else { ch })
            .collect::<String>();
        let dst_name = format!("{:02}_{sanitized}", idx + 1);
        let dst = images_dir.join(dst_name);
        let copied = match std::fs::copy(src, &dst) {
            Ok(bytes) => {
                cached_image_paths.push(dst.clone());
                manifest_images.push(serde_json::json!({
                    "index": idx + 1,
                    "original_path": src.display().to_string(),
                    "cached_path": dst.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string(),
                    "bytes": bytes,
                }));
                true
            }
            Err(err) => {
                debug!(
                    "Gen3D: failed to cache input image {}: {err}",
                    src.display()
                );
                cached_image_paths.push(src.clone());
                manifest_images.push(serde_json::json!({
                    "index": idx + 1,
                    "original_path": src.display().to_string(),
                    "cached_path": null,
                    "error": err.to_string(),
                }));
                false
            }
        };
        if copied {
            debug!(
                "Gen3D: cached input image {}/{} to {}",
                idx + 1,
                image_paths.len(),
                dst.display()
            );
        }
    }

    let manifest = serde_json::json!({
        "version": 1,
        "user_prompt_file": "inputs/user_prompt.txt",
        "images_dir": "inputs/images",
        "images": manifest_images,
    });
    write_gen3d_json_artifact(Some(attempt_dir), "inputs_manifest.json", &manifest);

    Gen3dCachedInputs { cached_image_paths }
}

pub(super) fn start_gen3d_review_capture(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    run_dir: &Path,
    draft: &Gen3dDraft,
    include_overlay: bool,
    file_prefix: &str,
    views: &[Gen3dReviewView],
    width_px: u32,
    height_px: u32,
) -> Result<Gen3dReviewCaptureState, String> {
    let Some(root) = draft.root_def() else {
        return Err("Internal error: missing Gen3D draft root.".into());
    };

    let focus = super::super::preview::compute_draft_focus(draft);
    let half_extents = root.size.abs().max(Vec3::splat(0.01)) * 0.5;
    let aspect = width_px.max(1) as f32 / height_px.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;

    // Use a slightly above-the-object pitch for the horizontal ring views.
    // `GEN3D_PREVIEW_DEFAULT_PITCH` is used by the interactive preview and may be negative.
    let base_pitch = -GEN3D_PREVIEW_DEFAULT_PITCH.abs();

    if views.is_empty() {
        return Err("Internal error: no review views requested.".into());
    }

    // Pick a single distance that fits all views. This keeps scales comparable across screenshots,
    // and makes the object fill more of the frame than the previous overly conservative formula.
    let base_distance = views
        .iter()
        .map(|view| {
            let (yaw, pitch) = view.orbit_angles(base_pitch);
            crate::orbit_capture::required_distance_for_view(
                half_extents,
                yaw,
                pitch,
                fov_y,
                aspect,
                near,
            )
        })
        .fold(0.0f32, f32::max);
    let margin = if include_overlay { 1.15 } else { 1.08 };
    let distance = (base_distance * margin).clamp(near + 0.2, 250.0);

    let progress = Arc::new(Mutex::new(Gen3dReviewCaptureProgress {
        expected: views.len(),
        completed: 0,
    }));

    let mut cameras = Vec::with_capacity(views.len());
    let mut image_paths = Vec::with_capacity(views.len());

    for &view in views {
        let target = crate::orbit_capture::create_render_target(images, width_px, height_px);
        let (yaw, pitch) = view.orbit_angles(base_pitch);
        let transform = crate::orbit_capture::orbit_transform(yaw, pitch, distance, focus);

        let render_layers = if include_overlay {
            bevy::camera::visibility::RenderLayers::from_layers(&[
                GEN3D_PREVIEW_LAYER,
                GEN3D_REVIEW_LAYER,
            ])
        } else {
            bevy::camera::visibility::RenderLayers::layer(GEN3D_PREVIEW_LAYER)
        };

        let camera = commands
            .spawn((
                Camera3d::default(),
                bevy::camera::Projection::Perspective(projection.clone()),
                Camera {
                    clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                    ..default()
                },
                RenderTarget::Image(target.clone().into()),
                Tonemapping::TonyMcMapface,
                render_layers,
                transform,
                Gen3dReviewCaptureCamera,
            ))
            .id();
        cameras.push(camera);

        let path = run_dir.join(format!("{file_prefix}_{}.png", view.file_stem()));
        image_paths.push(path.clone());

        let progress_clone = progress.clone();
        commands
            .spawn(Screenshot::image(target))
            .observe(move |event: On<ScreenshotCaptured>| {
                let mut saver = save_to_disk(path.clone());
                saver(event);
                if let Ok(mut guard) = progress_clone.lock() {
                    guard.completed = guard.completed.saturating_add(1);
                }
            });
    }

    Ok(Gen3dReviewCaptureState {
        cameras,
        image_paths,
        progress,
    })
}

fn write_gen3d_sprite_sheet_2x2(sheet_path: &Path, frames: &[PathBuf]) -> Result<(), String> {
    use image::GenericImage;

    if frames.len() != 4 {
        return Err(format!(
            "Expected 4 frames for sprite sheet, got {}.",
            frames.len()
        ));
    }

    let imgs: Vec<image::RgbaImage> = frames
        .iter()
        .map(|path| {
            image::open(path)
                .map(|img| img.to_rgba8())
                .map_err(|err| format!("Failed to read frame {}: {err}", path.display()))
        })
        .collect::<Result<_, _>>()?;

    let (w, h) = imgs[0].dimensions();
    for (idx, img) in imgs.iter().enumerate().skip(1) {
        if img.dimensions() != (w, h) {
            return Err(format!(
                "Sprite sheet frame size mismatch at index {idx}: expected {w}x{h}, got {:?}.",
                img.dimensions()
            ));
        }
    }

    let mut sheet = image::RgbaImage::new(w.saturating_mul(2), h.saturating_mul(2));
    sheet
        .copy_from(&imgs[0], 0, 0)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[1], w, 0)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[2], 0, h)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;
    sheet
        .copy_from(&imgs[3], w, h)
        .map_err(|err| format!("Failed to compose sprite sheet: {err}"))?;

    image::DynamicImage::ImageRgba8(sheet)
        .save(sheet_path)
        .map_err(|err| {
            format!(
                "Failed to write sprite sheet {}: {err}",
                sheet_path.display()
            )
        })?;

    Ok(())
}

pub(super) fn poll_gen3d_motion_capture(
    time: &Time,
    commands: &mut Commands,
    images: &mut Assets<Image>,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
    draft: &Gen3dDraft,
    preview_model: &mut Query<
        (
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    let Some(pass_dir) = job.pass_dir.clone() else {
        debug!("Gen3D: motion capture skipped (missing pass dir).");
        job.motion_capture = None;
        return;
    };

    let mut iter = preview_model.iter_mut();
    let Some((mut channels, mut locomotion, mut attack_clock)) = iter.next() else {
        debug!("Gen3D: motion capture skipped (missing preview model root).");
        job.motion_capture = None;
        return;
    };

    let Some(motion) = job.motion_capture.as_mut() else {
        return;
    };

    const FRAMES: [f32; 4] = [0.0, 0.25, 0.5, 0.75];
    if motion.frame_idx as usize >= FRAMES.len() {
        motion.frame_idx = 0;
    }

    let total_frames = FRAMES.len() as u8;
    let frame_idx = motion.frame_idx.min(total_frames.saturating_sub(1));
    let sample_phase_01 = FRAMES[frame_idx as usize];

    fn infer_move_cycle_m(
        rig_move_cycle_m: Option<f32>,
        components: &[Gen3dPlannedComponent],
    ) -> f32 {
        if let Some(v) = rig_move_cycle_m
            .filter(|v| v.is_finite())
            .map(|v| v.abs())
            .filter(|v| *v > 1e-3)
        {
            return v;
        }

        let mut best: Option<f32> = None;
        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            let Some(slot) = att.animations.iter().find(|s| s.channel.as_ref() == "move") else {
                continue;
            };
            if !matches!(
                slot.spec.driver,
                PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
            ) {
                continue;
            }
            let (duration_secs, repeats) = match &slot.spec.clip {
                PartAnimationDef::Loop { duration_secs, .. }
                | PartAnimationDef::Once { duration_secs, .. } => (*duration_secs, 1.0),
                PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0),
                PartAnimationDef::Spin { .. } => continue,
            };
            if !duration_secs.is_finite() || duration_secs <= 0.0 {
                continue;
            }
            let speed_scale = slot.spec.speed_scale.max(1e-6);
            let effective = (repeats * duration_secs / speed_scale).abs();
            if !effective.is_finite() || effective <= 1e-3 {
                continue;
            }
            best = Some(best.map_or(effective, |b| b.max(effective)));
        }

        best.unwrap_or(1.0)
    }

    fn infer_attack_window_secs(draft: &Gen3dDraft, components: &[Gen3dPlannedComponent]) -> f32 {
        if let Some(v) = draft
            .root_def()
            .and_then(|def| def.attack.as_ref())
            .map(|a| a.anim_window_secs)
            .filter(|v| v.is_finite())
            .map(|v| v.abs())
            .filter(|v| *v > 1e-3)
        {
            return v;
        }

        let mut best: Option<f32> = None;
        for comp in components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            for slot in att.animations.iter() {
                if slot.channel.as_ref() != "attack_primary" {
                    continue;
                }
                let (duration_secs, repeats) = match &slot.spec.clip {
                    PartAnimationDef::Loop { duration_secs, .. }
                    | PartAnimationDef::Once { duration_secs, .. } => (*duration_secs, 1.0),
                    PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0),
                    PartAnimationDef::Spin { .. } => continue,
                };
                if !duration_secs.is_finite() || duration_secs <= 0.0 {
                    continue;
                }
                let speed = slot.spec.speed_scale.max(1e-3);
                let wall_duration = (repeats * duration_secs / speed).abs();
                if !wall_duration.is_finite() || wall_duration <= 1e-3 {
                    continue;
                }
                best = Some(best.map_or(wall_duration, |b| b.max(wall_duration)));
            }
        }

        best.unwrap_or(1.0)
    }

    if motion.frame_capture.is_none() {
        match motion.kind {
            Gen3dMotionCaptureKind::Move => {
                let cycle_m =
                    infer_move_cycle_m(job.rig_move_cycle_m, &job.planned_components).max(1e-3);
                let sample_m = (sample_phase_01 * cycle_m).clamp(0.0, cycle_m);
                channels.moving = true;
                channels.attacking_primary = false;
                locomotion.t = sample_m;
                locomotion.distance_m = sample_m;
                locomotion.signed_distance_m = sample_m;
                locomotion.speed_mps = 1.0;
            }
            Gen3dMotionCaptureKind::Attack => {
                let window_secs =
                    infer_attack_window_secs(draft, &job.planned_components).max(1e-3);
                let sample_secs = (sample_phase_01 * window_secs).clamp(0.0, window_secs);
                channels.moving = false;
                channels.attacking_primary = true;
                attack_clock.duration_secs = window_secs;
                attack_clock.started_at_secs = time.elapsed_secs() - sample_secs;
            }
        }

        let prefix = format!("{}_frame{:02}", motion.kind.label(), frame_idx + 1);
        if let Some(progress) = job.shared_progress.as_ref() {
            set_progress(
                progress,
                format!(
                    "Capturing {} animation… (frame {}/{})",
                    motion.kind.label(),
                    frame_idx + 1,
                    total_frames
                ),
            );
        }

        let views = [Gen3dReviewView::Front];
        match start_gen3d_review_capture(
            commands,
            images,
            &pass_dir,
            draft,
            false,
            &prefix,
            &views,
            GEN3D_REVIEW_CAPTURE_WIDTH_PX,
            GEN3D_REVIEW_CAPTURE_HEIGHT_PX,
        ) {
            Ok(state) => {
                motion.frame_capture = Some(state);
            }
            Err(err) => {
                warn!("Gen3D: motion capture failed to start: {err}");
                workshop.error = Some(err);
                job.motion_capture = None;
            }
        }
        return;
    }

    let Some(state) = motion.frame_capture.as_ref() else {
        return;
    };
    let (done, expected) = state
        .progress
        .lock()
        .map(|g| (g.completed, g.expected))
        .unwrap_or((0, 1));
    if done < expected {
        return;
    }

    // Clean up capture cameras.
    for cam in state.cameras.iter().copied() {
        commands.entity(cam).try_despawn();
    }
    let paths = state.image_paths.clone();
    motion.frame_capture = None;

    for path in &paths {
        if std::fs::metadata(path).is_err() {
            warn!("Gen3D: motion capture missing frame: {}", path.display());
            job.motion_capture = None;
            return;
        }
    }
    motion.frame_paths.extend(paths);

    motion.frame_idx = motion.frame_idx.saturating_add(1);
    if motion.frame_idx < total_frames {
        return;
    }

    let sheet_path = pass_dir.join(motion.kind.sheet_filename());
    if let Err(err) = write_gen3d_sprite_sheet_2x2(&sheet_path, &motion.frame_paths) {
        warn!("Gen3D: failed to compose {}: {err}", sheet_path.display());
    } else {
        job.review_static_paths.push(sheet_path);
    }

    // Prepare next sheet or finish.
    motion.frame_idx = 0;
    motion.frame_paths.clear();
    motion.frame_capture = None;
    motion.kind = match motion.kind {
        Gen3dMotionCaptureKind::Move => Gen3dMotionCaptureKind::Attack,
        Gen3dMotionCaptureKind::Attack => {
            channels.moving = false;
            channels.attacking_primary = false;
            job.motion_capture = None;
            return;
        }
    };
}

pub(super) fn compute_gen3d_plan_hash(
    assembly_notes: &str,
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
) -> String {
    let mut comps: Vec<&Gen3dPlannedComponent> = components.iter().collect();
    comps.sort_by(|a, b| a.name.cmp(&b.name));
    let comps_json: Vec<serde_json::Value> = comps
        .into_iter()
        .map(|c| {
            let anchors: Vec<serde_json::Value> = c
                .anchors
                .iter()
                .map(|a| {
                    let pos = a.transform.translation;
                    let q = a.transform.rotation.normalize();
                    serde_json::json!({
                        "name": a.name.as_ref(),
                        "pos": [pos.x, pos.y, pos.z],
                        "rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                    })
                })
                .collect();

            let contacts: Vec<serde_json::Value> = {
                let mut contacts: Vec<&AiContactJson> = c.contacts.iter().collect();
                contacts.sort_by(|a, b| a.name.cmp(&b.name));
                contacts
                    .into_iter()
                    .map(|contact| {
                        let stance = contact.stance.as_ref().map(|s| {
                            serde_json::json!({
                                "phase_01": s.phase_01,
                                "duty_factor_01": s.duty_factor_01,
                            })
                        });
                        serde_json::json!({
                            "name": contact.name.as_str(),
                            "kind": match contact.kind {
                                AiContactKindJson::Ground => "ground",
                                AiContactKindJson::Unknown => "unknown",
                            },
                            "anchor": contact.anchor.as_str(),
                            "stance": stance,
                        })
                    })
                    .collect()
            };

            let attach_to = c.attach_to.as_ref().map(|att| {
                let pos = att.offset.translation;
                let q = att.offset.rotation.normalize();
                let s = att.offset.scale;
                let channels: Vec<&str> = att
                    .animations
                    .iter()
                    .map(|slot| slot.channel.as_ref())
                    .collect();
                let joint = att.joint.as_ref().map(|j| {
                    serde_json::json!({
                        "kind": match j.kind {
                            AiJointKindJson::Fixed => "fixed",
                            AiJointKindJson::Hinge => "hinge",
                            AiJointKindJson::Ball => "ball",
                            AiJointKindJson::Free => "free",
                            AiJointKindJson::Unknown => "unknown",
                        },
                        "axis_join": j.axis_join,
                        "limits_degrees": j.limits_degrees,
                        "swing_limits_degrees": j.swing_limits_degrees,
                        "twist_limits_degrees": j.twist_limits_degrees,
                    })
                });
                serde_json::json!({
                    "parent": att.parent.as_str(),
                    "parent_anchor": att.parent_anchor.as_str(),
                    "child_anchor": att.child_anchor.as_str(),
                    "offset_pos": [pos.x, pos.y, pos.z],
                    "offset_rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                    "offset_scale": [s.x, s.y, s.z],
                    "joint": joint,
                    "animation_channels": channels,
                })
            });

            serde_json::json!({
                "name": c.name.as_str(),
                "purpose": c.purpose.as_str(),
                "modeling_notes": c.modeling_notes.as_str(),
                "planned_size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                "attach_to": attach_to,
                "anchors": anchors,
                "contacts": contacts,
            })
        })
        .collect();

    let plan_state = serde_json::json!({
        "version": 1,
        "assembly_notes": assembly_notes.trim(),
        "rig_move_cycle_m": rig_move_cycle_m,
        "components": comps_json,
    });
    let text = serde_json::to_string(&plan_state).unwrap_or_else(|_| plan_state.to_string());
    let digest = Sha256::digest(text.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for b in digest {
        hex.push_str(&format!("{:02x}", b));
    }
    format!("sha256:{hex}")
}

pub(super) fn build_gen3d_scene_graph_summary(
    run_id: &str,
    attempt: u32,
    pass: u32,
    plan_hash: &str,
    assembly_rev: u32,
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    fn anchor_frame_json(
        anchors: &[crate::object::registry::AnchorDef],
        name: &str,
    ) -> Option<(serde_json::Value, Transform)> {
        if name == "origin" {
            return Some((
                serde_json::json!({
                    "pos": [0.0, 0.0, 0.0],
                    "forward": [0.0, 0.0, 1.0],
                    "up": [0.0, 1.0, 0.0],
                }),
                Transform::IDENTITY,
            ));
        }
        let anchor = anchors.iter().find(|a| a.name.as_ref() == name)?;
        let pos = anchor.transform.translation;
        let forward = anchor.transform.rotation * Vec3::Z;
        let up = anchor.transform.rotation * Vec3::Y;
        Some((
            serde_json::json!({
                "pos": [pos.x, pos.y, pos.z],
                "forward": [forward.x, forward.y, forward.z],
                "up": [up.x, up.y, up.z],
            }),
            anchor.transform,
        ))
    }

    let mut name_to_component: std::collections::HashMap<&str, &Gen3dPlannedComponent> =
        std::collections::HashMap::new();
    for c in components.iter() {
        name_to_component.insert(c.name.as_str(), c);
    }

    let root = draft.root_def();
    let root_json = root.map(|root| {
        let collider = match root.collider {
            crate::object::registry::ColliderProfile::None => serde_json::json!({"kind":"none"}),
            crate::object::registry::ColliderProfile::CircleXZ { radius } => {
                serde_json::json!({"kind":"circle_xz","radius": radius})
            }
            crate::object::registry::ColliderProfile::AabbXZ { half_extents } => serde_json::json!({
                "kind":"aabb_xz",
                "half_extents":[half_extents.x, half_extents.y]
            }),
        };
        let mobility = root.mobility.as_ref().map(|m| {
            serde_json::json!({
                "kind": match m.mode {
                    crate::object::registry::MobilityMode::Ground => "ground",
                    crate::object::registry::MobilityMode::Air => "air",
                },
                "max_speed": m.max_speed,
            })
        });
        let attack = root.attack.as_ref().map(|a| {
            serde_json::json!({
                "kind": match a.kind {
                    crate::object::registry::UnitAttackKind::Melee => "melee",
                    crate::object::registry::UnitAttackKind::RangedProjectile => "ranged_projectile",
                },
                "cooldown_secs": a.cooldown_secs,
                "damage": a.damage,
                "anim_window_secs": a.anim_window_secs,
            })
        });
        let object_id_uuid = Uuid::from_u128(root.object_id).to_string();
        serde_json::json!({
            "object_id_uuid": object_id_uuid,
            "size": [root.size.x, root.size.y, root.size.z],
            "collider": collider,
            "mobility": mobility,
            "attack": attack,
        })
    });

    let components_json: Vec<serde_json::Value> = components
        .iter()
        .map(|c| {
            let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{}", c.name));
            let component_id_uuid = Uuid::from_u128(object_id).to_string();
            let forward = c.rot * Vec3::Z;
            let up = c.rot * Vec3::Y;
            let anchors: Vec<serde_json::Value> = c
                .anchors
                .iter()
                .map(|a| {
                    let pos = a.transform.translation;
                    let forward = a.transform.rotation * Vec3::Z;
                    let up = a.transform.rotation * Vec3::Y;
                    serde_json::json!({
                        "name": a.name.as_ref(),
                        "pos": [pos.x, pos.y, pos.z],
                        "forward": [forward.x, forward.y, forward.z],
                        "up": [up.x, up.y, up.z],
                    })
                })
                .collect();

            let attach_to = c.attach_to.as_ref().map(|att| {
                let parent_id =
                    builtin_object_id(&format!("gravimera/gen3d/component/{}", att.parent));
                let parent_component_id_uuid = Uuid::from_u128(parent_id).to_string();
                let parent_component = name_to_component.get(att.parent.as_str()).copied();
                let parent_anchor = parent_component
                    .and_then(|pc| anchor_frame_json(&pc.anchors, att.parent_anchor.as_str()));
                let child_anchor = anchor_frame_json(&c.anchors, att.child_anchor.as_str());
                let pos = att.offset.translation;
                let q = att.offset.rotation.normalize();
                let s = att.offset.scale;
                let forward = att.offset.rotation * Vec3::Z;
                let up = att.offset.rotation * Vec3::Y;
                let join_forward_world = parent_component
                    .zip(parent_anchor.as_ref())
                    .map(|(pc, (_, t))| pc.rot * (t.rotation * Vec3::Z));
                let join_up_world = parent_component
                    .zip(parent_anchor.as_ref())
                    .map(|(pc, (_, t))| pc.rot * (t.rotation * Vec3::Y));
                let join_right_world = join_up_world
                    .zip(join_forward_world)
                    .and_then(|(u, f)| {
                        let v = u.cross(f);
                        if !v.is_finite() || v.length_squared() <= 1e-6 {
                            None
                        } else {
                            Some(v.normalize())
                        }
                    });
                let joint = att.joint.as_ref().map(|joint| {
                    let mut json = serde_json::Map::new();
                    json.insert(
                        "kind".into(),
                        serde_json::Value::String(match joint.kind {
                            AiJointKindJson::Fixed => "fixed",
                            AiJointKindJson::Hinge => "hinge",
                            AiJointKindJson::Ball => "ball",
                            AiJointKindJson::Free => "free",
                            AiJointKindJson::Unknown => "unknown",
                        }
                        .to_string()),
                    );
                    if let Some(axis) = joint.axis_join {
                        json.insert(
                            "axis_join".into(),
                            serde_json::json!([axis[0], axis[1], axis[2]]),
                        );
                    }
                    if let Some(limits) = joint.limits_degrees {
                        json.insert(
                            "limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    if let Some(limits) = joint.swing_limits_degrees {
                        json.insert(
                            "swing_limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    if let Some(limits) = joint.twist_limits_degrees {
                        json.insert(
                            "twist_limits_degrees".into(),
                            serde_json::json!([limits[0], limits[1]]),
                        );
                    }
                    serde_json::Value::Object(json)
                });
                let animations: Vec<serde_json::Value> = att
                    .animations
                    .iter()
                    .map(|slot| {
                        let spec = &slot.spec;
                        let clip = match &spec.clip {
                            crate::object::registry::PartAnimationDef::Loop {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"loop",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::Once {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"once",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::PingPong {
                                duration_secs,
                                keyframes,
                            } => serde_json::json!({
                                "kind":"ping_pong",
                                "duration_secs": duration_secs,
                                "keyframes_count": keyframes.len(),
                                "keyframe_times": keyframes.iter().map(|k| k.time_secs).collect::<Vec<f32>>(),
                            }),
                            crate::object::registry::PartAnimationDef::Spin { axis, radians_per_unit } => {
                                serde_json::json!({
                                    "kind":"spin",
                                    "axis":[axis.x, axis.y, axis.z],
                                    "radians_per_unit": radians_per_unit,
                                })
                            }
                        };
                        serde_json::json!({
                            "channel": slot.channel.as_ref(),
                            "driver": match spec.driver {
                                crate::object::registry::PartAnimationDriver::Always => "always",
                                crate::object::registry::PartAnimationDriver::MovePhase => "move_phase",
                                crate::object::registry::PartAnimationDriver::MoveDistance => "move_distance",
                                crate::object::registry::PartAnimationDriver::AttackTime => "attack_time",
                            },
                            "speed_scale": spec.speed_scale,
                            "clip": clip,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "parent_component_id_uuid": parent_component_id_uuid,
                    "parent_component_name": att.parent.as_str(),
                    "parent_anchor": att.parent_anchor.as_str(),
                    "child_anchor": att.child_anchor.as_str(),
                    "parent_anchor_frame": parent_anchor.as_ref().map(|(json, _)| json.clone()),
                    "child_anchor_frame": child_anchor.as_ref().map(|(json, _)| json.clone()),
                    "join_forward_world": join_forward_world.map(|v| [v.x, v.y, v.z]),
                    "join_up_world": join_up_world.map(|v| [v.x, v.y, v.z]),
                    "join_right_world": join_right_world.map(|v| [v.x, v.y, v.z]),
                    "offset": {
                        "pos": [pos.x, pos.y, pos.z],
                        "forward": [forward.x, forward.y, forward.z],
                        "up": [up.x, up.y, up.z],
                        "rot_quat_xyzw": [q.x, q.y, q.z, q.w],
                        "scale": [s.x, s.y, s.z],
                    },
                    "joint": joint,
                    "animations": animations,
                })
            });

            let geometry = draft.defs.iter().find(|d| d.object_id == object_id).map(|def| {
                let geometry_parts: Vec<&crate::object::registry::ObjectPartDef> = def
                    .parts
                    .iter()
                    .filter(|p| {
                        !(matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::ObjectRef { .. }
                        ) && p.attachment.is_some())
                    })
                    .collect();

                if geometry_parts.len() == 1 {
                    let part = geometry_parts[0];
                    if part.attachment.is_none() {
                        if let crate::object::registry::ObjectPartKind::ObjectRef { object_id } =
                            &part.kind
                        {
                            let source_id = *object_id;
                            let source_uuid = Uuid::from_u128(source_id).to_string();
                            let source_name = components
                                .iter()
                                .find(|cmp| {
                                    builtin_object_id(&format!(
                                        "gravimera/gen3d/component/{}",
                                        cmp.name
                                    )) == source_id
                                })
                                .map(|cmp| cmp.name.as_str());
                            return serde_json::json!({
                                "kind": "linked_copy",
                                "source_component_id_uuid": source_uuid,
                                "source_component_name": source_name,
                            });
                        }
                    }
                }

                let primitive_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::Primitive { .. }
                        )
                    })
                    .count();
                let model_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(&p.kind, crate::object::registry::ObjectPartKind::Model { .. })
                    })
                    .count();
                let object_ref_parts = geometry_parts
                    .iter()
                    .filter(|p| {
                        matches!(
                            &p.kind,
                            crate::object::registry::ObjectPartKind::ObjectRef { .. }
                        )
                    })
                    .count();

                serde_json::json!({
                    "kind": if geometry_parts.is_empty() { "empty" } else { "geometry" },
                    "parts_total": def.parts.len(),
                    "geometry_parts": geometry_parts.len(),
                    "primitive_parts": primitive_parts,
                    "model_parts": model_parts,
                    "object_ref_parts": object_ref_parts,
                })
            });

            serde_json::json!({
                "component_id_uuid": component_id_uuid,
                "name": c.name.as_str(),
                "generated": c.actual_size.is_some(),
                "planned_size": [c.planned_size.x, c.planned_size.y, c.planned_size.z],
                "actual_size": c.actual_size.map(|s| [s.x, s.y, s.z]),
                "resolved_transform": {
                    "pos": [c.pos.x, c.pos.y, c.pos.z],
                    "forward": [forward.x, forward.y, forward.z],
                    "up": [up.x, up.y, up.z],
                },
                "geometry": geometry,
                "anchors": anchors,
                "attach_to": attach_to,
            })
        })
        .collect();

    serde_json::json!({
        "version": 1,
        "run_id": run_id,
        "attempt": attempt,
        "pass": pass,
        "plan_hash": plan_hash,
        "assembly_rev": assembly_rev,
        "root": root_json,
        "components": components_json,
    })
}

pub(super) fn build_gen3d_smoke_results(
    raw_prompt: &str,
    has_images: bool,
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    let prompt = raw_prompt.trim().to_ascii_lowercase();
    let attack_required = prompt.contains("can attack")
        || prompt.contains("attackable")
        || prompt.contains("weapon")
        || prompt.contains("gun")
        || prompt.contains("shoot")
        || prompt.contains("spear")
        || prompt.contains("axe")
        || prompt.contains("bow");

    let root = draft.root_def();
    let mobility_present = root.and_then(|r| r.mobility.as_ref()).is_some();
    let attack_present = root.and_then(|r| r.attack.as_ref()).is_some();

    let mut issues: Vec<serde_json::Value> = Vec::new();
    if attack_required && (!mobility_present || !attack_present) {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Prompt implies the object should be attack-capable, but the draft has no mobility/attack profile.",
        }));
    }
    if attack_present && !mobility_present {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Draft has an attack profile but is not movable (missing mobility).",
        }));
    }

    for c in components {
        let component_id = Uuid::from_u128(builtin_object_id(&format!(
            "gravimera/gen3d/component/{}",
            c.name
        )))
        .to_string();
        if !c.pos.is_finite() || !c.rot.is_finite() || !c.planned_size.is_finite() {
            issues.push(serde_json::json!({
                "severity":"error",
                "component_id": component_id.as_str(),
                "component": c.name.as_str(),
                "message":"Component has non-finite transform or size.",
            }));
        }
        if let Some(att) = c.attach_to.as_ref() {
            if !att.offset.translation.is_finite()
                || !att.offset.rotation.is_finite()
                || !att.offset.scale.is_finite()
            {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "component_id": component_id.as_str(),
                    "component": c.name.as_str(),
                    "message":"Attachment offset has non-finite values.",
                }));
            }

            // Animation sanity: for most attached spinning parts (wheels, propellers, turrets),
            // the spin axis should be the attachment direction (+Z in the join frame). If this is
            // wrong, the part will visibly spin around a strange axis.
            for slot in att.animations.iter() {
                let crate::object::registry::PartAnimationDef::Spin { axis, .. } = &slot.spec.clip
                else {
                    continue;
                };
                let axis = *axis;
                if !axis.is_finite() || axis.length_squared() <= 1e-6 {
                    continue;
                }
                let axis = axis.normalize();
                let align = axis.dot(Vec3::Z).abs();
                if align < 0.7 {
                    // Provide a robust suggestion: in component-local space, set the spin axis to
                    // the child anchor's forward vector (attachment direction).
                    let child_forward = if att.child_anchor == "origin" {
                        Vec3::Z
                    } else {
                        c.anchors
                            .iter()
                            .find(|a| a.name.as_ref() == att.child_anchor)
                            .map(|a| a.transform.rotation * Vec3::Z)
                            .unwrap_or(Vec3::Z)
                    };
                    issues.push(serde_json::json!({
                        "severity":"warn",
                        "component_id": component_id.as_str(),
                        "component": c.name.as_str(),
                        "channel": slot.channel.as_ref(),
                        "message":"Spin axis is not aligned with the attachment direction (+Z in the join frame). This often makes wheels/props/turrets spin around the wrong axis.",
                        "suggested_component_local_axis": [child_forward.x, child_forward.y, child_forward.z],
                    }));
                }
            }
        }
        for a in c.anchors.iter() {
            if !a.transform.translation.is_finite() || !a.transform.rotation.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "component_id": component_id.as_str(),
                    "component": c.name.as_str(),
                    "anchor": a.name.as_ref(),
                    "message":"Anchor has non-finite transform.",
                }));
            }
        }
    }

    let ok = issues
        .iter()
        .all(|i| i.get("severity").and_then(|v| v.as_str()) != Some("error"));

    let motion_report =
        motion_validation::build_motion_validation_report(rig_move_cycle_m, components);
    let motion_ok = motion_report
        .motion_validation
        .get("ok")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let ok = ok && motion_ok;

    serde_json::json!({
        "version": 1,
        "has_images": has_images,
        "attack_required_by_prompt": attack_required,
        "mobility_present": mobility_present,
        "attack_present": attack_present,
        "components_total": components.len(),
        "components_generated": components.iter().filter(|c| c.actual_size.is_some()).count(),
        "draft_defs": draft.defs.len(),
        "rig_summary": motion_report.rig_summary,
        "motion_validation": motion_report.motion_validation,
        "issues": issues,
        "ok": ok,
    })
}

pub(super) fn build_gen3d_validate_results(
    components: &[Gen3dPlannedComponent],
    draft: &Gen3dDraft,
) -> serde_json::Value {
    use crate::object::registry::ObjectPartKind;

    let mut issues: Vec<serde_json::Value> = Vec::new();

    let root_id = gen3d_draft_object_id();
    let root_present = draft.defs.iter().any(|d| d.object_id == root_id);
    if !root_present {
        issues.push(serde_json::json!({
            "severity":"error",
            "message":"Draft is missing the Gen3D root object def.",
        }));
    }

    let mut seen_ids: std::collections::HashSet<u128> = std::collections::HashSet::new();
    for def in &draft.defs {
        if !seen_ids.insert(def.object_id) {
            issues.push(serde_json::json!({
                "severity":"error",
                "object_id": format!("{:#x}", def.object_id),
                "message":"Duplicate object_id in draft defs.",
            }));
        }
        if !def.size.is_finite() || def.size.abs().max_element() <= 1e-6 {
            issues.push(serde_json::json!({
                "severity":"error",
                "object_id": format!("{:#x}", def.object_id),
                "label": def.label.as_ref(),
                "message":"ObjectDef.size is non-finite or near-zero.",
            }));
        }

        let mut anchor_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for a in &def.anchors {
            let name = a.name.as_ref().trim();
            if name.is_empty() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "message":"Anchor has empty name.",
                }));
                continue;
            }
            if !anchor_names.insert(name.to_string()) {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "anchor": name,
                    "message":"Duplicate anchor name on ObjectDef.",
                }));
            }
            if !a.transform.translation.is_finite() || !a.transform.rotation.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "anchor": name,
                    "message":"Anchor transform is non-finite.",
                }));
            }
        }
    }

    let defs_map: std::collections::HashMap<u128, &crate::object::registry::ObjectDef> =
        draft.defs.iter().map(|d| (d.object_id, d)).collect();
    for def in &draft.defs {
        let mut anchor_names: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for a in &def.anchors {
            anchor_names.insert(a.name.as_ref());
        }

        for (idx, part) in def.parts.iter().enumerate() {
            let t = part.transform;
            if !t.translation.is_finite() || !t.rotation.is_finite() || !t.scale.is_finite() {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "message":"Part transform has non-finite values.",
                }));
            }
            if t.scale.abs().max_element() <= 1e-6 {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "message":"Part scale is near-zero.",
                }));
            }

            let ObjectPartKind::ObjectRef {
                object_id: child_id,
            } = &part.kind
            else {
                continue;
            };
            let child_id = *child_id;
            let Some(child_def) = defs_map.get(&child_id).copied() else {
                issues.push(serde_json::json!({
                    "severity":"error",
                    "object_id": format!("{:#x}", def.object_id),
                    "label": def.label.as_ref(),
                    "part_index": idx,
                    "missing_child_object_id": format!("{:#x}", child_id),
                    "message":"ObjectRef points at a missing object def.",
                }));
                continue;
            };

            if let Some(att) = part.attachment.as_ref() {
                let parent_anchor = att.parent_anchor.as_ref();
                let child_anchor = att.child_anchor.as_ref();
                if parent_anchor != "origin" && !anchor_names.contains(parent_anchor) {
                    issues.push(serde_json::json!({
                        "severity":"error",
                        "object_id": format!("{:#x}", def.object_id),
                        "label": def.label.as_ref(),
                        "part_index": idx,
                        "parent_anchor": parent_anchor,
                        "message":"Attachment references a missing parent_anchor.",
                    }));
                }
                if child_anchor != "origin"
                    && !child_def
                        .anchors
                        .iter()
                        .any(|a| a.name.as_ref() == child_anchor)
                {
                    issues.push(serde_json::json!({
                        "severity":"error",
                        "object_id": format!("{:#x}", def.object_id),
                        "label": def.label.as_ref(),
                        "part_index": idx,
                        "child_object_id": format!("{:#x}", child_id),
                        "child_anchor": child_anchor,
                        "message":"Attachment references a missing child_anchor.",
                    }));
                }
            }
        }
    }

    // Ensure planned components point at existing defs (useful for diagnosing plan/draft mismatches).
    for c in components {
        let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{}", c.name));
        if !defs_map.contains_key(&object_id) {
            issues.push(serde_json::json!({
                "severity":"warn",
                "component": c.name.as_str(),
                "message":"Planned component has no matching object def in draft.",
            }));
        }
    }

    let ok = issues
        .iter()
        .all(|i| i.get("severity").and_then(|v| v.as_str()) != Some("error"));

    serde_json::json!({
        "version": 1,
        "draft_defs": draft.defs.len(),
        "components_total": components.len(),
        "components_generated": components.iter().filter(|c| c.actual_size.is_some()).count(),
        "issues": issues,
        "ok": ok,
    })
}

pub(super) fn spawn_gen3d_ai_text_thread(
    shared: SharedResult<Gen3dAiTextResponse, String>,
    progress: Arc<Mutex<Gen3dAiProgress>>,
    cancel: Option<Arc<AtomicBool>>,
    session: Gen3dAiSessionState,
    expected_schema: Option<Gen3dAiJsonSchemaKind>,
    require_structured_outputs: bool,
    ai: Gen3dAiServiceConfig,
    reasoning_effort: String,
    system_instructions: String,
    user_text: String,
    image_paths: Vec<PathBuf>,
    run_dir: PathBuf,
    prefix: String,
) {
    let service = ai.service_label();
    let model = ai.model().to_string();
    let base_url = ai.base_url().to_string();
    let run_dir_for_store = run_dir.clone();
    let prefix_for_store = prefix.clone();
    let thread_name = format!("gravimera_gen3d_ai_{prefix}");

    spawn_worker_thread(
        thread_name,
        shared,
        move || {
            let thread_id = std::thread::current().id();
            let started_at = std::time::Instant::now();
            append_gen3d_run_log(
                Some(&run_dir),
                format!(
                    "request_thread_started prefix={} service={} model={} images={} base_url={} reasoning_effort={} thread={:?}",
                    prefix,
                    service,
                    model,
                    image_paths.len(),
                    base_url,
                    reasoning_effort,
                    thread_id
                ),
            );
            debug!(
                "Gen3D: request started (prefix={}, service={}, model={}, images={}, base_url={}, cache_dir={}, thread={:?})",
                prefix,
                service,
                model,
                image_paths.len(),
                base_url,
                run_dir.display(),
                thread_id,
            );
            let result = generate_text_via_ai_service(
                &progress,
                session,
                cancel,
                expected_schema,
                require_structured_outputs,
                &ai,
                &reasoning_effort,
                &system_instructions,
                &user_text,
                &image_paths,
                Some(&run_dir),
                &prefix,
            );
            let elapsed_ms = started_at.elapsed().as_millis();
            append_gen3d_run_log(
                Some(&run_dir),
                format!(
                    "request_thread_ai_done prefix={} ok={} elapsed_ms={}",
                    prefix,
                    result.is_ok(),
                    elapsed_ms
                ),
            );
            debug!(
                "Gen3D: request thread AI done (prefix={}, ok={}, elapsed_ms={}, thread={:?})",
                prefix,
                result.is_ok(),
                elapsed_ms,
                thread_id,
            );
            result
        },
        move |metrics| {
            let thread_id = std::thread::current().id();
            append_gen3d_run_log(
                Some(&run_dir_for_store),
                format!(
                    "request_thread_shared_lock_acquired prefix={} wait_ms={}",
                    prefix_for_store, metrics.lock_wait_ms
                ),
            );
            if metrics.poisoned {
                warn!(
                    "Gen3D: shared_result lock poisoned; continuing (prefix={}, thread={:?})",
                    prefix_for_store, thread_id
                );
            }
            if metrics.lock_wait_ms >= 1_000 {
                warn!(
                    "Gen3D: shared_result lock wait high (prefix={}, wait_ms={}, thread={:?})",
                    prefix_for_store, metrics.lock_wait_ms, thread_id
                );
            } else {
                debug!(
                    "Gen3D: shared_result lock acquired (prefix={}, wait_ms={}, thread={:?})",
                    prefix_for_store, metrics.lock_wait_ms, thread_id
                );
            }
            append_gen3d_run_log(
                Some(&run_dir_for_store),
                format!("request_thread_shared_set prefix={}", prefix_for_store),
            );
        },
    )
    .expect("Failed to spawn Gen3D AI thread");
}

pub(super) fn spawn_prefab_descriptor_meta_enrichment_thread_best_effort(
    job: &Gen3dAiJob,
    descriptor_path: PathBuf,
    prefab_label: String,
    roles: Vec<String>,
    size_m: Vec3,
    ground_origin_y_m: f32,
    mobility: Option<String>,
    attack_kind: Option<String>,
    anchors: Vec<String>,
    animation_channels: Vec<String>,
    plan_extracted_text: Option<String>,
    motion_summary_json: Option<serde_json::Value>,
) {
    let Some(ai) = job.ai.clone() else {
        return;
    };

    let session = job.session.clone();
    let pass_dir = job.pass_dir.clone();
    let user_prompt = job.user_prompt_raw.clone();
    let require_structured_outputs = job.require_structured_outputs;

    std::thread::spawn(move || {
        let progress: Arc<Mutex<Gen3dAiProgress>> = Arc::new(Mutex::new(Gen3dAiProgress {
            message: "Generating prefab metadata…".into(),
        }));
        let system = super::prompts::build_gen3d_descriptor_meta_system_instructions();
        let user_text = super::prompts::build_gen3d_descriptor_meta_user_text(
            &prefab_label,
            &user_prompt,
            &roles,
            size_m,
            ground_origin_y_m,
            mobility.as_deref(),
            attack_kind.as_deref(),
            &anchors,
            &animation_channels,
            plan_extracted_text.as_deref(),
            motion_summary_json.as_ref(),
        );

        let reasoning_effort = openai::cap_reasoning_effort(ai.model_reasoning_effort(), "low");
        let resp = generate_text_via_ai_service(
            &progress,
            session,
            None,
            Some(Gen3dAiJsonSchemaKind::DescriptorMetaV1),
            require_structured_outputs,
            &ai,
            &reasoning_effort,
            &system,
            &user_text,
            &[],
            pass_dir.as_deref(),
            "descriptor_meta",
        );

        let meta = match resp {
            Ok(resp) => match parse::parse_ai_descriptor_meta_from_text(&resp.text) {
                Ok(meta) => meta,
                Err(err) => {
                    warn!("Gen3D: failed to parse descriptor-meta response: {err}");
                    return;
                }
            },
            Err(err) => {
                warn!("Gen3D: descriptor-meta request failed: {err}");
                return;
            }
        };

        let bytes = match std::fs::read(&descriptor_path) {
            Ok(b) => b,
            Err(err) => {
                warn!(
                    "Gen3D: descriptor-meta could not read {}: {err}",
                    descriptor_path.display()
                );
                return;
            }
        };
        let json: serde_json::Value = match serde_json::from_slice(&bytes) {
            Ok(v) => v,
            Err(err) => {
                warn!(
                    "Gen3D: descriptor-meta invalid JSON {}: {err}",
                    descriptor_path.display()
                );
                return;
            }
        };
        let mut doc: crate::prefab_descriptors::PrefabDescriptorFileV1 =
            match serde_json::from_value(json) {
                Ok(v) => v,
                Err(err) => {
                    warn!(
                        "Gen3D: descriptor-meta schema mismatch {}: {err}",
                        descriptor_path.display()
                    );
                    return;
                }
            };

        let mut should_update_short = true;
        if let Some(text) = doc.text.as_ref().and_then(|t| t.short.as_deref()) {
            if !text.trim().is_empty() {
                should_update_short = false;
                if let Some(prompt) = doc
                    .provenance
                    .as_ref()
                    .and_then(|p| p.gen3d.as_ref())
                    .and_then(|g| g.prompt.as_deref())
                {
                    if let Some(first_line) = prompt.lines().find(|l| !l.trim().is_empty()) {
                        if text.trim() == first_line.trim() {
                            should_update_short = true;
                        }
                    }
                }
            }
        }

        if should_update_short && !meta.short.trim().is_empty() {
            let text = doc.text.get_or_insert_with(Default::default);
            text.short = Some(meta.short.trim().to_string());
        }

        let mut merged_tags: Vec<String> = doc.tags;
        merged_tags.extend(meta.tags);
        doc.tags = merged_tags;

        if let Err(err) =
            crate::prefab_descriptors::save_prefab_descriptor_file(&descriptor_path, &doc)
        {
            warn!(
                "Gen3D: descriptor-meta failed to save {}: {err}",
                descriptor_path.display()
            );
        }
    });
}

pub(super) fn set_progress(progress: &Arc<Mutex<Gen3dAiProgress>>, message: impl Into<String>) {
    if let Ok(mut guard) = progress.lock() {
        guard.message = message.into();
    }
}

pub(super) fn truncate_for_ui(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::with_capacity(max_chars + 32);
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("…(truncated)");
    out
}

pub(super) fn record_gen3d_tooling_feedback(
    config: &AppConfig,
    workshop: &mut Gen3dWorkshop,
    history: &mut Gen3dToolFeedbackHistory,
    job: &Gen3dAiJob,
    feedbacks: &[AiToolingFeedbackJsonV1],
) {
    use bevy::log::{info, warn};

    let created_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let run_id = job.run_id.map(|id| id.to_string());
    let attempt = Some(job.attempt);
    let pass = Some(job.pass);
    let run_dir = job.run_dir.as_deref();
    let pass_dir = job.pass_dir.as_deref();

    for feedback in feedbacks {
        let priority = feedback.priority.trim();
        let priority = if priority.is_empty() {
            "medium".to_string()
        } else {
            priority.to_string()
        };
        let title = feedback.title.trim();
        let title = if title.is_empty() {
            "Tooling feedback".to_string()
        } else {
            title.to_string()
        };
        let summary = feedback.summary.trim();
        let summary = if summary.is_empty() {
            "No summary provided.".to_string()
        } else {
            summary.to_string()
        };

        let mut evidence_paths: Vec<String> = Vec::new();
        if let Some(dir) = run_dir {
            evidence_paths.push(dir.display().to_string());
            evidence_paths.push(dir.join("tool_feedback.jsonl").display().to_string());
        }
        if let Some(dir) = pass_dir {
            evidence_paths.push(dir.display().to_string());
            evidence_paths.push(dir.join("gen3d_run.log").display().to_string());
            evidence_paths.push(dir.join("gravimera.log").display().to_string());
            evidence_paths.push(dir.join("review_*.png").display().to_string());
        }
        evidence_paths.push(
            gen3d_tool_feedback_history_path(config)
                .display()
                .to_string(),
        );

        let raw = serde_json::to_value(feedback).unwrap_or(serde_json::Value::Null);

        let entry = Gen3dToolFeedbackEntry {
            version: 1,
            entry_id: Uuid::new_v4().to_string(),
            created_at_ms,
            run_id: run_id.clone(),
            attempt,
            pass,
            priority,
            title,
            summary,
            feedback: raw,
            evidence_paths,
        };

        let entry_priority = entry.priority.clone();
        let entry_title = entry.title.clone();
        let entry_summary = entry.summary.clone();
        let entry_id = entry.entry_id.clone();

        append_gen3d_tool_feedback_entry(config, run_dir, &entry);
        history.entries.push(entry);
        if matches!(workshop.side_tab, Gen3dSideTab::Status) {
            workshop.tool_feedback_unread = true;
        }

        // Codex-style developer breadcrumbs: surface tool feedback in terminal/logs.
        if let Some(pass_dir) = pass_dir {
            append_gen3d_run_log(
                Some(pass_dir),
                format!(
                    "tool_feedback_received priority={} title={:?} entry_id={} summary={:?}",
                    entry_priority,
                    entry_title.trim(),
                    entry_id,
                    entry_summary.trim()
                ),
            );
        } else if let Some(run_dir) = run_dir {
            // Best effort: if we don't have a pass_dir, at least write into the run root.
            append_gen3d_run_log(
                Some(run_dir),
                format!(
                    "tool_feedback_received priority={} title={:?} entry_id={} summary={:?}",
                    entry_priority,
                    entry_title.trim(),
                    entry_id,
                    entry_summary.trim()
                ),
            );
        }

        if entry_priority.trim().eq_ignore_ascii_case("high")
            || entry_priority.trim().eq_ignore_ascii_case("critical")
        {
            warn!(
                "Gen3D tooling feedback ({}) {}: {}",
                entry_priority,
                entry_title.trim(),
                entry_summary.trim()
            );
        } else {
            info!(
                "Gen3D tooling feedback ({}) {}: {}",
                entry_priority,
                entry_title.trim(),
                entry_summary.trim()
            );
        }
    }
}
