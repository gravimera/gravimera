use bevy::prelude::*;

use crate::gen3d::Gen3dAiJob;
use crate::gen3d::Gen3dManualTweakState;
use crate::gen3d::Gen3dPreview;
use crate::gen3d::Gen3dWorkshop;
use crate::gen3d::{Gen3dExitButton, Gen3dGenerateButton, Gen3dSaveButton};
use crate::genfloor::runtime::{set_active_world_floor, ActiveWorldFloor};
use crate::genfloor::state::GenFloorWorkshop;
use crate::genfloor::{genfloor_cancel_ai_job, genfloor_start_ai_job, GenFloorAiJob};
use crate::realm::ActiveRealmScene;
use crate::types::{BuildScene, GameMode};
use crate::workspace_ui::{TopPanelTab, TopPanelUiState};
use crate::{gen3d, realm_floor_packages};

pub(crate) fn enter_genfloor_mode(
    commands: Commands,
    images: ResMut<Assets<Image>>,
    assets: Res<crate::assets::SceneAssets>,
    materials: ResMut<Assets<StandardMaterial>>,
    job: Res<Gen3dAiJob>,
    mut workshop: ResMut<Gen3dWorkshop>,
    preview_state: ResMut<Gen3dPreview>,
    tweak: ResMut<Gen3dManualTweakState>,
    meta_state: ResMut<crate::motion_ui::MotionAlgorithmUiState>,
    meta_roots: Query<&mut Visibility, With<crate::motion_ui::MotionAlgorithmUiRoot>>,
    windows: Query<&mut Window, With<bevy::window::PrimaryWindow>>,
    floor_workshop: ResMut<GenFloorWorkshop>,
) {
    workshop.prompt = floor_workshop.prompt.clone();
    workshop.status = floor_workshop.status.clone();
    workshop.error = floor_workshop.error.clone();
    if workshop.status.trim().is_empty() {
        workshop.status = "Describe the terrain and click Build.".to_string();
    }
    gen3d::enter_gen3d_mode(
        commands,
        images,
        assets,
        materials,
        job,
        workshop,
        preview_state,
        tweak,
        meta_state,
        meta_roots,
        windows,
    );
}

pub(crate) fn exit_genfloor_mode(
    mut commands: Commands,
    roots: Query<Entity, With<gen3d::Gen3dWorkshopRoot>>,
    preview_cameras: Query<Entity, With<gen3d::Gen3dPreviewCamera>>,
    review_cameras: Query<Entity, With<gen3d::Gen3dReviewCaptureCamera>>,
    preview_roots: Query<Entity, With<gen3d::Gen3dPreviewSceneRoot>>,
    preview_lights: Query<Entity, With<gen3d::Gen3dPreviewLight>>,
    viewer_roots: Query<Entity, With<gen3d::Gen3dImageViewerRoot>>,
    job: Res<Gen3dAiJob>,
    task_queue: Res<gen3d::Gen3dTaskQueue>,
    preview_export: Res<gen3d::Gen3dPreviewExportRuntime>,
    preview_state: ResMut<Gen3dPreview>,
    workshop: ResMut<Gen3dWorkshop>,
    mut floor_workshop: ResMut<GenFloorWorkshop>,
    mut preview_floors: Query<Entity, With<crate::genfloor::GenfloorPreviewFloor>>,
) {
    let prompt = workshop.prompt.clone();
    let status = workshop.status.clone();
    let error = workshop.error.clone();

    for entity in &mut preview_floors {
        commands.entity(entity).try_despawn();
    }

    // Reuse Gen3D cleanup to remove UI roots if present.
    gen3d::exit_gen3d_mode(
        commands,
        roots,
        preview_cameras,
        review_cameras,
        preview_roots,
        preview_lights,
        viewer_roots,
        job,
        task_queue,
        preview_export,
        preview_state,
        workshop,
    );
    floor_workshop.prompt = prompt;
    floor_workshop.status = status;
    floor_workshop.error = error;
}

pub(crate) fn genfloor_set_status_from_gen3d(
    mut floor_workshop: ResMut<GenFloorWorkshop>,
    workshop: Res<Gen3dWorkshop>,
) {
    floor_workshop.status = workshop.status.clone();
    floor_workshop.error = workshop.error.clone();
}

pub(crate) fn genfloor_generate_button(
    build_scene: Res<State<BuildScene>>,
    config: Res<crate::config::AppConfig>,
    mut job: ResMut<GenFloorAiJob>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut floor_workshop: ResMut<GenFloorWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dGenerateButton>,
    >,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::FloorPreview) {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85));
                *border = BorderColor::all(Color::srgb(0.25, 0.80, 0.45));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.18, 0.13, 0.92));
                *border = BorderColor::all(Color::srgb(0.30, 0.88, 0.50));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.12, 0.20, 0.15, 0.98));
                *border = BorderColor::all(Color::srgb(0.35, 0.95, 0.55));

                if matches!(*last_interaction, Some(Interaction::Pressed)) {
                    continue;
                }

                if job.running {
                    genfloor_cancel_ai_job(&mut job, &mut workshop);
                } else {
                    let prompt = workshop.prompt.clone();
                    genfloor_start_ai_job(
                        &config,
                        &prompt,
                        &mut job,
                        &mut workshop,
                        &mut floor_workshop,
                    );
                }
            }
        }
        *last_interaction = Some(*interaction);
    }
}

pub(crate) fn genfloor_save_button(
    build_scene: Res<State<BuildScene>>,
    active: Res<ActiveRealmScene>,
    mut active_floor: ResMut<ActiveWorldFloor>,
    mut floor_workshop: ResMut<GenFloorWorkshop>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut floor_library: ResMut<crate::floor_library_ui::FloorLibraryUiState>,
    mut genfloor_job: ResMut<crate::genfloor::GenFloorAiJob>,
    mut thumbnail_capture: ResMut<crate::genfloor::GenfloorThumbnailCaptureRuntime>,
    mut top_panel: ResMut<TopPanelUiState>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
            &mut Node,
        ),
        With<Gen3dSaveButton>,
    >,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::FloorPreview) {
        return;
    }

    let Ok((interaction, mut bg, mut border, mut vis, mut node)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    let has_draft = floor_workshop.draft.is_some();
    if !has_draft {
        node.display = Display::None;
        *vis = Visibility::Hidden;
        *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
        *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
        *last_interaction = None;
        return;
    }

    node.display = Display::Flex;
    *vis = Visibility::Inherited;

    match *interaction {
        Interaction::None => {
            *bg = BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.80));
            *border = BorderColor::all(Color::srgb(0.30, 0.55, 0.95));
        }
        Interaction::Hovered => {
            *bg = BackgroundColor(Color::srgba(0.08, 0.13, 0.20, 0.88));
            *border = BorderColor::all(Color::srgb(0.35, 0.60, 1.00));
        }
        Interaction::Pressed => {
            *bg = BackgroundColor(Color::srgba(0.10, 0.16, 0.25, 0.96));
            *border = BorderColor::all(Color::srgb(0.40, 0.65, 1.00));

            if matches!(*last_interaction, Some(Interaction::Pressed)) {
                return;
            }

            if let Some(def) = floor_workshop.draft.take() {
                let floor_id = genfloor_job
                    .save_overwrite_floor_id()
                    .unwrap_or_else(|| uuid::Uuid::new_v4().as_u128());
                if let Err(err) =
                    realm_floor_packages::save_realm_floor_def(&active.realm_id, floor_id, &def)
                {
                    workshop.error = Some(err);
                } else {
                    let source_dir = realm_floor_packages::realm_floor_package_genfloor_source_dir(
                        &active.realm_id,
                        floor_id,
                    );
                    let _ = std::fs::write(source_dir.join("prompt.txt"), workshop.prompt.as_str());
                    set_active_world_floor(&mut active_floor, Some(floor_id), def);
                    crate::genfloor::genfloor_queue_thumbnail_capture(
                        &mut thumbnail_capture,
                        active.realm_id.clone(),
                        floor_id,
                    );
                    genfloor_job.set_edit_base_floor_id(Some(floor_id));
                    genfloor_job.set_save_overwrite_floor_id(Some(floor_id));
                    genfloor_job.set_last_saved_floor_id(Some(floor_id));
                    floor_library.mark_models_dirty();
                    workshop.status = "Terrain saved.".to_string();
                    match crate::scene_floor_selection::save_scene_floor_selection(
                        &active.realm_id,
                        &active.scene_id,
                        Some(floor_id),
                    ) {
                        Ok(()) => {
                            workshop.error = None;
                        }
                        Err(err) => {
                            workshop.error = Some(format!(
                                "Terrain saved, but failed to persist selection: {err}"
                            ));
                        }
                    }
                    top_panel.selected = Some(TopPanelTab::Floors);
                    next_build_scene.set(BuildScene::Realm);
                }
            }
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn genfloor_exit_button(
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut top_panel: ResMut<TopPanelUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dExitButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::FloorPreview) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                next_build_scene.set(BuildScene::Realm);
                top_panel.selected = Some(TopPanelTab::Floors);
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.75));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60));
            }
        }
    }
}

pub(crate) fn genfloor_exit_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut top_panel: ResMut<TopPanelUiState>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if !matches!(mode.get(), GameMode::Build) {
        return;
    }
    if !matches!(build_scene.get(), BuildScene::FloorPreview) {
        return;
    }
    if workshop.image_viewer.is_some() {
        return;
    }
    next_build_scene.set(BuildScene::Realm);
    top_panel.selected = Some(TopPanelTab::Floors);
}
