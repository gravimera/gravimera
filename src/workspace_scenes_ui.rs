use bevy::ecs::message::MessageWriter;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::Ime;
use bevy::window::PrimaryWindow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};

use crate::realm::ActiveRealmScene;
use crate::rich_text::set_rich_text_line;
use crate::scene_store::SceneSaveRequest;
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts, UiToastCommand, UiToastKind};
use crate::ui::{set_ime_position_for_rich_text, ImeAnchorXPolicy};
use crate::workspace_ui::{TopPanelTab, TopPanelUiState};

const SCENE_NAME_MAX_CHARS: usize = 128;
pub(crate) const SCENES_PANEL_WIDTH_PX: f32 = 260.0;
pub(crate) const SCENES_PANEL_WIDTH_MANAGE_PX: f32 = 320.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScenesUiField {
    None,
    AddSceneName,
}

#[derive(Resource, Debug)]
pub(crate) struct ScenesPanelUiState {
    pub(crate) scenes_dirty: bool,
    pub(crate) add_open: bool,
    pub(crate) multi_select_mode: bool,
    pub(crate) focused_field: ScenesUiField,
    pub(crate) name: String,
    pub(crate) error: Option<String>,
    pub(crate) multi_selected_scenes: HashSet<String>,
    pub(crate) export_dialog_pending_ids: Vec<String>,
    pub(crate) export_dialog_pending_realm: Option<String>,
    pub(crate) import_dialog_pending_realm: Option<String>,
    scrollbar_drag: Option<ScenesPanelScrollbarDrag>,
    last_realm_id: Option<String>,
    last_panel_open: bool,
}

#[derive(Resource)]
pub(crate) struct ScenesPanelExportJob {
    receiver: Mutex<Option<mpsc::Receiver<Result<crate::scene_zip::SceneZipExportReport, String>>>>,
}

impl Default for ScenesPanelExportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ScenesPanelImportJob {
    receiver: Mutex<Option<mpsc::Receiver<Result<crate::scene_zip::SceneZipImportReport, String>>>>,
}

impl Default for ScenesPanelImportJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ScenesPanelExportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for ScenesPanelExportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

#[derive(Resource)]
pub(crate) struct ScenesPanelImportDialogJob {
    receiver: Mutex<Option<mpsc::Receiver<Option<std::path::PathBuf>>>>,
}

impl Default for ScenesPanelImportDialogJob {
    fn default() -> Self {
        Self {
            receiver: Mutex::new(None),
        }
    }
}

impl Default for ScenesPanelUiState {
    fn default() -> Self {
        Self {
            scenes_dirty: true,
            add_open: false,
            multi_select_mode: false,
            focused_field: ScenesUiField::None,
            name: String::new(),
            error: None,
            multi_selected_scenes: HashSet::new(),
            export_dialog_pending_ids: Vec::new(),
            export_dialog_pending_realm: None,
            import_dialog_pending_realm: None,
            scrollbar_drag: None,
            last_realm_id: None,
            last_panel_open: false,
        }
    }
}

impl ScenesPanelUiState {
    pub(crate) fn is_drag_active(&self) -> bool {
        self.scrollbar_drag.is_some()
    }
}

#[derive(Component)]
pub(crate) struct ScenesAddSceneButton;

#[derive(Component)]
pub(crate) struct ScenesAddSceneButtonText;

#[derive(Component)]
pub(crate) struct ScenesManageButton;

#[derive(Component)]
pub(crate) struct ScenesManageButtonText;

#[derive(Component)]
pub(crate) struct ScenesImportButton;

#[derive(Component)]
pub(crate) struct ScenesExportButton;

#[derive(Component)]
pub(crate) struct ScenesDeleteButton;

#[derive(Component)]
pub(crate) struct ScenesSelectAllButton;

#[derive(Component)]
pub(crate) struct ScenesSelectNoneButton;

#[derive(Component)]
pub(crate) struct ScenesManageOnlyAction;

#[derive(Component)]
pub(crate) struct ScenesListScrollPanel;

#[derive(Component)]
pub(crate) struct ScenesList;

#[derive(Component)]
pub(crate) struct ScenesListItem;

#[derive(Component)]
pub(crate) struct ScenesScrollbarTrack;

#[derive(Component)]
pub(crate) struct ScenesScrollbarThumb;

#[derive(Component)]
pub(crate) struct SceneSelectButton {
    pub(crate) scene_id: String,
}

#[derive(Component)]
pub(crate) struct AddScenePanelRoot;

#[derive(Component)]
pub(crate) struct AddSceneNameField;

#[derive(Component)]
pub(crate) struct AddSceneNameFieldText;

#[derive(Component)]
pub(crate) struct AddSceneAddButton;

#[derive(Component)]
pub(crate) struct AddSceneCancelButton;

#[derive(Component)]
pub(crate) struct AddSceneErrorText;

#[derive(Debug, Clone, Copy)]
struct ScenesPanelScrollbarDrag {
    grab_offset: f32,
}

fn scenes_panel_open(
    mode: &State<GameMode>,
    build_scene: &State<BuildScene>,
    top: &TopPanelUiState,
) -> bool {
    matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
        && top.selected == Some(TopPanelTab::Scenes)
}

pub(crate) fn scenes_panel_sync_active_realm(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
) {
    let open = scenes_panel_open(&mode, &build_scene, &top);
    if !open && state.add_open {
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
        state.error = None;
    }
    if !open {
        state.scrollbar_drag = None;
    }

    let realm_changed = state.last_realm_id.as_deref() != Some(active.realm_id.as_str());
    if realm_changed {
        state.last_realm_id = Some(active.realm_id.clone());
        state.scenes_dirty = true;
        state.multi_selected_scenes.clear();
        state.export_dialog_pending_ids.clear();
        state.export_dialog_pending_realm = None;
        state.import_dialog_pending_realm = None;
    }

    if open && !state.last_panel_open {
        state.scenes_dirty = true;
    }

    if state.multi_select_mode {
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
    }

    state.last_panel_open = open;
}

pub(crate) fn scenes_panel_set_add_panel_visibility(
    state: Res<ScenesPanelUiState>,
    mut panels: Query<&mut Node, With<AddScenePanelRoot>>,
) {
    let Ok(mut node) = panels.single_mut() else {
        return;
    };
    node.display = if state.add_open {
        Display::Flex
    } else {
        Display::None
    };
}

pub(crate) fn scenes_panel_update_panel_width(
    state: Res<ScenesPanelUiState>,
    mut roots: Query<&mut Node, With<crate::workspace_ui::ScenesPanelRoot>>,
) {
    let Ok(mut node) = roots.single_mut() else {
        return;
    };
    node.width = Val::Px(if state.multi_select_mode {
        SCENES_PANEL_WIDTH_MANAGE_PX
    } else {
        SCENES_PANEL_WIDTH_PX
    });
}

pub(crate) fn scenes_panel_update_action_visibility(
    state: Res<ScenesPanelUiState>,
    mut params: ParamSet<(
        Query<&mut Node, With<ScenesAddSceneButton>>,
        Query<&mut Node, With<ScenesManageButton>>,
        Query<&mut Node, With<ScenesManageOnlyAction>>,
    )>,
) {
    if let Ok(mut node) = params.p0().single_mut() {
        node.display = if state.multi_select_mode {
            Display::None
        } else {
            Display::Flex
        };
    }
    if let Ok(mut node) = params.p1().single_mut() {
        node.display = Display::Flex;
    }
    for mut node in &mut params.p2() {
        node.display = if state.multi_select_mode {
            Display::Flex
        } else {
            Display::None
        };
    }
}

pub(crate) fn scenes_panel_rebuild_list_ui(
    mut commands: Commands,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    lists: Query<Entity, With<ScenesList>>,
    existing_items: Query<Entity, With<ScenesListItem>>,
    mut panels: Query<&mut ScrollPosition, With<ScenesListScrollPanel>>,
) {
    if !state.scenes_dirty {
        return;
    }
    let Ok(list_entity) = lists.single() else {
        return;
    };

    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    let mut scenes: Vec<(String, u128)> = crate::realm::list_scenes(&active.realm_id)
        .into_iter()
        .map(|scene_id| {
            let created_at_ms = metadata_created_or_modified_ms(&crate::paths::scene_dir(
                &active.realm_id,
                &scene_id,
            ));
            (scene_id, created_at_ms)
        })
        .collect();
    scenes.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if state.multi_select_mode && !state.multi_selected_scenes.is_empty() {
        let listed: HashSet<String> = scenes
            .iter()
            .map(|(scene_id, _)| scene_id.clone())
            .collect();
        state
            .multi_selected_scenes
            .retain(|scene_id| listed.contains(scene_id));
    }
    if scenes.is_empty() {
        commands.entity(list_entity).with_children(|list| {
            list.spawn((
                Text::new("No scenes yet."),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                ScenesListItem,
            ));
        });
        state.scenes_dirty = false;
        return;
    }

    commands.entity(list_entity).with_children(|list| {
        for (scene_id, _created_at_ms) in scenes {
            let label = if scene_id == active.scene_id {
                format!("{scene_id} (Current)")
            } else {
                scene_id.clone()
            };
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    min_width: Val::Px(0.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ScenesListItem,
                SceneSelectButton {
                    scene_id: scene_id.clone(),
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(label),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                ));
            });
        }
    });

    if let Ok(mut scroll) = panels.single_mut() {
        scroll.y = 0.0;
    }

    state.scenes_dirty = false;
}

pub(crate) fn scenes_panel_add_scene_button_actions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ScenesAddSceneButton>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for interaction in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        if state.multi_select_mode {
            continue;
        }
        if state.add_open {
            continue;
        }
        state.add_open = true;
        state.focused_field = ScenesUiField::AddSceneName;
        state.name.clear();
        state.error = None;
    }
}

pub(crate) fn scenes_panel_manage_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ScenesManageButton>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for interaction in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        state.multi_select_mode = !state.multi_select_mode;
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
        state.error = None;
        if !state.multi_select_mode {
            state.multi_selected_scenes.clear();
        }
    }
}

pub(crate) fn scenes_panel_import_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    import_job: Res<ScenesPanelImportJob>,
    import_dialog: Res<ScenesPanelImportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ScenesImportButton>),
    >,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
                *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.84));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));

                if let Ok(guard) = import_job.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Scene import already running.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }
                if let Ok(guard) = import_dialog.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Scene import dialog already open.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }

                let (tx, rx) = mpsc::channel();
                if let Ok(mut guard) = import_dialog.receiver.lock() {
                    *guard = Some(rx);
                }
                state.import_dialog_pending_realm = Some(active.realm_id.clone());
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter("Scene Zip", &["zip"])
                        .pick_file();
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn scenes_panel_export_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    export_job: Res<ScenesPanelExportJob>,
    export_dialog: Res<ScenesPanelExportDialogJob>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ScenesExportButton>),
    >,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.multi_select_mode {
        return;
    }

    fn ensure_zip_extension(path: std::path::PathBuf) -> std::path::PathBuf {
        match path.extension().and_then(|value| value.to_str()) {
            Some(ext) if ext.eq_ignore_ascii_case("zip") => path,
            _ => path.with_extension("zip"),
        }
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
                *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.84));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));

                if let Ok(guard) = export_job.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Scene export already running.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }
                if let Ok(guard) = export_dialog.receiver.lock() {
                    if guard.is_some() {
                        toasts.write(UiToastCommand::Show {
                            text: "Scene export dialog already open.".to_string(),
                            kind: UiToastKind::Warn,
                            ttl_secs: 3.0,
                        });
                        continue;
                    }
                }
                if state.multi_selected_scenes.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select scenes to export first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut ids: Vec<String> = state.multi_selected_scenes.iter().cloned().collect();
                ids.sort();
                ids.dedup();
                state.export_dialog_pending_ids = ids;
                state.export_dialog_pending_realm = Some(active.realm_id.clone());

                let (tx, rx) = mpsc::channel();
                if let Ok(mut guard) = export_dialog.receiver.lock() {
                    *guard = Some(rx);
                }
                toasts.write(UiToastCommand::Show {
                    text: "Select scene export location…".to_string(),
                    kind: UiToastKind::Info,
                    ttl_secs: 3.0,
                });
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .add_filter("Scene Zip", &["zip"])
                        .set_file_name("scenes.zip")
                        .save_file()
                        .map(ensure_zip_extension);
                    let _ = tx.send(path);
                });
            }
        }
    }
}

pub(crate) fn scenes_panel_select_all_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ScenesSelectAllButton>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.multi_select_mode {
        return;
    }

    for interaction in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        state.multi_selected_scenes = crate::realm::list_scenes(&active.realm_id)
            .into_iter()
            .collect();
    }
}

pub(crate) fn scenes_panel_select_none_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<ScenesSelectNoneButton>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.multi_select_mode {
        return;
    }

    for interaction in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        state.multi_selected_scenes.clear();
    }
}

pub(crate) fn scenes_panel_delete_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut toasts: MessageWriter<UiToastCommand>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ScenesDeleteButton>),
    >,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.multi_select_mode {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
                *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.84));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));

                if state.multi_selected_scenes.is_empty() {
                    toasts.write(UiToastCommand::Show {
                        text: "Select scenes to delete first.".to_string(),
                        kind: UiToastKind::Warn,
                        ttl_secs: 4.0,
                    });
                    continue;
                }

                let mut deleted = 0usize;
                let mut failed = 0usize;
                let mut skipped_active = 0usize;
                let mut ids: Vec<String> = state.multi_selected_scenes.iter().cloned().collect();
                ids.sort();
                ids.dedup();

                for scene_id in &ids {
                    if *scene_id == active.scene_id {
                        skipped_active += 1;
                        continue;
                    }

                    let scene_dir = crate::paths::scene_dir(&active.realm_id, scene_id);
                    match std::fs::remove_dir_all(&scene_dir) {
                        Ok(()) => deleted += 1,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                        Err(err) => {
                            failed += 1;
                            warn!("Failed to delete {}: {err}", scene_dir.display());
                        }
                    }
                }

                state.multi_selected_scenes.clear();
                if deleted > 0 {
                    state.scenes_dirty = true;
                }

                let text = if failed == 0 && skipped_active == 0 {
                    format!("Deleted {} scene(s).", deleted)
                } else if failed == 0 {
                    format!("Deleted {}, skipped active {}.", deleted, skipped_active)
                } else {
                    format!(
                        "Deleted {}, failed {}, skipped active {}.",
                        deleted, failed, skipped_active
                    )
                };
                let kind = if failed > 0 || skipped_active > 0 {
                    UiToastKind::Warn
                } else {
                    UiToastKind::Info
                };
                toasts.write(UiToastCommand::Show {
                    text,
                    kind,
                    ttl_secs: 5.0,
                });
            }
        }
    }
}

pub(crate) fn scenes_panel_scene_select_button_actions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<(&Interaction, &SceneSelectButton), Changed<Interaction>>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        return;
    }

    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }
        if state.multi_select_mode {
            if state.multi_selected_scenes.contains(&button.scene_id) {
                state.multi_selected_scenes.remove(&button.scene_id);
            } else {
                state.multi_selected_scenes.insert(button.scene_id.clone());
            }
            state.error = None;
            continue;
        }
        if button.scene_id == active.scene_id {
            continue;
        }

        pending.target = Some(ActiveRealmScene {
            realm_id: active.realm_id.clone(),
            scene_id: button.scene_id.clone(),
        });
        saves.write(SceneSaveRequest::new("switch scene"));
        state.error = None;
        state.add_open = false;
        state.focused_field = ScenesUiField::None;
    }
}

pub(crate) fn scenes_panel_add_panel_buttons(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut state: ResMut<ScenesPanelUiState>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<
        (
            &Interaction,
            Option<&AddSceneAddButton>,
            Option<&AddSceneCancelButton>,
        ),
        (
            Changed<Interaction>,
            Or<(With<AddSceneAddButton>, With<AddSceneCancelButton>)>,
        ),
    >,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open || state.multi_select_mode {
        return;
    }

    for (interaction, add, cancel) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }

        if cancel.is_some() {
            state.add_open = false;
            state.focused_field = ScenesUiField::None;
            state.name.clear();
            state.error = None;
            continue;
        }

        if add.is_none() {
            continue;
        }

        let validated = match validate_scene_dir_name(&state.name) {
            Ok(v) => v,
            Err(err) => {
                state.error = Some(err);
                continue;
            }
        };

        let scene_dir = crate::paths::scene_dir(&active.realm_id, &validated);
        if scene_dir.exists() {
            state.error = Some("Scene already exists.".to_string());
            continue;
        }

        if let Err(err) = crate::realm::ensure_realm_scene_scaffold(&active.realm_id, &validated) {
            state.error = Some(err);
            continue;
        }

        if let Err(err) =
            crate::scene_store::ensure_default_scene_dat_exists(&active.realm_id, &validated)
        {
            state.error = Some(err);
            continue;
        }

        state.add_open = false;
        state.focused_field = ScenesUiField::None;
        state.name.clear();
        state.error = None;
        state.scenes_dirty = true;

        pending.target = Some(ActiveRealmScene {
            realm_id: active.realm_id.clone(),
            scene_id: validated,
        });
        saves.write(SceneSaveRequest::new("add scene"));
    }
}

pub(crate) fn scenes_panel_export_dialog_poll(
    mut state: ResMut<ScenesPanelUiState>,
    export_dialog: Res<ScenesPanelExportDialogJob>,
    export_job: Res<ScenesPanelExportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = export_dialog.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    let path = match receiver.try_recv() {
        Ok(path) => {
            *guard = None;
            path
        }
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            state.export_dialog_pending_ids.clear();
            state.export_dialog_pending_realm = None;
            toasts.write(UiToastCommand::Show {
                text: "Scene export canceled: dialog failed.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 4.0,
            });
            return;
        }
    };

    let Some(path) = path else {
        state.export_dialog_pending_ids.clear();
        state.export_dialog_pending_realm = None;
        return;
    };
    let Some(realm_id) = state.export_dialog_pending_realm.take() else {
        return;
    };
    if state.export_dialog_pending_ids.is_empty() {
        return;
    }

    let (tx, rx) = mpsc::channel();
    if let Ok(mut job_guard) = export_job.receiver.lock() {
        if job_guard.is_some() {
            toasts.write(UiToastCommand::Show {
                text: "Scene export already running.".to_string(),
                kind: UiToastKind::Warn,
                ttl_secs: 3.0,
            });
            return;
        }
        *job_guard = Some(rx);
    }

    let mut ids = state.export_dialog_pending_ids.clone();
    state.export_dialog_pending_ids.clear();
    ids.sort();
    ids.dedup();
    toasts.write(UiToastCommand::Show {
        text: "Exporting scenes…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::scene_zip::export_scene_packages_to_zip(&realm_id, &ids, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn scenes_panel_export_job_poll(
    export_job: Res<ScenesPanelExportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = export_job.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            *guard = None;
            match result {
                Ok(report) => {
                    toasts.write(UiToastCommand::Show {
                        text: format!(
                            "Exported {} scene(s) with {} prefab package(s).",
                            report.exported_scenes, report.exported_prefabs
                        ),
                        kind: UiToastKind::Info,
                        ttl_secs: 4.0,
                    });
                }
                Err(err) => {
                    toasts.write(UiToastCommand::Show {
                        text: err,
                        kind: UiToastKind::Error,
                        ttl_secs: 5.0,
                    });
                }
            }
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            toasts.write(UiToastCommand::Show {
                text: "Scene export failed: worker disconnected.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 5.0,
            });
        }
    }
}

pub(crate) fn scenes_panel_import_dialog_poll(
    mut state: ResMut<ScenesPanelUiState>,
    import_dialog: Res<ScenesPanelImportDialogJob>,
    import_job: Res<ScenesPanelImportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = import_dialog.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    let path = match receiver.try_recv() {
        Ok(path) => {
            *guard = None;
            path
        }
        Err(mpsc::TryRecvError::Empty) => return,
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            state.import_dialog_pending_realm = None;
            toasts.write(UiToastCommand::Show {
                text: "Scene import canceled: dialog failed.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 4.0,
            });
            return;
        }
    };

    let Some(path) = path else {
        state.import_dialog_pending_realm = None;
        return;
    };
    let Some(realm_id) = state.import_dialog_pending_realm.take() else {
        return;
    };

    let (tx, rx) = mpsc::channel();
    if let Ok(mut job_guard) = import_job.receiver.lock() {
        if job_guard.is_some() {
            toasts.write(UiToastCommand::Show {
                text: "Scene import already running.".to_string(),
                kind: UiToastKind::Warn,
                ttl_secs: 3.0,
            });
            return;
        }
        *job_guard = Some(rx);
    }

    toasts.write(UiToastCommand::Show {
        text: "Importing scenes…".to_string(),
        kind: UiToastKind::Info,
        ttl_secs: 3.0,
    });
    std::thread::spawn(move || {
        let result = crate::scene_zip::import_scene_packages_from_zip(&realm_id, &path);
        let _ = tx.send(result);
    });
}

pub(crate) fn scenes_panel_import_job_poll(
    mut state: ResMut<ScenesPanelUiState>,
    import_job: Res<ScenesPanelImportJob>,
    mut toasts: MessageWriter<UiToastCommand>,
) {
    let Ok(mut guard) = import_job.receiver.lock() else {
        return;
    };
    let Some(receiver) = guard.as_ref() else {
        return;
    };

    match receiver.try_recv() {
        Ok(result) => {
            *guard = None;
            match result {
                Ok(report) => {
                    state.scenes_dirty = true;
                    toasts.write(UiToastCommand::Show {
                        text: format!(
                            "Scenes imported {}, skipped {}, invalid {}; prefabs imported {}, skipped {}, invalid {}.",
                            report.imported_scenes,
                            report.skipped_scenes,
                            report.invalid_scenes,
                            report.imported_prefabs,
                            report.skipped_prefabs,
                            report.invalid_prefabs
                        ),
                        kind: if report.invalid_scenes > 0 || report.invalid_prefabs > 0 {
                            UiToastKind::Warn
                        } else {
                            UiToastKind::Info
                        },
                        ttl_secs: 5.0,
                    });
                }
                Err(err) => {
                    toasts.write(UiToastCommand::Show {
                        text: err,
                        kind: UiToastKind::Error,
                        ttl_secs: 5.0,
                    });
                }
            }
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            *guard = None;
            toasts.write(UiToastCommand::Show {
                text: "Scene import failed: worker disconnected.".to_string(),
                kind: UiToastKind::Error,
                ttl_secs: 5.0,
            });
        }
    }
}

pub(crate) fn scenes_panel_name_field_focus(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut fields: Query<&Interaction, (Changed<Interaction>, With<AddSceneNameField>)>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        return;
    }

    for interaction in &mut fields {
        if matches!(*interaction, Interaction::Pressed) {
            state.focused_field = ScenesUiField::AddSceneName;
            if let Ok(mut window) = windows.single_mut() {
                window.ime_enabled = true;
            }
        }
    }
}

pub(crate) fn scenes_panel_update_ime_position(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    state: Res<ScenesPanelUiState>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    fields: Query<(&ComputedNode, &UiGlobalTransform), With<AddSceneNameField>>,
    text_root: Query<Entity, With<AddSceneNameFieldText>>,
    children: Query<&Children>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&Text>,
        Option<&TextSpan>,
        Option<&ImageNode>,
        Option<&Visibility>,
    )>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        return;
    }
    if state.focused_field != ScenesUiField::AddSceneName {
        return;
    }
    let Ok((node, transform)) = fields.single() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let rich_root = text_root.iter().next();
    set_ime_position_for_rich_text(
        &mut window,
        node,
        *transform,
        rich_root,
        ImeAnchorXPolicy::LineEnd,
        &children,
        &nodes,
    );
}

pub(crate) fn scenes_panel_text_input(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    scene_ui: Res<crate::scene_authoring_ui::SceneAuthoringUiState>,
    mut state: ResMut<ScenesPanelUiState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<KeyboardInput>,
    mut ime_events: bevy::ecs::message::MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if scene_ui.is_open() {
        keyboard.clear();
        ime_events.clear();
        return;
    }
    if !scenes_panel_open(&mode, &build_scene, &top) || !state.add_open {
        keyboard.clear();
        ime_events.clear();
        return;
    }
    if state.focused_field != ScenesUiField::AddSceneName {
        return;
    }

    for event in ime_events.read() {
        if let Ime::Commit { value, .. } = event {
            if !value.is_empty() {
                push_text(&mut state.name, value);
            }
        }
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        state.error = None;

        match event.key_code {
            KeyCode::Backspace => {
                state.name.pop();
            }
            KeyCode::Escape => {
                state.focused_field = ScenesUiField::None;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
                ime_events.clear();
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                state.focused_field = ScenesUiField::None;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
                ime_events.clear();
            }
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if modifier {
                    if let Some(text) = crate::clipboard::read_text() {
                        push_text(&mut state.name, &text);
                    }
                    continue;
                }
                if let Some(text) = &event.text {
                    push_text(&mut state.name, text);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                push_text(&mut state.name, text);
            }
        }
    }
}

fn push_text(target: &mut String, text: &str) {
    let remaining = SCENE_NAME_MAX_CHARS.saturating_sub(target.chars().count());
    if remaining == 0 {
        return;
    }

    let mut inserted = 0usize;
    for ch in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
        if ch.is_control() || ch == '\n' || ch == '\t' {
            continue;
        }
        target.push(ch);
        inserted += 1;
        if inserted >= remaining {
            break;
        }
    }
}

pub(crate) fn scenes_panel_clear_keyboard_state_when_captured(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    scene_ui: Res<crate::scene_authoring_ui::SceneAuthoringUiState>,
    state: Res<ScenesPanelUiState>,
    mut keys: Option<ResMut<ButtonInput<KeyCode>>>,
) {
    if scene_ui.is_open() {
        return;
    }
    if !scenes_panel_open(&mode, &build_scene, &top)
        || !state.add_open
        || state.focused_field == ScenesUiField::None
    {
        return;
    }
    if let Some(keys) = keys.as_deref_mut() {
        keys.clear();
        let pressed_now: Vec<KeyCode> = keys.get_pressed().copied().collect();
        for key in pressed_now {
            keys.release(key);
            let _ = keys.clear_just_released(key);
        }
    }
}

pub(crate) fn scenes_panel_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    state: Res<ScenesPanelUiState>,
    roots: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<crate::workspace_ui::ScenesPanelRoot>,
    >,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ScenesListScrollPanel>>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) || state.scrollbar_drag.is_some() {
        for _ in mouse_wheel.read() {}
        return;
    }

    let Ok(window) = windows.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let Ok((root_node, root_transform, root_vis)) = roots.single() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    if *root_vis == Visibility::Hidden || !root_node.contains_point(*root_transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

    let Ok((panel_node, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    let mut delta_lines = 0.0f32;
    for ev in mouse_wheel.read() {
        let lines = match ev.unit {
            MouseScrollUnit::Line => ev.y,
            MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() < 1e-4 {
        return;
    }

    // `ScrollPosition` is in logical pixels. Approximate a line step as 24px.
    let delta_px = delta_lines * 24.0;

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);
    scroll.y = (scroll.y - delta_px).clamp(0.0, max_scroll);
}

pub(crate) fn scenes_panel_scrollbar_drag(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    top: Res<TopPanelUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ScenesPanelUiState>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ScenesListScrollPanel>>,
    tracks: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<ScenesScrollbarTrack>>,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<ScenesScrollbarThumb>>,
) {
    if !scenes_panel_open(&mode, &build_scene, &top) {
        state.scrollbar_drag = None;
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        state.scrollbar_drag = None;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };
    let Ok((panel_node, mut scroll)) = panels.single_mut() else {
        return;
    };
    let Ok((track_node, track_transform, track_vis)) = tracks.single() else {
        return;
    };
    if *track_vis == Visibility::Hidden {
        state.scrollbar_drag = None;
        return;
    }
    let Ok((interaction, thumb_node, thumb_layout)) = thumbs.single() else {
        return;
    };

    if state.scrollbar_drag.is_none() && *interaction == Interaction::Pressed {
        if let Some(local) = track_transform
            .try_inverse()
            .map(|transform| transform.transform_point2(cursor))
        {
            let track_scale = track_node.inverse_scale_factor();
            let thumb_scale = thumb_node.inverse_scale_factor();
            let cursor_in_track = (local.y + track_node.size.y * 0.5) * track_scale;
            let thumb_top = match thumb_layout.top {
                Val::Px(value) => value,
                _ => 0.0,
            };
            let grab_offset =
                (cursor_in_track - thumb_top).clamp(0.0, thumb_node.size.y.max(1.0) * thumb_scale);
            state.scrollbar_drag = Some(ScenesPanelScrollbarDrag { grab_offset });
        }
    }

    let Some(drag) = state.scrollbar_drag else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        return;
    }

    let track_scale = track_node.inverse_scale_factor();
    let thumb_scale = thumb_node.inverse_scale_factor();
    let track_h = track_node.size.y.max(1.0) * track_scale;
    let thumb_h = thumb_node.size.y.max(1.0) * thumb_scale;
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    if max_thumb_top <= 1e-4 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(1.0);

    let Some(local) = track_transform
        .try_inverse()
        .map(|transform| transform.transform_point2(cursor))
    else {
        return;
    };
    let cursor_in_track = ((local.y + track_node.size.y * 0.5) * track_scale).clamp(0.0, track_h);
    let thumb_top = (cursor_in_track - drag.grab_offset).clamp(0.0, max_thumb_top);

    scroll.y = (thumb_top / max_thumb_top * max_scroll).clamp(0.0, max_scroll);
}

pub(crate) fn scenes_panel_update_scrollbar_ui(
    panels: Query<(&ComputedNode, &ScrollPosition), With<ScenesListScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<ScenesScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<ScenesScrollbarThumb>>,
) {
    let Ok((panel, scroll_pos)) = panels.single() else {
        return;
    };
    let Ok((track_node, mut track_vis)) = tracks.single_mut() else {
        return;
    };
    let Ok(mut thumb) = thumbs.single_mut() else {
        return;
    };

    let panel_scale = panel.inverse_scale_factor();
    let track_scale = track_node.inverse_scale_factor();
    let viewport_h = panel.size.y.max(0.0) * panel_scale;
    let content_h = panel.content_size.y.max(0.0) * panel_scale;
    let track_h = track_node.size.y.max(1.0) * track_scale;

    if viewport_h < 1.0 || content_h < 1.0 {
        *track_vis = Visibility::Hidden;
        return;
    }

    if content_h <= viewport_h + 0.5 {
        *track_vis = Visibility::Hidden;
        thumb.top = Val::Px(0.0);
        thumb.height = Val::Px(track_h);
        return;
    }

    *track_vis = Visibility::Inherited;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = scroll_pos.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(min_thumb_h, track_h);
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    let thumb_top = (max_thumb_top * (scroll_y / max_scroll)).clamp(0.0, max_thumb_top);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

fn system_time_ms(time: std::time::SystemTime) -> u128 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn metadata_created_or_modified_ms(path: &Path) -> u128 {
    std::fs::metadata(path)
        .ok()
        .and_then(|meta| meta.created().or_else(|_| meta.modified()).ok())
        .map(system_time_ms)
        .unwrap_or(0)
}

pub(crate) fn scenes_panel_update_texts(
    state: Res<ScenesPanelUiState>,
    mut commands: Commands,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
    mut texts: Query<
        (
            &mut Text,
            Option<&AddSceneNameFieldText>,
            Option<&AddSceneErrorText>,
            Option<&ScenesManageButtonText>,
        ),
        Or<(
            With<AddSceneNameFieldText>,
            With<AddSceneErrorText>,
            With<ScenesManageButtonText>,
        )>,
    >,
    name_field: Query<Entity, With<AddSceneNameFieldText>>,
    mut last_name: Local<Option<String>>,
) {
    for (mut text, _name, error, manage) in &mut texts {
        if error.is_some() {
            *text = Text::new(state.error.clone().unwrap_or_default());
        } else if manage.is_some() {
            *text = Text::new(if state.multi_select_mode {
                "Done"
            } else {
                "Manage"
            });
        }
    }

    let Some(entity) = name_field.iter().next() else {
        return;
    };
    let needs_update = match last_name.as_ref() {
        Some(prev) => prev != &state.name,
        None => true,
    };
    if needs_update {
        set_rich_text_line(
            &mut commands,
            entity,
            &state.name,
            &ui_fonts,
            &emoji_atlas,
            &asset_server,
            14.0,
            Color::srgb(0.92, 0.92, 0.96),
            None,
        );
        *last_name = Some(state.name.clone());
    }
}

pub(crate) fn scenes_panel_update_styles(
    state: Res<ScenesPanelUiState>,
    active: Res<ActiveRealmScene>,
    mut params: ParamSet<(
        Query<(&Interaction, &mut BackgroundColor, &mut BorderColor), With<ScenesAddSceneButton>>,
        Query<(
            &Interaction,
            &SceneSelectButton,
            &mut BackgroundColor,
            &mut BorderColor,
        )>,
        Query<(&Interaction, &mut BackgroundColor, &mut BorderColor), With<AddSceneNameField>>,
        Query<
            (
                &Interaction,
                Option<&AddSceneAddButton>,
                Option<&AddSceneCancelButton>,
                &mut BackgroundColor,
                &mut BorderColor,
            ),
            Or<(With<AddSceneAddButton>, With<AddSceneCancelButton>)>,
        >,
        Query<
            (
                &Interaction,
                Option<&ScenesManageButton>,
                Option<&ScenesImportButton>,
                Option<&ScenesExportButton>,
                Option<&ScenesDeleteButton>,
                Option<&ScenesSelectAllButton>,
                Option<&ScenesSelectNoneButton>,
                &mut BackgroundColor,
                &mut BorderColor,
            ),
            Or<(
                With<ScenesManageButton>,
                With<ScenesImportButton>,
                With<ScenesExportButton>,
                With<ScenesDeleteButton>,
                With<ScenesSelectAllButton>,
                With<ScenesSelectNoneButton>,
            )>,
        >,
    )>,
) {
    {
        let mut add_button = params.p0();
        if let Ok((interaction, mut bg, mut border)) = add_button.single_mut() {
            let (mut bg_color, mut border_color) = if state.add_open {
                (
                    Color::srgba(0.07, 0.07, 0.09, 0.84),
                    Color::srgba(0.35, 0.35, 0.42, 0.75),
                )
            } else {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };
            match *interaction {
                Interaction::Pressed => {
                    bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
                    border_color = Color::srgba(0.45, 0.45, 0.55, 0.85);
                }
                Interaction::Hovered => {
                    bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
                    border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
                }
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }

    {
        let mut scene_buttons = params.p1();
        for (interaction, button, mut bg, mut border) in &mut scene_buttons {
            let selected = if state.multi_select_mode {
                state.multi_selected_scenes.contains(&button.scene_id)
            } else {
                button.scene_id == active.scene_id
            };
            apply_option_style(selected, *interaction, &mut bg, &mut border);
        }
    }

    {
        let mut name_field = params.p2();
        if let Ok((interaction, mut bg, mut border)) = name_field.single_mut() {
            let focused = state.focused_field == ScenesUiField::AddSceneName && state.add_open;
            let (mut bg_color, border_color) = if focused {
                (
                    Color::srgba(0.03, 0.03, 0.04, 0.78),
                    Color::srgba(0.45, 0.45, 0.55, 0.80),
                )
            } else {
                (
                    Color::srgba(0.02, 0.02, 0.03, 0.65),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };
            match *interaction {
                Interaction::Pressed => bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92),
                Interaction::Hovered => bg_color = Color::srgba(0.03, 0.03, 0.04, 0.70),
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }

    {
        let mut add_panel_buttons = params.p3();
        for (interaction, add, cancel, mut bg, mut border) in &mut add_panel_buttons {
            let base = if add.is_some() {
                (
                    Color::srgba(0.06, 0.10, 0.07, 0.78),
                    Color::srgb(0.25, 0.80, 0.45),
                )
            } else if cancel.is_some() {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            } else {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };

            let (mut bg_color, mut border_color) = base;
            match *interaction {
                Interaction::Pressed => {
                    bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
                    border_color = Color::srgba(0.45, 0.45, 0.55, 0.85);
                }
                Interaction::Hovered => {
                    bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
                    border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
                }
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }

    {
        let mut action_buttons = params.p4();
        for (interaction, manage, _import, export, delete, _all, _none, mut bg, mut border) in
            &mut action_buttons
        {
            let selected = manage.is_some() && state.multi_select_mode;
            let base = if delete.is_some() {
                (
                    Color::srgba(0.12, 0.06, 0.06, 0.78),
                    Color::srgb(0.88, 0.40, 0.40),
                )
            } else if export.is_some() {
                (
                    Color::srgba(0.06, 0.08, 0.12, 0.78),
                    Color::srgb(0.40, 0.62, 0.92),
                )
            } else if selected {
                (
                    Color::srgba(0.07, 0.07, 0.09, 0.84),
                    Color::srgba(0.35, 0.35, 0.42, 0.75),
                )
            } else {
                (
                    Color::srgba(0.05, 0.05, 0.06, 0.75),
                    Color::srgba(0.25, 0.25, 0.30, 0.65),
                )
            };

            let (mut bg_color, mut border_color) = base;
            match *interaction {
                Interaction::Pressed => {
                    bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
                    border_color = Color::srgba(0.45, 0.45, 0.55, 0.85);
                }
                Interaction::Hovered => {
                    bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
                    border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
                }
                Interaction::None => {}
            }
            *bg = BackgroundColor(bg_color);
            *border = BorderColor::all(border_color);
        }
    }
}

fn apply_option_style(
    selected: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
    border: &mut BorderColor,
) {
    let (mut bg_color, mut border_color) = if selected {
        (
            Color::srgba(0.06, 0.10, 0.07, 0.85),
            Color::srgb(0.25, 0.80, 0.45),
        )
    } else {
        (
            Color::srgba(0.05, 0.05, 0.06, 0.75),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };
    match interaction {
        Interaction::Pressed => {
            bg_color = Color::srgba(0.10, 0.18, 0.13, 0.92);
        }
        Interaction::Hovered => {
            bg_color = Color::srgba(0.07, 0.07, 0.09, 0.84);
            if !selected {
                border_color = Color::srgba(0.35, 0.35, 0.42, 0.75);
            }
        }
        Interaction::None => {}
    }
    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

fn validate_scene_dir_name(raw: &str) -> Result<String, String> {
    let name = raw.trim();
    if name.is_empty() {
        return Err("Name cannot be empty.".to_string());
    }
    if name == "." || name == ".." {
        return Err("Name cannot be '.' or '..'.".to_string());
    }
    if name.ends_with('.') {
        return Err("Name cannot end with '.'.".to_string());
    }
    if name.chars().any(|c| c == '/' || c == '\\') {
        return Err("Name cannot contain path separators.".to_string());
    }
    if let Some(ch) = name
        .chars()
        .find(|c| c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*' | '\0'))
    {
        return Err(format!("Name contains invalid character: '{ch}'"));
    }
    if is_windows_reserved_name(name) {
        return Err("Name is reserved on Windows.".to_string());
    }
    Ok(name.to_string())
}

fn is_windows_reserved_name(name: &str) -> bool {
    let upper = name.trim().to_ascii_uppercase();
    if upper.is_empty() {
        return false;
    }
    let base = upper.split('.').next().unwrap_or("");
    matches!(
        base,
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

#[allow(dead_code)]
fn scene_build_scene_dat_path(realm_id: &str, scene_id: &str) -> PathBuf {
    let scene_dat = crate::paths::scene_dat_path(realm_id, scene_id);
    let dir = scene_dat
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    dir.join("scene.build.grav")
}
