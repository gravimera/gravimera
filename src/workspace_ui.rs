use bevy::prelude::*;

use crate::scene_authoring_ui::SceneAuthoringUiState;
use crate::types::{BuildScene, GameMode};

const WORKSPACE_UI_Z_INDEX: i32 = 960;
const DROPDOWN_WIDTH_PX: f32 = 170.0;
const BUTTON_HEIGHT_PX: f32 = 34.0;
const DROPDOWN_LIST_TOP_PX: f32 = 40.0;

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

    fn action_label(self) -> &'static str {
        match self {
            WorkspaceTab::ObjectPreview => "Gen3D",
            WorkspaceTab::SceneBuild => "Scene Build",
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct WorkspaceUiState {
    pub(crate) tab: WorkspaceTab,
    dropdown_open: bool,
}

impl Default for WorkspaceUiState {
    fn default() -> Self {
        Self {
            tab: WorkspaceTab::ObjectPreview,
            dropdown_open: false,
        }
    }
}

#[derive(Component)]
pub(crate) struct WorkspaceUiRoot;

#[derive(Component)]
pub(crate) struct WorkspaceDropdownButton;

#[derive(Component)]
pub(crate) struct WorkspaceDropdownButtonText;

#[derive(Component)]
pub(crate) struct WorkspaceDropdownList;

#[derive(Component)]
pub(crate) struct WorkspaceDropdownOptionButton {
    tab: WorkspaceTab,
}

impl WorkspaceDropdownOptionButton {
    fn new(tab: WorkspaceTab) -> Self {
        Self { tab }
    }
}

#[derive(Component)]
pub(crate) struct WorkspaceActionButton;

#[derive(Component)]
pub(crate) struct WorkspaceActionButtonText;

pub(crate) fn setup_workspace_ui(mut commands: Commands) {
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
                    width: Val::Px(DROPDOWN_WIDTH_PX),
                    height: Val::Px(BUTTON_HEIGHT_PX),
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
                WorkspaceDropdownButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Object Preview v"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    WorkspaceDropdownButtonText,
                ));
            });

            root.spawn((
                Button,
                Node {
                    width: Val::Px(132.0),
                    height: Val::Px(BUTTON_HEIGHT_PX),
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
                WorkspaceActionButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Gen3D"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    WorkspaceActionButtonText,
                ));
            });

            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(DROPDOWN_LIST_TOP_PX),
                    left: Val::Px(0.0),
                    width: Val::Px(DROPDOWN_WIDTH_PX),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    row_gap: Val::Px(6.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.92)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                Outline {
                    width: Val::Px(1.0),
                    color: Color::srgba(0.25, 0.25, 0.30, 0.75),
                    offset: Val::Px(0.0),
                },
                ZIndex(WORKSPACE_UI_Z_INDEX + 1),
                Visibility::Hidden,
                WorkspaceDropdownList,
            ))
            .with_children(|list| {
                for tab in [WorkspaceTab::ObjectPreview, WorkspaceTab::SceneBuild] {
                    list.spawn((
                        Button,
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Px(30.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.82)),
                        BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                        WorkspaceDropdownOptionButton::new(tab),
                    ))
                    .with_children(|b| {
                        b.spawn((
                            Text::new(tab.label()),
                            TextFont {
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ));
                    });
                }
            });
        });
}

pub(crate) fn workspace_ui_update_visibility(
    mode: Res<State<GameMode>>,
    mut state: ResMut<WorkspaceUiState>,
    mut roots: Query<&mut Visibility, With<WorkspaceUiRoot>>,
) {
    let visible = matches!(mode.get(), GameMode::Build);
    for mut v in &mut roots {
        *v = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !visible {
        state.dropdown_open = false;
    }
}

pub(crate) fn workspace_ui_dropdown_button(
    mut state: ResMut<WorkspaceUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<WorkspaceDropdownButton>),
    >,
) {
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                state.dropdown_open = !state.dropdown_open;
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

pub(crate) fn workspace_ui_dropdown_list_visibility(
    state: Res<WorkspaceUiState>,
    mut lists: Query<(&mut Node, &mut Visibility), With<WorkspaceDropdownList>>,
) {
    for (mut node, mut vis) in &mut lists {
        let open = state.dropdown_open;
        node.display = if open { Display::Flex } else { Display::None };
        *vis = if open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn workspace_ui_dropdown_option_buttons(
    mut ui_state: ResMut<WorkspaceUiState>,
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut scene_ui: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (
            &Interaction,
            &WorkspaceDropdownOptionButton,
            &mut BackgroundColor,
        ),
        Changed<Interaction>,
    >,
) {
    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                ui_state.tab = button.tab;
                ui_state.dropdown_open = false;

                // Switching tabs should always return to the main realm view.
                if matches!(build_scene.get(), BuildScene::Preview) {
                    next_build_scene.set(BuildScene::Realm);
                }

                // Scene Build panel should not stay open in Object Preview.
                if matches!(button.tab, WorkspaceTab::ObjectPreview) {
                    scene_ui.set_open(false);
                }

                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.92));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.09, 0.88));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.82));
            }
        }
    }
}

pub(crate) fn workspace_ui_update_labels(
    state: Res<WorkspaceUiState>,
    mut texts: ParamSet<(
        Query<&mut Text, With<WorkspaceDropdownButtonText>>,
        Query<&mut Text, With<WorkspaceActionButtonText>>,
    )>,
) {
    let dropdown_label = format!("{} v", state.tab.label());
    for mut text in &mut texts.p0() {
        **text = dropdown_label.clone();
    }
    for mut text in &mut texts.p1() {
        **text = state.tab.action_label().into();
    }
}

pub(crate) fn workspace_ui_action_button(
    mut ui_state: ResMut<WorkspaceUiState>,
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut scene_ui: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<WorkspaceActionButton>),
    >,
) {
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                match ui_state.tab {
                    WorkspaceTab::ObjectPreview => {
                        scene_ui.set_open(false);
                        if matches!(build_scene.get(), BuildScene::Preview) {
                            next_build_scene.set(BuildScene::Realm);
                        } else {
                            next_build_scene.set(BuildScene::Preview);
                        }
                    }
                    WorkspaceTab::SceneBuild => {
                        ui_state.dropdown_open = false;
                        scene_ui.toggle_open();
                    }
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
