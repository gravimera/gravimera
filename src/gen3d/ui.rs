use bevy::prelude::*;

use crate::assets::SceneAssets;
use crate::types::GameMode;

use super::ai::Gen3dAiJob;
use super::preview;
use super::state::*;

pub(crate) fn handle_gen3d_toggle_button(
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dToggleButton>),
    >,
    mode: Res<State<GameMode>>,
    mut next_mode: ResMut<NextState<GameMode>>,
    mut return_mode: ResMut<Gen3dReturnMode>,
) {
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
                match mode.get() {
                    GameMode::Gen3D => next_mode.set(return_mode.0),
                    other => {
                        return_mode.0 = *other;
                        next_mode.set(GameMode::Gen3D);
                    }
                }
            }
        }
    }
}

pub(crate) fn update_gen3d_toggle_button_label(
    mode: Res<State<GameMode>>,
    mut texts: Query<&mut Text, With<Gen3dToggleButtonText>>,
) {
    let label = match mode.get() {
        GameMode::Gen3D => "Back",
        _ => "Gen3D",
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
    mut workshop: ResMut<Gen3dWorkshop>,
    mut preview_state: ResMut<Gen3dPreview>,
) {
    workshop.status = format!(
        "Drop 0–{} images and/or type a prompt, then click Build.",
        super::GEN3D_MAX_IMAGES
    );
    workshop.error = None;
    workshop.prompt_focused = true;
    workshop.image_viewer = None;
    workshop.speed_mode = Gen3dSpeedMode::Level3;
    workshop.side_tab = Gen3dSideTab::Status;
    workshop.side_panel_open = false;
    workshop.tool_feedback_unread = false;
    preview_state.animation = Gen3dPreviewAnimation::Idle;
    preview_state.animation_dropdown_open = false;

    let target = preview::setup_preview_scene(
        &mut commands,
        &mut images,
        &assets,
        &mut materials,
        &mut preview_state,
    );

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
                // Left: photos.
                row.spawn((
                    Node {
                        width: Val::Px(260.0),
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
                        .with_children(|row| {
                            row.spawn((
                                Button,
                                Node {
                                    width: Val::Px(68.0),
                                    height: Val::Px(28.0),
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
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                    Gen3dClearImagesButtonText,
                                ));
                            });
                        });
                    panel.spawn((
                        Text::new(format!(
                            "Drop up to {} images (optional).\nAccepted: png/jpg/webp\nHover a thumbnail to see its name.\nClick to open (↑/↓ navigate, Esc to close).",
                            super::GEN3D_MAX_IMAGES
                        )),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.85, 0.85, 0.90)),
                        Gen3dImagesTipText,
                    ));

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
                                Gen3dImagesScrollPanel,
                            ))
                            .with_children(|scroll| {
                                scroll.spawn((
                                    Node {
                                        width: Val::Percent(100.0),
                                        flex_direction: FlexDirection::Row,
                                        flex_wrap: FlexWrap::Wrap,
                                        justify_content: JustifyContent::FlexStart,
                                        align_content: AlignContent::FlexStart,
                                        align_items: AlignItems::FlexStart,
                                        column_gap: Val::Px(6.0),
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                    Gen3dImagesList,
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
                                Gen3dImagesScrollbarTrack,
                            ))
                            .with_children(|track| {
                                track.spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        left: Val::Px(1.0),
                                        right: Val::Px(1.0),
                                        top: Val::Px(0.0),
                                        height: Val::Px(18.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.95, 0.85, 0.25, 0.85)),
                                    Gen3dImagesScrollbarThumb,
                                ));
                            });
                        });
                });

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

                    panel
                        .spawn((
                            Button,
                            Node {
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_height: Val::Px(0.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            Gen3dPreviewPanel,
                        ))
                        .with_children(|preview| {
                            preview.spawn((
                                ImageNode::new(target.clone()),
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Percent(100.0),
                                    ..default()
                                },
                            ));
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
                                                align_items: AlignItems::Center,
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
                                                Button,
                                                Node {
                                                    width: Val::Px(120.0),
                                                    height: Val::Px(22.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.70)),
                                                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                                Gen3dPreviewAnimationDropdownButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Idle ▾"),
                                                    TextFont {
                                                        font_size: 13.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    Gen3dPreviewAnimationDropdownButtonText,
                                                ));
                                            });
                                        });

                                    stats
                                        .spawn((
                                            Node {
                                                width: Val::Px(140.0),
                                                flex_direction: FlexDirection::Column,
                                                row_gap: Val::Px(2.0),
                                                padding: UiRect::all(Val::Px(4.0)),
                                                border: UiRect::all(Val::Px(1.0)),
                                                display: Display::None,
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.92)),
                                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                            Visibility::Hidden,
                                            Gen3dPreviewAnimationDropdownList,
                                        ))
                                        .with_children(|list| {
                                            for animation in [
                                                Gen3dPreviewAnimation::Idle,
                                                Gen3dPreviewAnimation::Move,
                                                Gen3dPreviewAnimation::Attack,
                                            ] {
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
                                                    BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.65)),
                                                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                                                    Gen3dPreviewAnimationOptionButton::new(animation),
                                                ))
                                                .with_children(|button| {
                                                    button.spawn((
                                                        Text::new(animation.label()),
                                                        TextFont {
                                                            font_size: 13.0,
                                                            ..default()
                                                        },
                                                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    ));
                                                });
                                            }
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

                            // Collapsible Status / Tool Feedback overlay (hidden by default).
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
                                                Gen3dSideTabButton::new(Gen3dSideTab::ToolFeedback),
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Tool Feedback"),
                                                    TextFont {
                                                        font_size: 14.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    Visibility::Inherited,
                                                    Gen3dSideTabButtonText::new(Gen3dSideTab::ToolFeedback),
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

                                    // Tool Feedback tab content (hidden by default).
                                    panel
                                        .spawn((
                                            Node {
                                                width: Val::Percent(100.0),
                                                flex_grow: 1.0,
                                                flex_basis: Val::Px(0.0),
                                                min_height: Val::Px(0.0),
                                                flex_direction: FlexDirection::Column,
                                                row_gap: Val::Px(8.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                            Visibility::Hidden,
                                            Gen3dToolFeedbackPanelRoot,
                                        ))
                                        .with_children(|tab| {
                                            tab.spawn((
                                                Node {
                                                    width: Val::Percent(100.0),
                                                    flex_direction: FlexDirection::Row,
                                                    column_gap: Val::Px(8.0),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::NONE),
                                                Visibility::Inherited,
                                            ))
                                            .with_children(|row| {
                                                row.spawn((
                                                    Button,
                                                    Node {
                                                        flex_grow: 1.0,
                                                        height: Val::Px(34.0),
                                                        justify_content: JustifyContent::Center,
                                                        align_items: AlignItems::Center,
                                                        border: UiRect::all(Val::Px(1.0)),
                                                        ..default()
                                                    },
                                                    BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.80)),
                                                    BorderColor::all(Color::srgb(0.30, 0.55, 0.95)),
                                                    Visibility::Inherited,
                                                    Gen3dCopyFeedbackCodexButton,
                                                ))
                                                .with_children(|button| {
                                                    button.spawn((
                                                        Text::new("Copy for Codex (last run)"),
                                                        TextFont {
                                                            font_size: 13.0,
                                                            ..default()
                                                        },
                                                        TextColor(Color::srgb(0.82, 0.90, 1.0)),
                                                        Visibility::Inherited,
                                                        Gen3dCopyFeedbackCodexButtonText,
                                                    ));
                                                });

                                                row.spawn((
                                                    Button,
                                                    Node {
                                                        width: Val::Px(92.0),
                                                        height: Val::Px(34.0),
                                                        justify_content: JustifyContent::Center,
                                                        align_items: AlignItems::Center,
                                                        border: UiRect::all(Val::Px(1.0)),
                                                        ..default()
                                                    },
                                                    BackgroundColor(Color::srgba(0.08, 0.10, 0.12, 0.78)),
                                                    BorderColor::all(Color::srgba(0.30, 0.30, 0.35, 0.70)),
                                                    Visibility::Inherited,
                                                    Gen3dCopyFeedbackJsonButton,
                                                ))
                                                .with_children(|button| {
                                                    button.spawn((
                                                        Text::new("Copy JSON"),
                                                        TextFont {
                                                            font_size: 13.0,
                                                            ..default()
                                                        },
                                                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                        Visibility::Inherited,
                                                        Gen3dCopyFeedbackJsonButtonText,
                                                    ));
                                                });
                                            });

                                            tab.spawn((
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
                                                    Gen3dToolFeedbackScrollPanel,
                                                ))
                                                .with_children(|scroll| {
                                                    scroll.spawn((
                                                        Text::new(""),
                                                        TextFont {
                                                            font_size: 13.0,
                                                            ..default()
                                                        },
                                                        TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                                        Visibility::Inherited,
                                                        Gen3dToolFeedbackText,
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
                                                    Gen3dToolFeedbackScrollbarTrack,
                                                ))
                                                .with_children(|track| {
                                                    track.spawn((
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
                                                        Gen3dToolFeedbackScrollbarThumb,
                                                    ));
                                                });
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
                    height: Val::Px(160.0),
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
                    Button,
                    Node {
                        flex_grow: 1.0,
                        flex_basis: Val::Px(0.0),
                        height: Val::Percent(100.0),
                        border: UiRect::all(Val::Px(1.0)),
                        flex_direction: FlexDirection::Row,
                        min_height: Val::Px(0.0),
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
                            scroll.spawn((
                                Text::new("Optional: style/notes… (default: Voxel/Pixel Art)"),
                                TextFont {
                                    font_size: 16.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                Gen3dPromptText,
                            ));
                        });

                    prompt
                        .spawn((
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
    mut preview_state: ResMut<Gen3dPreview>,
    mut workshop: ResMut<Gen3dWorkshop>,
) {
    for entity in &roots {
        commands.entity(entity).try_despawn();
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
    for entity in &viewer_roots {
        commands.entity(entity).try_despawn();
    }

    preview_state.target = None;
    preview_state.camera = None;
    preview_state.root = None;
    preview_state.last_cursor = None;
    preview_state.collision_dirty = false;
    preview_state.animation = Gen3dPreviewAnimation::Idle;
    preview_state.animation_dropdown_open = false;
    workshop.image_viewer = None;
}

pub(crate) fn gen3d_prompt_box_focus(
    mut workshop: ResMut<Gen3dWorkshop>,
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
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70));
            }
            Interaction::None => {
                let alpha = if workshop.prompt_focused { 0.70 } else { 0.65 };
                *bg = BackgroundColor(Color::srgba(0.02, 0.02, 0.03, alpha));
            }
        }
    }
}

pub(crate) fn gen3d_side_panel_toggle_button(
    mode: Res<State<GameMode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dSidePanelToggleButton>),
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    mode: Res<State<GameMode>>,
    workshop: Res<Gen3dWorkshop>,
    mut panels: Query<(&mut Node, &mut Visibility), With<Gen3dSidePanelRoot>>,
    mut texts: Query<&mut Text, With<Gen3dSidePanelToggleButtonText>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    mode: Res<State<GameMode>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<Gen3dPromptScrollPanel>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    scroll.y = (scroll.y - delta_px).max(0.0);
}

pub(crate) fn gen3d_update_prompt_scrollbar_ui(
    mode: Res<State<GameMode>>,
    panels: Query<&ComputedNode, With<Gen3dPromptScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<Gen3dPromptScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<Gen3dPromptScrollbarThumb>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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

    let viewport_h = panel.size.y.max(0.0);
    let content_h = panel.content_size.y.max(0.0);
    let track_h = track_node.size.y.max(1.0);

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

    *track_vis = Visibility::Visible;

    let max_scroll = (content_h - viewport_h).max(1.0);
    let scroll_y = panel.scroll_position.y.clamp(0.0, max_scroll);

    let min_thumb_h = 14.0;
    let thumb_h = (track_h * (viewport_h / content_h)).clamp(min_thumb_h, track_h);
    let thumb_top = (track_h - thumb_h) * (scroll_y / max_scroll);

    thumb.top = Val::Px(thumb_top);
    thumb.height = Val::Px(thumb_h);
}

pub(crate) fn gen3d_prompt_text_input(
    mode: Res<State<GameMode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<bevy::input::keyboard::KeyboardInput>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    if !workshop.prompt_focused {
        return;
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        match event.key_code {
            KeyCode::Backspace => {
                workshop.prompt.pop();
            }
            KeyCode::Escape => {
                workshop.prompt_focused = false;
            }
            KeyCode::Enter | KeyCode::NumpadEnter => {}
            KeyCode::KeyV => {
                let modifier = keys.pressed(KeyCode::ControlLeft)
                    || keys.pressed(KeyCode::ControlRight)
                    || keys.pressed(KeyCode::SuperLeft)
                    || keys.pressed(KeyCode::SuperRight);
                if !modifier {
                    if let Some(text) = &event.text {
                        push_prompt_text(&mut workshop.prompt, text);
                    }
                    continue;
                }
                if let Some(text) = crate::clipboard::read_text() {
                    push_prompt_text(&mut workshop.prompt, &text);
                }
            }
            _ => {
                let Some(text) = &event.text else {
                    continue;
                };
                push_prompt_text(&mut workshop.prompt, text);
            }
        }
    }
}

fn push_prompt_text(prompt: &mut String, text: &str) {
    let mut inserted = 0usize;
    for ch in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
        if ch.is_control() && ch != '\n' && ch != '\t' {
            continue;
        }
        prompt.push(ch);
        inserted += 1;
        if inserted >= 4096 {
            break;
        }
    }
}

pub(crate) fn gen3d_collision_toggle_button(
    mode: Res<State<GameMode>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dCollisionToggleButton>),
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    mode: Res<State<GameMode>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (
            Changed<Interaction>,
            With<Gen3dPreviewAnimationDropdownButton>,
        ),
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    mode: Res<State<GameMode>>,
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
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    for (interaction, button, mut bg, mut border) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            preview_state.animation = button.animation();
            preview_state.animation_dropdown_open = false;
        }
        let selected = button.animation() == preview_state.animation;
        apply_gen3d_preview_animation_option_style(selected, *interaction, &mut bg, &mut border);
    }
}

pub(crate) fn gen3d_update_preview_animation_dropdown_ui(
    mode: Res<State<GameMode>>,
    preview_state: Res<Gen3dPreview>,
    mut last_state: Local<Option<(Gen3dPreviewAnimation, bool)>>,
    mut dropdown_button: Query<
        (&Interaction, &mut BackgroundColor),
        (
            With<Gen3dPreviewAnimationDropdownButton>,
            Without<Gen3dPreviewAnimationOptionButton>,
        ),
    >,
    mut dropdown_text: Query<&mut Text, With<Gen3dPreviewAnimationDropdownButtonText>>,
    mut list: Query<(&mut Node, &mut Visibility), With<Gen3dPreviewAnimationDropdownList>>,
    mut option_buttons: Query<
        (
            &Interaction,
            &Gen3dPreviewAnimationOptionButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Without<Gen3dPreviewAnimationDropdownButton>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    let state = (
        preview_state.animation,
        preview_state.animation_dropdown_open,
    );
    if last_state.as_ref() == Some(&state) {
        return;
    }
    *last_state = Some(state);

    let label = format!("{} ▾", preview_state.animation.label());
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

    for (interaction, button, mut bg, mut border) in &mut option_buttons {
        let selected = button.animation() == preview_state.animation;
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
    mode: Res<State<GameMode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    job: Res<Gen3dAiJob>,
    mut scroll_panels: Query<&mut ScrollPosition, With<Gen3dPromptScrollPanel>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dClearPromptButton>),
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
    mode: Res<State<GameMode>>,
    workshop: Res<Gen3dWorkshop>,
    preview_state: Res<Gen3dPreview>,
    draft: Res<Gen3dDraft>,
    job: Res<Gen3dAiJob>,
    mut texts: ParamSet<(
        Query<&mut Text, With<Gen3dPromptText>>,
        Query<&mut Text, With<Gen3dStatusText>>,
        Query<&mut Text, With<Gen3dGenerateButtonText>>,
        Query<&mut Text, With<Gen3dCollisionToggleText>>,
        Query<&mut Text, With<Gen3dPreviewStatsText>>,
    )>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }

    let prompt_text = if workshop.prompt.trim().is_empty() {
        "Optional: style/notes… (default: Voxel/Pixel Art)".to_string()
    } else {
        workshop.prompt.clone()
    };
    {
        let mut prompt = texts.p0();
        for mut text in &mut prompt {
            **text = prompt_text.clone();
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
        let mut status = texts.p1();
        for mut text in &mut status {
            **text = status_text.clone();
        }
    }

    let label = if job.is_running() { "Stop" } else { "Build" };
    {
        let mut button = texts.p2();
        for mut text in &mut button {
            **text = label.into();
        }
    }

    let collision_label = if preview_state.show_collision {
        "Collision: On"
    } else {
        "Collision: Off"
    };
    {
        let mut collision = texts.p3();
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
        let mut stats = texts.p4();
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
