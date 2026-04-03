use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::{Ime, PrimaryWindow};

use crate::config::AppConfig;
use crate::gen3d::{Gen3dAiJob, Gen3dDraft, Gen3dTaskQueue, Gen3dTaskState, Gen3dWorkshop};
use crate::genfloor::GenFloorAiJob;
use crate::rich_text::set_rich_text_line;
use crate::types::{BuildScene, EmojiAtlas, GameMode, UiFonts};
use crate::ui::{set_ime_position_for_rich_text, ImeAnchorXPolicy};
use crate::workspace_scenes_ui::ScenesPanelUiState;

use super::job::{gen_scene_cancel_job, gen_scene_request_build};
use super::state::*;

const GEN_SCENE_PREVIEW_WIDTH_PX: u32 = 960;
const GEN_SCENE_PREVIEW_HEIGHT_PX: u32 = 540;
const GEN_SCENE_PROMPT_BAR_HEIGHT_PX: f32 = 160.0;
const GEN_SCENE_MAX_COMPONENT_LABELS: usize = 64;

const GEN_SCENE_PREVIEW_YAW: f32 = std::f32::consts::FRAC_PI_6;
const GEN_SCENE_PREVIEW_PITCH: f32 = -0.45;

#[derive(Component)]
pub(crate) struct GenSceneWorkshopRoot;
#[derive(Component)]
pub(crate) struct GenSceneExitButton;
#[derive(Component)]
pub(crate) struct GenScenePreviewPanel;
#[derive(Component)]
pub(crate) struct GenScenePreviewPanelImage;
#[derive(Component)]
pub(crate) struct GenScenePreviewOverlayRoot;
#[derive(Component)]
pub(crate) struct GenScenePreviewHoverFrame;
#[derive(Component)]
pub(crate) struct GenScenePreviewHoverInfoCard;
#[derive(Component)]
pub(crate) struct GenScenePreviewHoverInfoText;
#[derive(Component)]
pub(crate) struct GenScenePreviewComponentLabelsRoot;
#[derive(Component)]
pub(crate) struct GenScenePreviewComponentLabel {
    index: usize,
}

impl GenScenePreviewComponentLabel {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct GenScenePreviewComponentLabelText {
    index: usize,
}

impl GenScenePreviewComponentLabelText {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct GenScenePreviewStatsText;
#[derive(Component)]
pub(crate) struct GenScenePreviewAnimationDropdownButton;
#[derive(Component)]
pub(crate) struct GenScenePreviewAnimationDropdownButtonText;
#[derive(Component)]
pub(crate) struct GenScenePreviewAnimationDropdownList;
#[derive(Component)]
pub(crate) struct GenScenePreviewAnimationOptionButton {
    index: usize,
}

impl GenScenePreviewAnimationOptionButton {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct GenScenePreviewAnimationOptionButtonText {
    index: usize,
}

impl GenScenePreviewAnimationOptionButtonText {
    pub(crate) fn new(index: usize) -> Self {
        Self { index }
    }

    pub(crate) fn index(&self) -> usize {
        self.index
    }
}

#[derive(Component)]
pub(crate) struct GenScenePreviewExplodeToggleButton;
#[derive(Component)]
pub(crate) struct GenScenePreviewExplodeToggleButtonText;
#[derive(Component)]
pub(crate) struct GenScenePreviewExportButton;
#[derive(Component)]
pub(crate) struct GenScenePreviewExportButtonText;
#[derive(Component)]
pub(crate) struct GenSceneSidePanelToggleButton;
#[derive(Component)]
pub(crate) struct GenSceneSidePanelToggleButtonText;
#[derive(Component)]
pub(crate) struct GenSceneSidePanelRoot;
#[derive(Component)]
pub(crate) struct GenSceneStatusPanelRoot;
#[derive(Component)]
pub(crate) struct GenSceneStatusText;
#[derive(Component)]
pub(crate) struct GenSceneStatusScrollPanel;
#[derive(Component)]
pub(crate) struct GenSceneStatusLogsText;
#[derive(Component)]
pub(crate) struct GenSceneStatusScrollbarTrack;
#[derive(Component)]
pub(crate) struct GenSceneStatusScrollbarThumb;
#[derive(Component)]
pub(crate) struct GenScenePrefabPanelRoot;
#[derive(Component)]
pub(crate) struct GenScenePrefabScrollPanel;
#[derive(Component)]
pub(crate) struct GenScenePrefabDetailsText;
#[derive(Component)]
pub(crate) struct GenScenePrefabScrollbarTrack;
#[derive(Component)]
pub(crate) struct GenScenePrefabScrollbarThumb;
#[derive(Component)]
pub(crate) struct GenScenePromptBox;
#[derive(Component)]
pub(crate) struct GenScenePromptScrollPanel;
#[derive(Component)]
pub(crate) struct GenScenePromptHintText;
#[derive(Component)]
pub(crate) struct GenScenePromptRichText;
#[derive(Component)]
pub(crate) struct GenScenePromptScrollbarTrack;
#[derive(Component)]
pub(crate) struct GenScenePromptScrollbarThumb;
#[derive(Component)]
pub(crate) struct GenSceneGenerateButton;
#[derive(Component)]
pub(crate) struct GenSceneGenerateButtonText;
#[derive(Component)]
pub(crate) struct GenSceneSaveButton;
#[derive(Component)]
pub(crate) struct GenSceneSaveButtonText;
#[derive(Component)]
pub(crate) struct GenSceneStopButton;
#[derive(Component)]
pub(crate) struct GenSceneStopButtonText;
#[derive(Component)]
pub(crate) struct GenScenePreviewCamera;
#[derive(Component)]
pub(crate) struct GenSceneImagesInlinePanel;
#[derive(Component)]
pub(crate) struct GenSceneImagesList;
#[derive(Component)]
pub(crate) struct GenSceneClearImagesButton;
#[derive(Component)]
pub(crate) struct GenSceneClearImagesButtonText;
#[derive(Component)]
pub(crate) struct GenSceneThumbnailTooltipRoot;
#[derive(Component)]
pub(crate) struct GenSceneThumbnailTooltipText;

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

pub(crate) fn enter_gen_scene_mode(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut preview: ResMut<GenScenePreview>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut job: ResMut<GenSceneJob>,
) {
    *job = GenSceneJob::default();

    let target = crate::orbit_capture::create_render_target(
        &mut images,
        GEN_SCENE_PREVIEW_WIDTH_PX,
        GEN_SCENE_PREVIEW_HEIGHT_PX,
    );

    preview.target = Some(target.clone());
    preview.focus = Vec3::ZERO;
    preview.half_extents = Vec3::new(20.0, 6.0, 20.0);
    preview.dirty = true;
    preview.active = false;

    let aspect = GEN_SCENE_PREVIEW_WIDTH_PX.max(1) as f32
        / GEN_SCENE_PREVIEW_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;
    let fov_y = projection.fov;
    let near = projection.near;
    let distance = crate::orbit_capture::required_distance_for_view(
        preview.half_extents,
        GEN_SCENE_PREVIEW_YAW,
        GEN_SCENE_PREVIEW_PITCH,
        fov_y,
        aspect,
        near,
    )
    .clamp(near + 0.2, 500.0);
    let camera_transform = crate::orbit_capture::orbit_transform(
        GEN_SCENE_PREVIEW_YAW,
        GEN_SCENE_PREVIEW_PITCH,
        distance,
        preview.focus,
    );

    let camera = commands
        .spawn((
            Camera3d::default(),
            bevy::camera::Projection::Perspective(projection),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                is_active: false,
                ..default()
            },
            bevy::camera::RenderTarget::Image(target.clone().into()),
            bevy::core_pipeline::tonemapping::Tonemapping::TonyMcMapface,
            camera_transform,
            GenScenePreviewCamera,
        ))
        .id();

    preview.camera = Some(camera);

    spawn_gen_scene_ui(&mut commands, target);

    workshop.open = true;
    workshop.prompt.clear();
    workshop.prompt_focused = false;
    workshop.status.clear();
    workshop.error = None;
    workshop.running = false;
    workshop.close_locked = false;
    workshop.run_id = None;
    workshop.active_scene_id = None;
    workshop.prompt_scrollbar_drag = None;
    workshop.side_panel_open = false;
    workshop.side_tab = GenSceneSideTab::Status;
}

pub(crate) fn exit_gen_scene_mode(
    mut commands: Commands,
    roots: Query<Entity, With<GenSceneWorkshopRoot>>,
    cameras: Query<Entity, With<GenScenePreviewCamera>>,
    mut preview: ResMut<GenScenePreview>,
    mut workshop: ResMut<GenSceneWorkshop>,
) {
    for entity in &roots {
        commands.entity(entity).try_despawn();
    }
    for entity in &cameras {
        commands.entity(entity).try_despawn();
    }

    if let Some(target) = preview.target.take() {
        let id = target.id();
        commands.queue(move |world: &mut World| {
            if let Some(mut images) = world.get_resource_mut::<Assets<Image>>() {
                images.remove(id);
            }
        });
    }

    preview.camera = None;
    preview.focus = Vec3::ZERO;
    preview.half_extents = Vec3::ZERO;
    preview.dirty = false;
    preview.active = false;

    workshop.open = false;
    workshop.prompt_focused = false;
    workshop.prompt_scrollbar_drag = None;
    workshop.side_panel_open = false;
    workshop.side_tab = GenSceneSideTab::Status;
}

fn spawn_gen_scene_preview_panel<F>(
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
            GenScenePreviewPanel,
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
                GenScenePreviewPanelImage,
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
                    GenScenePreviewOverlayRoot,
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
                            GenScenePreviewHoverFrame,
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

                            // Corner overhangs to frame the object neatly.
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
                            GenScenePreviewHoverInfoCard,
                        ))
                        .with_children(|card| {
                            card.spawn((
                                Text::new(""),
                                TextFont {
                                    font_size: 13.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.92, 0.94, 0.97)),
                                GenScenePreviewHoverInfoText,
                            ));
                        });
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
                            GenScenePreviewComponentLabelsRoot,
                        ))
                        .with_children(|labels| {
                            for index in 0..GEN_SCENE_MAX_COMPONENT_LABELS {
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
                                        GenScenePreviewComponentLabel::new(index),
                                    ))
                                    .with_children(|label| {
                                        label.spawn((
                                            Text::new(""),
                                            TextFont {
                                                font_size: 12.0,
                                                ..default()
                                            },
                                            TextColor(Color::srgb(0.94, 0.94, 0.96)),
                                            GenScenePreviewComponentLabelText::new(index),
                                        ));
                                    });
                            }
                        });
                });

            extra_children(preview);
        })
        .id()
}

fn spawn_gen_scene_ui(commands: &mut Commands, target: Handle<Image>) {
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
            GenSceneWorkshopRoot,
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
                GenSceneExitButton,
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
                    spawn_gen_scene_preview_panel(
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
                                        GenScenePreviewStatsText,
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
                                                        GenScenePreviewAnimationDropdownButton,
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
                                                            GenScenePreviewAnimationDropdownButtonText,
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
                                                        GenScenePreviewAnimationDropdownList,
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
                                                GenScenePreviewExplodeToggleButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Explode Off"),
                                                    TextFont {
                                                        font_size: 13.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    GenScenePreviewExplodeToggleButtonText,
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
                                                GenScenePreviewExportButton,
                                            ))
                                            .with_children(|button| {
                                                button.spawn((
                                                    Text::new("Export Preview"),
                                                    TextFont {
                                                        font_size: 13.0,
                                                        ..default()
                                                    },
                                                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                                                    GenScenePreviewExportButtonText,
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
                GenSceneSidePanelToggleButton,
            ))
            .with_children(|button| {
                button.spawn((
                    Text::new("≡"),
                    TextFont {
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    GenSceneSidePanelToggleButtonText,
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
                GenSceneSidePanelRoot,
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
                            GenSceneSideTabButton::new(GenSceneSideTab::Status),
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
                                GenSceneSideTabButtonText::new(GenSceneSideTab::Status),
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
                            GenSceneSideTabButton::new(GenSceneSideTab::Prefab),
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
                                GenSceneSideTabButtonText::new(GenSceneSideTab::Prefab),
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
                        GenSceneStatusPanelRoot,
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
                                GenSceneStatusText,
                            ));
                        });

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
                                GenSceneStatusScrollPanel,
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
                                    GenSceneStatusLogsText,
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
                                GenSceneStatusScrollbarTrack,
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
                                    GenSceneStatusScrollbarThumb,
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
                        GenScenePrefabPanelRoot,
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
                            GenScenePrefabScrollPanel,
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
                                GenScenePrefabDetailsText,
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
                            GenScenePrefabScrollbarTrack,
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
                                GenScenePrefabScrollbarThumb,
                            ));
                        });
                    });
            });

            // Bottom: prompt + generate + status.
            root.spawn((
                Node {
                    height: Val::Px(GEN_SCENE_PROMPT_BAR_HEIGHT_PX),
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
                        GenScenePromptBox,
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
                                                GenScenePromptScrollPanel,
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
                                                                        GenScenePromptHintText,
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
                                                                        GenScenePromptRichText,
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
                                                                GenSceneImagesInlinePanel,
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
                                                                                BackgroundColor(Color::srgba(
                                                                                    0.02, 0.02, 0.03, 0.65,
                                                                                )),
                                                                                BorderColor::all(Color::srgba(
                                                                                    0.25, 0.25, 0.30, 0.65,
                                                                                )),
                                                                                GenSceneClearImagesButton,
                                                                            ))
                                                                            .with_children(|button| {
                                                                                button.spawn((
                                                                                    Text::new("Clear"),
                                                                                    TextFont {
                                                                                        font_size: 12.0,
                                                                                        ..default()
                                                                                    },
                                                                                    TextColor(Color::srgb(
                                                                                        0.92, 0.92, 0.96,
                                                                                    )),
                                                                                    GenSceneClearImagesButtonText,
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
                                                                    GenSceneImagesList,
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
                                                GenScenePromptScrollbarTrack,
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
                                                    GenScenePromptScrollbarThumb,
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
                                GenSceneGenerateButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Build"),
                                    TextFont {
                                        font_size: 18.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.70, 1.0, 0.82)),
                                    GenSceneGenerateButtonText,
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
                                GenSceneSaveButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Save Snapshot"),
                                    TextFont {
                                        font_size: 16.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(0.82, 0.90, 1.0)),
                                    GenSceneSaveButtonText,
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
                                GenSceneStopButton,
                            ))
                            .with_children(|button| {
                                button.spawn((
                                    Text::new("Stop"),
                                    TextFont {
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(Color::srgb(1.0, 0.86, 0.82)),
                                    GenSceneStopButtonText,
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
                GenSceneThumbnailTooltipRoot,
            ))
            .with_children(|tip| {
                tip.spawn((
                    Text::new(""),
                    TextFont {
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.92, 0.92, 0.96)),
                    GenSceneThumbnailTooltipText,
                ));
            });
        });
}


pub(crate) fn gen_scene_exit_button(
    build_scene: Res<State<BuildScene>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    job: Res<GenSceneJob>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<GenSceneExitButton>)>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }
    for (interaction, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                if job.running || workshop.close_locked {
                    workshop.status = "Build running: press Stop to close.".to_string();
                } else {
                    next_build_scene.set(BuildScene::Realm);
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

pub(crate) fn gen_scene_exit_on_escape(
    build_scene: Res<State<BuildScene>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    job: Res<GenSceneJob>,
    mut workshop: ResMut<GenSceneWorkshop>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }

    if job.running || workshop.close_locked {
        workshop.status = "Build running: press Stop to close.".to_string();
        return;
    }

    next_build_scene.set(BuildScene::Realm);
}

pub(crate) fn gen_scene_side_panel_toggle_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<GenSceneSidePanelToggleButton>),
    >,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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

pub(crate) fn gen_scene_update_side_panel_ui(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<GenSceneWorkshop>,
    mut panels: Query<(&mut Node, &mut Visibility), With<GenSceneSidePanelRoot>>,
    mut texts: Query<&mut Text, With<GenSceneSidePanelToggleButtonText>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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

pub(crate) fn gen_scene_side_tab_buttons(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut buttons: Query<
        (&Interaction, &GenSceneSideTabButton, &mut BackgroundColor),
        Changed<Interaction>,
    >,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    for (interaction, button, mut bg) in &mut buttons {
        match *interaction {
            Interaction::Pressed => {
                workshop.side_tab = button.tab();
                *bg = BackgroundColor(Color::srgba(0.08, 0.12, 0.16, 0.92));
            }
            Interaction::Hovered => {
                *bg = BackgroundColor(Color::srgba(0.04, 0.04, 0.05, 0.78));
            }
            Interaction::None => {}
        }
    }
}

pub(crate) fn gen_scene_update_side_tab_ui(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<GenSceneWorkshop>,
    mut panels: ParamSet<(
        Query<(&mut Node, &mut Visibility), With<GenSceneStatusPanelRoot>>,
        Query<(&mut Node, &mut Visibility), With<GenScenePrefabPanelRoot>>,
    )>,
    mut buttons: Query<(&GenSceneSideTabButton, &Interaction, &mut BackgroundColor)>,
    mut texts: Query<(&GenSceneSideTabButtonText, &mut Text)>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    for (mut node, mut vis) in panels.p0().iter_mut() {
        let active = matches!(workshop.side_tab, GenSceneSideTab::Status);
        node.display = if active { Display::Flex } else { Display::None };
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    for (mut node, mut vis) in panels.p1().iter_mut() {
        let active = matches!(workshop.side_tab, GenSceneSideTab::Prefab);
        node.display = if active { Display::Flex } else { Display::None };
        *vis = if active {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }

    for (tab, interaction, mut bg) in buttons.iter_mut() {
        let active = tab.tab() == workshop.side_tab;
        *bg = match (*interaction, active) {
            (Interaction::Pressed, _) => BackgroundColor(Color::srgba(0.08, 0.12, 0.16, 0.92)),
            (_, true) => BackgroundColor(Color::srgba(0.06, 0.10, 0.16, 0.78)),
            (Interaction::Hovered, false) => BackgroundColor(Color::srgba(0.04, 0.04, 0.05, 0.78)),
            _ => BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.70)),
        };
    }

    for (tab, mut text) in &mut texts {
        let label = match tab.tab() {
            GenSceneSideTab::Status => "Status".to_string(),
            GenSceneSideTab::Prefab => "Prefab".to_string(),
        };
        **text = label.into();
    }
}

pub(crate) fn gen_scene_build_button(
    build_scene: Res<State<BuildScene>>,
    config: Res<AppConfig>,
    mode: Res<State<GameMode>>,
    active: Res<crate::realm::ActiveRealmScene>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut job: ResMut<GenSceneJob>,
    mut scenes_state: ResMut<ScenesPanelUiState>,
    mut pending_switch: ResMut<crate::realm::PendingRealmSceneSwitch>,
    mut saves: MessageWriter<crate::scene_store::SceneSaveRequest>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
            &mut Node,
        ),
        With<GenSceneGenerateButton>,
    >,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    for (interaction, mut bg, mut border, mut vis, mut node) in &mut buttons {
        if job.running {
            node.display = Display::None;
            *vis = Visibility::Hidden;
            *bg = BackgroundColor(Color::srgba(0.07, 0.07, 0.08, 0.70));
            *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.75));
            *last_interaction = None;
            continue;
        }

        node.display = Display::Flex;
        *vis = Visibility::Inherited;

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

                match gen_scene_request_build(
                    &config,
                    Some(&mode),
                    &active,
                    &mut workshop,
                    &mut job,
                    &mut scenes_state,
                    &mut pending_switch,
                    &mut saves,
                ) {
                    Ok(()) => {}
                    Err(err) => {
                        workshop.error = Some(err);
                    }
                }
            }
        }

    *last_interaction = Some(*interaction);
    }
}

pub(crate) fn gen_scene_stop_button(
    build_scene: Res<State<BuildScene>>,
    mut job: ResMut<GenSceneJob>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut gen3d_queue: ResMut<Gen3dTaskQueue>,
    mut gen3d_workshop: ResMut<Gen3dWorkshop>,
    mut gen3d_job: ResMut<Gen3dAiJob>,
    mut gen3d_draft: ResMut<Gen3dDraft>,
    mut genfloor_job: ResMut<GenFloorAiJob>,
    mut buttons: Query<(&Interaction, &mut BackgroundColor, &mut BorderColor, &mut Visibility, &mut Node), With<GenSceneStopButton>>,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    let Ok((interaction, mut bg, mut border, mut vis, mut node)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    if !job.running {
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
            *bg = BackgroundColor(Color::srgba(0.16, 0.07, 0.06, 0.80));
            *border = BorderColor::all(Color::srgb(0.85, 0.38, 0.30));
        }
        Interaction::Hovered => {
            *bg = BackgroundColor(Color::srgba(0.20, 0.09, 0.08, 0.88));
            *border = BorderColor::all(Color::srgb(0.92, 0.45, 0.36));
        }
        Interaction::Pressed => {
            *bg = BackgroundColor(Color::srgba(0.26, 0.11, 0.10, 0.94));
            *border = BorderColor::all(Color::srgb(1.0, 0.55, 0.45));
            if matches!(*last_interaction, Some(Interaction::Pressed)) {
                *last_interaction = Some(*interaction);
                return;
            }
            gen_scene_cancel_job(
                &mut job,
                &mut workshop,
                &mut gen3d_queue,
                &mut gen3d_workshop,
                &mut gen3d_job,
                &mut gen3d_draft,
                &mut genfloor_job,
            );
        }
    }
    *last_interaction = Some(*interaction);
}

pub(crate) fn gen_scene_save_button(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    job: Res<GenSceneJob>,
    mut saves: MessageWriter<crate::scene_store::SceneSaveRequest>,
    mut buttons: Query<
        (
            &Interaction,
            &mut BackgroundColor,
            &mut BorderColor,
            &mut Visibility,
        ),
        With<GenSceneSaveButton>,
    >,
    mut last_interaction: Local<Option<Interaction>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    let Ok((interaction, mut bg, mut border, mut vis)) = buttons.single_mut() else {
        *last_interaction = None;
        return;
    };

    *vis = Visibility::Hidden;
    *bg = BackgroundColor(Color::srgba(0.10, 0.10, 0.11, 0.55));
    *border = BorderColor::all(Color::srgba(0.30, 0.30, 0.34, 0.70));

    if matches!(*interaction, Interaction::Pressed)
        && !matches!(*last_interaction, Some(Interaction::Pressed))
    {
        saves.write(crate::scene_store::SceneSaveRequest::new(
            "gen scene snapshot",
        ));
        if job.running {
            workshop.status = "Snapshot save requested while build is running.".into();
        } else {
            workshop.status = "Snapshot save requested.".into();
        }
    }

    *last_interaction = Some(*interaction);
}

pub(crate) fn gen_scene_prompt_box_focus(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    mut prompt_boxes: Query<(&Interaction, &mut BackgroundColor), With<GenScenePromptBox>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

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

pub(crate) fn gen_scene_prompt_ime_position(
    build_scene: Res<State<BuildScene>>,
    workshop: Res<GenSceneWorkshop>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<GenScenePromptScrollPanel>>,
    rich_text: Query<Entity, With<GenScenePromptRichText>>,
    hint_text: Query<Entity, With<GenScenePromptHintText>>,
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
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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

pub(crate) fn gen_scene_prompt_text_input(
    build_scene: Res<State<BuildScene>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: bevy::ecs::message::MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut ime_events: bevy::ecs::message::MessageReader<Ime>,
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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

fn clear_prompt_limit_error(workshop: &mut GenSceneWorkshop) {
    if workshop
        .error
        .as_ref()
        .is_some_and(|err| err.starts_with("Prompt limit"))
    {
        workshop.error = None;
    }
}

fn push_prompt_text(workshop: &mut GenSceneWorkshop, text: &str) {
    let max_words = crate::gen3d::GEN3D_PROMPT_MAX_WORDS;
    let max_chars = crate::gen3d::GEN3D_PROMPT_MAX_CHARS;

    let mut words = crate::gen3d::gen3d_count_whitespace_separated_words(&workshop.prompt);
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
        let words_now = crate::gen3d::gen3d_count_whitespace_separated_words(&workshop.prompt);
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

#[derive(SystemParam)]
pub(crate) struct GenSceneUpdateUiTextDeps<'w, 's> {
    commands: Commands<'w, 's>,
    workshop: Res<'w, GenSceneWorkshop>,
    job: Res<'w, GenSceneJob>,
    gen3d_queue: Res<'w, Gen3dTaskQueue>,
    genfloor_job: Res<'w, GenFloorAiJob>,
    ui_fonts: Res<'w, UiFonts>,
    emoji_atlas: Res<'w, EmojiAtlas>,
    asset_server: Res<'w, AssetServer>,
    rich_text: Query<'w, 's, Entity, With<GenScenePromptRichText>>,
    hint_text: Query<'w, 's, Entity, With<GenScenePromptHintText>>,
    prompt_nodes: Query<
        'w,
        's,
        &'static mut Node,
        (With<GenScenePromptRichText>, Without<GenScenePromptHintText>),
    >,
    hint_nodes: Query<
        'w,
        's,
        &'static mut Node,
        (With<GenScenePromptHintText>, Without<GenScenePromptRichText>),
    >,
    texts: ParamSet<
        'w,
        's,
        (
            Query<
                'w,
                's,
                &'static mut Text,
                (With<GenSceneStatusText>, Without<GenSceneStatusLogsText>),
            >,
            Query<
                'w,
                's,
                &'static mut Text,
                (With<GenSceneStatusLogsText>, Without<GenSceneStatusText>),
            >,
            Query<'w, 's, &'static mut Text, With<GenScenePreviewStatsText>>,
        ),
    >,
}

pub(crate) fn gen_scene_update_ui_text(
    build_scene: Res<State<BuildScene>>,
    deps: GenSceneUpdateUiTextDeps,
    mut last_prompt: Local<Option<String>>,
    mut last_prompt_entity: Local<Option<Entity>>,
    mut last_hint: Local<Option<String>>,
    mut last_hint_entity: Local<Option<Entity>>,
    mut autoscroll_frames: Local<u8>,
) {
    let GenSceneUpdateUiTextDeps {
        mut commands,
        workshop,
        job,
        gen3d_queue,
        genfloor_job,
        ui_fonts,
        emoji_atlas,
        asset_server,
        rich_text,
        hint_text,
        mut prompt_nodes,
        mut hint_nodes,
        mut texts,
    } = deps;
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
        return;
    }

    let prompt_empty = workshop.prompt.trim().is_empty();
    let hint_text_value = "Describe the scene you want to build.".to_string();
    let prompt_text = workshop.prompt.clone();

    let prompt_entity = rich_text.single().ok();
    if prompt_entity != *last_prompt_entity {
        *last_prompt_entity = prompt_entity;
        *last_prompt = None;
    }
    let hint_entity = hint_text.single().ok();
    if hint_entity != *last_hint_entity {
        *last_hint_entity = hint_entity;
        *last_hint = None;
    }

    if let Ok(mut node) = prompt_nodes.single_mut() {
        node.display = if prompt_empty { Display::None } else { Display::Flex };
    }
    if let Ok(mut node) = hint_nodes.single_mut() {
        node.display = if prompt_empty { Display::Flex } else { Display::None };
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

    let progress = gen_scene_progress_summary(&job, &gen3d_queue, &genfloor_job);
    let status_display = if workshop.status.trim().is_empty() {
        progress.clone()
    } else if progress.trim().is_empty() {
        workshop.status.clone()
    } else {
        format!("{}\n{}", workshop.status, progress)
    };

    for mut text in &mut texts.p0() {
        text.0 = status_display.clone();
    }
    for mut text in &mut texts.p1() {
        if let Some(err) = workshop.error.as_ref() {
            text.0 = format!("{}\n\nError: {err}", status_display);
        } else {
            text.0 = status_display.clone();
        }
    }

    let state = if job.running {
        "Running".to_string()
    } else if matches!(job.phase, GenScenePhase::Done) {
        "Done ✓".to_string()
    } else if matches!(job.phase, GenScenePhase::Canceled) {
        "Stopped".to_string()
    } else if matches!(job.phase, GenScenePhase::Failed) {
        "Failed".to_string()
    } else {
        "Idle".to_string()
    };
    let scene_id = workshop
        .active_scene_id
        .as_deref()
        .unwrap_or("-");
    let assets = job.resolved_prefabs.len();
    let placements = job.placements.len();
    let stats_text = format!(
        "State: {state}\nScene: {scene_id}\nAssets: {assets}\nPlacements: {placements}",
    );
    for mut text in &mut texts.p2() {
        **text = stats_text.clone();
    }
}

pub(crate) fn gen_scene_update_preview_panel_image_fit(
    mut nodes: Query<&mut Node, With<GenScenePreviewPanelImage>>,
    panels: Query<&ComputedNode, With<GenScenePreviewPanel>>,
) {
    let Ok(panel) = panels.single() else {
        return;
    };
    let Ok(mut node) = nodes.single_mut() else {
        return;
    };

    let aspect = GEN_SCENE_PREVIEW_WIDTH_PX.max(1) as f32 / GEN_SCENE_PREVIEW_HEIGHT_PX.max(1) as f32;
    let (w, h) = aspect_fit_size(panel.size.x, panel.size.y, aspect);
    node.width = Val::Px(w);
    node.height = Val::Px(h);
}

pub(crate) fn gen_scene_update_preview_camera(
    mut preview: ResMut<GenScenePreview>,
    focus: Res<crate::types::CameraFocus>,
    mut cameras: ParamSet<(
        Query<(&mut Transform, &mut Camera), With<GenScenePreviewCamera>>,
        Query<&Transform, (With<crate::types::MainCamera>, Without<GenScenePreviewCamera>)>,
    )>,
) {
    let offset = {
        let main_q = cameras.p1();
        main_q
            .single()
            .ok()
            .map(|t| t.translation - focus.position)
    };

    let mut preview_q = cameras.p0();
    let Ok((mut transform, mut camera)) = preview_q.single_mut() else {
        return;
    };

    camera.is_active = preview.active;

    if let Some(offset) = offset {
        let mut next = Transform::from_translation(preview.focus + offset);
        next.look_at(preview.focus, Vec3::Y);
        *transform = next;
        preview.dirty = false;
        return;
    }

    if preview.dirty {
        let aspect = GEN_SCENE_PREVIEW_WIDTH_PX.max(1) as f32
            / GEN_SCENE_PREVIEW_HEIGHT_PX.max(1) as f32;
        let mut projection = bevy::camera::PerspectiveProjection::default();
        projection.aspect_ratio = aspect;
        let fov_y = projection.fov;
        let near = projection.near;
        let distance = crate::orbit_capture::required_distance_for_view(
            preview.half_extents,
            GEN_SCENE_PREVIEW_YAW,
            GEN_SCENE_PREVIEW_PITCH,
            fov_y,
            aspect,
            near,
        )
        .clamp(near + 0.2, 500.0);

        *transform = crate::orbit_capture::orbit_transform(
            GEN_SCENE_PREVIEW_YAW,
            GEN_SCENE_PREVIEW_PITCH,
            distance,
            preview.focus,
        );
        preview.dirty = false;
    }
}

fn gen_scene_progress_summary(
    job: &GenSceneJob,
    gen3d_queue: &Gen3dTaskQueue,
    genfloor_job: &GenFloorAiJob,
) -> String {
    let mut lines = Vec::new();

    let plan = job.plan.as_ref();
    if let Some(plan) = plan {
        let total_models = plan.assets.len();
        let gen3d_models = plan
            .assets
            .iter()
            .filter(|asset| asset.gen3d_prompt.as_ref().is_some_and(|v| !v.trim().is_empty()))
            .count();
        let existing_models = total_models.saturating_sub(gen3d_models);
        let terrain_plan = if plan
            .terrain
            .existing_floor_id
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty())
        {
            "existing"
        } else if plan
            .terrain
            .genfloor_prompt
            .as_ref()
            .is_some_and(|v| !v.trim().is_empty())
        {
            "genfloor"
        } else {
            "unknown"
        };
        lines.push(format!(
            "Plan: terrain=1 ({terrain_plan}), models={total_models} (existing {existing_models}, gen3d {gen3d_models})"
        ));
    } else {
        lines.push("Plan: pending".to_string());
    }

    let terrain_line = match job.floor_choice.as_ref() {
        Some(GenSceneFloorChoice::Default) => "Terrain: default".to_string(),
        Some(GenSceneFloorChoice::Existing(id)) => {
            let short = uuid::Uuid::from_u128(*id).to_string();
            format!("Terrain: existing {short}")
        }
        Some(GenSceneFloorChoice::GeneratedPrompt(prompt)) => {
            let prompt = truncate_inline(prompt, 40);
            if genfloor_job.running {
                format!("Terrain: generating \"{prompt}\"")
            } else {
                format!("Terrain: queued \"{prompt}\"")
            }
        }
        None => "Terrain: pending".to_string(),
    };
    lines.push(terrain_line);

    let total_models = plan.map(|p| p.assets.len()).unwrap_or(0);
    let ready_models = job.resolved_prefabs.len();

    let mut current = None;
    for task in &job.model_tasks {
        if let Some(meta) = gen3d_queue.metas.get(&task.session_id) {
            if matches!(meta.task_state, Gen3dTaskState::Running) {
                current = Some((task.asset_key.clone(), "running"));
                break;
            }
        }
    }
    if current.is_none() {
        for task in &job.model_tasks {
            if let Some(meta) = gen3d_queue.metas.get(&task.session_id) {
                if matches!(meta.task_state, Gen3dTaskState::Waiting) {
                    current = Some((task.asset_key.clone(), "queued"));
                    break;
                }
            }
        }
    }
    let current_line = if let Some((key, state)) = current {
        format!("current: {key} ({state})")
    } else {
        "current: -".to_string()
    };
    lines.push(format!(
        "Models: {ready_models}/{total_models} ready, {current_line}"
    ));

    lines.join("\n")
}

fn truncate_inline(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    let mut out = String::new();
    for ch in trimmed.chars().take(max_chars) {
        out.push(ch);
    }
    if trimmed.chars().count() > max_chars {
        out.push('…');
    }
    out
}

pub(crate) fn gen_scene_prompt_scroll_wheel(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    mut panels: Query<
        (&ComputedNode, &UiGlobalTransform, &mut ScrollPosition),
        With<GenScenePromptScrollPanel>,
    >,
    workshop: Res<GenSceneWorkshop>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 18.0,
        };
        delta_lines += lines;
    }
    if delta_lines.abs() <= f32::EPSILON {
        return;
    }

    let panel_scale = node.inverse_scale_factor();
    let viewport_h = node.size.y.max(0.0) * panel_scale;
    let content_h = node.content_size.y.max(0.0) * panel_scale;
    if viewport_h < 1.0 || content_h <= viewport_h + 0.5 {
        scroll.y = 0.0;
        return;
    }
    let max_scroll = (content_h - viewport_h).max(0.0);

    scroll.y = (scroll.y - delta_lines * 18.0).clamp(0.0, max_scroll);
}

pub(crate) fn gen_scene_prompt_scrollbar_drag(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut workshop: ResMut<GenSceneWorkshop>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<GenScenePromptScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<GenScenePromptScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<GenScenePromptScrollbarThumb>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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
                workshop.prompt_scrollbar_drag = Some(GenScenePromptScrollbarDrag { grab_offset });
            } else if track_clicked {
                let grab_offset = (cursor_in_track - thumb_top).clamp(0.0, thumb_h);
                workshop.prompt_scrollbar_drag = Some(GenScenePromptScrollbarDrag { grab_offset });
            }
        }
    }

    let Some(drag) = workshop.prompt_scrollbar_drag else {
        return;
    };

    let panel_scale = panel_node.inverse_scale_factor().max(0.01);
    let viewport_h = panel_node.size.y.max(1.0) * panel_scale;
    let content_h = panel_node.content_size.y.max(1.0) * panel_scale;
    let max_scroll = (content_h - viewport_h).max(0.0);

    let track_scale = track_node.inverse_scale_factor().max(0.01);
    let track_h = track_node.size.y.max(1.0) * track_scale;
    let thumb_scale = thumb_node.inverse_scale_factor().max(0.01);
    let thumb_h = thumb_node.size.y.max(1.0) * thumb_scale;
    let max_thumb_top = (track_h - thumb_h).max(0.0);

    let cursor_in_track = if let Some(local) = track_transform
        .try_inverse()
        .map(|transform| transform.transform_point2(cursor))
    {
        (local.y + track_node.size.y * 0.5) * track_scale
    } else {
        return;
    };

    let thumb_top = (cursor_in_track - drag.grab_offset).clamp(0.0, max_thumb_top);
    let ratio = if max_thumb_top > 0.0 {
        thumb_top / max_thumb_top
    } else {
        0.0
    };
    scroll.y = (ratio * max_scroll).clamp(0.0, max_scroll);
}

pub(crate) fn gen_scene_update_prompt_scrollbar_ui(
    build_scene: Res<State<BuildScene>>,
    panels: Query<&ComputedNode, With<GenScenePromptScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<GenScenePromptScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<GenScenePromptScrollbarThumb>>,
) {
    if !matches!(build_scene.get(), BuildScene::ScenePreview) {
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
