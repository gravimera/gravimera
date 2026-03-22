use bevy::prelude::*;

use crate::assets::SceneAssets;
use crate::config::AppConfig;
use crate::types::BuildScene;

use super::ai::{
    gen3d_resume_build_from_api, gen3d_start_build_from_api,
    gen3d_start_edit_run_from_current_draft_from_api,
    gen3d_start_edit_session_from_prefab_id_from_api,
    gen3d_start_fork_session_from_prefab_id_from_api,
};
use super::preview;
use super::state::{Gen3dDraft, Gen3dPreview, Gen3dSpeedMode, Gen3dWorkshop};
use super::task_queue::{Gen3dSessionId, Gen3dSessionKind, Gen3dTaskQueue, Gen3dTaskState};

fn session_state_mut<'a>(
    queue: &'a mut Gen3dTaskQueue,
    id: Gen3dSessionId,
    workshop: &'a mut Gen3dWorkshop,
    job: &'a mut super::ai::Gen3dAiJob,
    draft: &'a mut Gen3dDraft,
) -> Option<SessionStateMut<'a>> {
    if id == queue.active_session_id {
        return Some(SessionStateMut {
            workshop,
            job,
            draft,
        });
    }
    queue
        .inactive_states
        .get_mut(&id)
        .map(|state| SessionStateMut {
            workshop: &mut state.workshop,
            job: &mut state.job,
            draft: &mut state.draft,
        })
}

struct SessionStateMut<'a> {
    workshop: &'a mut Gen3dWorkshop,
    job: &'a mut super::ai::Gen3dAiJob,
    draft: &'a mut Gen3dDraft,
}

fn ensure_preview_scene_setup(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    assets: &SceneAssets,
    materials: &mut Assets<StandardMaterial>,
    preview_state: &mut Gen3dPreview,
) {
    let needs_setup = preview_state.target.is_none()
        || preview_state.root.is_none()
        || preview_state.camera.is_none();
    if needs_setup {
        preview::setup_preview_scene(commands, images, assets, materials, preview_state);
    }
}

fn start_or_resume_active_session(
    build_scene: &State<BuildScene>,
    config: &AppConfig,
    log_sinks: Option<crate::app::Gen3dLogSinks>,
    workshop: &mut Gen3dWorkshop,
    job: &mut super::ai::Gen3dAiJob,
    draft: &mut Gen3dDraft,
) -> Result<(), String> {
    if job.edit_base_prefab_id().is_some() {
        if job.has_prior_run() {
            gen3d_start_edit_run_from_current_draft_from_api(
                build_scene,
                config,
                log_sinks,
                workshop,
                job,
                draft,
            )
        } else {
            gen3d_resume_build_from_api(build_scene, config, log_sinks, workshop, job)
        }
    } else if job.can_resume() {
        gen3d_resume_build_from_api(build_scene, config, log_sinks, workshop, job)
    } else {
        gen3d_start_build_from_api(build_scene, config, log_sinks, workshop, job, draft)
    }
}

pub(crate) fn gen3d_task_queue_runner(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    active: Res<crate::realm::ActiveRealmScene>,
    log_sinks: Option<Res<crate::app::Gen3dLogSinks>>,
    mut commands: Commands,
    assets: Res<SceneAssets>,
    mut images: ResMut<Assets<Image>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut queue: ResMut<Gen3dTaskQueue>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut job: ResMut<super::ai::Gen3dAiJob>,
    mut draft: ResMut<Gen3dDraft>,
) {
    // Keep preview scene alive whenever anything is running or queued to run next.
    if queue.running_session_id.is_some() || job.is_running() || !queue.queue.is_empty() {
        ensure_preview_scene_setup(
            &mut commands,
            &mut images,
            &assets,
            &mut materials,
            &mut preview_state,
        );
    }

    // 1) Sync `running_session_id` if the active session is running.
    if job.is_running() {
        let active_id = queue.active_session_id;
        queue.running_session_id = Some(active_id);
        queue.set_task_state(active_id, Gen3dTaskState::Running);
        return;
    }

    // 2) If an inactive session is marked running, see if it finished.
    if let Some(running_id) = queue.running_session_id {
        if running_id == queue.active_session_id {
            // Active session stopped since last tick; treat as finished/canceled based on flags.
            let end_state = if job.is_build_complete() {
                Gen3dTaskState::Done
            } else if workshop.error.is_some() {
                Gen3dTaskState::Failed
            } else {
                Gen3dTaskState::Canceled
            };
            queue.set_task_state(running_id, end_state);
            queue.running_session_id = None;
        } else if let Some(state) = queue.inactive_states.get_mut(&running_id) {
            if state.job.is_running() {
                return;
            }
            let end_state = if state.job.is_build_complete() {
                Gen3dTaskState::Done
            } else if state.workshop.error.is_some() {
                Gen3dTaskState::Failed
            } else {
                Gen3dTaskState::Canceled
            };
            queue.set_task_state(running_id, end_state);
            queue.running_session_id = None;
        } else {
            queue.running_session_id = None;
        }
    }

    // 3) If nothing is running, start the next waiting task (FIFO).
    while queue.running_session_id.is_none() {
        let Some(next_id) = queue.queue.pop_front() else {
            break;
        };

        let Some(meta) = queue.metas.get(&next_id).cloned() else {
            continue;
        };

        let Some(session) =
            session_state_mut(&mut queue, next_id, &mut workshop, &mut job, &mut draft)
        else {
            continue;
        };

        // Ensure baseline UI fields make sense even when running headless/in background.
        if session.workshop.status.trim().is_empty() && !session.job.is_running() {
            session.workshop.speed_mode = Gen3dSpeedMode::Level3;
            session.workshop.status =
                "Queued Gen3D task is starting… (session activated by task queue)".to_string();
        }

        let sinks = log_sinks.as_deref().cloned();

        let started = match meta.kind {
            Gen3dSessionKind::NewBuild => start_or_resume_active_session(
                &build_scene,
                &config,
                sinks.clone(),
                session.workshop,
                session.job,
                session.draft,
            ),
            Gen3dSessionKind::EditOverwrite { prefab_id } => {
                let seeded_ok = session.job.edit_base_prefab_id() == Some(prefab_id)
                    && session.job.save_overwrite_prefab_id() == Some(prefab_id);
                let seeded = if seeded_ok {
                    Ok(())
                } else {
                    gen3d_start_edit_session_from_prefab_id_from_api(
                        &build_scene,
                        &config,
                        sinks.clone(),
                        session.workshop,
                        session.job,
                        session.draft,
                        &active.realm_id,
                        &active.scene_id,
                        prefab_id,
                    )
                };
                seeded.and_then(|()| {
                    start_or_resume_active_session(
                        &build_scene,
                        &config,
                        sinks.clone(),
                        session.workshop,
                        session.job,
                        session.draft,
                    )
                })
            }
            Gen3dSessionKind::Fork { prefab_id } => {
                let seeded_ok = session.job.edit_base_prefab_id() == Some(prefab_id)
                    && session.job.save_overwrite_prefab_id().is_none();
                let seeded = if seeded_ok {
                    Ok(())
                } else {
                    gen3d_start_fork_session_from_prefab_id_from_api(
                        &build_scene,
                        &config,
                        sinks.clone(),
                        session.workshop,
                        session.job,
                        session.draft,
                        &active.realm_id,
                        &active.scene_id,
                        prefab_id,
                    )
                };
                seeded.and_then(|()| {
                    start_or_resume_active_session(
                        &build_scene,
                        &config,
                        sinks.clone(),
                        session.workshop,
                        session.job,
                        session.draft,
                    )
                })
            }
        };

        match started {
            Ok(()) => {
                queue.running_session_id = Some(next_id);
                queue.set_task_state(next_id, Gen3dTaskState::Running);
                break;
            }
            Err(err) => {
                session.workshop.error = Some(err);
                queue.set_task_state(next_id, Gen3dTaskState::Failed);
                // Keep looping to attempt the next queued task.
                continue;
            }
        }
    }
}
