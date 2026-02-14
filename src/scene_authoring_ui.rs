use bevy::ecs::message::MessageReader;
use bevy::input::keyboard::KeyboardInput;
use bevy::prelude::*;

use crate::object::registry::ObjectLibrary;
use crate::scene_sources::SceneSourcesIndexPaths;
use crate::scene_sources_runtime::{
    compile_scene_sources_all_layers, import_scene_sources_replace_world, regenerate_scene_layer,
    reload_scene_sources_in_workspace, scene_signature_summary, validate_scene_sources,
    SceneSourcesWorkspace, SceneWorldInstance,
};
use crate::scene_validation::{HardGateSpecV1, ScorecardSpecV1};
use crate::types::{BuildObject, Commandable, ObjectId, ObjectPrefabId, ObjectTint, Player, SceneLayerOwner};

const PANEL_Z_INDEX: i32 = 940;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneUiTab {
    Pipeline,
    Author,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneUiField {
    None,
    SrcDir,
    GridLayerId,
    GridPrefabId,
    GridOriginX,
    GridOriginY,
    GridOriginZ,
    GridCountX,
    GridCountZ,
    GridStepX,
    GridStepZ,
    PolyLayerId,
    PolyPrefabId,
    PolySpacing,
    PolyStartOffset,
    PolyPoints,
}

#[derive(Clone, Debug)]
struct GridLayerForm {
    layer_id: String,
    prefab_id: String,
    origin_x: String,
    origin_y: String,
    origin_z: String,
    count_x: String,
    count_z: String,
    step_x: String,
    step_z: String,
}

impl Default for GridLayerForm {
    fn default() -> Self {
        Self {
            layer_id: "grid_a".to_string(),
            prefab_id: "".to_string(),
            origin_x: "0".to_string(),
            origin_y: "0".to_string(),
            origin_z: "0".to_string(),
            count_x: "2".to_string(),
            count_z: "3".to_string(),
            step_x: "1".to_string(),
            step_z: "2".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
struct PolylineLayerForm {
    layer_id: String,
    prefab_id: String,
    spacing: String,
    start_offset: String,
    points: String,
}

impl Default for PolylineLayerForm {
    fn default() -> Self {
        Self {
            layer_id: "path_a".to_string(),
            prefab_id: "".to_string(),
            spacing: "1".to_string(),
            start_offset: "0".to_string(),
            points: "0,0,0\n2,0,0\n2,0,2\n".to_string(),
        }
    }
}

#[derive(Resource, Debug)]
pub(crate) struct SceneAuthoringUiState {
    open: bool,
    tab: SceneUiTab,
    focused_field: SceneUiField,
    src_dir: String,
    status: String,
    error: Option<String>,
    layers_dirty: bool,
    prefabs_dirty: bool,
    grid: GridLayerForm,
    poly: PolylineLayerForm,
}

impl Default for SceneAuthoringUiState {
    fn default() -> Self {
        Self {
            open: false,
            tab: SceneUiTab::Pipeline,
            focused_field: SceneUiField::None,
            src_dir: String::new(),
            status: "Ready.".to_string(),
            error: None,
            layers_dirty: true,
            prefabs_dirty: true,
            grid: GridLayerForm::default(),
            poly: PolylineLayerForm::default(),
        }
    }
}

pub(crate) fn scene_ui_closed(state: Res<SceneAuthoringUiState>) -> bool {
    !state.open
}

#[derive(Component)]
pub(crate) struct SceneUiToggleButton;

#[derive(Component)]
pub(crate) struct SceneUiToggleButtonText;

#[derive(Component)]
pub(crate) struct SceneUiPanelRoot;

#[derive(Component)]
pub(crate) struct SceneUiCloseButton;

#[derive(Component)]
pub(crate) struct SceneUiStatusText;

#[derive(Component)]
pub(crate) struct SceneUiErrorText;

#[derive(Component)]
pub(crate) struct SceneUiTabButton {
    tab: SceneUiTab,
}

#[derive(Component)]
pub(crate) struct SceneUiTabRoot {
    tab: SceneUiTab,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneUiAction {
    Import,
    Reload,
    Compile,
    Validate,
    ExportPinned,
    WriteGridLayer,
    WritePolylineLayer,
}

#[derive(Component)]
pub(crate) struct SceneUiLayersList;

#[derive(Component)]
pub(crate) struct SceneUiLayerRow;

#[derive(Component)]
pub(crate) struct SceneUiRegenLayerButton {
    layer_id: String,
}

#[derive(Component)]
pub(crate) struct SceneUiPrefabsList;

#[derive(Component)]
pub(crate) struct SceneUiPrefabRow;

#[derive(Component)]
pub(crate) struct SceneUiUsePrefabButton {
    prefab_id: u128,
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
                SceneUiToggleButtonText,
            ));
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(600.0),
                max_height: Val::Px(660.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
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
                    Text::new("Scene Sources"),
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

            // src_dir input row.
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
                    Text::new("src_dir:"),
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
                    SceneUiTextField {
                        field: SceneUiField::SrcDir,
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
                            field: SceneUiField::SrcDir,
                        },
                    ));
                });
            });

            // Actions row.
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
                spawn_action_button(row, "Import", SceneUiAction::Import);
                spawn_action_button(row, "Reload", SceneUiAction::Reload);
                spawn_action_button(row, "Compile", SceneUiAction::Compile);
                spawn_action_button(row, "Validate", SceneUiAction::Validate);
                spawn_action_button(row, "Export pinned", SceneUiAction::ExportPinned);
            });

            // Status.
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

            // Tabs.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                spawn_tab_button(row, "Pipeline", SceneUiTab::Pipeline);
                spawn_tab_button(row, "Author", SceneUiTab::Author);
            });

            // Tab contents.
            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_grow: 1.0,
                    min_height: Val::Px(0.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|content| {
                // Pipeline tab.
                content
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        SceneUiTabRoot {
                            tab: SceneUiTab::Pipeline,
                        },
                    ))
                    .with_children(|tab| {
                        tab.spawn((
                            Text::new("Layers"),
                            TextFont {
                                font_size: 16.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ));

                        tab.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(6.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            SceneUiLayersList,
                        ));
                    });

                // Author tab.
                content
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(10.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        Visibility::Hidden,
                        SceneUiTabRoot { tab: SceneUiTab::Author },
                    ))
                    .with_children(|tab| {
                        spawn_author_grid_section(tab);
                        spawn_author_polyline_section(tab);
                        tab.spawn((
                            Text::new("Prefabs (click to use)"),
                            TextFont {
                                font_size: 16.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ));
                        tab.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            SceneUiPrefabsList,
                        ));
                    });
            });
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

fn spawn_tab_button(parent: &mut ChildSpawnerCommands, label: &str, tab: SceneUiTab) {
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
            SceneUiTabButton { tab },
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

fn spawn_author_grid_section(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Text::new("New grid_instances layer"),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.92, 0.96)),
    ));

    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
    ))
    .with_children(|form| {
        spawn_labeled_field(form, "layer_id", SceneUiField::GridLayerId);
        spawn_labeled_field(form, "prefab_id (uuid)", SceneUiField::GridPrefabId);

        form.spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("origin"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.90)),
            ));
            spawn_compact_field(row, "x", SceneUiField::GridOriginX);
            spawn_compact_field(row, "y", SceneUiField::GridOriginY);
            spawn_compact_field(row, "z", SceneUiField::GridOriginZ);
        });

        form.spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("count"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.90)),
            ));
            spawn_compact_field(row, "x", SceneUiField::GridCountX);
            spawn_compact_field(row, "z", SceneUiField::GridCountZ);
        });

        form.spawn((
            Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
        ))
        .with_children(|row| {
            row.spawn((
                Text::new("step"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.90)),
            ));
            spawn_compact_field(row, "x", SceneUiField::GridStepX);
            spawn_compact_field(row, "z", SceneUiField::GridStepZ);
        });

        spawn_action_button(form, "Write grid layer", SceneUiAction::WriteGridLayer);
    });
}

fn spawn_author_polyline_section(parent: &mut ChildSpawnerCommands) {
    parent.spawn((
        Text::new("New polyline_instances layer"),
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.92, 0.92, 0.96)),
    ));

    parent.spawn((
        Node {
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            ..default()
        },
        BackgroundColor(Color::NONE),
    ))
    .with_children(|form| {
        spawn_labeled_field(form, "layer_id", SceneUiField::PolyLayerId);
        spawn_labeled_field(form, "prefab_id (uuid)", SceneUiField::PolyPrefabId);
        spawn_labeled_field(form, "spacing", SceneUiField::PolySpacing);
        spawn_labeled_field(form, "start_offset", SceneUiField::PolyStartOffset);

        form.spawn((
            Text::new("points (one per line: x,y,z)"),
            TextFont {
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb(0.85, 0.85, 0.90)),
        ));
        form.spawn((
            Button,
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(90.0),
                padding: UiRect::all(Val::Px(10.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.70)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            SceneUiTextField {
                field: SceneUiField::PolyPoints,
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
                    field: SceneUiField::PolyPoints,
                },
            ));
        });

        spawn_action_button(form, "Write polyline layer", SceneUiAction::WritePolylineLayer);
    });
}

fn spawn_labeled_field(parent: &mut ChildSpawnerCommands, label: &str, field: SceneUiField) {
    parent.spawn((
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
            Text::new(format!("{label}:")),
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
            SceneUiTextField { field },
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                SceneUiTextFieldText { field },
            ));
        });
    });
}

fn spawn_compact_field(parent: &mut ChildSpawnerCommands, label: &str, field: SceneUiField) {
    parent.spawn((
        Text::new(format!("{label}:")),
        TextFont {
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(0.85, 0.85, 0.90)),
    ));
    parent
        .spawn((
            Button,
            Node {
                width: Val::Px(70.0),
                padding: UiRect::axes(Val::Px(8.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.70)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            SceneUiTextField { field },
        ))
        .with_children(|b| {
            b.spawn((
                Text::new(""),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                SceneUiTextFieldText { field },
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
                    state.layers_dirty = true;
                    state.prefabs_dirty = true;
                } else {
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

pub(crate) fn scene_ui_tab_buttons(
    mut state: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &SceneUiTabButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !state.open {
        return;
    }
    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                state.tab = button.tab;
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.90));
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

pub(crate) fn scene_ui_tab_visibility(
    state: Res<SceneAuthoringUiState>,
    mut roots: Query<(&SceneUiTabRoot, &mut Visibility)>,
) {
    if !state.open {
        return;
    }
    for (tab_root, mut vis) in &mut roots {
        *vis = if tab_root.tab == state.tab {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn scene_ui_text_field_focus(
    mut state: ResMut<SceneAuthoringUiState>,
    mut fields: Query<(&Interaction, &SceneUiTextField, &mut BackgroundColor), Changed<Interaction>>,
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
        let allow_newlines = focused_field == SceneUiField::PolyPoints;

        match event.key_code {
            KeyCode::Backspace => {
                field_string_mut(&mut state, focused_field).pop();
            }
            KeyCode::Escape => {
                state.focused_field = SceneUiField::None;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {
                if allow_newlines {
                    field_string_mut(&mut state, focused_field).push('\n');
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
                    }
                    continue;
                }
                if let Some(text) = &event.text {
                    let target = field_string_mut(&mut state, focused_field);
                    push_text(target, text, allow_newlines);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                let target = field_string_mut(&mut state, focused_field);
                push_text(target, text, allow_newlines);
            }
        }
    }
}

fn field_string_mut(state: &mut SceneAuthoringUiState, field: SceneUiField) -> &mut String {
    match field {
        SceneUiField::SrcDir => &mut state.src_dir,
        SceneUiField::GridLayerId => &mut state.grid.layer_id,
        SceneUiField::GridPrefabId => &mut state.grid.prefab_id,
        SceneUiField::GridOriginX => &mut state.grid.origin_x,
        SceneUiField::GridOriginY => &mut state.grid.origin_y,
        SceneUiField::GridOriginZ => &mut state.grid.origin_z,
        SceneUiField::GridCountX => &mut state.grid.count_x,
        SceneUiField::GridCountZ => &mut state.grid.count_z,
        SceneUiField::GridStepX => &mut state.grid.step_x,
        SceneUiField::GridStepZ => &mut state.grid.step_z,
        SceneUiField::PolyLayerId => &mut state.poly.layer_id,
        SceneUiField::PolyPrefabId => &mut state.poly.prefab_id,
        SceneUiField::PolySpacing => &mut state.poly.spacing,
        SceneUiField::PolyStartOffset => &mut state.poly.start_offset,
        SceneUiField::PolyPoints => &mut state.poly.points,
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

pub(crate) fn scene_ui_update_texts(
    state: Res<SceneAuthoringUiState>,
    mut status: Query<
        &mut Text,
        (
            With<SceneUiStatusText>,
            Without<SceneUiErrorText>,
            Without<SceneUiTextFieldText>,
        ),
    >,
    mut errors: Query<
        &mut Text,
        (
            With<SceneUiErrorText>,
            Without<SceneUiStatusText>,
            Without<SceneUiTextFieldText>,
        ),
    >,
    mut fields: Query<
        (&SceneUiTextFieldText, &mut Text),
        (Without<SceneUiStatusText>, Without<SceneUiErrorText>),
    >,
) {
    if !state.open {
        return;
    }

    for mut t in &mut status {
        **t = state.status.clone().into();
    }
    for mut t in &mut errors {
        **t = state.error.clone().unwrap_or_default().into();
    }

    for (field, mut text) in &mut fields {
        let mut value = match field.field {
            SceneUiField::SrcDir => state.src_dir.clone(),
            SceneUiField::GridLayerId => state.grid.layer_id.clone(),
            SceneUiField::GridPrefabId => state.grid.prefab_id.clone(),
            SceneUiField::GridOriginX => state.grid.origin_x.clone(),
            SceneUiField::GridOriginY => state.grid.origin_y.clone(),
            SceneUiField::GridOriginZ => state.grid.origin_z.clone(),
            SceneUiField::GridCountX => state.grid.count_x.clone(),
            SceneUiField::GridCountZ => state.grid.count_z.clone(),
            SceneUiField::GridStepX => state.grid.step_x.clone(),
            SceneUiField::GridStepZ => state.grid.step_z.clone(),
            SceneUiField::PolyLayerId => state.poly.layer_id.clone(),
            SceneUiField::PolyPrefabId => state.poly.prefab_id.clone(),
            SceneUiField::PolySpacing => state.poly.spacing.clone(),
            SceneUiField::PolyStartOffset => state.poly.start_offset.clone(),
            SceneUiField::PolyPoints => state.poly.points.clone(),
            SceneUiField::None => String::new(),
        };

        if state.focused_field == field.field {
            value.push('|');
        }
        if value.is_empty() {
            value = "<click to edit; Ctrl/Cmd+V to paste>".to_string();
        }
        **text = value.into();
    }
}

pub(crate) fn scene_ui_action_buttons(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    library: Res<ObjectLibrary>,
    mut workspace: ResMut<SceneSourcesWorkspace>,
    mut buttons: Query<(&Interaction, &SceneUiActionButton, &mut BackgroundColor), Changed<Interaction>>,
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

                let result = match button.action {
                    SceneUiAction::Import => {
                        let src = state.src_dir.trim();
                        if src.is_empty() {
                            Err("src_dir is empty".to_string())
                        } else {
                            let src_dir = std::path::PathBuf::from(src);
                            let existing_entities = scene_instances.iter().map(|(e, _, _, _, _, _)| e);
                            import_scene_sources_replace_world(
                                &mut commands,
                                &mut workspace,
                                &library,
                                &src_dir,
                                existing_entities,
                            )
                            .map(|report| {
                                state.layers_dirty = true;
                                format!(
                                    "Imported scene sources from {} (pinned instances: {}).",
                                    src_dir.display(),
                                    report.instance_count
                                )
                            })
                        }
                    }
                    SceneUiAction::Reload => reload_scene_sources_in_workspace(&mut workspace)
                        .map(|_| {
                            state.layers_dirty = true;
                            "Reloaded scene sources from disk.".to_string()
                        }),
                    SceneUiAction::Compile => {
                        let existing = scene_instances.iter().map(|(e, t, id, prefab, tint, owner)| {
                            SceneWorldInstance {
                                entity: e,
                                instance_id: *id,
                                prefab_id: *prefab,
                                transform: t.clone(),
                                tint: tint.map(|t| t.0),
                                owner_layer_id: owner.map(|o| o.layer_id.clone()),
                            }
                        });

                        compile_scene_sources_all_layers(&mut commands, &workspace, &library, existing)
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
                                state.layers_dirty = true;
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
                    }
                    SceneUiAction::Validate => {
                        let scorecard = default_scorecard();
                        let existing = scene_instances.iter().map(|(e, t, id, prefab, tint, owner)| {
                            SceneWorldInstance {
                                entity: e,
                                instance_id: *id,
                                prefab_id: *prefab,
                                transform: t.clone(),
                                tint: tint.map(|t| t.0),
                                owner_layer_id: owner.map(|o| o.layer_id.clone()),
                            }
                        });
                        validate_scene_sources(&workspace, &library, existing, &scorecard).map(|report| {
                            let violations = report.violations.len();
                            if report.hard_gates_passed {
                                format!("Validation OK (violations: {violations}).")
                            } else {
                                format!("Validation FAILED (violations: {violations}).")
                            }
                        })
                    }
                    SceneUiAction::ExportPinned => {
                        let Some(out_dir) = workspace.loaded_from_dir.as_deref() else {
                            return;
                        };
                        let objects = scene_instances.iter().filter_map(|(_e, t, id, prefab, tint, owner)| {
                            owner.is_none().then_some((t, id, prefab, tint))
                        });
                        crate::scene_sources_runtime::export_scene_sources_from_world(
                            &workspace,
                            objects,
                            out_dir,
                        )
                        .map(|report| format!("Exported pinned instances to {} (count: {}).", out_dir.display(), report.instance_count))
                    }
                    SceneUiAction::WriteGridLayer => write_grid_layer(&mut state, &workspace),
                    SceneUiAction::WritePolylineLayer => write_polyline_layer(&mut state, &workspace),
                };

                match result {
                    Ok(msg) => state.status = msg,
                    Err(err) => {
                        state.error = Some(err);
                    }
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

fn write_grid_layer(state: &mut SceneAuthoringUiState, workspace: &SceneSourcesWorkspace) -> Result<String, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory is loaded (import first).".to_string());
    };
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources are loaded (import first).".to_string());
    };
    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid index.json: {err}"))?;

    let layer_id = state.grid.layer_id.trim();
    if layer_id.is_empty() {
        return Err("grid.layer_id is empty".to_string());
    }

    let prefab_id = parse_uuid_str("grid.prefab_id", state.grid.prefab_id.as_str())?;
    let origin_x = parse_f32_str("grid.origin_x", state.grid.origin_x.as_str())?;
    let origin_y = parse_f32_str("grid.origin_y", state.grid.origin_y.as_str())?;
    let origin_z = parse_f32_str("grid.origin_z", state.grid.origin_z.as_str())?;
    let count_x = parse_u32_str("grid.count_x", state.grid.count_x.as_str())?;
    let count_z = parse_u32_str("grid.count_z", state.grid.count_z.as_str())?;
    let step_x = parse_f32_str("grid.step_x", state.grid.step_x.as_str())?;
    let step_z = parse_f32_str("grid.step_z", state.grid.step_z.as_str())?;

    let doc = serde_json::json!({
        "count": { "x": count_x, "z": count_z },
        "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
        "kind": "grid_instances",
        "layer_id": layer_id,
        "origin": { "x": origin_x, "y": origin_y, "z": origin_z },
        "prefab_id": prefab_id.to_string(),
        "step": { "x": step_x, "z": step_z },
    });

    let rel_path = index_paths.layers_dir.join(format!("{layer_id}.json"));
    let abs_path = src_dir.join(&rel_path);
    write_json_atomic(&abs_path, &doc)?;

    state.layers_dirty = true;
    Ok(format!("Wrote layer {}", rel_path.display()))
}

fn write_polyline_layer(state: &mut SceneAuthoringUiState, workspace: &SceneSourcesWorkspace) -> Result<String, String> {
    let Some(src_dir) = workspace.loaded_from_dir.as_deref() else {
        return Err("No scene sources directory is loaded (import first).".to_string());
    };
    let Some(sources) = workspace.sources.as_ref() else {
        return Err("No scene sources are loaded (import first).".to_string());
    };
    let index_paths = SceneSourcesIndexPaths::from_index_json_value(&sources.index_json)
        .map_err(|err| format!("Invalid index.json: {err}"))?;

    let layer_id = state.poly.layer_id.trim();
    if layer_id.is_empty() {
        return Err("poly.layer_id is empty".to_string());
    }

    let prefab_id = parse_uuid_str("poly.prefab_id", state.poly.prefab_id.as_str())?;
    let spacing = parse_f32_str("poly.spacing", state.poly.spacing.as_str())?;
    let start_offset = parse_f32_str("poly.start_offset", state.poly.start_offset.as_str())?;

    let mut points = Vec::new();
    for (idx, line) in state.poly.points.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line
            .split(|c| c == ',' || c == ' ' || c == '\t')
            .filter(|p| !p.trim().is_empty())
            .collect();
        if parts.len() != 3 {
            return Err(format!(
                "poly.points line {} must have 3 numbers (x,y,z): {:?}",
                idx + 1,
                line
            ));
        }
        let x = parse_f32_str("poly.points.x", parts[0])?;
        let y = parse_f32_str("poly.points.y", parts[1])?;
        let z = parse_f32_str("poly.points.z", parts[2])?;
        points.push(serde_json::json!({ "x": x, "y": y, "z": z }));
    }
    if points.len() < 2 {
        return Err("poly.points must contain at least 2 points".to_string());
    }

    let doc = serde_json::json!({
        "format_version": crate::scene_sources::SCENE_SOURCES_FORMAT_VERSION,
        "kind": "polyline_instances",
        "layer_id": layer_id,
        "points": points,
        "prefab_id": prefab_id.to_string(),
        "spacing": spacing,
        "start_offset": start_offset,
    });

    let rel_path = index_paths.layers_dir.join(format!("{layer_id}.json"));
    let abs_path = src_dir.join(&rel_path);
    write_json_atomic(&abs_path, &doc)?;

    state.layers_dirty = true;
    Ok(format!("Wrote layer {}", rel_path.display()))
}

fn parse_uuid_str(field: &str, value: &str) -> Result<uuid::Uuid, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{field} is empty"));
    }
    uuid::Uuid::parse_str(v).map_err(|err| format!("{field} invalid UUID: {err}"))
}

fn parse_f32_str(field: &str, value: &str) -> Result<f32, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{field} is empty"));
    }
    let num: f32 = v
        .parse()
        .map_err(|_| format!("{field} must be a number, got {value:?}"))?;
    if !num.is_finite() {
        return Err(format!("{field} must be finite"));
    }
    Ok(num)
}

fn parse_u32_str(field: &str, value: &str) -> Result<u32, String> {
    let v = value.trim();
    if v.is_empty() {
        return Err(format!("{field} is empty"));
    }
    v.parse()
        .map_err(|_| format!("{field} must be an integer, got {value:?}"))
}

fn write_json_atomic(path: &std::path::Path, value: &serde_json::Value) -> Result<(), String> {
    let Some(parent) = path.parent() else {
        return Err(format!("Invalid path (no parent): {}", path.display()));
    };
    std::fs::create_dir_all(parent).map_err(|err| format!("create_dir_all failed: {err}"))?;
    let text = serde_json::to_string_pretty(value).map_err(|err| format!("json serialize failed: {err}"))?;
    let bytes = format!("{text}\n").into_bytes();
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, bytes).map_err(|err| format!("write tmp failed: {err}"))?;
    std::fs::rename(&tmp, path).map_err(|err| format!("rename failed: {err}"))?;
    Ok(())
}

pub(crate) fn scene_ui_rebuild_layers_list(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    workspace: Res<SceneSourcesWorkspace>,
    lists: Query<Entity, With<SceneUiLayersList>>,
    existing_rows: Query<Entity, With<SceneUiLayerRow>>,
) {
    if !state.open || state.tab != SceneUiTab::Pipeline {
        return;
    }
    if !state.layers_dirty {
        return;
    }
    state.layers_dirty = false;

    let Some(sources) = workspace.sources.as_ref() else {
        // Clear rows if any.
        for row in &existing_rows {
            commands.entity(row).try_despawn();
        }
        return;
    };

    let index_paths = match SceneSourcesIndexPaths::from_index_json_value(&sources.index_json) {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut layers = Vec::new();
    for (rel_path, doc) in &sources.extra_json_files {
        if !rel_path.starts_with(&index_paths.layers_dir) {
            continue;
        }
        let layer_id = doc
            .get("layer_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        let kind = doc
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .to_string();
        if layer_id.is_empty() {
            continue;
        }
        layers.push((layer_id, kind));
    }
    layers.sort_by(|a, b| a.0.cmp(&b.0));

    for row in &existing_rows {
        commands.entity(row).try_despawn();
    }

    let Ok(list_root) = lists.single() else {
        return;
    };

    commands.entity(list_root).with_children(|list| {
        if layers.is_empty() {
            list.spawn((
                Text::new("(no layers found)"),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                SceneUiLayerRow,
            ));
            return;
        }

        for (layer_id, kind) in layers {
            list.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::SpaceBetween,
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(Color::NONE),
                SceneUiLayerRow,
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("{layer_id}  ({kind})")),
                    TextFont {
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
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
                    SceneUiRegenLayerButton { layer_id: layer_id.clone() },
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Regen"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    ));
                });
            });
        }
    });
}

pub(crate) fn scene_ui_regen_layer_buttons(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    library: Res<ObjectLibrary>,
    workspace: Res<SceneSourcesWorkspace>,
    mut buttons: Query<
        (&Interaction, &SceneUiRegenLayerButton, &mut BackgroundColor),
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

                let existing = scene_instances.iter().map(|(e, t, id, prefab, tint, owner)| {
                    SceneWorldInstance {
                        entity: e,
                        instance_id: *id,
                        prefab_id: *prefab,
                        transform: t.clone(),
                        tint: tint.map(|t| t.0),
                        owner_layer_id: owner.map(|o| o.layer_id.clone()),
                    }
                });
                match regenerate_scene_layer(
                    &mut commands,
                    &workspace,
                    &library,
                    existing,
                    &button.layer_id,
                ) {
                    Ok(report) => {
                        state.status = format!(
                            "Regenerated {} (spawned={} updated={} despawned={}).",
                            button.layer_id, report.spawned, report.updated, report.despawned
                        );
                    }
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

pub(crate) fn scene_ui_rebuild_prefabs_list(
    mut commands: Commands,
    mut state: ResMut<SceneAuthoringUiState>,
    library: Res<ObjectLibrary>,
    lists: Query<Entity, With<SceneUiPrefabsList>>,
    existing_rows: Query<Entity, With<SceneUiPrefabRow>>,
) {
    if !state.open || state.tab != SceneUiTab::Author {
        return;
    }
    if !state.prefabs_dirty {
        return;
    }
    state.prefabs_dirty = false;

    for row in &existing_rows {
        commands.entity(row).try_despawn();
    }

    let Ok(list_root) = lists.single() else {
        return;
    };

    let mut items: Vec<(String, u128)> = library
        .iter()
        .map(|(id, def)| (def.label.to_string(), *id))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));

    commands.entity(list_root).with_children(|list| {
        for (label, id) in items {
            let uuid = uuid::Uuid::from_u128(id);
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.65)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.55)),
                SceneUiUsePrefabButton { prefab_id: id },
                SceneUiPrefabRow,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new(format!("{label} — {uuid}")),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                ));
            });
        }
    });
}

pub(crate) fn scene_ui_use_prefab_buttons(
    mut state: ResMut<SceneAuthoringUiState>,
    mut buttons: Query<
        (&Interaction, &SceneUiUsePrefabButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !state.open {
        return;
    }
    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                let uuid = uuid::Uuid::from_u128(button.prefab_id).to_string();
                state.grid.prefab_id = uuid.clone();
                state.poly.prefab_id = uuid;
                state.status = "Selected prefab id for authoring forms.".to_string();
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.85));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.75));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.65));
            }
        }
    }
}
