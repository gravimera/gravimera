use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::window::{Ime, PrimaryWindow};

use crate::assets::SceneAssets;
use crate::rich_text::set_rich_text_line;
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts};

use super::ai::Gen3dAiJob;
use super::preview;
use super::state::*;

pub(crate) fn spawn_gen3d_preview_panel<F>(
    parent: &mut ChildSpawnerCommands,
    node: Node,
    target: Handle<Image>,
    extra_children: F,
) -> Entity
where
    F: FnOnce(&mut ChildSpawnerCommands),
{
    parent
        .spawn((
            Button,
            node,
            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            Gen3dPreviewPanel,
        ))
        .with_children(|preview| {
            preview.spawn((
                ImageNode::new(target),
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
            ));
            extra_children(preview);
        })
        .id()
}

pub(crate) fn handle_gen3d_toggle_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dToggleButton>),
    >,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
) {
    if !matches!(mode.get(), GameMode::Build) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.60));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.08, 0.08, 0.10, 0.75));
            }
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.85));
                match build_scene.get() {
                    BuildScene::Preview => next_build_scene.set(BuildScene::Realm),
                    BuildScene::Realm => next_build_scene.set(BuildScene::Preview),
                }
            }
        }
    }
}

pub(crate) fn update_gen3d_toggle_button_label(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
    mut buttons: Query<&mut Visibility, With<Gen3dToggleButton>>,
    mut texts: Query<&mut Text, With<Gen3dToggleButtonText>>,
) {
    let visible = matches!(mode.get(), GameMode::Build);
    for mut visibility in &mut buttons {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    let label = match (mode.get(), build_scene.get()) {
        (GameMode::Build, BuildScene::Preview) => "Realm",
        (GameMode::Build, BuildScene::Realm) => "Preview",
        _ => "Preview",
    };
    for mut text in &mut texts {
        **text = label.into();
    }
}

pub(crate) fn enter_gen3d_mode(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    assets: Res<SceneAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    job: Res<Gen3dAiJob>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut meta_state: ResMut<crate::motion_ui::MotionAlgorithmUiState>,
    mut meta_roots: Query<&mut Visibility, With<crate::motion_ui::MotionAlgorithmUiRoot>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !job.is_running() {
        workshop.status = format!(
            "Drop 0–{} images (optional) and/or type a prompt, then click Build.",
            super::GEN3D_MAX_IMAGES
        );
        workshop.speed_mode = Gen3dSpeedMode::Level3;
    }
    workshop.error = None;
    workshop.prompt_focused = true;
    workshop.image_viewer = None;
    workshop.prompt_scrollbar_drag = None;
    workshop.side_tab = Gen3dSideTab::Status;
    workshop.side_panel_open = false;
    workshop.tool_feedback_unread = false;
    preview_state.animation_channel = "idle".to_string();
    preview_state.animation_channels.clear();
    preview_state.animation_dropdown_open = false;
    if meta_state.open {
        meta_state.close();
    }
    if let Ok(mut visibility) = meta_roots.single_mut() {
        *visibility = Visibility::Hidden;
    }
    if let Ok(mut window) = windows.single_mut() {
        window.ime_enabled = true;
    }

    let needs_setup = preview_state.target.is_none()
        || preview_state.root.is_none()
        || preview_state.camera.is_none();
    let target = if needs_setup {
        preview::setup_preview_scene(
            &mut commands,
            &mut images,
            &assets,
            &mut materials,
            &mut preview_state,
        )
    } else {
        preview_state.target.clone().unwrap_or_else(|| {
            preview::setup_preview_scene(
                &mut commands,
                &mut images,
                &assets,
                &mut materials,
                &mut preview_state,
            )
        })
    };

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(0.01, 0.01, 0.015, 0.94)),
            ZIndex(900),
            Gen3dWorkshopRoot,
        ))
        .with_children(|root| {
            root.spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(12.0),
                    left: Val::Px(12.0),
                    width: Val::Px(92.0),
                    height: Val::Px(34.0),
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
                ZIndex(910),
                Gen3dToggleButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Realm"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    Gen3dToggleButtonText,
                ));
            });

            root.spawn((
                Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    min_height: Val::Px(0.0),
                    ..default()
                },
                BackgroundColor(Color::NONE),
            ))
            .with_children(|row| {
                // Center: preview.
                row.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        padding: UiRect::all(Val::Px(10.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        min_height: Val::Px(0.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Preview"),
                        TextFont {
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.85, 0.25)),
                    ));

                    spawn_gen3d_preview_panel(
                        panel,
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            min_height: Val::Px(0.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(1.0)),
                            ..default()
                        },
                        target.clone(),
                        |preview| {
                            preview
                                .spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: Val::Px(8.0),
                                        top: Val::Px(8.0),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(6.0),
                                        align_items: AlignItems::FlexStart,
                                        padding: UiRect::all(Val::Px(6.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                ))
                                .with_children(|stats| {
                                    stats.spawn((
                                        Text::new(""),
                                        TextFont {
                                            font_size: 13.0,
                                            ..default()
                                        },
                                        TextColor(Color::srgb(0.82, 0.90, 1.0)),
                                        Gen3dPreviewStatsText,
                                    ));

                                    stats
                                        .spawn((
                                            Node {
                                                flex_direction: FlexDirection::Row,
                                                column_gap: Val::Px(6.0),
                                                align_items: AlignItems::FlexStart,
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                        ))
                                        .with_children(|row| {
                                            row.spawn((
                                                Text::new("Anim:"),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                            ));

                                            row.spawn((
                                                Node {
                                                    width: Val::Px(140.0),
                                                    flex_direction: FlexDirection::Column,
                                                    row_gap: Val::Px(2.0),
                                                    align_items: AlignItems::Stretch,
                                                    ..default()
                                                },
                                                BackgroundColor(Color::NONE),
                                            ))
                                            .with_children(|dropdown| {
                                                dropdown
                                                    .spawn((
                                                        Button,
                                                        Node {
                                                            width: Val::Percent(100.0),
                                                            height: Val::Px(22.0),
                                                            justify_content: JustifyContent::Center,
                                                            align_items: AlignItems::Center,
                                                            border: UiRect::all(Val::Px(1.0)),
                                                            ..default()
                                                        },
                                                        BackgroundColor(Color::srgba(
                                                            0.02, 0.02, 0.03, 0.70,
                                                        )),
                                                        BorderColor::all(Color::srgba(
                                                            0.25, 0.25, 0.30, 0.65,
                                                        )),
                                                        Gen3dPreviewAnimationDropdownButton,
                                                    ))
                                                    .with_children(|button| {
                                                        button.spawn((
                                                            Text::new("Idle ▾"),
                                                            TextFont {
                                                                font_size: 13.0,
                                                                ..default()
                                                            },
                                                            TextColor(Color::srgb(
                                                                0.92, 0.92, 0.96,
                                                            )),
                                                            Gen3dPreviewAnimationDropdownButtonText,
                                                        ));
                                                    });

                                                dropdown
                                                    .spawn((
                                                        Node {
                                                            width: Val::Percent(100.0),
                                                            max_height: Val::Px(240.0),
                                                            min_height: Val::Px(0.0),
                                                            flex_direction: FlexDirection::Column,
                                                            row_gap: Val::Px(2.0),
                                                            padding: UiRect::all(Val::Px(4.0)),
                                                            border: UiRect::all(Val::Px(1.0)),
                                                            display: Display::None,
                                                            overflow: Overflow::scroll_y(),
                                                            ..default()
                                                        },
                                                        BackgroundColor(Color::srgba(
                                                            0.02, 0.02, 0.03, 0.92,
                                                        )),
                                                        BorderColor::all(Color::srgba(
                                                            0.25, 0.25, 0.30, 0.65,
                                                        )),
                                                        Visibility::Hidden,
                                                        ScrollPosition::default(),
                                                        Gen3dPreviewAnimationDropdownList,
                                                    ))
                                                    .with_children(|_list| {});
                                            });
                                        });
                                });

                            // Collapsible side panel toggle.
                            preview
                                .spawn((
                                    Button,
                                    Node {
                                        position_type: PositionType::Absolute,
                                        right: Val::Px(8.0),
                                        top: Val::Px(8.0),
                                        width: Val::Px(28.0),
                                        height: Val::Px(28.0),
                                        justify_content: JustifyContent::Center,
                                        align_items: AlignItems::Center,
                                        border: UiRect::all(Val::Px(1.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.80)),
                                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.70)),
                                    ZIndex(2150),
                                    Gen3dSidePanelToggleButton,
                                ))
                                .with_children(|button| {
                                    button.spawn((
                                        Text::new("≡"),
                                        TextFont {
                                            font_size: 16.0,
                                            ..default()
                                        },
                                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                        Gen3dSidePanelToggleButtonText,
                                    ));
                                });

                            // Collapsible Status overlay (hidden by default).
                            preview
                                .spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        right: Val::Px(8.0),
                                        top: Val::Px(44.0),
                                        bottom: Val::Px(8.0),
                                        width: Val::Px(360.0),
                                        flex_direction: FlexDirection::Column,
                                        row_gap: Val::Px(6.0),
                                        padding: UiRect::all(Val::Px(8.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        min_height: Val::Px(0.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.92)),
                                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                    ZIndex(2140),
                                    Visibility::Hidden,
                                    Gen3dSidePanelRoot,
                                ))
                                .with_children(|panel| {
                                    // Side tab bar.
                                    panel
                                        .spawn((
                                            Node {
                                                width: Val::Percent(100.0),
                                                flex_direction: FlexDirection::Row,
                                                column_gap: Val::Px(6.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                            Visibility::Inherited,
                                        ))
                                        .with_children(|tabs| {
                                            tabs.spawn((
                                                Button,
                                                Node {
                                                    flex_grow: 1.0,
                                                    height: Val::Px(30.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70)),
                                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.70)),
                                                Visibility::Inherited,
                                                Gen3dSideTabButton::new(Gen3dSideTab::Status),
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Status"),
                                                    TextFont {
                                                        font_size: 14.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    Visibility::Inherited,
                                                    Gen3dSideTabButtonText::new(Gen3dSideTab::Status),
                                                ));
                                            });
                                        });

                                    // Status tab content.
                                    panel
                                        .spawn((
                                            Node {
                                                width: Val::Percent(100.0),
                                                flex_grow: 1.0,
                                                flex_basis: Val::Px(0.0),
                                                min_height: Val::Px(0.0),
                                                flex_direction: FlexDirection::Row,
                                                column_gap: Val::Px(6.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                            Visibility::Inherited,
                                            Gen3dStatusPanelRoot,
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
                                                Visibility::Inherited,
                                                ScrollPosition::default(),
                                                Gen3dStatusScrollPanel,
                                            ))
                                            .with_children(|scroll| {
                                                scroll.spawn((
                                                    Text::new(""),
                                                    TextFont {
                                                        font_size: 14.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                                    Visibility::Inherited,
                                                    Gen3dStatusText,
                                                ));
                                            });

                                            row.spawn((
                                                Node {
                                                    width: Val::Px(8.0),
                                                    height: Val::Percent(100.0),
                                                    position_type: PositionType::Relative,
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.45)),
                                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                                Visibility::Hidden,
                                                Gen3dStatusScrollbarTrack,
                                            ))
                                            .with_children(|track| {
                                                track.spawn((
                                                    Button,
                                                    Node {
                                                        position_type: PositionType::Absolute,
                                                        left: Val::Px(1.0),
                                                        right: Val::Px(1.0),
                                                        top: Val::Px(0.0),
                                                        height: Val::Px(18.0),
                                                        ..default()
                                                    },
                                                    BackgroundColor(Color::srgba(0.85, 0.88, 0.95, 0.85)),
                                                    Visibility::Inherited,
                                                    Gen3dStatusScrollbarThumb,
                                                ));
                                            });
                                        });

                                });
                        });

                    panel
                        .spawn((
                            Node {
                                width: Val::Percent(100.0),
                                flex_direction: FlexDirection::Row,
                                justify_content: JustifyContent::SpaceBetween,
                                align_items: AlignItems::Center,
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            Visibility::Inherited,
                        ))
                        .with_children(|row| {
                            row.spawn((
                                Button,
                                Node {
                                    width: Val::Px(130.0),
                                    height: Val::Px(28.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                Visibility::Inherited,
                                Gen3dCollisionToggleButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Collision: Off"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                    Visibility::Inherited,
                                    Gen3dCollisionToggleText,
                                ));
                            });
                        });
                });
            });

            // Bottom: prompt + generate + status.
            root.spawn((
                Node {
                    height: Val::Px(super::GEN3D_PROMPT_BAR_HEIGHT_PX),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    padding: UiRect::all(Val::Px(12.0)),
                    border: UiRect::top(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.01, 0.01, 0.015, 0.96)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
            ))
            .with_children(|bar| {
                bar.spawn((
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(12.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                ))
                .with_children(|row| {
                    row.spawn((
                        Button,
                        Node {
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            height: Val::Percent(100.0),
                            border: UiRect::all(Val::Px(1.0)),
                            flex_direction: FlexDirection::Row,
                            min_height: Val::Px(0.0),
                            overflow: Overflow::clip(),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                        BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                        Gen3dPromptBox,
                    ))
                    .with_children(|prompt| {
                        prompt
                            .spawn((
                                Node {
                                    flex_grow: 1.0,
                                    flex_basis: Val::Px(0.0),
                                    height: Val::Percent(100.0),
                                    flex_direction: FlexDirection::Row,
                                    column_gap: Val::Px(10.0),
                                    min_height: Val::Px(0.0),
                                    ..default()
                                },
                                BackgroundColor(Color::NONE),
                            ))
                            .with_children(|content| {
                                content
                                    .spawn((
                                        Node {
                                            flex_grow: 1.0,
                                            flex_basis: Val::Px(0.0),
                                            height: Val::Percent(100.0),
                                            flex_direction: FlexDirection::Row,
                                            min_height: Val::Px(0.0),
                                            ..default()
                                        },
                                        BackgroundColor(Color::NONE),
                                    ))
                                    .with_children(|prompt_row| {
                                        prompt_row
                                            .spawn((
                                                Node {
                                                    flex_grow: 1.0,
                                                    flex_basis: Val::Px(0.0),
                                                    min_height: Val::Px(0.0),
                                                    padding: UiRect::all(Val::Px(10.0)),
                                                    overflow: Overflow::scroll_y(),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::NONE),
                                                ScrollPosition::default(),
                                                Gen3dPromptScrollPanel,
                                            ))
                                            .with_children(|scroll| {
                                                scroll
                                                    .spawn((
                                                        Node {
                                                            width: Val::Percent(100.0),
                                                            flex_direction: FlexDirection::Row,
                                                            align_items: AlignItems::FlexStart,
                                                            column_gap: Val::Px(10.0),
                                                            ..default()
                                                        },
                                                        BackgroundColor(Color::NONE),
                                                    ))
                                                    .with_children(|content_row| {
                                                        content_row
                                                            .spawn((
                                                                Node {
                                                                    flex_grow: 1.0,
                                                                    flex_basis: Val::Px(0.0),
                                                                    min_width: Val::Px(0.0),
                                                                    flex_direction: FlexDirection::Column,
                                                                    ..default()
                                                                },
                                                                BackgroundColor(Color::NONE),
                                                            ))
                                                            .with_children(|text_column| {
                                                                text_column
                                                                    .spawn((
                                                                        Node {
                                                                            width: Val::Percent(100.0),
                                                                            flex_wrap: FlexWrap::Wrap,
                                                                            justify_content: JustifyContent::FlexStart,
                                                                            align_items: AlignItems::FlexStart,
                                                                            column_gap: Val::Px(1.0),
                                                                            row_gap: Val::Px(2.0),
                                                                            ..default()
                                                                        },
                                                                        Gen3dPromptRichText,
                                                                    ))
                                                                    .with_children(|_| {});
                                                            });

                                                        content_row
                                                            .spawn((
                                                                Node {
                                                                    width: Val::Px(240.0),
                                                                    flex_shrink: 0.0,
                                                                    flex_direction: FlexDirection::Column,
                                                                    row_gap: Val::Px(6.0),
                                                                    min_height: Val::Px(0.0),
                                                                    ..default()
                                                                },
                                                                BackgroundColor(Color::NONE),
                                                                Gen3dImagesInlinePanel,
                                                            ))
                                                            .with_children(|panel| {
                                                                panel
                                                                    .spawn((
                                                                        Node {
                                                                            width: Val::Percent(100.0),
                                                                            flex_direction: FlexDirection::Row,
                                                                            justify_content: JustifyContent::FlexEnd,
                                                                            align_items: AlignItems::Center,
                                                                            ..default()
                                                                        },
                                                                        BackgroundColor(Color::NONE),
                                                                    ))
                                                                    .with_children(|header| {
                                                                        header
                                                                            .spawn((
                                                                                Button,
                                                                                Node {
                                                                                    width: Val::Px(64.0),
                                                                                    height: Val::Px(24.0),
                                                                                    justify_content: JustifyContent::Center,
                                                                                    align_items: AlignItems::Center,
                                                                                    border: UiRect::all(Val::Px(1.0)),
                                                                                    ..default()
                                                                                },
                                                                                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                                                                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                                                                Gen3dClearImagesButton,
                                                                            ))
                                                                            .with_children(|button| {
                                                                                button.spawn((
                                                                                    Text::new("Clear"),
                                                                                    TextFont {
                                                                                        font_size: 12.0,
                                                                                        ..default()
                                                                                    },
                                                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                                                    Gen3dClearImagesButtonText,
                                                                                ));
                                                                            });
                                                                    });

                                                                panel.spawn((
                                                                    Node {
                                                                        width: Val::Percent(100.0),
                                                                        flex_direction: FlexDirection::Row,
                                                                        flex_wrap: FlexWrap::Wrap,
                                                                        justify_content: JustifyContent::FlexStart,
                                                                        align_content: AlignContent::FlexStart,
                                                                        align_items: AlignItems::Stretch,
                                                                        column_gap: Val::Px(0.0),
                                                                        row_gap: Val::Px(0.0),
                                                                        ..default()
                                                                    },
                                                                    BackgroundColor(Color::NONE),
                                                                    Gen3dImagesList,
                                                                ));
                                                            });
                                                    });
                                            });

                                        prompt_row
                                            .spawn((
                                                Button,
                                                Node {
                                                    width: Val::Px(8.0),
                                                    height: Val::Percent(100.0),
                                                    position_type: PositionType::Relative,
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.45)),
                                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                                Visibility::Hidden,
                                                Gen3dPromptScrollbarTrack,
                                            ))
                                            .with_children(|track| {
                                                track.spawn((
                                                    Button,
                                                    Node {
                                                        position_type: PositionType::Absolute,
                                                        left: Val::Px(1.0),
                                                        right: Val::Px(1.0),
                                                        top: Val::Px(0.0),
                                                        height: Val::Px(18.0),
                                                        ..default()
                                                    },
                                                    BackgroundColor(Color::srgba(0.85, 0.88, 0.95, 0.85)),
                                                    Gen3dPromptScrollbarThumb,
                                                ));
                                            });
                                    });

                            });
                    });
                });

                bar.spawn((
                    Node {
                        width: Val::Px(240.0),
                        height: Val::Percent(100.0),
                        flex_direction: FlexDirection::Column,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                ))
                .with_children(|column| {
                    column
                        .spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(42.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            Gen3dClearPromptButton,
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Clear Prompt"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                Gen3dClearPromptButtonText,
                            ));
                        });

                    column
                        .spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(52.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85)),
                            BorderColor::all(Color::srgb(0.25, 0.80, 0.45)),
                            Outline {
                                width: Val::Px(1.0),
                                color: Color::srgb(0.25, 0.80, 0.45),
                                offset: Val::Px(0.0),
                            },
                            Gen3dGenerateButton,
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Build"),
                                TextFont {
                                    font_size: 18.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.70, 1.0, 0.82)),
                                Gen3dGenerateButtonText,
                            ));
                        });

                    column
                        .spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(42.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.11, 0.08, 0.80)),
                            BorderColor::all(Color::srgb(0.20, 0.65, 0.35)),
                            Gen3dContinueButton,
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Continue"),
                                TextFont {
                                    font_size: 16.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.70, 1.0, 0.82)),
                                Gen3dContinueButtonText,
                            ));
                        });

                    column
                        .spawn((
                            Button,
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(42.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.80)),
                            BorderColor::all(Color::srgb(0.30, 0.55, 0.95)),
                            Gen3dSaveButton,
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Save"),
                                TextFont {
                                    font_size: 16.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.82, 0.90, 1.0)),
                                Gen3dSaveButtonText,
                            ));
                        });
                });
            });

            // Hover tooltip for thumbnails (shown near cursor).
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(0.0),
                    max_width: Val::Px(320.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.95)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.85)),
                ZIndex(2200),
                Visibility::Hidden,
                Gen3dThumbnailTooltipRoot,
            ))
            .with_children(|tip| {
                tip.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    Gen3dThumbnailTooltipText,
                ));
            });
        });
}

pub(crate) fn exit_gen3d_mode(
    mut commands: Commands,
    roots: Query<Entity, With<Gen3dWorkshopRoot>>,
    preview_cameras: Query<Entity, With<Gen3dPreviewCamera>>,
    review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>,
    preview_roots: Query<Entity, With<Gen3dPreviewSceneRoot>>,
    preview_lights: Query<Entity, With<Gen3dPreviewLight>>,
    viewer_roots: Query<Entity, With<Gen3dImageViewerRoot>>,
    job: Res<Gen3dAiJob>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut workshop: ResMut<Gen3dWorkshop>,
) {
    for entity in &roots {
        commands.entity(entity).try_despawn();
    }
    for entity in &viewer_roots {
        commands.entity(entity).try_despawn();
    }

    if !job.is_running() {
        for entity in &preview_cameras {
            commands.entity(entity).try_despawn();
        }
        for entity in &review_cameras {
            commands.entity(entity).try_despawn();
        }
        for entity in &preview_roots {
            commands.entity(entity).try_despawn();
        }
        for entity in &preview_lights {
            commands.entity(entity).try_despawn();
        }

        preview_state.target = None;
        preview_state.camera = None;
        preview_state.root = None;
        preview_state.last_cursor = None;
        preview_state.collision_dirty = false;
        preview_state.animation_channel = "idle".to_string();
        preview_state.animation_channels.clear();
        preview_state.animation_dropdown_open = false;
    } else {
        // Keep the preview scene alive so Gen3D can keep rendering/reviewing in the background.
        preview_state.last_cursor = None;
        preview_state.animation_dropdown_open = false;
    }
    workshop.image_viewer = None;
    workshop.prompt_scrollbar_drag = None;
}

pub(crate) fn gen3d_cleanup_preview_scene_when_idle(
    mut commands: Commands,
    job: Res<Gen3dAiJob>,
    preview_cameras: Query<Entity, With<Gen3dPreviewCamera>>,
    review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>,
    preview_roots: Query<Entity, With<Gen3dPreviewSceneRoot>>,
    preview_lights: Query<Entity, With<Gen3dPreviewLight>>,
    mut preview_state: ResMut<Gen3dPreview>,
) {
    if job.is_running() {
        return;
    }

    let should_cleanup = preview_state.root.is_some()
        || preview_state.camera.is_some()
        || preview_state.target.is_some()
        || !preview_roots.is_empty()
        || !preview_lights.is_empty()
        || !preview_cameras.is_empty()
        || !review_cameras.is_empty();
    if !should_cleanup {
        return;
    }

    for entity in &preview_cameras {
        commands.entity(entity).try_despawn();
    }
    for entity in &review_cameras {
        commands.entity(entity).try_despawn();
    }
    for entity in &preview_roots {
        commands.entity(entity).try_despawn();
    }
    for entity in &preview_lights {
        commands.entity(entity).try_despawn();
    }

    preview_state.target = None;
    preview_state.camera = None;
    preview_state.root = None;
    preview_state.last_cursor = None;
    preview_state.collision_dirty = false;
    preview_state.animation_channel = "idle".to_string();
    preview_state.animation_channels.clear();
    preview_state.animation_dropdown_open = false;
}

pub(crate) fn gen3d_prompt_box_focus(
    mut workshop: ResMut<Gen3dWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut prompt_boxes: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dPromptBox>),
    >,
) {
    for (interaction, mut bg) in &mut prompt_boxes {
        match *interaction {
            Interaction::Pressed => {
                workshop.prompt_focused = true;
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = true;
                }
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                let alpha = if workshop.prompt_focused { 0.70 } else { 0.65 };
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, alpha));
                if !workshop.prompt_focused {
                    if let Ok(mut window) = windows.single_mut() {
                        window.ime_enabled = false;
                    }
                }
            }
        }
    }
}

pub(crate) fn gen3d_side_panel_toggle_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dSidePanelToggleButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                workshop.side_panel_open = !workshop.side_panel_open;
                *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.12, 0.90));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.06, 0.06, 0.08, 0.86));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.80));
            }
        }
    }
}

pub(crate) fn gen3d_update_side_panel_ui(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut panels: Query<(&mut Node, &mut Visibility), With<Gen3dSidePanelRoot>>,
    mut texts: Query<&mut Text, With<Gen3dSidePanelToggleButtonText>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    for (mut node, mut vis) in &mut panels {
        let open = workshop.side_panel_open;
        node.display = if open { Display::Flex } else { Display::None };
        *vis = if open {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }

    let label = if workshop.side_panel_open {
        "×".to_string()
    } else if workshop.tool_feedback_unread {
        "≡*".to_string()
    } else {
        "≡".to_string()
    };

    for mut text in &mut texts {
        **text = label.clone().into();
    }
}

pub(crate) fn gen3d_prompt_scroll_wheel(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<Gen3dPromptScrollPanel>,
    >,
    workshop: Res<Gen3dWorkshop>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if workshop.prompt_scrollbar_drag.is_some() {
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
    let Ok((node, transform, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };

    if !node.contains_point(*transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

    let mut delta_lines = 0.0f32;
    for ev in mouse_wheel.read() {
        let lines = match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() < 1e-4 {
        return;
    }

    let delta_px = delta_lines * 24.0;
    let panel_scale = node.inverse_scale_factor();
    let viewport_h = node.size.y.max(0.0) * panel_scale;
    let content_h = node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);
    scroll.y = (scroll.y - delta_px).clamp(0.0, max_scroll);
}

pub(crate) fn gen3d_update_prompt_scrollbar_ui(
    build_scene: Res<State<BuildScene>>,
    panels: Query<&ComputedNode, With<Gen3dPromptScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<Gen3dPromptScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<Gen3dPromptScrollbarThumb>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    let Ok(panel) = panels.single() else {
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
    let scroll_y = panel.scroll_position.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (viewport_h * viewport_h / content_h).clamp(min_thumb_h, track_h);
    let max_thumb_top = (track_h - thumb_h).max(0.0);
    let thumb_top = (max_thumb_top * (scroll_y / max_scroll)).clamp(0.0, max_thumb_top);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

pub(crate) fn gen3d_prompt_scrollbar_drag(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<Gen3dPromptScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<Gen3dPromptScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<Gen3dPromptScrollbarThumb>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        workshop.prompt_scrollbar_drag = None;
        return;
    }

    if !mouse_buttons.pressed(MouseButton::Left) {
        workshop.prompt_scrollbar_drag = None;
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
        workshop.prompt_scrollbar_drag = None;
        return;
    }
    let Ok((interaction, thumb_node, thumb_layout)) = thumbs.single() else {
        return;
    };

    let mouse_just_pressed = mouse_buttons.just_pressed(MouseButton::Left);
    let track_clicked = matches!(track_vis, Visibility::Visible | Visibility::Inherited)
        && track_node.contains_point(*track_transform, cursor)
        && mouse_just_pressed;
    if workshop.prompt_scrollbar_drag.is_none() && (mouse_just_pressed || *interaction == Interaction::Pressed) {
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
            let thumb_h = thumb_node.size.y.max(1.0) * thumb_scale;
            let over_thumb = cursor_in_track >= thumb_top && cursor_in_track <= thumb_top + thumb_h;
            if *interaction == Interaction::Pressed || (mouse_just_pressed && over_thumb) {
                let grab_offset = (cursor_in_track - thumb_top).clamp(0.0, thumb_h);
                workshop.prompt_scrollbar_drag = Some(Gen3dPromptScrollbarDrag { grab_offset });
            } else if track_clicked {
                let grab_offset = (cursor_in_track - thumb_top).clamp(0.0, thumb_h);
                workshop.prompt_scrollbar_drag = Some(Gen3dPromptScrollbarDrag { grab_offset });
            }
        }
    }

    let Some(drag) = workshop.prompt_scrollbar_drag else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor();
    let viewport_h = panel_node.size.y.max(0.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
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

pub(crate) fn gen3d_prompt_text_input(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut ime_events: bevy::ecs::message::MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    let mut accept_input = workshop.prompt_focused;
    if accept_input {
        if let Ok(mut window) = windows.single_mut() {
            window.ime_enabled = true;
        }
    }

    for event in ime_events.read() {
        if let Ime::Commit { value, .. } = event {
            if !value.is_empty() {
                if !accept_input {
                    accept_input = true;
                    workshop.prompt_focused = true;
                    if let Ok(mut window) = windows.single_mut() {
                        window.ime_enabled = true;
                    }
                }
                if accept_input {
                    push_prompt_text(&mut workshop, value);
                }
            }
        }
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        let mut handled = false;
        if !accept_input {
            if let Some(text) = &event.text {
                if !text.is_empty() {
                    accept_input = true;
                    workshop.prompt_focused = true;
                    if let Ok(mut window) = windows.single_mut() {
                        window.ime_enabled = true;
                    }
                }
            } else if matches!(event.key_code, KeyCode::KeyV) {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if modifier {
                    if let Some(text) = crate::clipboard::read_text() {
                        accept_input = true;
                        workshop.prompt_focused = true;
                        if let Ok(mut window) = windows.single_mut() {
                            window.ime_enabled = true;
                        }
                        if accept_input {
                            push_prompt_text(&mut workshop, &text);
                            handled = true;
                        }
                    }
                }
                if !accept_input {
                    continue;
                }
            } else {
                continue;
            }
            if !accept_input {
                continue;
            }
        }
        if handled {
            continue;
        }
        match event.key_code {
            KeyCode::Backspace => {
                workshop.prompt.pop();
                clear_prompt_limit_error(&mut workshop);
            }
            KeyCode::Escape => {
                workshop.prompt_focused = false;
                if let Ok(mut window) = windows.single_mut() {
                    window.ime_enabled = false;
                }
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {}
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if !modifier {
                    if let Some(text) = &event.text {
                        push_prompt_text(&mut workshop, text);
                    }
                    continue;
                }
                if let Some(text) = crate::clipboard::read_text() {
                    push_prompt_text(&mut workshop, &text);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                push_prompt_text(&mut workshop, text);
            }
        }
    }
}

fn clear_prompt_limit_error(workshop: &mut Gen3dWorkshop) {
    if workshop
        .error
        .as_ref()
        .is_some_and(|err| err.starts_with("Prompt limit"))
    {
        workshop.error = None;
    }
}

fn push_prompt_text(workshop: &mut Gen3dWorkshop, text: &str) {
    let max_words = super::GEN3D_PROMPT_MAX_WORDS;
    let max_chars = super::GEN3D_PROMPT_MAX_CHARS;

    let mut words = super::gen3d_count_whitespace_separated_words(&workshop.prompt);
    let mut in_word = workshop
        .prompt
        .chars()
        .last()
        .is_some_and(|ch| !ch.is_whitespace());
    let mut chars = workshop.prompt.chars().count();

    let mut hit_words = false;
    let mut hit_chars = false;
    let mut inserted_any = false;

    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    for ch in normalized.chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        if chars >= max_chars {
            hit_chars = true;
            break;
        }

        let is_ws = ch.is_whitespace();
        if !is_ws && !in_word {
            if words >= max_words {
                hit_words = true;
                break;
            }
            words += 1;
            in_word = true;
        } else if is_ws {
            in_word = false;
        }

        workshop.prompt.push(ch);
        chars += 1;
        inserted_any = true;
    }

    if hit_words || hit_chars {
        let words_now = super::gen3d_count_whitespace_separated_words(&workshop.prompt);
        let chars_now = workshop.prompt.chars().count();
        let reason = if hit_words && hit_chars {
            "word+char limits"
        } else if hit_words {
            "word limit"
        } else {
            "char limit"
        };
        workshop.error = Some(format!(
            "Prompt limit reached ({reason}). words={words_now}/{max_words} chars={chars_now}/{max_chars}. Extra input ignored."
        ));
    } else if inserted_any {
        clear_prompt_limit_error(workshop);
    }
}

pub(crate) fn gen3d_collision_toggle_button(
    build_scene: Res<State<BuildScene>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dCollisionToggleButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                preview_state.show_collision = !preview_state.show_collision;
                preview_state.collision_dirty = true;
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65));
            }
        }
    }
}

pub(crate) fn gen3d_preview_animation_dropdown_button(
    build_scene: Res<State<BuildScene>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Changed<Interaction>,
            With<Gen3dPreviewAnimationDropdownButton>,
        ),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    for (interaction, mut bg) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            preview_state.animation_dropdown_open = !preview_state.animation_dropdown_open;
        }
        apply_gen3d_preview_animation_dropdown_button_style(
            preview_state.animation_dropdown_open,
            *interaction,
            &mut bg,
        );
    }
}

pub(crate) fn gen3d_preview_animation_option_buttons(
    build_scene: Res<State<BuildScene>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (
            &Interaction,
            &Gen3dPreviewAnimationOptionButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    for (interaction, button, mut bg, mut border) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            if let Some(channel) = preview_state
                .animation_channels
                .get(button.index())
                .cloned()
                .filter(|v| !v.trim().is_empty())
            {
                preview_state.animation_channel = channel;
            }
            preview_state.animation_dropdown_open = false;
        }
        let selected = preview_state
            .animation_channels
            .get(button.index())
            .is_some_and(|v| v == &preview_state.animation_channel);
        apply_gen3d_preview_animation_option_style(selected, *interaction, &mut bg, &mut border);
    }
}

pub(crate) fn gen3d_rebuild_preview_animation_dropdown_options_ui(
    build_scene: Res<State<BuildScene>>,
    preview_state: Res<Gen3dPreview>,
    mut last_hash: Local<Option<u64>>,
    mut scroll_panels: Query<&mut ScrollPosition, With<Gen3dPreviewAnimationDropdownList>>,
    lists: Query<Entity, With<Gen3dPreviewAnimationDropdownList>>,
    existing_text: Query<Entity, With<Gen3dPreviewAnimationOptionButtonText>>,
    existing_buttons: Query<Entity, With<Gen3dPreviewAnimationOptionButton>>,
    mut commands: Commands,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let Ok(list_entity) = lists.single() else {
        return;
    };

    let channels_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        preview_state.animation_channels.hash(&mut hasher);
        hasher.finish()
    };
    let ui_missing = !preview_state.animation_channels.is_empty() && existing_buttons.is_empty();
    if last_hash.as_ref() == Some(&channels_hash) && !ui_missing {
        return;
    }
    *last_hash = Some(channels_hash);

    if let Ok(mut scroll) = scroll_panels.single_mut() {
        scroll.y = 0.0;
    }

    for entity in &existing_text {
        commands.entity(entity).try_despawn();
    }
    for entity in &existing_buttons {
        commands.entity(entity).try_despawn();
    }
    commands.entity(list_entity).with_children(|list| {
        for (index, channel) in preview_state.animation_channels.iter().enumerate() {
            let selected = channel == &preview_state.animation_channel;
            let mut bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65));
            let mut border = BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65));
            apply_gen3d_preview_animation_option_style(
                selected,
                Interaction::None,
                &mut bg,
                &mut border,
            );

            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    height: Val::Px(22.0),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                bg,
                border,
                Gen3dPreviewAnimationOptionButton::new(index),
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new(gen3d_ui_motion_label(channel)),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    Gen3dPreviewAnimationOptionButtonText::new(index),
                ));
            });
        }
    });
}

fn gen3d_ui_motion_label(channel: &str) -> String {
    let channel = channel.trim();
    if channel.is_empty() {
        return "Idle".to_string();
    }
    if channel == "attack_primary" {
        return "Attack".to_string();
    }

    let mut words: Vec<String> = Vec::new();
    for w in channel.split('_').filter(|w| !w.is_empty()) {
        let mut chars = w.chars();
        let Some(first) = chars.next() else {
            continue;
        };
        let rest = chars.as_str();
        let mut word = first.to_uppercase().to_string();
        word.push_str(rest);
        words.push(word);
    }
    if words.is_empty() {
        return channel.to_string();
    }
    words.join(" ")
}

pub(crate) fn gen3d_preview_animation_dropdown_scroll_wheel(
    build_scene: Res<State<BuildScene>>,
    preview_state: Res<Gen3dPreview>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (
            &ComputedNode,
            &UiGlobalTransform,
            Option<&Visibility>,
            &mut ScrollPosition,
        ),
        With<Gen3dPreviewAnimationDropdownList>,
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        for _ in mouse_wheel.read() {}
        return;
    }
    if !preview_state.animation_dropdown_open {
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

    let Ok((node, transform, vis, mut scroll)) = panels.single_mut() else {
        for _ in mouse_wheel.read() {}
        return;
    };
    let visible = vis
        .map(|v| !matches!(*v, Visibility::Hidden))
        .unwrap_or(true);
    if !visible || !node.contains_point(*transform, cursor) {
        for _ in mouse_wheel.read() {}
        return;
    }

    let mut delta_lines = 0.0f32;
    for ev in mouse_wheel.read() {
        let lines = match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 120.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() < 1e-4 {
        return;
    }

    let delta_px = delta_lines * 24.0;
    scroll.y = (scroll.y - delta_px).max(0.0);
}

pub(crate) fn gen3d_update_preview_animation_dropdown_ui(
    build_scene: Res<State<BuildScene>>,
    preview_state: Res<Gen3dPreview>,
    mut last_state: Local<Option<(String, bool, u64)>>,
    mut dropdown_button: Query<
        (&Interaction, &mut BackgroundColor),
        (
            With<Gen3dPreviewAnimationDropdownButton>,
            Without<Gen3dPreviewAnimationOptionButton>,
        ),
    >,
    mut dropdown_text: Query<
        &mut Text,
        (
            With<Gen3dPreviewAnimationDropdownButtonText>,
            Without<Gen3dPreviewAnimationOptionButtonText>,
        ),
    >,
    mut list: Query<
        (&mut Node, &mut Visibility),
        (
            With<Gen3dPreviewAnimationDropdownList>,
            Without<Gen3dPreviewAnimationOptionButton>,
        ),
    >,
    mut option_texts: Query<
        (&Gen3dPreviewAnimationOptionButtonText, &mut Text),
        (
            Without<Gen3dPreviewAnimationDropdownButton>,
            Without<Gen3dPreviewAnimationDropdownButtonText>,
        ),
    >,
    mut option_buttons: Query<
        (
            &Interaction,
            &Gen3dPreviewAnimationOptionButton,
            &mut Node,
            &mut Visibility,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Without<Gen3dPreviewAnimationDropdownButton>,
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let channels_hash = {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        preview_state.animation_channels.hash(&mut hasher);
        hasher.finish()
    };
    let state = (
        preview_state.animation_channel.clone(),
        preview_state.animation_dropdown_open,
        channels_hash,
    );
    if last_state.as_ref() == Some(&state) {
        return;
    }
    *last_state = Some(state);

    let label = format!(
        "{} ▾",
        gen3d_ui_motion_label(&preview_state.animation_channel)
    );
    for mut text in &mut dropdown_text {
        **text = label.clone().into();
    }

    if let Ok((mut node, mut vis)) = list.single_mut() {
        if preview_state.animation_dropdown_open {
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        } else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
    }

    for (interaction, mut bg) in &mut dropdown_button {
        apply_gen3d_preview_animation_dropdown_button_style(
            preview_state.animation_dropdown_open,
            *interaction,
            &mut bg,
        );
    }

    for (marker, mut text) in &mut option_texts {
        let label = preview_state
            .animation_channels
            .get(marker.index())
            .map(|v| gen3d_ui_motion_label(v))
            .unwrap_or_default();
        **text = label.into();
    }

    for (interaction, button, mut node, mut vis, mut bg, mut border) in &mut option_buttons {
        let channel = preview_state.animation_channels.get(button.index());
        if channel.is_some() {
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        } else {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }

        let selected = channel.is_some_and(|v| v == &preview_state.animation_channel);
        apply_gen3d_preview_animation_option_style(selected, *interaction, &mut bg, &mut border);
    }
}

fn apply_gen3d_preview_animation_dropdown_button_style(
    open: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
) {
    let mut color = if open {
        Color::srgba(0.03, 0.03, 0.04, 0.82)
    } else {
        Color::srgba(0.02, 0.02, 0.03, 0.70)
    };

    match interaction {
        Interaction::Pressed => {
            color = Color::srgba(0.10, 0.10, 0.12, 0.92);
        }
        Interaction::Hovered => {
            color = Color::srgba(0.06, 0.06, 0.08, 0.86);
        }
        Interaction::None => {}
    }

    *bg = BackgroundColor(color);
}

fn apply_gen3d_preview_animation_option_style(
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

pub(crate) fn gen3d_clear_prompt_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    job: Res<Gen3dAiJob>,
    mut scroll_panels: Query<&mut ScrollPosition, With<Gen3dPromptScrollPanel>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dClearPromptButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                if job.is_running() {
                    workshop.error = Some("Cannot clear prompt while building.".into());
                } else {
                    workshop.prompt.clear();
                    workshop.error = None;
                    workshop.prompt_focused = true;
                    if let Ok(mut scroll) = scroll_panels.single_mut() {
                        scroll.y = 0.0;
                    }
                }
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.78));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65));
            }
        }
    }
}

pub(crate) fn gen3d_update_ui_text(
    mut commands: Commands,
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    preview_state: Res<Gen3dPreview>,
    draft: Res<Gen3dDraft>,
    job: Res<Gen3dAiJob>,
    ui_fonts: Res<UiFonts>,
    emoji_atlas: Res<EmojiAtlas>,
    asset_server: Res<AssetServer>,
    mut continue_buttons: Query<(&mut Node, &mut Visibility), With<Gen3dContinueButton>>,
    mut prompt_scroll: Query<(&ComputedNode, &mut ScrollPosition), With<Gen3dPromptScrollPanel>>,
    mut texts: ParamSet<(
        Query<&mut Text, With<Gen3dStatusText>>,
        Query<&mut Text, With<Gen3dGenerateButtonText>>,
        Query<&mut Text, With<Gen3dCollisionToggleText>>,
        Query<&mut Text, With<Gen3dPreviewStatsText>>,
    )>,
    rich_text: Query<Entity, With<Gen3dPromptRichText>>,
    mut last_prompt: Local<Option<String>>,
    mut last_prompt_entity: Local<Option<Entity>>,
    mut autoscroll_frames: Local<u8>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let prompt_text = if workshop.prompt.trim().is_empty() {
        "Drop images (optional) and add style/notes… (default: Voxel/Pixel Art)".to_string()
    } else {
        workshop.prompt.clone()
    };

    let prompt_entity = rich_text.single().ok();
    if prompt_entity != *last_prompt_entity {
        *last_prompt_entity = prompt_entity;
        // The prompt UI can be despawned/recreated (e.g. switching Preview ↔ Realm).
        // Force a re-render so the rich text starts with the correct current prompt.
        *last_prompt = None;
    }

    let prompt_changed = last_prompt.as_ref() != Some(&prompt_text);
    if prompt_changed {
        if let Some(entity) = prompt_entity {
            set_rich_text_line(
                &mut commands,
                entity,
                &prompt_text,
                &ui_fonts,
                &emoji_atlas,
                &asset_server,
                16.0,
                Color::srgb(0.92, 0.92, 0.96),
                None,
            );
            *last_prompt = Some(prompt_text.clone());
        }
    }
    if prompt_changed && workshop.prompt_focused {
        *autoscroll_frames = 3;
    } else if !workshop.prompt_focused {
        *autoscroll_frames = 0;
    }

    if *autoscroll_frames > 0 && workshop.prompt_scrollbar_drag.is_none() {
        if let Ok((node, mut scroll)) = prompt_scroll.single_mut() {
            let panel_scale = node.inverse_scale_factor();
            let viewport_h = node.size.y.max(0.0) * panel_scale;
            let content_h = node.content_size.y.max(0.0) * panel_scale;
            if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
                scroll.y = 0.0;
                *autoscroll_frames = 0;
            } else {
                let max_scroll = (content_h - viewport_h).max(0.0);
                scroll.y = max_scroll;
                *autoscroll_frames = (*autoscroll_frames).saturating_sub(1);
            }
        } else {
            *autoscroll_frames = 0;
        }
    }

    let mut status_text = workshop.status.clone();
    if job.is_running() {
        if let Some(msg) = job.progress_message() {
            let msg = msg.trim();
            if !msg.is_empty() {
                status_text.push_str("\nStep: ");
                status_text.push_str(msg);
            }
        }
    }
    let chat_fallbacks = job.chat_fallbacks_this_run();
    if chat_fallbacks > 0 {
        status_text.push_str(&format!(
            "\n\nNote: Used /chat/completions fallback ×{chat_fallbacks}. Results may be less consistent."
        ));
    }
    if !draft.defs.is_empty() {
        let primitives = draft.total_primitive_parts();
        let components = draft.component_count();
        if components > 0 {
            status_text.push_str(&format!(
                "\nDraft components: {components} | primitives: {primitives}"
            ));
        } else {
            status_text.push_str(&format!("\nDraft primitives: {primitives}"));
        }
    }
    if let Some(metrics) = job.status_metrics_text() {
        status_text.push_str(&metrics);
    }
    if let Some(err) = &workshop.error {
        status_text.push_str("\n\nError:\n");
        status_text.push_str(err);
    }
    {
        let mut status = texts.p0();
        for mut text in &mut status {
            **text = status_text.clone();
        }
    }

    let label = if job.is_running() { "Stop" } else { "Build" };
    {
        let mut button = texts.p1();
        for mut text in &mut button {
            **text = label.into();
        }
    }

    let show_continue = job.can_resume();
    for (mut node, mut vis) in &mut continue_buttons {
        if show_continue {
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        } else {
            // `Visibility::Hidden` keeps the element in the layout, so also disable it via `Display::None`.
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
    }

    let collision_label = if preview_state.show_collision {
        "Collision: On"
    } else {
        "Collision: Off"
    };
    {
        let mut collision = texts.p2();
        for mut text in &mut collision {
            **text = collision_label.into();
        }
    }

    let run_time = job
        .run_elapsed()
        .map(|d| {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{:.1}s", d.as_secs_f32())
            } else {
                format!("{}m {}s", secs / 60, secs % 60)
            }
        })
        .unwrap_or_else(|| "—".into());
    let run_tokens = format_compact_count(job.current_run_tokens());
    let total_tokens = format_compact_count(job.total_tokens());
    let stats_text = format!(
        "Run time: {run_time}\nTokens (run): {run_tokens}\nTokens (total): {total_tokens}",
    );
    {
        let mut stats = texts.p3();
        for mut text in &mut stats {
            **text = stats_text.clone();
        }
    }
}

fn format_compact_count(value: u64) -> String {
    const K: f64 = 1_000.0;
    const M: f64 = 1_000_000.0;
    const B: f64 = 1_000_000_000.0;

    let v = value as f64;
    if v >= B {
        format!("{:.2}B", v / B)
    } else if v >= M {
        format!("{:.2}M", v / M)
    } else if v >= K {
        format!("{:.2}K", v / K)
    } else {
        value.to_string()
    }
}
