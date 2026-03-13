use bevy::prelude::*;

use crate::rich_text::spawn_rich_text_line;
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts};
use crate::workspace_scenes_ui::{
    AddSceneAddButton, AddSceneCancelButton, AddSceneErrorText, AddSceneNameField,
    AddSceneNameFieldText, AddScenePanelRoot, ScenesAddSceneButton, ScenesAddSceneButtonText,
    ScenesList, ScenesListScrollPanel,
};

const WORKSPACE_UI_Z_INDEX: i32 = 960;
const SIDE_PANEL_Z_INDEX: i32 = 930;
const TOOLBAR_BUTTON_WIDTH_PX: f32 = 132.0;
const TOOLBAR_BUTTON_HEIGHT_PX: f32 = 34.0;
const SCENE_BUILDER_BUTTON_WIDTH_PX: f32 = 168.0;
const SIDE_PANEL_WIDTH_PX: f32 = 260.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WorkspaceTab {
    ObjectPreview,
    SceneBuild,
}

impl WorkspaceTab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            WorkspaceTab::ObjectPreview => "Object Preview",
            WorkspaceTab::SceneBuild => "Scene Build",
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct WorkspaceUiState {
    pub(crate) tab: WorkspaceTab,
}

impl Default for WorkspaceUiState {
    fn default() -> Self {
        Self {
            tab: WorkspaceTab::ObjectPreview,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopPanelTab {
    Scenes,
    Models,
}

#[derive(Resource, Debug)]
pub(crate) struct TopPanelUiState {
    pub(crate) selected: Option<TopPanelTab>,
}

impl Default for TopPanelUiState {
    fn default() -> Self {
        Self {
            selected: Some(TopPanelTab::Models),
        }
    }
}

impl TopPanelUiState {
    pub(crate) fn toggle(&mut self, tab: TopPanelTab) {
        self.selected = if self.selected == Some(tab) {
            None
        } else {
            Some(tab)
        };
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkspaceTabSwitch {
    pub(crate) from: WorkspaceTab,
    pub(crate) to: WorkspaceTab,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WorkspaceCameraSnapshot {
    pub(crate) zoom_t: f32,
    pub(crate) yaw: f32,
    pub(crate) yaw_initialized: bool,
    pub(crate) pitch: f32,
    pub(crate) focus: Vec3,
    pub(crate) focus_initialized: bool,
}

impl Default for WorkspaceCameraSnapshot {
    fn default() -> Self {
        Self {
            zoom_t: crate::constants::CAMERA_ZOOM_DEFAULT,
            yaw: 0.0,
            yaw_initialized: false,
            pitch: 0.0,
            focus: Vec3::ZERO,
            focus_initialized: false,
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct WorkspaceCameraState {
    object_preview: WorkspaceCameraSnapshot,
    scene_build: WorkspaceCameraSnapshot,
}

impl Default for WorkspaceCameraState {
    fn default() -> Self {
        Self {
            object_preview: WorkspaceCameraSnapshot::default(),
            scene_build: WorkspaceCameraSnapshot::default(),
        }
    }
}

impl WorkspaceCameraState {
    pub(crate) fn get(&self, tab: WorkspaceTab) -> WorkspaceCameraSnapshot {
        match tab {
            WorkspaceTab::ObjectPreview => self.object_preview,
            WorkspaceTab::SceneBuild => self.scene_build,
        }
    }

    pub(crate) fn set(&mut self, tab: WorkspaceTab, snapshot: WorkspaceCameraSnapshot) {
        match tab {
            WorkspaceTab::ObjectPreview => self.object_preview = snapshot,
            WorkspaceTab::SceneBuild => self.scene_build = snapshot,
        }
    }
}

#[derive(Resource, Default, Debug)]
pub(crate) struct PendingWorkspaceSwitch {
    pending: Option<WorkspaceTabSwitch>,
}

impl PendingWorkspaceSwitch {
    pub(crate) fn request(&mut self, from: WorkspaceTab, to: WorkspaceTab) {
        self.pending = Some(WorkspaceTabSwitch { from, to });
    }

    pub(crate) fn take(&mut self) -> Option<WorkspaceTabSwitch> {
        self.pending.take()
    }
}

#[derive(Component)]
pub(crate) struct WorkspaceUiRoot;

#[derive(Component)]
pub(crate) struct WorkspaceScenesToggleButton;

#[derive(Component)]
pub(crate) struct WorkspaceScenesToggleButtonText;

#[derive(Component)]
pub(crate) struct WorkspaceModelsToggleButton;

#[derive(Component)]
pub(crate) struct WorkspaceModelsToggleButtonText;

#[derive(Component)]
pub(crate) struct WorkspaceSceneBuilderButton;

#[derive(Component)]
pub(crate) struct WorkspaceSceneBuilderButtonText;

#[derive(Component)]
pub(crate) struct ScenesPanelRoot;

pub(crate) fn setup_workspace_ui(
    mut commands: Commands,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                left: Val::Px(10.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                align_items: AlignItems::Center,
                ..default()
            },
            ZIndex(WORKSPACE_UI_Z_INDEX),
            WorkspaceUiRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Button,
                Node {
                    width: Val::Px(SCENE_BUILDER_BUTTON_WIDTH_PX),
                    height: Val::Px(TOOLBAR_BUTTON_HEIGHT_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
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
                WorkspaceScenesToggleButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Scenes"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    WorkspaceScenesToggleButtonText,
                ));
            });

            root.spawn((
                Button,
                Node {
                    width: Val::Px(TOOLBAR_BUTTON_WIDTH_PX),
                    height: Val::Px(TOOLBAR_BUTTON_HEIGHT_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
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
                WorkspaceModelsToggleButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Prefabs"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    WorkspaceModelsToggleButtonText,
                ));
            });

            root.spawn((
                Button,
                Node {
                    width: Val::Px(TOOLBAR_BUTTON_WIDTH_PX),
                    height: Val::Px(TOOLBAR_BUTTON_HEIGHT_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
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
                WorkspaceSceneBuilderButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Scene Builder"),
                    TextLayout::new_with_no_wrap(),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    WorkspaceSceneBuilderButtonText,
                ));
            });

            root.spawn((
                Button,
                Node {
                    width: Val::Px(92.0),
                    height: Val::Px(TOOLBAR_BUTTON_HEIGHT_PX),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
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
                crate::build::GameModeToggleButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Play"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    crate::build::GameModeToggleButtonText,
                ));
            });
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(SIDE_PANEL_WIDTH_PX),
                height: Val::Px(680.0),
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
            ZIndex(SIDE_PANEL_Z_INDEX),
            Visibility::Hidden,
            ScenesPanelRoot,
        ))
        .with_children(|root| {
            // Header row.
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
                    Text::new("Scenes"),
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
                    ScenesAddSceneButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Add Scene"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ScenesAddSceneButtonText,
                    ));
                });
            });

            // Add Scene panel (hidden by default).
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    display: Display::None,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    padding: UiRect::all(Val::Px(10.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                AddScenePanelRoot,
            ))
            .with_children(|panel| {
                panel
                    .spawn((
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
                            Text::new("Name:"),
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
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            AddSceneNameField,
                        ))
                        .with_children(|b| {
                            spawn_rich_text_line(
                                b,
                                "",
                                &ui_fonts,
                                &emoji_atlas,
                                &asset_server,
                                14.0,
                                Color::srgb(0.92, 0.92, 0.96),
                                (
                                    Node {
                                        width: Val::Percent(100.0),
                                        flex_wrap: FlexWrap::Wrap,
                                        justify_content: JustifyContent::FlexStart,
                                        align_items: AlignItems::Center,
                                        column_gap: Val::Px(1.0),
                                        row_gap: Val::Px(2.0),
                                        ..default()
                                    },
                                    AddSceneNameFieldText,
                                ),
                                None,
                            );
                        });
                    });

                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::FlexEnd,
                            align_items: AlignItems::Center,
                            column_gap: Val::Px(8.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.10, 0.07, 0.78)),
                            BorderColor::all(Color::srgb(0.25, 0.80, 0.45)),
                            AddSceneAddButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Add"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });

                        row.spawn((
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            AddSceneCancelButton,
                        ))
                        .with_children(|b| {
                            b.spawn((
                                Text::new("Cancel"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                            ));
                        });
                    });

                panel.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.95, 0.55, 0.45)),
                    AddSceneErrorText,
                ));
            });

            // Scenes list.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    flex_grow: 1.0,
                    flex_basis: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                row.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        min_height: Val::Px(0.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    ScrollPosition::default(),
                    ScenesListScrollPanel,
                ))
                .with_children(|scroll| {
                    scroll.spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        ScenesList,
                    ));
                });
            });
        });
}

pub(crate) fn workspace_ui_update_visibility(
    build_scene: Res<State<BuildScene>>,
    mut roots: Query<&mut Visibility, With<WorkspaceUiRoot>>,
) {
    let visible = matches!(build_scene.get(), BuildScene::Realm);
    for mut v in &mut roots {
        *v = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn workspace_toolbar_toggle_buttons(
    mut state: ResMut<TopPanelUiState>,
    mut buttons: Query<
        (
            &Interaction,
            Option<&WorkspaceScenesToggleButton>,
            Option<&WorkspaceModelsToggleButton>,
        ),
        (
            Changed<Interaction>,
            Or<(
                With<WorkspaceScenesToggleButton>,
                With<WorkspaceModelsToggleButton>,
            )>,
        ),
    >,
) {
    for (interaction, scenes, models) in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        if scenes.is_some() {
            state.toggle(TopPanelTab::Scenes);
        } else if models.is_some() {
            state.toggle(TopPanelTab::Models);
        }
    }
}

pub(crate) fn workspace_toolbar_sync_model_library_open(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<TopPanelUiState>,
    mut model_library: ResMut<crate::model_library_ui::ModelLibraryUiState>,
) {
    let open = matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
        && state.selected == Some(TopPanelTab::Models);
    model_library.set_open(open);
}

pub(crate) fn workspace_toolbar_update_visibility(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut buttons: Query<
        (&mut Node, &mut Visibility),
        Or<(
            With<WorkspaceScenesToggleButton>,
            With<WorkspaceModelsToggleButton>,
            With<WorkspaceSceneBuilderButton>,
        )>,
    >,
) {
    let visible =
        matches!(mode.get(), GameMode::Build) && matches!(build_scene.get(), BuildScene::Realm);
    for (mut node, mut vis) in &mut buttons {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn workspace_toolbar_update_toggle_button_styles(
    state: Res<TopPanelUiState>,
    mut buttons: Query<
        (
            &Interaction,
            Option<&WorkspaceScenesToggleButton>,
            Option<&WorkspaceModelsToggleButton>,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Or<(
            With<WorkspaceScenesToggleButton>,
            With<WorkspaceModelsToggleButton>,
        )>,
    >,
) {
    for (interaction, scenes, models, mut bg, mut border) in &mut buttons {
        let selected = if scenes.is_some() {
            state.selected == Some(TopPanelTab::Scenes)
        } else if models.is_some() {
            state.selected == Some(TopPanelTab::Models)
        } else {
            false
        };

        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.80));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::None => {
                if selected {
                    *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.85));
                    *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60));
                    *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
                }
            }
        }
    }
}

pub(crate) fn workspace_toolbar_scene_builder_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut scene_ui: ResMut<crate::scene_authoring_ui::SceneAuthoringUiState>,
    mut buttons: Query<&Interaction, (Changed<Interaction>, With<WorkspaceSceneBuilderButton>)>,
) {
    if !matches!(mode.get(), GameMode::Build) || !matches!(build_scene.get(), BuildScene::Realm) {
        return;
    }

    for interaction in &mut buttons {
        if *interaction != Interaction::Pressed {
            continue;
        }
        scene_ui.toggle_open();
    }
}

pub(crate) fn workspace_toolbar_update_scene_builder_button_styles(
    scene_ui: Res<crate::scene_authoring_ui::SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<WorkspaceSceneBuilderButton>,
    >,
) {
    for (interaction, mut bg, mut border) in &mut buttons {
        let selected = scene_ui.is_open();
        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
                *border = BorderColor::all(Color::srgba(0.45, 0.45, 0.55, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.80));
                *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
            }
            Interaction::None => {
                if selected {
                    *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.85));
                    *border = BorderColor::all(Color::srgba(0.35, 0.35, 0.42, 0.75));
                } else {
                    *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60));
                    *border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
                }
            }
        }
    }
}

pub(crate) fn workspace_toolbar_update_scenes_panel_visibility(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    state: Res<TopPanelUiState>,
    mut roots: Query<(&mut Node, &mut Visibility), With<ScenesPanelRoot>>,
) {
    let visible = matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), BuildScene::Realm)
        && state.selected == Some(TopPanelTab::Scenes);
    for (mut node, mut vis) in &mut roots {
        node.display = if visible {
            Display::Flex
        } else {
            Display::None
        };
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}
