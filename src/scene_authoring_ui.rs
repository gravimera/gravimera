use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::object::registry::ObjectLibrary;
use crate::realm::ActiveRealmScene;
use crate::scene_sources_runtime::{
    compile_scene_sources_all_layers, reload_scene_sources_in_workspace, scene_signature_summary,
    validate_scene_sources, SceneSourcesWorkspace, SceneWorldInstance,
};
use crate::scene_store::SceneSaveRequest;
use crate::scene_validation::{HardGateSpecV1, ScorecardSpecV1};
use crate::types::{
    BuildObject, Commandable, ObjectId, ObjectPrefabId, ObjectTint, Player, SceneLayerOwner,
};

const PANEL_Z_INDEX: i32 = 940;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneUiField {
    None,
    SceneDescription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneUiAction {
    SaveDescription,
    Build,
    Compile,
    Validate,
}

#[derive(Resource, Debug)]
pub(crate) struct SceneAuthoringUiState {
    open: bool,
    focused_field: SceneUiField,
    status: String,
    error: Option<String>,
    realm_dropdown_open: bool,
    realms_dirty: bool,
    scenes_dirty: bool,
    description: String,
    description_dirty: bool,
    last_active: Option<(String, String)>,
}

impl Default for SceneAuthoringUiState {
    fn default() -> Self {
        Self {
            open: false,
            focused_field: SceneUiField::None,
            status: "Ready.".to_string(),
            error: None,
            realm_dropdown_open: false,
            realms_dirty: true,
            scenes_dirty: true,
            description: String::new(),
            description_dirty: false,
            last_active: None,
        }
    }
}

impl SceneAuthoringUiState {
    pub(crate) fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    pub(crate) fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
    }

    pub(crate) fn clear_error(&mut self) {
        self.error = None;
    }
}

pub(crate) fn scene_ui_closed(state: Res<SceneAuthoringUiState>) -> bool {
    !state.open
}

#[derive(Component)]
pub(crate) struct SceneUiToggleButton;

#[derive(Component)]
pub(crate) struct SceneUiPanelRoot;

#[derive(Component)]
pub(crate) struct SceneUiCloseButton;

#[derive(Component)]
pub(crate) struct SceneUiStatusText;

#[derive(Component)]
pub(crate) struct SceneUiErrorText;

#[derive(Component)]
pub(crate) struct SceneUiBuildProgressText;

#[derive(Component)]
pub(crate) struct SceneUiRealmDropdownButton;

#[derive(Component)]
pub(crate) struct SceneUiRealmDropdownButtonText;

#[derive(Component)]
pub(crate) struct SceneUiRealmDropdownList;

#[derive(Component)]
pub(crate) struct SceneUiRealmOptionButton {
    realm_id: String,
}

#[derive(Component)]
pub(crate) struct SceneUiSceneTabsRoot;

#[derive(Component)]
pub(crate) struct SceneUiSceneTabButton {
    scene_id: String,
}

#[derive(Component)]
pub(crate) struct SceneUiTextField {
    field: SceneUiField,
}

#[derive(Component)]
pub(crate) struct SceneUiTextFieldText {
    field: SceneUiField,
}

#[derive(Component)]
pub(crate) struct SceneUiActionButton {
    action: SceneUiAction,
}

pub(crate) fn setup_scene_authoring_ui(mut commands: Commands) {
    commands
        .spawn((
            Button,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(92.0),
                padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.65),
                offset: Val::Px(0.0),
            },
            ZIndex(PANEL_Z_INDEX + 10),
            SceneUiToggleButton,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new("Scene"),
                TextFont {
                    font_size: 16.0,
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.96)),
            ));
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(560.0),
                max_height: Val::Px(680.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(10.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.88)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
            Outline {
                width: Val::Px(1.0),
                color: Color::srgba(0.25, 0.25, 0.30, 0.75),
                offset: Val::Px(0.0),
            },
            ZIndex(PANEL_Z_INDEX),
            Visibility::Hidden,
            SceneUiPanelRoot,
        ))
        .with_children(|root| {
            // Header.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new("Scene Builder"),
                    TextFont {
                        font_size: 18.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.95, 0.97)),
                ));

                row.spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    SceneUiCloseButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("X"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
            });

            // Realm selection row.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new("Realm:"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.85, 0.90)),
                ));

                row.spawn((
                    Button,
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.70)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    SceneUiRealmDropdownButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        SceneUiRealmDropdownButtonText,
                    ));
                });
            });

            // Realm dropdown list.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.92)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                Visibility::Hidden,
                SceneUiRealmDropdownList,
            ));

            // Scene tabs row.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new("Scene:"),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.85, 0.85, 0.90)),
                ));

                row.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        flex_direction: FlexDirection::Row,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: Val::Px(8.0),
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    SceneUiSceneTabsRoot,
                ));
            });

            // Description label.
            root.spawn((
                Text::new("Scene description (terrain, buildings, units, story setup):"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.90)),
            ));

            // Description text area.
            root.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(220.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                SceneUiTextField {
                    field: SceneUiField::SceneDescription,
                },
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    SceneUiTextFieldText {
                        field: SceneUiField::SceneDescription,
                    },
                ));
            });

            // Actions.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(8.0),
                    row_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                spawn_action_button(row, "Build", SceneUiAction::Build);
                spawn_action_button(row, "Compile", SceneUiAction::Compile);
                spawn_action_button(row, "Validate", SceneUiAction::Validate);
                spawn_action_button(row, "Save desc", SceneUiAction::SaveDescription);
            });

            // Build progress.
            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 13.0,
                    ..default()
                },
                TextColor(Color::srgb(0.78, 0.86, 0.98)),
                SceneUiBuildProgressText,
            ));

            // Status + error.
            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.92, 0.85)),
                SceneUiStatusText,
            ));
            root.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.95, 0.55, 0.45)),
                SceneUiErrorText,
            ));
        });
}

fn spawn_action_button(parent: &mut ChildSpawnerCommands, label: &str, action: SceneUiAction) {
    parent
        .spawn((
            Button,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            SceneUiActionButton { action },
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

pub(crate) fn scene_ui_toggle_button(
    mut state: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SceneUiToggleButton>),
    >,
) {
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                state.open = !state.open;
                if state.open {
                    state.realms_dirty = true;
                    state.scenes_dirty = true;
                    state.error = None;
                } else {
                    state.realm_dropdown_open = false;
                    state.focused_field = SceneUiField::None;
                }
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

pub(crate) fn scene_ui_close_button(
    mut state: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SceneUiCloseButton>),
    >,
) {
    if !state.open {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                state.open = false;
                state.realm_dropdown_open = false;
                state.focused_field = SceneUiField::None;
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.75));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
            }
        }
    }
}

pub(crate) fn scene_ui_panel_visibility(
    state: Res<SceneAuthoringUiState>,
    mut roots: Query<&mut Visibility, With<SceneUiPanelRoot>>,
) {
    let vis = if state.open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut roots {
        *v = vis;
    }
}

pub(crate) fn scene_ui_sync_active_scene(
    mut state: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
) {
    if !state.open {
        return;
    }
    let key = (active.realm_id.clone(), active.scene_id.clone());
    if state.last_active.as_ref() == Some(&key) {
        return;
    }
    state.last_active = Some(key);
    state.realms_dirty = true;
    state.scenes_dirty = true;
    state.realm_dropdown_open = false;

    let src_dir = crate::realm::scene_src_dir(&active);
    match crate::realm::load_scene_description(&src_dir) {
        Ok(desc) => {
            state.description = desc;
            state.description_dirty = false;
            state.status = format!(
                "Loaded description for {}/{}.",
                active.realm_id, active.scene_id
            );
        }
        Err(err) => {
            state.error = Some(err);
        }
    }
}

pub(crate) fn scene_ui_realm_dropdown_button(
    mut state: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<SceneUiRealmDropdownButton>),
    >,
) {
    if !state.open {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            state.realm_dropdown_open = !state.realm_dropdown_open;
        }

        let mut color = if state.realm_dropdown_open {
            Color::srgba(0.03, 0.03, 0.04, 0.82)
        } else {
            Color::srgba(0.02, 0.02, 0.03, 0.70)
        };
        match *interaction {
            Interaction::Pressed => color = Color::srgba(0.10, 0.10, 0.12, 0.92),
            Interaction::Hovered => color = Color::srgba(0.06, 0.06, 0.08, 0.86),
            Interaction::None => {}
        }
        *bg = BackgroundColor(color);
    }
}

pub(crate) fn scene_ui_rebuild_realm_list(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    list: Query<(Entity, Option<&Children>), With<SceneUiRealmDropdownList>>,
) {
    if !state.open || !state.realms_dirty {
        return;
    }
    state.realms_dirty = false;

    let Ok((list_entity, children)) = list.single() else {
        return;
    };
    if let Some(children) = children {
        for child in children.iter() {
            commands.entity(child).try_despawn();
        }
    }

    commands.entity(list_entity).with_children(|parent| {
        for realm_id in crate::realm::list_realms() {
            parent
                .spawn((
                    Button,
                    Node {
                        width: Val::Percent(100.0),
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    SceneUiRealmOptionButton {
                        realm_id: realm_id.clone(),
                    },
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(realm_id),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
        }
    });
}

pub(crate) fn scene_ui_rebuild_scene_tabs(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    tabs: Query<(Entity, Option<&Children>), With<SceneUiSceneTabsRoot>>,
) {
    if !state.open || !state.scenes_dirty {
        return;
    }
    state.scenes_dirty = false;

    let Ok((tabs_entity, children)) = tabs.single() else {
        return;
    };
    if let Some(children) = children {
        for child in children.iter() {
            commands.entity(child).try_despawn();
        }
    }

    commands.entity(tabs_entity).with_children(|parent| {
        for scene_id in crate::realm::list_scenes(&active.realm_id) {
            parent
                .spawn((
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                    SceneUiSceneTabButton {
                        scene_id: scene_id.clone(),
                    },
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new(scene_id),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
        }
    });
}

pub(crate) fn scene_ui_realm_option_buttons(
    mut state: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<(&Interaction, &SceneUiRealmOptionButton), Changed<Interaction>>,
) {
    if !state.open || !state.realm_dropdown_open {
        return;
    }

    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
            continue;
        }

        let mut scene_id = active.scene_id.clone();
        let scenes = crate::realm::list_scenes(&button.realm_id);
        if !scenes.iter().any(|s| s == &scene_id) {
            scene_id = scenes
                .first()
                .cloned()
                .unwrap_or_else(|| crate::paths::default_scene_id().to_string());
        }

        pending.target = Some(ActiveRealmScene {
            realm_id: button.realm_id.clone(),
            scene_id,
        });
        saves.write(SceneSaveRequest::new("switch realm/scene"));
        state.realm_dropdown_open = false;
        state.status = "Switching realm/scene (saving current scene first)...".to_string();
    }
}

pub(crate) fn scene_ui_scene_tab_buttons(
    mut state: ResMut<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    mut pending: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<SceneSaveRequest>,
    mut buttons: Query<(&Interaction, &SceneUiSceneTabButton), Changed<Interaction>>,
) {
    if !state.open {
        return;
    }

    for (interaction, button) in &mut buttons {
        if !matches!(*interaction, Interaction::Pressed) {
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
        state.status = "Switching scene (saving current scene first)...".to_string();
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
            Color::srgba(0.02, 0.02, 0.03, 0.65),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };
    match interaction {
        Interaction::Pressed => {
            bg_color = Color::srgba(0.10, 0.18, 0.13, 0.92);
        }
        Interaction::Hovered => {
            bg_color = Color::srgba(0.06, 0.06, 0.075, 0.78);
            if !selected {
                border_color = Color::srgba(0.35, 0.35, 0.40, 0.70);
            }
        }
        Interaction::None => {}
    }
    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

pub(crate) fn scene_ui_update_realm_scene_button_styles(
    state: Res<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    mut realm_buttons: Query<
        (
            &Interaction,
            &SceneUiRealmOptionButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Without<SceneUiSceneTabButton>,
    >,
    mut scene_buttons: Query<
        (
            &Interaction,
            &SceneUiSceneTabButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Without<SceneUiRealmOptionButton>,
    >,
) {
    if !state.open {
        return;
    }

    for (interaction, button, mut bg, mut border) in &mut realm_buttons {
        let selected = button.realm_id == active.realm_id;
        apply_option_style(selected, *interaction, &mut bg, &mut border);
    }

    for (interaction, button, mut bg, mut border) in &mut scene_buttons {
        let selected = button.scene_id == active.scene_id;
        apply_option_style(selected, *interaction, &mut bg, &mut border);
    }
}

pub(crate) fn scene_ui_text_field_focus(
    mut state: ResMut<SceneAuthoringUiState>,
    mut fields: Query<
        (&Interaction, &SceneUiTextField, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !state.open {
        return;
    }
    for (interaction, field, mut bg) in &mut fields {
        match *interaction {
            Interaction::Pressed => {
                state.focused_field = field.field;
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                let alpha = if state.focused_field == field.field {
                    0.70
                } else {
                    0.65
                };
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, alpha));
            }
        }
    }
}

pub(crate) fn scene_ui_text_input(
    mut state: ResMut<SceneAuthoringUiState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: MessageReader<KeyboardInput>,
) {
    if !state.open {
        keyboard.clear();
        return;
    }
    if state.focused_field == SceneUiField::None {
        return;
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }

        let focused_field = state.focused_field;
        let allow_newlines = focused_field == SceneUiField::SceneDescription;

        match event.key_code {
            KeyCode::Backspace => {
                field_string_mut(&mut state, focused_field).pop();
                state.description_dirty = true;
            }
            KeyCode::Escape => {
                state.focused_field = SceneUiField::None;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if allow_newlines {
                    field_string_mut(&mut state, focused_field).push('\n');
                    state.description_dirty = true;
                } else {
                    state.focused_field = SceneUiField::None;
                }
            }
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if modifier {
                    if let Some(text) = crate::clipboard::read_text() {
                        let target = field_string_mut(&mut state, focused_field);
                        push_text(target, &text, allow_newlines);
                        state.description_dirty = true;
                    }
                    continue;
                }
                if let Some(text) = &event.text {
                    let target = field_string_mut(&mut state, focused_field);
                    push_text(target, text, allow_newlines);
                    state.description_dirty = true;
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                let target = field_string_mut(&mut state, focused_field);
                push_text(target, text, allow_newlines);
                state.description_dirty = true;
            }
        }
    }
}

fn field_string_mut(state: &mut SceneAuthoringUiState, field: SceneUiField) -> &mut String {
    match field {
        SceneUiField::SceneDescription => &mut state.description,
        SceneUiField::None => panic!("field_string_mut called with None"),
    }
}

fn push_text(target: &mut String, text: &str, allow_newlines: bool) {
    let mut inserted = 0usize;
    for ch in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
        if ch.is_control() && !(allow_newlines && ch == '\n') && ch != '\t' {
            continue;
        }
        target.push(ch);
        inserted += 1;
        if inserted >= 4096 {
            break;
        }
    }
}

pub(crate) fn scene_ui_clear_keyboard_state_when_captured(
    state: Res<SceneAuthoringUiState>,
    mut keys: Option<ResMut<ButtonInput<KeyCode>>>,
) {
    if !state.open || state.focused_field == SceneUiField::None {
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

pub(crate) fn scene_ui_action_buttons(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    config: Res<crate::config::AppConfig>,
    mut build_ai: ResMut<crate::scene_build_ai::SceneBuildAiRuntime>,
    active: Res<ActiveRealmScene>,
    library: Res<ObjectLibrary>,
    mut workspace: ResMut<SceneSourcesWorkspace>,
    mut buttons: Query<
        (&Interaction, &SceneUiActionButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
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
    if !state.open {
        return;
    }

    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.90));
                state.error = None;

                let src_dir = crate::realm::scene_src_dir(&active);
                if workspace.loaded_from_dir.as_deref() != Some(src_dir.as_path()) {
                    workspace.loaded_from_dir = Some(src_dir.clone());
                    workspace.sources = None;
                }

                let result: Result<String, String> = match button.action {
                    SceneUiAction::SaveDescription => crate::realm::save_scene_description(
                        &src_dir,
                        state.description.trim_end_matches('\n'),
                    )
                    .map(|_| {
                        state.description_dirty = false;
                        "Saved scene description.".to_string()
                    }),
                    SceneUiAction::Build => crate::realm::save_scene_description(
                        &src_dir,
                        state.description.trim_end_matches('\n'),
                    )
                    .and_then(|_| {
                        state.description_dirty = false;
                        crate::scene_build_ai::start_scene_build_from_description(
                            &mut build_ai,
                            &config,
                            &active,
                            &library,
                            state.description.trim_end_matches('\n'),
                        )
                        .map(|run_id| format!("Build started (run_id={run_id})."))
                    }),
                    SceneUiAction::Compile => {
                        let do_compile =
                            |commands: &mut Commands, workspace: &SceneSourcesWorkspace| {
                                let existing = scene_instances.iter().map(
                                    |(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                                        entity: e,
                                        instance_id: *id,
                                        prefab_id: *prefab,
                                        transform: t.clone(),
                                        tint: tint.map(|t| t.0),
                                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                                    },
                                );

                                compile_scene_sources_all_layers(commands, workspace, &library, existing)
                                    .map(|report| {
                                        let sig = scene_signature_summary(scene_instances.iter().map(
                                            |(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                                                entity: e,
                                                instance_id: *id,
                                                prefab_id: *prefab,
                                                transform: t.clone(),
                                                tint: tint.map(|t| t.0),
                                                owner_layer_id: owner.map(|o| o.layer_id.clone()),
                                            },
                                        ));
                                        match sig {
                                            Ok(sig) => format!(
                                                "Compiled: spawned={} updated={} despawned={} (total_instances={}; overall_sig={}).",
                                                report.spawned,
                                                report.updated,
                                                report.despawned,
                                                sig.total_instances,
                                                sig.overall_sig
                                            ),
                                            Err(_) => format!(
                                                "Compiled: spawned={} updated={} despawned={}.",
                                                report.spawned, report.updated, report.despawned
                                            ),
                                        }
                                    })
                            };

                        reload_scene_sources_in_workspace(&mut workspace)
                            .and_then(|_| do_compile(&mut commands, &workspace))
                    }
                    SceneUiAction::Validate => {
                        let scorecard = default_scorecard();
                        let do_validate =
                            |workspace: &SceneSourcesWorkspace| {
                                let existing = scene_instances.iter().map(
                                    |(e, t, id, prefab, tint, owner)| SceneWorldInstance {
                                        entity: e,
                                        instance_id: *id,
                                        prefab_id: *prefab,
                                        transform: t.clone(),
                                        tint: tint.map(|t| t.0),
                                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                                    },
                                );
                                validate_scene_sources(workspace, &library, existing, &scorecard)
                                    .map(|report| {
                                        let violations = report.violations.len();
                                        if report.hard_gates_passed {
                                            format!("Validation OK (violations: {violations}).")
                                        } else {
                                            format!("Validation FAILED (violations: {violations}).")
                                        }
                                    })
                            };

                        reload_scene_sources_in_workspace(&mut workspace)
                            .and_then(|_| do_validate(&workspace))
                    }
                };

                match result {
                    Ok(msg) => state.status = msg,
                    Err(err) => state.error = Some(err),
                }
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.80));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75));
            }
        }
    }
}

pub(crate) fn scene_ui_update_texts(
    state: Res<SceneAuthoringUiState>,
    active: Res<ActiveRealmScene>,
    build_ai: Res<crate::scene_build_ai::SceneBuildAiRuntime>,
    mut realm_text: Query<&mut Text, With<SceneUiRealmDropdownButtonText>>,
    mut realm_list: Query<(&mut Node, &mut Visibility), With<SceneUiRealmDropdownList>>,
    mut status: Query<
        &mut Text,
        (
            With<SceneUiStatusText>,
            Without<SceneUiErrorText>,
            Without<SceneUiTextFieldText>,
            Without<SceneUiRealmDropdownButtonText>,
            Without<SceneUiBuildProgressText>,
        ),
    >,
    mut errors: Query<
        &mut Text,
        (
            With<SceneUiErrorText>,
            Without<SceneUiStatusText>,
            Without<SceneUiTextFieldText>,
            Without<SceneUiRealmDropdownButtonText>,
            Without<SceneUiBuildProgressText>,
        ),
    >,
    mut progress: Query<
        &mut Text,
        (
            With<SceneUiBuildProgressText>,
            Without<SceneUiStatusText>,
            Without<SceneUiErrorText>,
            Without<SceneUiTextFieldText>,
            Without<SceneUiRealmDropdownButtonText>,
        ),
    >,
    mut fields: Query<
        (&SceneUiTextFieldText, &mut Text),
        (
            Without<SceneUiStatusText>,
            Without<SceneUiErrorText>,
            Without<SceneUiRealmDropdownButtonText>,
            Without<SceneUiBuildProgressText>,
        ),
    >,
) {
    if !state.open {
        return;
    }

    for mut t in &mut realm_text {
        **t = format!("{} ▾", active.realm_id).into();
    }

    if let Ok((mut node, mut vis)) = realm_list.single_mut() {
        if state.realm_dropdown_open {
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        } else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
    }

    for mut t in &mut status {
        **t = state.status.clone().into();
    }
    for mut t in &mut errors {
        **t = state.error.clone().unwrap_or_default().into();
    }

    let progress_summary = build_ai.ui_progress_summary();
    for mut t in &mut progress {
        **t = progress_summary.clone().into();
    }

    for (field, mut text) in &mut fields {
        let mut value = match field.field {
            SceneUiField::SceneDescription => state.description.clone(),
            SceneUiField::None => String::new(),
        };

        if state.focused_field == field.field {
            value.push('|');
        }
        if value.trim().is_empty() {
            value = "<click to edit; paste scene description; then press Build>".to_string();
        }
        **text = value.into();
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
