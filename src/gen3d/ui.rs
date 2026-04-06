use bevy::ecs::hierarchy::{ChildOf, ChildSpawnerCommands};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{Ime, PrimaryWindow};
use std::path::PathBuf;
use std::sync::mpsc;

use crate::assets::SceneAssets;
use crate::object::registry::ObjectLibrary;
use crate::rich_text::set_rich_text_line;
use crate::types::{BuildScene, EmojiAtlas, UiFonts};
use crate::ui::{set_ime_position_for_rich_text, ImeAnchorXPolicy};

use super::ai::Gen3dAiJob;
use super::preview;
use super::state::*;
use super::task_queue::{Gen3dTaskQueue, Gen3dTaskState};

fn aspect_fit_size(content_w_px: f32, content_h_px: f32, aspect: f32) -> (f32, f32) {
    let content_w_px = content_w_px.max(1.0);
    let content_h_px = content_h_px.max(1.0);
    let aspect = aspect.clamp(0.05, 20.0);

    let box_aspect = (content_w_px / content_h_px).max(0.05);
    if aspect >= box_aspect {
        let w = content_w_px;
        (w, (w / aspect).max(1.0))
    } else {
        let h = content_h_px;
        ((h * aspect).max(1.0), h)
    }
}

fn active_session_is_queued(task_queue: &Gen3dTaskQueue) -> bool {
    task_queue
        .metas
        .get(&task_queue.active_session_id)
        .is_some_and(|meta| meta.task_state == Gen3dTaskState::Waiting)
}

fn active_session_queue_position(task_queue: &Gen3dTaskQueue) -> Option<(usize, usize)> {
    if !active_session_is_queued(task_queue) {
        return None;
    }
    let total = task_queue.queue.len();
    let pos = task_queue
        .queue
        .iter()
        .position(|id| *id == task_queue.active_session_id)?;
    Some((pos + 1, total))
}

pub(crate) fn spawn_gen3d_preview_panel<F>(
    parent: &mut ChildSpawnerCommands,
    node: Node,
    target: Handle<Image>,
    color_picker_palette: Handle<Image>,
    color_picker_value: Handle<Image>,
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
                    min_width: Val::Px(0.0),
                    min_height: Val::Px(0.0),
                    ..default()
                },
                Gen3dPreviewPanelImage,
            ));
            preview
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    BackgroundColor(Color::NONE),
                    Gen3dPreviewOverlayRoot,
                ))
                .with_children(|overlay| {
                    overlay
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                top: Val::Px(0.0),
                                width: Val::Px(0.0),
                                height: Val::Px(0.0),
                                display: Display::None,
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            Visibility::Hidden,
                            ZIndex(12),
                            Gen3dPreviewHoverFrame,
                        ))
                        .with_children(|frame| {
                            const CORNER_LEN_PX: f32 = 18.0;
                            const EDGE_THICKNESS_PX: f32 = 2.0;
                            const EDGE_INSET_PX: f32 = 14.0;
                            const EDGE_SPAN_PERCENT: f32 = 28.0;
                            const CORNER_OVERHANG_PX: f32 = 1.0;

                            let mut spawn_segment = |node: Node, color: Color| {
                                frame.spawn((
                                    Node {
                                        border_radius: BorderRadius::all(Val::Px(1.0)),
                                        ..node
                                    },
                                    BackgroundColor(color),
                                ));
                            };

                            let accent = Color::srgb(0.06, 0.84, 1.0);
                            let accent_soft = Color::srgba(0.92, 0.98, 1.0, 0.94);

                            // Edge segments with center gaps keep the frame readable without
                            // collapsing back into a plain rectangular box.
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(EDGE_INSET_PX),
                                    top: Val::Px(0.0),
                                    width: Val::Percent(EDGE_SPAN_PERCENT),
                                    height: Val::Px(EDGE_THICKNESS_PX),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(EDGE_INSET_PX),
                                    top: Val::Px(0.0),
                                    width: Val::Percent(EDGE_SPAN_PERCENT),
                                    height: Val::Px(EDGE_THICKNESS_PX),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(EDGE_INSET_PX),
                                    bottom: Val::Px(0.0),
                                    width: Val::Percent(EDGE_SPAN_PERCENT),
                                    height: Val::Px(EDGE_THICKNESS_PX),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(EDGE_INSET_PX),
                                    bottom: Val::Px(0.0),
                                    width: Val::Percent(EDGE_SPAN_PERCENT),
                                    height: Val::Px(EDGE_THICKNESS_PX),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(0.0),
                                    top: Val::Px(EDGE_INSET_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX),
                                    height: Val::Percent(EDGE_SPAN_PERCENT),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(0.0),
                                    bottom: Val::Px(EDGE_INSET_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX),
                                    height: Val::Percent(EDGE_SPAN_PERCENT),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(0.0),
                                    top: Val::Px(EDGE_INSET_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX),
                                    height: Val::Percent(EDGE_SPAN_PERCENT),
                                    ..default()
                                },
                                accent,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(0.0),
                                    bottom: Val::Px(EDGE_INSET_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX),
                                    height: Val::Percent(EDGE_SPAN_PERCENT),
                                    ..default()
                                },
                                accent,
                            );

                            // Top-left
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(-CORNER_OVERHANG_PX),
                                    top: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(CORNER_LEN_PX),
                                    height: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    ..default()
                                },
                                accent_soft,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(-CORNER_OVERHANG_PX),
                                    top: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    height: Val::Px(CORNER_LEN_PX),
                                    ..default()
                                },
                                accent,
                            );

                            // Top-right
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(-CORNER_OVERHANG_PX),
                                    top: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(CORNER_LEN_PX),
                                    height: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    ..default()
                                },
                                accent_soft,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(-CORNER_OVERHANG_PX),
                                    top: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    height: Val::Px(CORNER_LEN_PX),
                                    ..default()
                                },
                                accent,
                            );

                            // Bottom-left
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(-CORNER_OVERHANG_PX),
                                    bottom: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(CORNER_LEN_PX),
                                    height: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    ..default()
                                },
                                accent_soft,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(-CORNER_OVERHANG_PX),
                                    bottom: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    height: Val::Px(CORNER_LEN_PX),
                                    ..default()
                                },
                                accent,
                            );

                            // Bottom-right
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(-CORNER_OVERHANG_PX),
                                    bottom: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(CORNER_LEN_PX),
                                    height: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    ..default()
                                },
                                accent_soft,
                            );
                            spawn_segment(
                                Node {
                                    position_type: PositionType::Absolute,
                                    right: Val::Px(-CORNER_OVERHANG_PX),
                                    bottom: Val::Px(-CORNER_OVERHANG_PX),
                                    width: Val::Px(EDGE_THICKNESS_PX + 1.0),
                                    height: Val::Px(CORNER_LEN_PX),
                                    ..default()
                                },
                                accent,
                            );
                        });
                    overlay
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                top: Val::Px(0.0),
                                max_width: Val::Px(220.0),
                                padding: UiRect::all(Val::Px(8.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                display: Display::None,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.94)),
                            BorderColor::all(Color::srgba(0.30, 0.52, 0.66, 0.92)),
                            Visibility::Hidden,
                            ZIndex(13),
                            Gen3dPreviewHoverInfoCard,
                        ))
                        .with_children(|card| {
                            card.spawn((
                                Text::new(""),
                                TextFont {
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.94, 0.97)),
                                Gen3dPreviewHoverInfoText,
                            ));
                        });
                    overlay.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(0.0),
                            height: Val::Px(0.0),
                            border: UiRect::all(Val::Px(2.0)),
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        BorderColor::all(Color::srgba(0.82, 0.45, 1.0, 0.95)),
                        Visibility::Hidden,
                        ZIndex(14),
                        Gen3dTweakSelectedFrame,
                    ));
                    overlay
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                top: Val::Px(0.0),
                                max_width: Val::Px(240.0),
                                padding: UiRect::all(Val::Px(8.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(4.0),
                                display: Display::None,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.94)),
                            BorderColor::all(Color::srgba(0.58, 0.30, 0.70, 0.92)),
                            Visibility::Hidden,
                            ZIndex(15),
                            Gen3dTweakSelectedInfoCard,
                        ))
                        .with_children(|card| {
                            card.spawn((
                                Text::new(""),
                                TextFont {
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.96, 0.92, 0.99)),
                                Gen3dTweakSelectedInfoText,
                            ));
                        });

                    if color_picker_palette != Handle::default()
                        && color_picker_value != Handle::default()
                    {
                        overlay.spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                right: Val::Px(8.0),
                                bottom: Val::Px(8.0),
                                width: Val::Px(420.0),
                                padding: UiRect::all(Val::Px(10.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                flex_direction: FlexDirection::Column,
                                row_gap: Val::Px(8.0),
                                display: Display::None,
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.96)),
                            BorderColor::all(Color::srgba(0.30, 0.55, 0.95, 0.92)),
                            Visibility::Hidden,
                            ZIndex(20),
                            Gen3dManualTweakColorPickerRoot,
                        ))
                        .with_children(|picker| {
                            picker
                                .spawn((
                                    Node {
                                        flex_direction: FlexDirection::Row,
                                        column_gap: Val::Px(10.0),
                                        align_items: AlignItems::FlexStart,
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                ))
                                .with_children(|row| {
                                    row.spawn((
                                        Node {
                                            width: Val::Px(180.0),
                                            height: Val::Px(180.0),
                                            border: UiRect::all(Val::Px(1.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.85)),
                                        BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                        Gen3dManualTweakColorPickerPalette,
                                    ))
                                    .with_children(|palette| {
                                        palette.spawn((
                                            ImageNode::new(color_picker_palette.clone()),
                                            Node {
                                                width: Val::Percent(100.0),
                                                height: Val::Percent(100.0),
                                                ..default()
                                            },
                                        ));
                                        palette.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(0.0),
                                                top: Val::Px(0.0),
                                                width: Val::Px(12.0),
                                                height: Val::Px(12.0),
                                                border: UiRect::all(Val::Px(2.0)),
                                                border_radius: BorderRadius::all(Val::Px(999.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                            BorderColor::all(Color::srgba(1.0, 1.0, 1.0, 0.95)),
                                            ZIndex(21),
                                            Gen3dManualTweakColorPickerPaletteSelector,
                                        ));
                                    });

                                    row.spawn((
                                        Node {
                                            width: Val::Px(18.0),
                                            height: Val::Px(180.0),
                                            border: UiRect::all(Val::Px(1.0)),
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.85)),
                                        BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                        Gen3dManualTweakColorPickerValue,
                                    ))
                                    .with_children(|value| {
                                        value.spawn((
                                            ImageNode::new(color_picker_value.clone()),
                                            Node {
                                                width: Val::Percent(100.0),
                                                height: Val::Percent(100.0),
                                                ..default()
                                            },
                                        ));
                                        value.spawn((
                                            Node {
                                                position_type: PositionType::Absolute,
                                                left: Val::Px(0.0),
                                                top: Val::Px(0.0),
                                                width: Val::Px(18.0),
                                                height: Val::Px(4.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.95)),
                                            ZIndex(21),
                                            Gen3dManualTweakColorPickerValueSelector,
                                        ));
                                    });

                                    row.spawn((
                                        Node {
                                            width: Val::Px(190.0),
                                            flex_direction: FlexDirection::Column,
                                            row_gap: Val::Px(8.0),
                                            ..default()
                                        },
                                        BackgroundColor(Color::NONE),
                                    ))
                                    .with_children(|col| {
                                        col.spawn((
                                            Node {
                                                width: Val::Px(72.0),
                                                height: Val::Px(28.0),
                                                border: UiRect::all(Val::Px(1.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 1.0)),
                                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                            Gen3dManualTweakColorPickerPreviewSwatch,
                                        ));

                                        col.spawn((
                                            Text::new("#RRGGBB"),
                                            TextFont {
                                                font_size: 13.0,
                                                ..default()
                                            },
                                            TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                        ));

                                        col.spawn((
                                            Button,
                                            Node {
                                                width: Val::Px(190.0),
                                                height: Val::Px(30.0),
                                                padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                                                justify_content: JustifyContent::FlexStart,
                                                align_items: AlignItems::Center,
                                                border: UiRect::all(Val::Px(1.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.80)),
                                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.75)),
                                            Gen3dManualTweakColorPickerRgbField,
                                        ))
                                        .with_children(|field| {
                                            field.spawn((
                                                Text::new(""),
                                                TextFont {
                                                    font_size: 14.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                Gen3dManualTweakColorPickerRgbFieldText,
                                            ));
                                        });

                                        col.spawn((
                                            Node {
                                                width: Val::Px(190.0),
                                                flex_direction: FlexDirection::Row,
                                                column_gap: Val::Px(8.0),
                                                ..default()
                                            },
                                            BackgroundColor(Color::NONE),
                                        ))
                                        .with_children(|row| {
                                            row.spawn((
                                                Button,
                                                Node {
                                                    width: Val::Px(91.0),
                                                    height: Val::Px(34.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.85)),
                                                BorderColor::all(Color::srgb(0.25, 0.80, 0.45)),
                                                Gen3dManualTweakColorPickerApplyButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Apply"),
                                                    TextFont {
                                                        font_size: 14.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.70, 1.0, 0.82)),
                                                    Gen3dManualTweakColorPickerApplyButtonText,
                                                ));
                                            });

                                            row.spawn((
                                                Button,
                                                Node {
                                                    width: Val::Px(91.0),
                                                    height: Val::Px(34.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.80)),
                                                BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70)),
                                                Gen3dManualTweakColorPickerCancelButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Cancel"),
                                                    TextFont {
                                                        font_size: 14.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.94, 0.94, 0.96)),
                                                    Gen3dManualTweakColorPickerCancelButtonText,
                                                ));
                                            });
                                        });
                                    });
                                });

                            picker.spawn((
                                Text::new("Recent"),
                                TextFont {
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                            ));

                            picker
                                .spawn((
                                    Node {
                                        flex_direction: FlexDirection::Row,
                                        flex_wrap: FlexWrap::Wrap,
                                        column_gap: Val::Px(6.0),
                                        row_gap: Val::Px(6.0),
                                        ..default()
                                    },
                                    BackgroundColor(Color::NONE),
                                ))
                                .with_children(|history| {
                                    for index in 0..12usize {
                                        history.spawn((
                                            Button,
                                            Node {
                                                width: Val::Px(20.0),
                                                height: Val::Px(20.0),
                                                border: UiRect::all(Val::Px(1.0)),
                                                ..default()
                                            },
                                            BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55)),
                                            BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70)),
                                            Gen3dManualTweakColorPickerRecentSwatch::new(index),
                                        ));
                                    }
                                });
                        });
                    }
                    overlay
                        .spawn((
                            Node {
                                position_type: PositionType::Absolute,
                                left: Val::Px(0.0),
                                top: Val::Px(0.0),
                                width: Val::Percent(100.0),
                                height: Val::Percent(100.0),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            Gen3dPreviewComponentLabelsRoot,
                        ))
                        .with_children(|labels| {
                            for index in 0..super::GEN3D_MAX_COMPONENTS {
                                labels
                                    .spawn((
                                        Node {
                                            position_type: PositionType::Absolute,
                                            left: Val::Px(0.0),
                                            top: Val::Px(0.0),
                                            padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                                            border: UiRect::all(Val::Px(1.0)),
                                            display: Display::None,
                                            ..default()
                                        },
                                        BackgroundColor(Color::srgba(0.02, 0.02, 0.03, 0.86)),
                                        BorderColor::all(Color::srgba(0.22, 0.22, 0.28, 0.76)),
                                        Visibility::Hidden,
                                        ZIndex(11),
                                        Gen3dPreviewComponentLabel::new(index),
                                    ))
                                    .with_children(|label| {
                                        label.spawn((
                                            Text::new(""),
                                            TextFont {
                                                font_size: 12.0,
                                                ..default()
                                            },
                                            TextColor(Color::srgb(0.94, 0.94, 0.96)),
                                            Gen3dPreviewComponentLabelText::new(index),
                                        ));
                                    });
                            }
                        });
                });
            extra_children(preview);
        })
        .id()
}

pub(crate) fn gen3d_update_preview_panel_image_fit(
    images: Res<Assets<Image>>,
    panels: Query<&ComputedNode, With<Gen3dPreviewPanel>>,
    mut preview_images: Query<(&ChildOf, &ImageNode, &mut Node), With<Gen3dPreviewPanelImage>>,
) {
    for (parent, image_node, mut node) in &mut preview_images {
        let Ok(panel) = panels.get(parent.parent()) else {
            continue;
        };

        let Some(texture) = images.get(&image_node.image) else {
            continue;
        };
        let size = texture.size();
        if size.y == 0 {
            continue;
        }
        let aspect = (size.x.max(1) as f32 / size.y.max(1) as f32).clamp(0.05, 20.0);

        let scale = panel.inverse_scale_factor();
        let content_w_px = (panel.size.x
            - panel.border.min_inset.x
            - panel.border.max_inset.x
            - panel.padding.min_inset.x
            - panel.padding.max_inset.x)
            .max(0.0)
            * scale;
        let content_h_px = (panel.size.y
            - panel.border.min_inset.y
            - panel.border.max_inset.y
            - panel.padding.min_inset.y
            - panel.padding.max_inset.y)
            .max(0.0)
            * scale;
        if content_w_px < 1.0 || content_h_px < 1.0 {
            continue;
        }

        let (fit_w, fit_h) = aspect_fit_size(content_w_px, content_h_px, aspect);

        fn px_value(v: &Val) -> Option<f32> {
            match v {
                Val::Px(px) => Some(*px),
                _ => None,
            }
        }

        let needs_w = px_value(&node.width).is_none_or(|v| (v - fit_w).abs() > 0.5);
        let needs_h = px_value(&node.height).is_none_or(|v| (v - fit_h).abs() > 0.5);

        if needs_w {
            node.width = Val::Px(fit_w);
        }
        if needs_h {
            node.height = Val::Px(fit_h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aspect_fit_size_preserves_aspect_ratio_within_box() {
        let (w, h) = aspect_fit_size(100.0, 100.0, 2.0);
        assert!((w - 100.0).abs() < 1e-3);
        assert!((h - 50.0).abs() < 1e-3);

        let (w, h) = aspect_fit_size(100.0, 100.0, 0.5);
        assert!((w - 50.0).abs() < 1e-3);
        assert!((h - 100.0).abs() < 1e-3);

        let aspect = 16.0 / 9.0;
        let (w, h) = aspect_fit_size(200.0, 100.0, aspect);
        assert!(w <= 200.0 + 1e-3);
        assert!((h - 100.0).abs() < 1e-3);
        assert!(((w / h) - aspect).abs() < 1e-3);

        let (w, h) = aspect_fit_size(100.0, 200.0, aspect);
        assert!((w - 100.0).abs() < 1e-3);
        assert!(h <= 200.0 + 1e-3);
        assert!(((w / h) - aspect).abs() < 1e-3);
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
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut meta_state: ResMut<crate::motion_ui::MotionAlgorithmUiState>,
    mut meta_roots: Query<&mut Visibility, With<crate::motion_ui::MotionAlgorithmUiRoot>>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    let show_intro = !job.is_running()
        && job.run_id().is_none()
        && job.edit_base_prefab_id().is_none()
        && workshop.status.trim().is_empty();
    if show_intro {
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
    tweak.enabled = false;
    tweak.selected_part_id = None;
    tweak.color_picker_open = false;
    tweak.color_picker_rgb_focused = false;
    preview_state.animation_channel = "idle".to_string();
    preview_state.animation_channels.clear();
    preview_state.animation_dropdown_open = false;
    preview_state.explode_components = false;
    preview_state.hovered_component = None;
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

    if tweak.color_picker_palette_image == Handle::default() {
        let size: u32 = 256;
        let data = vec![255u8; (size * size * 4) as usize];
        tweak.color_picker_palette_image = images.add(Image::new(
            Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        ));
    }

    if tweak.color_picker_value_image == Handle::default() {
        let width: u32 = 16;
        let height: u32 = 256;
        let data = vec![255u8; (width * height * 4) as usize];
        tweak.color_picker_value_image = images.add(Image::new(
            Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            bevy::asset::RenderAssetUsages::default(),
        ));
    }

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
                    right: Val::Px(12.0),
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
                Gen3dExitButton,
            ))
            .with_children(|b| {
                b.spawn((
                    Text::new("Exit"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
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
                    let color_picker_palette = tweak.color_picker_palette_image.clone();
                    let color_picker_value = tweak.color_picker_value_image.clone();
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
                        color_picker_palette,
                        color_picker_value,
                        |preview| {
                            preview
                                .spawn((
                                    Node {
                                        position_type: PositionType::Absolute,
                                        right: Val::Px(8.0),
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
                                                Text::new("Inspect:"),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                            ));
                                            row.spawn((
                                                Button,
                                                Node {
                                                    min_width: Val::Px(112.0),
                                                    height: Val::Px(22.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    padding: UiRect::axes(
                                                        Val::Px(10.0),
                                                        Val::Px(0.0),
                                                    ),
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(
                                                    0.02, 0.02, 0.03, 0.70,
                                                )),
                                                BorderColor::all(Color::srgba(
                                                    0.25, 0.25, 0.30, 0.65,
                                                )),
                                                Gen3dPreviewExplodeToggleButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Explode Off"),
                                                    TextFont {
                                                        font_size: 13.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    Gen3dPreviewExplodeToggleButtonText,
                                                ));
                                            });
                                        });

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
                                                Text::new("Export:"),
                                                TextFont {
                                                    font_size: 13.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                            ));
                                            row.spawn((
                                                Button,
                                                Node {
                                                    min_width: Val::Px(112.0),
                                                    height: Val::Px(22.0),
                                                    justify_content: JustifyContent::Center,
                                                    align_items: AlignItems::Center,
                                                    padding: UiRect::axes(
                                                        Val::Px(10.0),
                                                        Val::Px(0.0),
                                                    ),
                                                    border: UiRect::all(Val::Px(1.0)),
                                                    ..default()
                                                },
                                                BackgroundColor(Color::srgba(
                                                    0.02, 0.02, 0.03, 0.70,
                                                )),
                                                BorderColor::all(Color::srgba(
                                                    0.25, 0.25, 0.30, 0.65,
                                                )),
                                                Gen3dPreviewExportButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Export Preview"),
                                                    TextFont {
                                                        font_size: 13.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    Gen3dPreviewExportButtonText,
                                                ));
                                            });
                                        });
                                });
                        },
                    );

                });
            });

            // Collapsible side panel toggle.
            root.spawn((
                Button,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(12.0),
                    top: Val::Px(12.0),
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
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(12.0),
                    top: Val::Px(48.0),
                    bottom: Val::Px(12.0),
                    width: Val::Px(520.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(6.0),
                    padding: UiRect::all(Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    min_height: Val::Px(0.0),
                    display: Display::None,
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
                            Gen3dSideTabButton::new(Gen3dSideTab::Prefab),
                        ))
                        .with_children(|button| {
                            button.spawn((
                                Text::new("Prefab"),
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                Visibility::Inherited,
                                Gen3dSideTabButtonText::new(Gen3dSideTab::Prefab),
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
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(8.0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        Visibility::Inherited,
                        Gen3dStatusPanelRoot,
                    ))
                    .with_children(|col| {
                        // Summary (keeps updating).
                        col.spawn((
                            Node {
                                width: Val::Percent(100.0),
                                height: Val::Px(148.0),
                                min_height: Val::Px(0.0),
                                padding: UiRect::all(Val::Px(8.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                overflow: Overflow::clip(),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.01, 0.01, 0.015, 0.55)),
                            BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                            Visibility::Inherited,
                        ))
                        .with_children(|summary| {
                            summary.spawn((
                                Text::new(""),
                                Node {
                                    width: Val::Percent(100.0),
                                    align_self: AlignSelf::FlexStart,
                                    ..default()
                                },
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                Visibility::Inherited,
                                Gen3dStatusText,
                            ));
                        });

                        // Logs (scrollable).
                        col.spawn((
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
                                    flex_direction: FlexDirection::Column,
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
                                    Node {
                                        width: Val::Percent(100.0),
                                        align_self: AlignSelf::FlexStart,
                                        ..default()
                                    },
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                    Visibility::Inherited,
                                    Gen3dStatusLogsText,
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

                // Prefab tab content.
                panel
                    .spawn((
                        Node {
                            width: Val::Percent(100.0),
                            flex_grow: 1.0,
                            flex_basis: Val::Px(0.0),
                            min_height: Val::Px(0.0),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            display: Display::None,
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        Visibility::Hidden,
                        Gen3dPrefabPanelRoot,
                    ))
                    .with_children(|row| {
                        row.spawn((
                            Node {
                                flex_grow: 1.0,
                                flex_basis: Val::Px(0.0),
                                min_height: Val::Px(0.0),
                                flex_direction: FlexDirection::Column,
                                overflow: Overflow::scroll_y(),
                                ..default()
                            },
                            BackgroundColor(Color::NONE),
                            Visibility::Inherited,
                            ScrollPosition::default(),
                            Gen3dPrefabScrollPanel,
                        ))
                        .with_children(|scroll| {
                            scroll.spawn((
                                Text::new(""),
                                Node {
                                    width: Val::Percent(100.0),
                                    align_self: AlignSelf::FlexStart,
                                    ..default()
                                },
                                TextFont {
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.85, 0.85, 0.90)),
                                Visibility::Inherited,
                                Gen3dPrefabDetailsText,
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
                            Gen3dPrefabScrollbarTrack,
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
                                Gen3dPrefabScrollbarThumb,
                            ));
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
                                                scroll.spawn((
                                                    Node {
                                                        position_type: PositionType::Absolute,
                                                        left: Val::Px(0.0),
                                                        top: Val::Px(0.0),
                                                        width: Val::Px(2.0),
                                                        height: Val::Px(18.0),
                                                        ..default()
                                                    },
                                                    BackgroundColor(Color::srgba(
                                                        0.92, 0.98, 1.0, 0.0,
                                                    )),
                                                    ZIndex(50),
                                                    Visibility::Hidden,
                                                    Gen3dPromptCaret,
                                                ));
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
                                                                        Gen3dPromptHintText,
                                                                    ))
                                                                    .with_children(|_| {});
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
                                                BackgroundColor(Color::srgba(
                                                    0.02, 0.02, 0.03, 0.45,
                                                )),
                                                BorderColor::all(Color::srgba(
                                                    0.25, 0.25, 0.30, 0.65,
                                                )),
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
                                                    BackgroundColor(Color::srgba(
                                                        0.85, 0.88, 0.95, 0.85,
                                                    )),
                                                    Gen3dPromptScrollbarThumb,
                                                ));
                                            });
                                    });

                            });
                    });
                });

                bar.spawn((Node {
                    width: Val::Px(240.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(8.0),
                    ..default()
                },))
                    .with_children(|column| {
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
                                Visibility::Hidden,
                                Gen3dSaveButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Save Snapshot"),
                                    TextFont {
                                        font_size: 16.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.82, 0.90, 1.0)),
                                    Gen3dSaveButtonText,
                                ));
                            });

                        column
                            .spawn((
                                Button,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(38.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55)),
                                BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70)),
                                Visibility::Hidden,
                                Gen3dManualTweakButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Manual Tweak"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                    Gen3dManualTweakButtonText,
                                ));
                            });

                        column
                            .spawn((
                                Button,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(34.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.08, 0.14, 0.10, 0.80)),
                                BorderColor::all(Color::srgb(0.25, 0.80, 0.45)),
                                Visibility::Hidden,
                                Gen3dManualTweakSaveButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Save"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.70, 1.0, 0.82)),
                                    Gen3dManualTweakSaveButtonText,
                                ));
                            });

                        column
                            .spawn((
                                Button,
                                Node {
                                    width: Val::Percent(100.0),
                                    height: Val::Px(34.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    border: UiRect::all(Val::Px(1.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.16, 0.07, 0.06, 0.80)),
                                BorderColor::all(Color::srgb(0.85, 0.38, 0.30)),
                                Visibility::Hidden,
                                Gen3dCancelQueueButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Cancel queue"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(1.0, 0.86, 0.82)),
                                    Gen3dCancelQueueButtonText,
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
    task_queue: Res<Gen3dTaskQueue>,
    preview_export: Res<super::Gen3dPreviewExportRuntime>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut workshop: ResMut<Gen3dWorkshop>,
) {
    for entity in &roots {
        commands.entity(entity).try_despawn();
    }
    for entity in &viewer_roots {
        commands.entity(entity).try_despawn();
    }

    let any_running =
        job.is_running() || task_queue.running_session_id.is_some() || preview_export.is_running();
    if !any_running {
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
        preview_state.capture_root = None;
        preview_state.last_cursor = None;
        preview_state.collision_dirty = false;
        preview_state.ui_applied_session_id = None;
        preview_state.ui_applied_assembly_rev = None;
        preview_state.capture_applied_session_id = None;
        preview_state.capture_applied_assembly_rev = None;
        preview_state.draft_focus = Vec3::ZERO;
        preview_state.view_pan = Vec3::ZERO;
        preview_state.animation_channel = "idle".to_string();
        preview_state.animation_channels.clear();
        preview_state.animation_dropdown_open = false;
        preview_state.explode_components = false;
        preview_state.hovered_component = None;
    } else {
        // Keep the preview scene alive so Gen3D can keep rendering/reviewing in the background.
        preview_state.last_cursor = None;
        preview_state.animation_dropdown_open = false;
        preview_state.explode_components = false;
        preview_state.hovered_component = None;
    }
    workshop.image_viewer = None;
    workshop.prompt_scrollbar_drag = None;
}

pub(crate) fn gen3d_cleanup_preview_scene_when_idle(
    mut commands: Commands,
    job: Res<Gen3dAiJob>,
    task_queue: Res<Gen3dTaskQueue>,
    preview_export: Res<super::Gen3dPreviewExportRuntime>,
    preview_cameras: Query<Entity, With<Gen3dPreviewCamera>>,
    review_cameras: Query<Entity, With<Gen3dReviewCaptureCamera>>,
    preview_roots: Query<Entity, With<Gen3dPreviewSceneRoot>>,
    preview_lights: Query<Entity, With<Gen3dPreviewLight>>,
    mut preview_state: ResMut<Gen3dPreview>,
) {
    if job.is_running() || task_queue.running_session_id.is_some() || preview_export.is_running() {
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
    preview_state.capture_root = None;
    preview_state.last_cursor = None;
    preview_state.collision_dirty = false;
    preview_state.ui_applied_session_id = None;
    preview_state.ui_applied_assembly_rev = None;
    preview_state.capture_applied_session_id = None;
    preview_state.capture_applied_assembly_rev = None;
    preview_state.draft_focus = Vec3::ZERO;
    preview_state.view_pan = Vec3::ZERO;
    preview_state.animation_channel = "idle".to_string();
    preview_state.animation_channels.clear();
    preview_state.animation_dropdown_open = false;
    preview_state.explode_components = false;
    preview_state.hovered_component = None;
}

pub(crate) fn gen3d_prompt_box_focus(
    mut workshop: ResMut<Gen3dWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut prompt_boxes: Query<(&Interaction, &mut BackgroundColor), With<Gen3dPromptBox>>,
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

pub(crate) fn gen3d_prompt_defocus_on_click_outside(
    build_scene: Res<State<BuildScene>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    prompt_boxes: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dPromptBox>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if !workshop.prompt_focused {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let Some(cursor) = window.physical_cursor_position() else {
        return;
    };
    let Ok((node, transform)) = prompt_boxes.single() else {
        return;
    };
    if node.contains_point(*transform, cursor) {
        return;
    }

    workshop.prompt_focused = false;
    workshop.prompt_scrollbar_drag = None;
    window.ime_enabled = false;
}

pub(crate) fn gen3d_prompt_ime_position(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dPromptScrollPanel>>,
    rich_text: Query<Entity, With<Gen3dPromptRichText>>,
    hint_text: Query<Entity, With<Gen3dPromptHintText>>,
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
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if !workshop.prompt_focused {
        return;
    }
    let Ok((node, transform)) = panels.single() else {
        return;
    };
    let Ok(mut window) = windows.single_mut() else {
        return;
    };
    let prompt_empty = workshop.prompt.trim().is_empty();
    let rich_root = if prompt_empty {
        hint_text.iter().next()
    } else {
        rich_text.iter().next()
    };
    let anchor_x = if prompt_empty {
        ImeAnchorXPolicy::ContentLeft
    } else {
        ImeAnchorXPolicy::LineEnd
    };
    set_ime_position_for_rich_text(
        &mut window,
        node,
        *transform,
        rich_root,
        anchor_x,
        &children,
        &nodes,
    );
}

pub(crate) fn gen3d_prompt_input_indicator(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<Gen3dWorkshop>,
    time: Res<Time>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dPromptScrollPanel>>,
    rich_text: Query<Entity, With<Gen3dPromptRichText>>,
    hint_text: Query<Entity, With<Gen3dPromptHintText>>,
    children: Query<&Children>,
    nodes: Query<(
        &ComputedNode,
        &UiGlobalTransform,
        Option<&Text>,
        Option<&TextSpan>,
        Option<&ImageNode>,
        Option<&Visibility>,
    ), Without<Gen3dPromptCaret>>,
    mut carets: Query<(&mut Node, &mut BackgroundColor, &mut Visibility), With<Gen3dPromptCaret>>,
    mut blink_t: Local<f32>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let Ok((panel_node, panel_transform)) = panels.single() else {
        return;
    };
    let Ok((mut caret_node, mut caret_bg, mut caret_vis)) = carets.single_mut() else {
        return;
    };

    if !workshop.prompt_focused {
        *caret_vis = Visibility::Hidden;
        *blink_t = 0.0;
        return;
    }
    *caret_vis = Visibility::Inherited;

    let prompt_empty = workshop.prompt.trim().is_empty();
    let rich_root = if prompt_empty {
        hint_text.iter().next()
    } else {
        rich_text.iter().next()
    };
    let anchor_x = if prompt_empty {
        ImeAnchorXPolicy::ContentLeft
    } else {
        ImeAnchorXPolicy::LineEnd
    };

    let Some(anchor_px) = crate::ui::rich_text_anchor_px(
        panel_node,
        *panel_transform,
        rich_root,
        anchor_x,
        &children,
        &nodes,
    ) else {
        return;
    };

    let Some(local) = panel_transform
        .try_inverse()
        .map(|transform| transform.transform_point2(anchor_px))
    else {
        return;
    };

    const CARET_W_PX: f32 = 2.0;
    const CARET_H_PX: f32 = 18.0;

    let panel_scale = panel_node.inverse_scale_factor();
    let panel_w = panel_node.size.x.max(0.0) * panel_scale;
    let panel_h = panel_node.size.y.max(0.0) * panel_scale;

    let left = ((local.x + panel_node.size.x * 0.5) * panel_scale)
        .clamp(0.0, (panel_w - CARET_W_PX).max(0.0));
    let bottom = ((local.y + panel_node.size.y * 0.5) * panel_scale).clamp(0.0, panel_h.max(0.0));
    let top = (bottom - CARET_H_PX).clamp(0.0, (panel_h - CARET_H_PX).max(0.0));

    caret_node.left = Val::Px(left);
    caret_node.top = Val::Px(top);
    caret_node.width = Val::Px(CARET_W_PX);
    caret_node.height = Val::Px(CARET_H_PX);

    *blink_t = (*blink_t + time.delta_secs()).clamp(0.0, 10_000.0);
    let blink_on = (*blink_t % 1.0) < 0.55;
    let alpha = if blink_on { 0.95 } else { 0.0 };
    *caret_bg = BackgroundColor(Color::srgba(0.92, 0.98, 1.0, alpha));
}

pub(crate) fn gen3d_exit_button(
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dExitButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                next_build_scene.set(BuildScene::Realm);
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

pub(crate) fn gen3d_cancel_queue_button(
    build_scene: Res<State<BuildScene>>,
    mut task_queue: ResMut<Gen3dTaskQueue>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
            &mut Node,
        ),
        With<Gen3dCancelQueueButton>,
    >,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let queued = active_session_is_queued(&task_queue);
    let active_id = task_queue.active_session_id;

    for (interaction, mut bg, mut border, mut vis, mut node) in &mut buttons {
        if !queued {
            node.display = Display::None;
            *vis = Visibility::Hidden;
            *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
            *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));
            *last_interaction = None;
            continue;
        }

        node.display = Display::Flex;
        *vis = Visibility::Inherited;

        match *interaction {
            Interaction::Pressed => {
                *bg = BackgroundColor(Color::srgba(0.24, 0.11, 0.10, 0.96));
                *border = BorderColor::all(Color::srgb(1.0, 0.52, 0.40));

                if matches!(*last_interaction, Some(Interaction::Pressed)) {
                    continue;
                }

                task_queue.queue.retain(|id| *id != active_id);
                task_queue.set_task_state(active_id, Gen3dTaskState::Idle);
                workshop.error = None;
                workshop.status = "Queue canceled; click Build to run.".to_string();
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.20, 0.09, 0.08, 0.88));
                *border = BorderColor::all(Color::srgb(0.95, 0.45, 0.35));
            }
            Interaction::None => {
                *bg = BackgroundColor(Color::srgba(0.16, 0.07, 0.06, 0.80));
                *border = BorderColor::all(Color::srgb(0.85, 0.38, 0.30));
            }
        }

        *last_interaction = Some(*interaction);
    }
}

pub(crate) fn gen3d_exit_on_escape(
    build_scene: Res<State<BuildScene>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut tweak: ResMut<Gen3dManualTweakState>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }
    if workshop.image_viewer.is_some() {
        return;
    }
    if workshop.prompt_focused {
        return;
    }
    if tweak.color_picker_open {
        tweak.color_picker_open = false;
        tweak.color_picker_rgb_focused = false;
        workshop.error = None;
        workshop.status = "Color picker closed.".into();
        return;
    }
    if tweak.enabled {
        tweak.enabled = false;
        tweak.selected_part_id = None;
        workshop.error = None;
        workshop.status = "Manual tweak exited.".into();
        return;
    }
    next_build_scene.set(BuildScene::Realm);
}

pub(crate) fn gen3d_side_panel_toggle_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Gen3dSidePanelToggleButton>),
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if workshop.prompt_scrollbar_drag.is_none()
        && (mouse_just_pressed || *interaction == Interaction::Pressed)
    {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let accept_input = workshop.prompt_focused;
    if accept_input {
        if let Ok(mut window) = windows.single_mut() {
            window.ime_enabled = true;
        }
    }

    for event in ime_events.read() {
        if let Ime::Commit { value, .. } = event {
            if accept_input && !value.is_empty() {
                push_prompt_text(&mut workshop, value);
            }
        }
    }

    for event in keyboard.read() {
        if event.state != bevy::input::ButtonState::Pressed {
            continue;
        }
        if !accept_input {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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

pub(crate) fn gen3d_preview_explode_toggle_button(
    build_scene: Res<State<BuildScene>>,
    mut preview_state: ResMut<Gen3dPreview>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<Gen3dPreviewExplodeToggleButton>),
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            preview_state.explode_components = !preview_state.explode_components;
        }
        apply_gen3d_preview_explode_toggle_style(
            preview_state.explode_components,
            *interaction,
            &mut bg,
            &mut border,
        );
    }
}

pub(crate) fn gen3d_preview_export_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    export_dialog: Res<Gen3dPreviewExportDialogJob>,
    runtime: Res<super::Gen3dPreviewExportRuntime>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<Gen3dPreviewExportButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    for (interaction, mut bg, mut border) in &mut buttons {
        if matches!(*interaction, Interaction::Pressed) {
            if runtime.is_running() {
                workshop.status = "Preview export already running.".to_string();
            } else if gen3d_preview_export_dialog_pending(&export_dialog) {
                workshop.status = "Preview export folder dialog already open.".to_string();
            } else {
                let (tx, rx) = mpsc::channel();
                if let Ok(mut guard) = export_dialog.receiver.lock() {
                    *guard = Some(rx);
                }

                let initial_dir = gen3d_preview_export_dialog_initial_dir(&runtime);
                workshop.error = None;
                workshop.status = "Select preview export folder…".to_string();
                std::thread::spawn(move || {
                    let path = rfd::FileDialog::new()
                        .set_directory(initial_dir)
                        .pick_folder();
                    let _ = tx.send(path);
                });
            }
        }
        apply_gen3d_preview_export_button_style(
            runtime.is_running() || gen3d_preview_export_dialog_pending(&export_dialog),
            *interaction,
            &mut bg,
            &mut border,
        );
    }
}

pub(crate) fn gen3d_preview_export_dialog_poll(
    build_scene: Res<State<BuildScene>>,
    job: Res<Gen3dAiJob>,
    preview_state: Res<Gen3dPreview>,
    draft: Res<Gen3dDraft>,
    library: Res<ObjectLibrary>,
    mut workshop: ResMut<Gen3dWorkshop>,
    export_dialog: Res<Gen3dPreviewExportDialogJob>,
    mut runtime: ResMut<super::Gen3dPreviewExportRuntime>,
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
            workshop.error = Some("Preview export canceled: folder dialog failed.".to_string());
            workshop.status = "Preview export canceled: folder dialog failed.".to_string();
            return;
        }
    };

    let Some(path) = path else {
        workshop.error = None;
        workshop.status = "Preview export canceled.".to_string();
        return;
    };

    match super::request_gen3d_preview_export(
        &build_scene,
        &draft,
        &preview_state,
        &library,
        &mut runtime,
        super::Gen3dPreviewExportRequest {
            out_dir: Some(path),
            channels: Vec::new(),
            export_id: job
                .save_overwrite_prefab_id()
                .or(job.edit_base_prefab_id())
                .or(job.last_saved_prefab_id())
                .map(|id| uuid::Uuid::from_u128(id).to_string())
                .or_else(|| job.run_id().map(|id| id.to_string())),
        },
    ) {
        Ok(status) => {
            workshop.error = None;
            workshop.status = status.message.clone();
        }
        Err(err) => {
            workshop.error = Some(err.clone());
            workshop.status = format!("Preview export failed: {err}");
        }
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if channel == "attack" {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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
    if !super::gen3d_ui_scene(build_scene.get()) {
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

pub(crate) fn gen3d_update_preview_explode_toggle_ui(
    build_scene: Res<State<BuildScene>>,
    preview_state: Res<Gen3dPreview>,
    mut last_open: Local<Option<bool>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dPreviewExplodeToggleButton>,
    >,
    mut texts: Query<&mut Text, With<Gen3dPreviewExplodeToggleButtonText>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if last_open.as_ref() == Some(&preview_state.explode_components) && !preview_state.is_changed()
    {
        return;
    }
    *last_open = Some(preview_state.explode_components);

    let label = if preview_state.explode_components {
        "Explode On"
    } else {
        "Explode Off"
    };
    for mut text in &mut texts {
        **text = label.into();
    }
    for (interaction, mut bg, mut border) in &mut buttons {
        apply_gen3d_preview_explode_toggle_style(
            preview_state.explode_components,
            *interaction,
            &mut bg,
            &mut border,
        );
    }
}

pub(crate) fn gen3d_update_preview_export_button_ui(
    build_scene: Res<State<BuildScene>>,
    export_dialog: Res<Gen3dPreviewExportDialogJob>,
    runtime: Res<super::Gen3dPreviewExportRuntime>,
    mut last_phase: Local<Option<(bool, super::Gen3dPreviewExportPhase, usize, usize)>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        With<Gen3dPreviewExportButton>,
    >,
    mut texts: Query<&mut Text, With<Gen3dPreviewExportButtonText>>,
) {
    if !matches!(build_scene.get(), BuildScene::Preview) {
        return;
    }

    let dialog_pending = gen3d_preview_export_dialog_pending(&export_dialog);
    let phase_state = (
        dialog_pending,
        runtime.status.phase,
        runtime.status.completed_channels,
        runtime.status.total_channels,
    );
    if last_phase.as_ref() == Some(&phase_state) && !runtime.is_changed() {
        return;
    }
    *last_phase = Some(phase_state);

    let label = if dialog_pending {
        "Choose Folder…".to_string()
    } else {
        match runtime.status.phase {
            super::Gen3dPreviewExportPhase::Running => {
                if runtime.status.total_channels > 0 {
                    format!(
                        "Exporting {}/{}",
                        runtime.status.completed_channels.saturating_add(1),
                        runtime.status.total_channels
                    )
                } else {
                    "Exporting…".to_string()
                }
            }
            _ => "Export Preview".to_string(),
        }
    };
    for mut text in &mut texts {
        **text = label.clone().into();
    }
    let running = runtime.is_running() || dialog_pending;
    for (interaction, mut bg, mut border) in &mut buttons {
        apply_gen3d_preview_export_button_style(running, *interaction, &mut bg, &mut border);
    }
}

fn gen3d_preview_export_dialog_pending(dialog: &Gen3dPreviewExportDialogJob) -> bool {
    dialog
        .receiver
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|_| true))
        .unwrap_or(false)
}

fn gen3d_preview_export_dialog_initial_dir(runtime: &super::Gen3dPreviewExportRuntime) -> PathBuf {
    let fallback = crate::paths::default_cache_dir().join("gen3d_preview_exports");
    let _ = std::fs::create_dir_all(&fallback);

    runtime
        .status
        .out_dir
        .as_ref()
        .filter(|path| path.is_dir())
        .cloned()
        .or_else(|| {
            runtime
                .status
                .out_dir
                .as_ref()
                .and_then(|path| path.parent().map(|parent| parent.to_path_buf()))
        })
        .unwrap_or(fallback)
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

fn apply_gen3d_preview_explode_toggle_style(
    enabled: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
    border: &mut BorderColor,
) {
    let (mut bg_color, mut border_color) = if enabled {
        (
            Color::srgba(0.09, 0.14, 0.08, 0.88),
            Color::srgb(0.34, 0.86, 0.44),
        )
    } else {
        (
            Color::srgba(0.02, 0.02, 0.03, 0.70),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };

    match interaction {
        Interaction::Pressed => {
            bg_color = if enabled {
                Color::srgba(0.14, 0.22, 0.12, 0.96)
            } else {
                Color::srgba(0.10, 0.10, 0.12, 0.92)
            };
        }
        Interaction::Hovered => {
            bg_color = if enabled {
                Color::srgba(0.12, 0.18, 0.10, 0.92)
            } else {
                Color::srgba(0.06, 0.06, 0.08, 0.86)
            };
            border_color = if enabled {
                Color::srgb(0.40, 0.90, 0.50)
            } else {
                Color::srgba(0.35, 0.35, 0.40, 0.70)
            };
        }
        Interaction::None => {}
    }

    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

fn apply_gen3d_preview_export_button_style(
    running: bool,
    interaction: Interaction,
    bg: &mut BackgroundColor,
    border: &mut BorderColor,
) {
    let (mut bg_color, mut border_color) = if running {
        (
            Color::srgba(0.10, 0.12, 0.15, 0.88),
            Color::srgb(0.48, 0.72, 0.90),
        )
    } else {
        (
            Color::srgba(0.02, 0.02, 0.03, 0.70),
            Color::srgba(0.25, 0.25, 0.30, 0.65),
        )
    };

    match interaction {
        Interaction::Pressed if !running => {
            bg_color = Color::srgba(0.10, 0.10, 0.12, 0.92);
        }
        Interaction::Hovered if !running => {
            bg_color = Color::srgba(0.06, 0.06, 0.08, 0.86);
            border_color = Color::srgba(0.35, 0.35, 0.40, 0.70);
        }
        _ => {}
    }

    *bg = BackgroundColor(bg_color);
    *border = BorderColor::all(border_color);
}

#[derive(SystemParam)]
pub(crate) struct Gen3dUpdateUiTextDeps<'w, 's> {
    commands: Commands<'w, 's>,
    workshop: Res<'w, Gen3dWorkshop>,
    preview_state: Res<'w, Gen3dPreview>,
    preview_export: Res<'w, super::Gen3dPreviewExportRuntime>,
    draft: Res<'w, Gen3dDraft>,
    job: Res<'w, Gen3dAiJob>,
    task_queue: Res<'w, Gen3dTaskQueue>,
    ui_fonts: Res<'w, UiFonts>,
    emoji_atlas: Res<'w, EmojiAtlas>,
    asset_server: Res<'w, AssetServer>,
    scroll_panels: ParamSet<
        'w,
        's,
        (
            Query<
                'w,
                's,
                (&'static ComputedNode, &'static mut ScrollPosition),
                With<Gen3dPromptScrollPanel>,
            >,
            Query<
                'w,
                's,
                (&'static ComputedNode, &'static mut ScrollPosition),
                With<Gen3dStatusScrollPanel>,
            >,
        ),
    >,
    texts: ParamSet<
        'w,
        's,
        (
            Query<'w, 's, &'static mut Text, With<Gen3dStatusText>>,
            Query<'w, 's, &'static mut Text, With<Gen3dStatusLogsText>>,
            Query<'w, 's, &'static mut Text, With<Gen3dGenerateButtonText>>,
            Query<'w, 's, &'static mut Text, With<Gen3dPreviewStatsText>>,
        ),
    >,
    rich_text: Query<'w, 's, Entity, With<Gen3dPromptRichText>>,
    hint_text: Query<'w, 's, Entity, With<Gen3dPromptHintText>>,
    prompt_nodes:
        Query<'w, 's, &'static mut Node, (With<Gen3dPromptRichText>, Without<Gen3dPromptHintText>)>,
    hint_nodes:
        Query<'w, 's, &'static mut Node, (With<Gen3dPromptHintText>, Without<Gen3dPromptRichText>)>,
}

pub(crate) fn gen3d_update_ui_text(
    build_scene: Res<State<BuildScene>>,
    deps: Gen3dUpdateUiTextDeps,
    mut last_prompt: Local<Option<String>>,
    mut last_prompt_entity: Local<Option<Entity>>,
    mut last_hint: Local<Option<String>>,
    mut last_hint_entity: Local<Option<Entity>>,
    mut autoscroll_frames: Local<u8>,
    mut last_status_log_rev: Local<Option<(usize, Option<u32>)>>,
    mut status_log_autoscroll_frames: Local<u8>,
    mut status_log_follow_tail: Local<bool>,
) {
    let Gen3dUpdateUiTextDeps {
        mut commands,
        workshop,
        preview_state,
        preview_export,
        draft,
        job,
        task_queue,
        ui_fonts,
        emoji_atlas,
        asset_server,
        mut scroll_panels,
        mut texts,
        rich_text,
        hint_text,
        mut prompt_nodes,
        mut hint_nodes,
    } = deps;

    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let prompt_empty = workshop.prompt.trim().is_empty();
    let hint_text_value = if job.edit_base_prefab_id().is_some() {
        "Want to improve it? Tell me!".to_string()
    } else {
        "Anything you want to create? Tell me!".to_string()
    };
    let prompt_text = workshop.prompt.clone();

    let prompt_entity = rich_text.single().ok();
    if prompt_entity != *last_prompt_entity {
        *last_prompt_entity = prompt_entity;
        // The prompt UI can be despawned/recreated (e.g. switching Preview ↔ Realm).
        // Force a re-render so the rich text starts with the correct current prompt.
        *last_prompt = None;
    }
    let hint_entity = hint_text.single().ok();
    if hint_entity != *last_hint_entity {
        *last_hint_entity = hint_entity;
        *last_hint = None;
    }

    if let Ok(mut node) = prompt_nodes.single_mut() {
        node.display = if prompt_empty {
            Display::None
        } else {
            Display::Flex
        };
    }
    if let Ok(mut node) = hint_nodes.single_mut() {
        node.display = if prompt_empty {
            Display::Flex
        } else {
            Display::None
        };
    }

    if prompt_empty {
        if let Some(entity) = hint_entity {
            let hint_changed = last_hint.as_ref() != Some(&hint_text_value);
            if hint_changed {
                set_rich_text_line(
                    &mut commands,
                    entity,
                    &hint_text_value,
                    &ui_fonts,
                    &emoji_atlas,
                    &asset_server,
                    16.0,
                    Color::srgb(0.70, 0.70, 0.74),
                    None,
                );
                *last_hint = Some(hint_text_value);
            }
        }
        *autoscroll_frames = 0;
    } else {
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
    }

    if *autoscroll_frames > 0 && workshop.prompt_scrollbar_drag.is_none() {
        if let Ok((node, mut scroll)) = scroll_panels.p0().single_mut() {
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

    fn truncate_ellipsis(text: &str, max_chars: usize) -> String {
        let text = text.trim();
        if text.chars().count() <= max_chars {
            return text.to_string();
        }
        let mut out: String = text.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }

    let components = draft.component_count();
    let parts = draft.total_primitive_parts();
    let motions = preview_state.animation_channels.len();

    let queued = active_session_is_queued(&task_queue);
    let queued_state = if queued {
        active_session_queue_position(&task_queue)
            .map(|(pos, total)| {
                if total > 0 {
                    format!("Queued (position {pos} of {total})")
                } else {
                    "Queued".to_string()
                }
            })
            .unwrap_or_else(|| "Queued".to_string())
    } else {
        String::new()
    };
    let state = if workshop.error.is_some() {
        "Error".to_string()
    } else if queued {
        queued_state
    } else if job.is_running() {
        "Building".to_string()
    } else if job.is_build_complete() {
        "Done".to_string()
    } else if job.can_resume() {
        "Stopped".to_string()
    } else {
        "Idle".to_string()
    };

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

    let run_input_tokens = format_compact_count(job.current_run_input_tokens());
    let run_output_tokens = format_compact_count(job.current_run_output_tokens());
    let run_unsplit_tokens_raw = job.current_run_unsplit_tokens();
    let run_unsplit_tokens = format_compact_count(run_unsplit_tokens_raw);
    let total_input_tokens = format_compact_count(job.total_input_tokens());
    let total_output_tokens = format_compact_count(job.total_output_tokens());
    let total_unsplit_tokens_raw = job.total_unsplit_tokens();
    let total_unsplit_tokens = format_compact_count(total_unsplit_tokens_raw);

    let run_tokens_summary = if run_unsplit_tokens_raw > 0 {
        format!("in {run_input_tokens} out {run_output_tokens} unk {run_unsplit_tokens}")
    } else {
        format!("in {run_input_tokens} out {run_output_tokens}")
    };
    let total_tokens_summary = if total_unsplit_tokens_raw > 0 {
        format!("in {total_input_tokens} out {total_output_tokens} unk {total_unsplit_tokens}")
    } else {
        format!("in {total_input_tokens} out {total_output_tokens}")
    };
    let pipeline_status = job
        .pipeline_progress()
        .map(|progress| {
            format!(
                "{}/{} | {}",
                progress.current, progress.total, progress.label
            )
        })
        .unwrap_or_else(|| "—".to_string());
    let reuse_counts = job.reuse_counts();
    let task_counts_summary = job
        .active_parallel_task_counts()
        .map(format_parallel_task_counts);

    let mut step_status = "—".to_string();
    if let Some(active) = workshop.status_log.active.as_ref() {
        let step = truncate_ellipsis(active.step.as_str(), 46);
        let elapsed = workshop
            .status_log
            .active_elapsed()
            .unwrap_or_else(|| std::time::Duration::from_secs(0));
        if let Some(tasks) = task_counts_summary.as_deref() {
            step_status = format!("{step} ({tasks}; running {})", format_duration(elapsed));
        } else {
            step_status = format!("{step} (running {})", format_duration(elapsed));
        }
    } else if let Some(last) = workshop.status_log.entries.last() {
        let step = truncate_ellipsis(last.step.as_str(), 36);
        let result = truncate_ellipsis(last.result.as_str(), 26);
        step_status = format!(
            "{step} → {result} ({})",
            format_duration_ms(last.duration_ms)
        );
    }

    let prefab_status = if job.last_saved_prefab_id().is_some() {
        "Saved"
    } else {
        "—"
    };

    let status_summary = format!(
        "State: {state} | Prefab: {prefab_status}\n\
	Draft: comps {components} | parts {parts} | motion {motions}\n\
	Run: attempt {} | step {} | time {run_time}\n\
	Pipeline: {pipeline_status}\n\
	Reuse: copied {} | mirrored {}\n\
	Tokens: run {run_tokens_summary} | total {total_tokens_summary}\n\
	Step: {step_status}",
        job.attempt() + 1,
        job.step() + 1,
        reuse_counts.copied,
        reuse_counts.mirrored,
    );
    {
        let mut status = texts.p0();
        for mut text in &mut status {
            **text = status_summary.clone();
        }
    }

    let log_rev = (
        workshop.status_log.entries.len(),
        workshop.status_log.active.as_ref().map(|s| s.seq),
    );
    if *last_status_log_rev != Some(log_rev) {
        *last_status_log_rev = Some(log_rev);
        if *status_log_follow_tail {
            *status_log_autoscroll_frames = 3;
        }
    }

    let mut logs_text = String::new();
    if workshop.status_log.entries.is_empty() && workshop.status_log.active.is_none() {
        logs_text.push_str("No logs yet.\n");
    } else {
        for entry in &workshop.status_log.entries {
            logs_text.push_str(&format!(
                "[{:03}] {} — {} → {} ({})\n",
                entry.seq,
                entry.step.trim(),
                entry.why.trim(),
                entry.result.trim(),
                format_duration_ms(entry.duration_ms)
            ));
        }
        if let Some(active) = workshop.status_log.active.as_ref() {
            let elapsed = workshop
                .status_log
                .active_elapsed()
                .unwrap_or_else(|| std::time::Duration::from_secs(0));
            if let Some(tasks) = task_counts_summary.as_deref() {
                logs_text.push_str(&format!(
                    "[{:03}] {} — {} | {} → running… ({})\n",
                    active.seq,
                    active.step.trim(),
                    active.why.trim(),
                    tasks,
                    format_duration(elapsed)
                ));
            } else {
                logs_text.push_str(&format!(
                    "[{:03}] {} — {} → running… ({})\n",
                    active.seq,
                    active.step.trim(),
                    active.why.trim(),
                    format_duration(elapsed)
                ));
            }
        }
    }
    {
        let mut logs = texts.p1();
        for mut text in &mut logs {
            **text = logs_text.clone();
        }
    }

    if *status_log_autoscroll_frames > 0 {
        if let Ok((node, mut scroll)) = scroll_panels.p1().single_mut() {
            let panel_scale = node.inverse_scale_factor();
            let viewport_h = node.size.y.max(0.0) * panel_scale;
            let content_h = node.content_size.y.max(0.0) * panel_scale;
            if viewport_h >= 1.0 && content_h > viewport_h + 0.5 {
                let max_scroll = (content_h - viewport_h).max(0.0);
                scroll.y = max_scroll;
                *status_log_autoscroll_frames = (*status_log_autoscroll_frames).saturating_sub(1);
            } else {
                scroll.y = 0.0;
                *status_log_autoscroll_frames = 0;
            }

            let max_scroll = (content_h - viewport_h).max(0.0);
            *status_log_follow_tail = max_scroll <= 1.0 || (max_scroll - scroll.y) <= 24.0;
        } else {
            *status_log_autoscroll_frames = 0;
        }
    } else if let Ok((node, scroll)) = scroll_panels.p1().single() {
        let panel_scale = node.inverse_scale_factor();
        let viewport_h = node.size.y.max(0.0) * panel_scale;
        let content_h = node.content_size.y.max(0.0) * panel_scale;
        let max_scroll = (content_h - viewport_h).max(0.0);
        *status_log_follow_tail = max_scroll <= 1.0 || (max_scroll - scroll.y) <= 24.0;
    }

    let label = if queued {
        "Queued"
    } else if job.is_running() {
        "Stop"
    } else if job.edit_base_prefab_id().is_some() {
        "Edit"
    } else {
        "Build"
    };
    {
        let mut button = texts.p2();
        for mut text in &mut button {
            **text = label.into();
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
    let run_input_tokens = format_compact_count(job.current_run_input_tokens());
    let run_output_tokens = format_compact_count(job.current_run_output_tokens());
    let run_unsplit_tokens_raw = job.current_run_unsplit_tokens();
    let run_unsplit_tokens = format_compact_count(run_unsplit_tokens_raw);
    let total_input_tokens = format_compact_count(job.total_input_tokens());
    let total_output_tokens = format_compact_count(job.total_output_tokens());
    let total_unsplit_tokens_raw = job.total_unsplit_tokens();
    let total_unsplit_tokens = format_compact_count(total_unsplit_tokens_raw);
    let run_tokens_line = if run_unsplit_tokens_raw > 0 {
        format!("in {run_input_tokens} | out {run_output_tokens} | unk {run_unsplit_tokens}")
    } else {
        format!("in {run_input_tokens} | out {run_output_tokens}")
    };
    let total_tokens_line = if total_unsplit_tokens_raw > 0 {
        format!("in {total_input_tokens} | out {total_output_tokens} | unk {total_unsplit_tokens}")
    } else {
        format!("in {total_input_tokens} | out {total_output_tokens}")
    };
    let state = if workshop.error.is_some() {
        "Error".to_string()
    } else if active_session_is_queued(&task_queue) {
        "Queued".to_string()
    } else if job.is_running() {
        "Running".to_string()
    } else if job.is_build_complete() {
        "Done ✓".to_string()
    } else if job.can_resume() {
        "Stopped".to_string()
    } else {
        "Idle".to_string()
    };
    let export_line = match preview_export.status.phase {
        super::Gen3dPreviewExportPhase::Idle => "Export: idle".to_string(),
        super::Gen3dPreviewExportPhase::Running => format!(
            "Export: {}/{} {}",
            preview_export.status.completed_channels.saturating_add(1),
            preview_export.status.total_channels.max(1),
            preview_export
                .status
                .current_channel
                .as_deref()
                .unwrap_or("preview")
        ),
        super::Gen3dPreviewExportPhase::Completed => preview_export
            .status
            .out_dir
            .as_ref()
            .map(|path| {
                format!(
                    "Export: done → {}",
                    truncate_ellipsis(&path.display().to_string(), 28)
                )
            })
            .unwrap_or_else(|| "Export: done".to_string()),
        super::Gen3dPreviewExportPhase::Failed => preview_export
            .status
            .error
            .as_deref()
            .map(|err| format!("Export: error → {}", truncate_ellipsis(err, 28)))
            .unwrap_or_else(|| "Export: error".to_string()),
    };
    let stats_text = format!(
        "State: {state}\nRun time: {run_time}\nTokens (run): {run_tokens_line}\nTokens (total): {total_tokens_line}\n{export_line}",
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

fn format_duration_ms(ms: u128) -> String {
    format_duration(std::time::Duration::from_millis(
        ms.min(u128::from(u64::MAX)) as u64,
    ))
}

fn format_parallel_task_counts(counts: crate::gen3d::ai::Gen3dParallelTaskCounts) -> String {
    format!(
        "tasks: running {} | queued {} | total {}",
        counts.running, counts.queued, counts.total
    )
}

fn format_duration(d: std::time::Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{:.1}s", d.as_secs_f32())
    } else if secs < 60 * 60 {
        format!("{}m {}s", secs / 60, secs % 60)
    } else {
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        format!("{hours}h {mins}m")
    }
}
