use bevy::prelude::*;
use std::borrow::Cow;
use std::time::{Duration, Instant};
use uuid::Uuid;

use crate::config::AppConfig;
use crate::model_library_ui::ModelLibraryUiState;
use crate::object::registry::{
    ColliderProfile, MaterialKey, MeshKey, ObjectDef, ObjectInteraction, ObjectPartDef,
    PrimitiveVisualDef,
};
use crate::prefab_descriptors::PrefabDescriptorLibrary;

use super::ai::Gen3dAiJob;
use super::save::{
    gen3d_request_prefab_thumbnail_capture_from_render_world,
    save_gen3d_snapshot_to_scene_and_library, Gen3dPrefabThumbnailCaptureRuntime,
    Gen3dSaveRenderWorld,
};
use super::state::{Gen3dDraft, Gen3dWorkshop};
use super::{
    gen3d_in_flight_label, remove_gen3d_in_flight_entry, upsert_gen3d_in_flight_entry,
    Gen3dInFlightStatus,
};

#[derive(Resource, Default)]
pub(crate) struct Gen3dMockJobManager {
    jobs: Vec<Gen3dMockJob>,
    active_run: Option<Uuid>,
}

struct Gen3dMockJob {
    run_id: Uuid,
    realm_id: String,
    scene_id: String,
    prompt: String,
    image_count: usize,
    created_at_ms: u128,
    status: Gen3dInFlightStatus,
    started_at: Option<Instant>,
    duration: Duration,
}

impl Gen3dMockJobManager {
    fn running_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| matches!(job.status, Gen3dInFlightStatus::Running))
            .count()
    }

    fn queued_indices_sorted(&self) -> Vec<usize> {
        let mut items: Vec<(usize, u128)> = self
            .jobs
            .iter()
            .enumerate()
            .filter(|(_idx, job)| matches!(job.status, Gen3dInFlightStatus::Queued))
            .map(|(idx, job)| (idx, job.created_at_ms))
            .collect();
        items.sort_by_key(|(_idx, created_at)| *created_at);
        items.into_iter().map(|(idx, _)| idx).collect()
    }

    fn remove_job(&mut self, run_id: Uuid) -> Option<Gen3dMockJob> {
        let index = self.jobs.iter().position(|job| job.run_id == run_id)?;
        Some(self.jobs.remove(index))
    }
}

pub(crate) fn gen3d_mock_select_active_run(
    manager: &mut Gen3dMockJobManager,
    run_id: Uuid,
) -> bool {
    if manager.jobs.iter().any(|job| job.run_id == run_id) {
        manager.active_run = Some(run_id);
        true
    } else {
        false
    }
}

pub(crate) fn gen3d_mock_cancel_run(
    manager: &mut Gen3dMockJobManager,
    realm_id: &str,
    run_id: Uuid,
) -> Result<bool, String> {
    if manager.remove_job(run_id).is_none() {
        return Ok(false);
    }
    if let Err(err) = remove_gen3d_in_flight_entry(realm_id, run_id) {
        return Err(err);
    }
    if manager.active_run == Some(run_id) {
        manager.active_run = None;
    }
    Ok(true)
}

pub(crate) fn gen3d_mock_enqueue_job(
    config: &AppConfig,
    manager: &mut Gen3dMockJobManager,
    realm_id: &str,
    scene_id: &str,
    prompt: &str,
    image_count: usize,
) -> Result<Uuid, String> {
    let run_id = Uuid::new_v4();
    let created_at_ms = now_ms();
    let running_slots = manager.running_count();
    let max_parallel = config.gen3d_max_parallel_jobs.max(1);

    let duration = Duration::from_secs(config.gen3d_mock_delay_seconds);

    let status = if running_slots < max_parallel {
        Gen3dInFlightStatus::Running
    } else {
        Gen3dInFlightStatus::Queued
    };

    let started_at = if matches!(status, Gen3dInFlightStatus::Running) {
        Some(Instant::now())
    } else {
        None
    };

    let label = gen3d_in_flight_label(prompt, image_count);
    upsert_gen3d_in_flight_entry(realm_id, run_id, label, status.clone(), None)?;

    manager.jobs.push(Gen3dMockJob {
        run_id,
        realm_id: realm_id.to_string(),
        scene_id: scene_id.to_string(),
        prompt: prompt.to_string(),
        image_count,
        created_at_ms,
        status,
        started_at,
        duration,
    });
    manager.active_run = Some(run_id);

    Ok(run_id)
}

pub(crate) fn gen3d_tick_mock_jobs(
    config: Res<AppConfig>,
    mut commands: Commands,
    mut render: Gen3dSaveRenderWorld,
    mut thumbnail_runtime: ResMut<Gen3dPrefabThumbnailCaptureRuntime>,
    mut library: ResMut<crate::object::registry::ObjectLibrary>,
    mut prefab_descriptors: ResMut<PrefabDescriptorLibrary>,
    mut model_library: ResMut<ModelLibraryUiState>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<Gen3dAiJob>,
    mut manager: ResMut<Gen3dMockJobManager>,
) {
    if !config.gen3d_mock_enabled {
        if job.is_mock_mode() {
            clear_active_mock_state(&mut workshop, &mut job);
        }
        return;
    }

    let mut changed = false;

    let max_parallel = config.gen3d_max_parallel_jobs.max(1);
    while manager.running_count() < max_parallel {
        let queued_indices = manager.queued_indices_sorted();
        let Some(next_idx) = queued_indices.first().copied() else {
            break;
        };
        let Some(next) = manager.jobs.get_mut(next_idx) else {
            break;
        };
        next.status = Gen3dInFlightStatus::Running;
        next.started_at = Some(Instant::now());
        if let Err(err) = upsert_gen3d_in_flight_entry(
            &next.realm_id,
            next.run_id,
            gen3d_in_flight_label(&next.prompt, next.image_count),
            Gen3dInFlightStatus::Running,
            None,
        ) {
            warn!("Failed to update Gen3D mock in-flight entry: {err}");
        } else {
            changed = true;
        }
    }

    let mut completed: Vec<Uuid> = Vec::new();
    for entry in manager.jobs.iter() {
        if !matches!(entry.status, Gen3dInFlightStatus::Running) {
            continue;
        }
        let Some(started_at) = entry.started_at else {
            continue;
        };
        if started_at.elapsed() >= entry.duration {
            completed.push(entry.run_id);
        }
    }

    for run_id in completed {
        let Some(entry) = manager.remove_job(run_id) else {
            continue;
        };
        match save_mock_prefab(
            &mut commands,
            &mut render,
            &mut thumbnail_runtime,
            &mut library,
            &mut prefab_descriptors,
            &entry,
        ) {
            Ok(_) => {
                if let Err(err) = remove_gen3d_in_flight_entry(&entry.realm_id, run_id) {
                    warn!("Failed to remove Gen3D mock in-flight entry: {err}");
                } else {
                    changed = true;
                }
            }
            Err(err) => {
                warn!("Gen3D mock save failed: {err}");
                if let Err(err) = upsert_gen3d_in_flight_entry(
                    &entry.realm_id,
                    run_id,
                    gen3d_in_flight_label(&entry.prompt, entry.image_count),
                    Gen3dInFlightStatus::Failed,
                    Some(err),
                ) {
                    warn!("Failed to mark Gen3D mock in-flight entry failed: {err}");
                } else {
                    changed = true;
                }
            }
        }

        if manager.active_run == Some(run_id) {
            manager.active_run = None;
            clear_active_mock_state(&mut workshop, &mut job);
        }
    }

    if let Some(active_run) = manager.active_run {
        if let Some(entry) = manager.jobs.iter().find(|job| job.run_id == active_run) {
            sync_active_mock_state(entry, &mut workshop, &mut job);
        } else {
            manager.active_run = None;
            clear_active_mock_state(&mut workshop, &mut job);
        }
    } else if job.is_mock_mode() {
        clear_active_mock_state(&mut workshop, &mut job);
    }

    if changed {
        model_library.mark_models_dirty();
    }
}

fn save_mock_prefab(
    commands: &mut Commands,
    render: &mut Gen3dSaveRenderWorld,
    thumbnail_runtime: &mut Gen3dPrefabThumbnailCaptureRuntime,
    library: &mut crate::object::registry::ObjectLibrary,
    prefab_descriptors: &mut PrefabDescriptorLibrary,
    entry: &Gen3dMockJob,
) -> Result<u128, String> {
    let label = gen3d_in_flight_label(&entry.prompt, entry.image_count);
    let root_label = if label.trim().is_empty() {
        "Mock prefab".to_string()
    } else {
        format!("Mock {label}")
    };

    let primitive = PrimitiveVisualDef::Mesh {
        mesh: MeshKey::UnitCube,
        material: MaterialKey::BuildBlock { index: 0 },
    };
    let root_def = ObjectDef {
        object_id: super::gen3d_draft_object_id(),
        label: Cow::Owned(root_label),
        size: Vec3::splat(1.0),
        ground_origin_y: Some(0.5),
        collider: ColliderProfile::AabbXZ {
            half_extents: Vec2::splat(0.5),
        },
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![ObjectPartDef::primitive(primitive, Transform::IDENTITY)],
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    };
    let draft = Gen3dDraft {
        defs: vec![root_def],
    };

    let mut job = Gen3dAiJob::default();
    job.set_run_id(entry.run_id);
    job.set_run_realm_id(entry.realm_id.clone());
    job.set_run_scene_id(entry.scene_id.clone());
    job.set_user_prompt_raw(entry.prompt.clone());
    job.set_plan_hash("mock".to_string());

    let mut workshop = Gen3dWorkshop::default();
    workshop.prompt = entry.prompt.clone();

    let (prefab_id, _def) = save_gen3d_snapshot_to_scene_and_library(
        &entry.realm_id,
        &entry.scene_id,
        library,
        Some(prefab_descriptors),
        &mut workshop,
        &mut job,
        &draft,
        false,
    )?;

    let thumbnail_path = crate::realm_prefab_packages::realm_prefab_package_thumbnail_path(
        &entry.realm_id,
        prefab_id,
    );

    if let Err(err) = gen3d_request_prefab_thumbnail_capture_from_render_world(
        commands,
        render,
        thumbnail_runtime,
        library,
        prefab_id,
        thumbnail_path,
    ) {
        warn!("Gen3D mock: thumbnail capture skipped: {err}");
    }

    Ok(prefab_id)
}

fn sync_active_mock_state(
    entry: &Gen3dMockJob,
    workshop: &mut Gen3dWorkshop,
    job: &mut Gen3dAiJob,
) {
    job.set_mock_mode(true);
    job.set_run_id(entry.run_id);
    job.set_run_realm_id(entry.realm_id.clone());
    job.set_run_scene_id(entry.scene_id.clone());
    job.set_user_prompt_raw(entry.prompt.clone());
    job.set_plan_hash("mock".to_string());
    job.set_running(matches!(
        entry.status,
        Gen3dInFlightStatus::Running | Gen3dInFlightStatus::Queued
    ));
    job.set_build_complete(false);

    workshop.prompt = entry.prompt.clone();
    workshop.error = None;

    let status = match entry.status {
        Gen3dInFlightStatus::Queued => "Mock queued…".to_string(),
        Gen3dInFlightStatus::Running => {
            let elapsed = entry
                .started_at
                .map(|start| start.elapsed())
                .unwrap_or_else(|| Duration::from_secs(0));
            let total = entry.duration.as_secs().max(1);
            let secs = elapsed.as_secs().min(total);
            format!("Mock generating… ({secs}/{total}s)")
        }
        Gen3dInFlightStatus::Failed => "Mock failed.".to_string(),
    };

    workshop.status = status;
}

fn clear_active_mock_state(workshop: &mut Gen3dWorkshop, job: &mut Gen3dAiJob) {
    job.set_mock_mode(false);
    job.set_running(false);
    job.set_build_complete(false);
    job.set_run_id_opt(None);
    job.set_run_realm_id_opt(None);
    job.set_run_scene_id_opt(None);
    workshop.status = "Mock build finished.".to_string();
}

fn now_ms() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u128)
        .unwrap_or(0)
}
