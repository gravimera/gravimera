use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy::ecs::system::SystemParam;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::geometry::{clamp_world_xz, normalize_flat_direction, snap_to_grid};
use crate::object::registry::ObjectLibrary;
use crate::object::registry::{ColliderProfile, MobilityMode};
use crate::object::visuals;
use crate::prefab_descriptors::PrefabDescriptorLibrary;
use crate::scene_store::SceneSaveRequest;
use crate::types::{
    AabbCollider, BuildDimensions, BuildObject, Collider, Commandable, GameMode, ObjectId,
    ObjectPrefabId, Player,
};

const PANEL_Z_INDEX: i32 = 930;
const PANEL_WIDTH_PX: f32 = 260.0;
const DRAG_START_THRESHOLD_PX: f32 = 6.0;

#[derive(SystemParam)]
pub(crate) struct ModelLibraryEnv<'w> {
    build_scene: Res<'w, State<crate::types::BuildScene>>,
    active: Res<'w, crate::realm::ActiveRealmScene>,
}

#[derive(Debug, Clone, Copy)]
struct ModelLibraryDrag {
    model_id: u128,
    start_cursor: Vec2,
    is_dragging: bool,
    preview_translation: Option<Vec3>,
}

#[derive(Debug, Clone, Copy)]
struct ModelLibraryScrollbarDrag {
    grab_offset: f32,
}

#[derive(Resource, Debug)]
pub(crate) struct ModelLibraryUiState {
    models_dirty: bool,
    open: bool,
    drag: Option<ModelLibraryDrag>,
    spawn_seq: u32,
    scrollbar_drag: Option<ModelLibraryScrollbarDrag>,
    last_rebuilt_scene: Option<(String, String)>,
}

impl Default for ModelLibraryUiState {
    fn default() -> Self {
        Self {
            models_dirty: true,
            open: true,
            drag: None,
            spawn_seq: 0,
            scrollbar_drag: None,
            last_rebuilt_scene: None,
        }
    }
}

impl ModelLibraryUiState {
    pub(crate) fn mark_models_dirty(&mut self) {
        self.models_dirty = true;
    }

    pub(crate) fn is_open(&self) -> bool {
        self.open
    }

    pub(crate) fn set_open(&mut self, open: bool) {
        if self.open == open {
            return;
        }
        self.open = open;
        if !open {
            self.drag = None;
            self.scrollbar_drag = None;
        }
    }

    pub(crate) fn is_drag_active(&self) -> bool {
        self.drag.is_some()
    }
}

#[derive(Component)]
pub(crate) struct ModelLibraryRoot;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollPanel;

#[derive(Component)]
pub(crate) struct ModelLibraryList;

#[derive(Component)]
pub(crate) struct ModelLibraryListItem;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarTrack;

#[derive(Component)]
pub(crate) struct ModelLibraryScrollbarThumb;

#[derive(Component)]
pub(crate) struct ModelLibraryItemButton {
    pub(crate) model_id: u128,
}

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButton;

#[derive(Component)]
pub(crate) struct ModelLibraryGen3dButtonText;

pub(crate) fn setup_model_library_ui(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(44.0),
                left: Val::Px(10.0),
                width: Val::Px(PANEL_WIDTH_PX),
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
            ZIndex(PANEL_Z_INDEX),
            ModelLibraryRoot,
        ))
        .with_children(|root| {
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
                    Text::new("Prefabs"),
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
                    ModelLibraryGen3dButton,
                ))
                .with_children(|b| {
                    b.spawn((
                        Text::new("Gen3D"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        ModelLibraryGen3dButtonText,
                    ));
                });
            });

            root.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(6.0),
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
                    ModelLibraryScrollPanel,
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
                        ModelLibraryList,
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
                    ModelLibraryScrollbarTrack,
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
                        BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.20)),
                        ModelLibraryScrollbarThumb,
                    ));
                });
            });
        });
}

pub(crate) fn model_library_update_visibility(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    state: Res<ModelLibraryUiState>,
    mut roots: Query<&mut Visibility, With<ModelLibraryRoot>>,
) {
    let visible = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    for mut visibility in &mut roots {
        *visibility = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

pub(crate) fn model_library_rebuild_list_ui(
    mut commands: Commands,
    active: Res<crate::realm::ActiveRealmScene>,
    library: Res<ObjectLibrary>,
    mut descriptors: ResMut<PrefabDescriptorLibrary>,
    mut state: ResMut<ModelLibraryUiState>,
    lists: Query<Entity, With<ModelLibraryList>>,
    existing_items: Query<Entity, With<ModelLibraryListItem>>,
) {
    let active_changed = match state.last_rebuilt_scene.as_ref() {
        Some((realm_id, scene_id)) => realm_id != &active.realm_id || scene_id != &active.scene_id,
        None => true,
    };
    if active_changed {
        state.models_dirty = true;
    }

    if !state.models_dirty {
        return;
    }
    let Ok(list_entity) = lists.single() else {
        return;
    };
    state.last_rebuilt_scene = Some((active.realm_id.clone(), active.scene_id.clone()));

    for entity in &existing_items {
        commands.entity(entity).try_despawn();
    }

    descriptors.clear();
    let scene_prefabs_dir = crate::scene_prefabs::scene_prefabs_root_dir(&active.realm_id, &active.scene_id);
    if let Err(err) =
        crate::prefab_descriptors::load_prefab_descriptors_from_dir(&scene_prefabs_dir, &mut *descriptors)
    {
        warn!("{err}");
    }

    let model_ids =
        crate::scene_prefabs::list_scene_prefab_packages(&active.realm_id, &active.scene_id)
            .unwrap_or_default();
    if model_ids.is_empty() {
        commands.entity(list_entity).with_children(|list| {
            list.spawn((
                Text::new("No scene prefabs yet.\nUse Gen3D to generate one."),
                TextFont {
                    font_size: 14.0,
                    ..default()
                },
                TextColor(Color::srgb(0.80, 0.80, 0.86)),
                ModelLibraryListItem,
            ));
        });
        state.models_dirty = false;
        return;
    }

    commands.entity(list_entity).with_children(|list| {
        for model_id in model_ids {
            let label = model_label(model_id, &library, &descriptors);
            list.spawn((
                Button,
                Node {
                    width: Val::Percent(100.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.06, 0.75)),
                BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ModelLibraryListItem,
                ModelLibraryItemButton { model_id },
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

    state.models_dirty = false;
}

pub(crate) fn model_library_update_scrollbar_ui(
    panels: Query<(&ComputedNode, &ScrollPosition), With<ModelLibraryScrollPanel>>,
    mut tracks: Query<(&ComputedNode, &mut Visibility), With<ModelLibraryScrollbarTrack>>,
    mut thumbs: Query<&mut Node, With<ModelLibraryScrollbarThumb>>,
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

pub(crate) fn model_library_scroll_wheel(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<MouseWheel>,
    state: Res<ModelLibraryUiState>,
    roots: Query<(&ComputedNode, &UiGlobalTransform, &Visibility), With<ModelLibraryRoot>>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ModelLibraryScrollPanel>>,
) {
    if !state.is_open()
        || !matches!(mode.get(), GameMode::Build)
        || !matches!(build_scene.get(), crate::types::BuildScene::Realm)
        || state.scrollbar_drag.is_some()
    {
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

pub(crate) fn model_library_scrollbar_drag(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut state: ResMut<ModelLibraryUiState>,
    mut panels: Query<(&ComputedNode, &mut ScrollPosition), With<ModelLibraryScrollPanel>>,
    tracks: Query<
        (&ComputedNode, &UiGlobalTransform, &Visibility),
        With<ModelLibraryScrollbarTrack>,
    >,
    thumbs: Query<(&Interaction, &ComputedNode, &Node), With<ModelLibraryScrollbarThumb>>,
) {
    let active = state.is_open()
        && matches!(mode.get(), GameMode::Build)
        && matches!(build_scene.get(), crate::types::BuildScene::Realm);
    if !active {
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
            state.scrollbar_drag = Some(ModelLibraryScrollbarDrag { grab_offset });
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

pub(crate) fn model_library_gen3d_button_interactions(
    mode: Res<State<GameMode>>,
    build_scene: Res<State<crate::types::BuildScene>>,
    mut next_build_scene: ResMut<NextState<crate::types::BuildScene>>,
    mut buttons: Query<
        (&Interaction, &mut BackgroundColor, &mut BorderColor),
        (Changed<Interaction>, With<ModelLibraryGen3dButton>),
    >,
) {
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

                if !matches!(mode.get(), GameMode::Build) {
                    continue;
                }

                match build_scene.get() {
                    crate::types::BuildScene::Realm => {
                        next_build_scene.set(crate::types::BuildScene::Preview);
                    }
                    crate::types::BuildScene::Preview => {
                        next_build_scene.set(crate::types::BuildScene::Realm);
                    }
                }
            }
        }
    }
}

pub(crate) fn model_library_item_button_interactions(
    mut state: ResMut<ModelLibraryUiState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut buttons: Query<
        (
            &Interaction,
            &ModelLibraryItemButton,
            &mut BackgroundColor,
            &mut BorderColor,
        ),
        Changed<Interaction>,
    >,
) {
    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    for (interaction, button, mut bg, mut border) in &mut buttons {
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
                if state.drag.is_none() {
                    if let Some(cursor) = cursor {
                        state.drag = Some(ModelLibraryDrag {
                            model_id: button.model_id,
                            start_cursor: cursor,
                            is_dragging: false,
                            preview_translation: None,
                        });
                    }
                }
            }
        }
    }
}

pub(crate) fn model_library_drag_update(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    env: ModelLibraryEnv,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<crate::types::MainCamera>>,
    player_q: Query<(&Transform, &Collider), With<Player>>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    mut scene_saves: bevy::ecs::message::MessageWriter<SceneSaveRequest>,
    mut state: ResMut<ModelLibraryUiState>,
) {
    if !state.is_open() || !matches!(env.build_scene.get(), crate::types::BuildScene::Realm) {
        state.drag = None;
        return;
    }

    let Some(mut drag) = state.drag else {
        return;
    };

    if !mouse_buttons.pressed(MouseButton::Left) {
        // Mouse was released; treat as either click-spawn (near hero) or drag-spawn.
        let prefab_id = drag.model_id;
        if let Err(err) = ensure_scene_prefab_loaded(&env.active, prefab_id, &mut library) {
            warn!("{err}");
            state.drag = None;
            return;
        }
        let spawn_translation = if drag.is_dragging && drag.preview_translation.is_some() {
            drag.preview_translation.unwrap()
        } else {
            let Ok((player_transform, player_collider)) = player_q.single() else {
                state.drag = None;
                return;
            };
            spawn_near_hero(
                prefab_id,
                player_transform,
                player_collider,
                &library,
                state.spawn_seq,
            )
        };

        let spawned = spawn_prefab_instance(
            &mut commands,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            &library,
            prefab_id,
            spawn_translation,
        );
        if spawned.is_some() {
            state.spawn_seq = state.spawn_seq.wrapping_add(1);
            scene_saves.write(SceneSaveRequest::new("spawned prefab from scene"));
        }

        state.drag = None;
        return;
    }

    let cursor = windows
        .single()
        .ok()
        .and_then(|window| window.cursor_position());
    if let Some(cursor) = cursor {
        if !drag.is_dragging && cursor.distance(drag.start_cursor) > DRAG_START_THRESHOLD_PX {
            drag.is_dragging = true;
        }

        if drag.is_dragging {
            if let Err(err) =
                ensure_scene_prefab_loaded(&env.active, drag.model_id, &mut library)
            {
                warn!("{err}");
                drag.preview_translation = None;
                state.drag = Some(drag);
                return;
            }

            let Ok((camera, camera_transform)) = camera_q.single() else {
                drag.preview_translation = None;
                state.drag = Some(drag);
                return;
            };
            let camera_global = GlobalTransform::from(*camera_transform);
            if let Ok(window) = windows.single() {
                if let Some(pick) = crate::cursor_pick::cursor_surface_pick(
                    window,
                    camera,
                    &camera_global,
                    &library,
                    &objects,
                ) {
                    drag.preview_translation = Some(spawn_at_pick(
                        drag.model_id,
                        pick.hit,
                        pick.surface_y,
                        &library,
                    ));
                } else {
                    drag.preview_translation = None;
                }
            }
        }
    }

    state.drag = Some(drag);
}

pub(crate) fn model_library_draw_drag_preview_gizmos(
    mut gizmos: Gizmos,
    mode: Res<State<GameMode>>,
    library: Res<ObjectLibrary>,
    state: Res<ModelLibraryUiState>,
) {
    if !matches!(mode.get(), GameMode::Build) {
        return;
    }
    let Some(drag) = state.drag else {
        return;
    };
    if !drag.is_dragging {
        return;
    }
    let Some(translation) = drag.preview_translation else {
        return;
    };

    let (size, half_xz, origin_y) = prefab_bounds(&library, drag.model_id, Vec3::ONE);
    let bottom_y = translation.y - origin_y;
    let top_y = bottom_y + size.y;

    let min = Vec3::new(
        translation.x - half_xz.x,
        bottom_y,
        translation.z - half_xz.y,
    );
    let max = Vec3::new(translation.x + half_xz.x, top_y, translation.z + half_xz.y);

    draw_dashed_box(&mut gizmos, min, max, Color::srgb(0.25, 0.95, 0.85));
}

fn model_label(
    model_id: u128,
    library: &ObjectLibrary,
    descriptors: &PrefabDescriptorLibrary,
) -> String {
    if let Some(desc) = descriptors.get(model_id) {
        if let Some(label) = desc
            .label
            .as_ref()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            return label.to_string();
        }
        if let Some(text) = desc
            .text
            .as_ref()
            .and_then(|t| t.short.as_ref())
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
        {
            return text.to_string();
        }
    }

    if let Some(def) = library.get(model_id) {
        let label = def.label.as_ref().trim();
        if !label.is_empty() {
            return label.to_string();
        }
    }

    uuid::Uuid::from_u128(model_id).to_string()
}

fn ensure_scene_prefab_loaded(
    active: &crate::realm::ActiveRealmScene,
    prefab_id: u128,
    library: &mut ObjectLibrary,
) -> Result<(), String> {
    if library.get(prefab_id).is_some() {
        return Ok(());
    }

    let loaded = crate::scene_prefabs::load_scene_prefab_package_defs_into_library(
        &active.realm_id,
        &active.scene_id,
        prefab_id,
        library,
    )?;
    if loaded == 0 {
        return Err(format!(
            "Prefab {} is not loaded and no scene-local prefab package was found under {}/{}.",
            uuid::Uuid::from_u128(prefab_id),
            active.realm_id,
            active.scene_id,
        ));
    }

    Ok(())
}

fn prefab_bounds(library: &ObjectLibrary, prefab_id: u128, scale: Vec3) -> (Vec3, Vec2, f32) {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));
    let scale = Vec3::new(
        if scale.x.is_finite() {
            scale.x.abs().max(1e-4)
        } else {
            1.0
        },
        if scale.y.is_finite() {
            scale.y.abs().max(1e-4)
        } else {
            1.0
        },
        if scale.z.is_finite() {
            scale.z.abs().max(1e-4)
        } else {
            1.0
        },
    );

    let half_unscaled = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => Vec2::splat(radius.max(0.01)),
        Some(ColliderProfile::AabbXZ { half_extents }) => Vec2::new(
            half_extents.x.abs().max(0.01),
            half_extents.y.abs().max(0.01),
        ),
        _ => Vec2::new(
            (base_size.x * 0.5).abs().max(0.01),
            (base_size.z * 0.5).abs().max(0.01),
        ),
    };

    let half_xz = Vec2::new(half_unscaled.x * scale.x, half_unscaled.y * scale.z);
    let size = Vec3::new(
        half_xz.x * 2.0,
        base_size.y.abs() * scale.y,
        half_xz.y * 2.0,
    );
    let origin_y = library.ground_origin_y_or_default(prefab_id) * scale.y;

    (size, half_xz, origin_y)
}

fn spawn_near_hero(
    prefab_id: u128,
    player_transform: &Transform,
    player_collider: &Collider,
    library: &ObjectLibrary,
    seq: u32,
) -> Vec3 {
    let (_size, half_xz, origin_y) = prefab_bounds(library, prefab_id, Vec3::ONE);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);
    let mobility_mode = library.mobility(prefab_id).map(|m| m.mode);

    let forward = normalize_flat_direction(player_transform.rotation * Vec3::Z).unwrap_or(Vec3::Z);
    let right = Vec3::Y.cross(forward).normalize_or_zero();
    let distance = player_collider.radius + object_radius + BUILD_UNIT_SIZE;

    let slots_per_ring: u32 = 12;
    let ring = seq / slots_per_ring;
    let index_in_ring = seq % slots_per_ring;
    let angle = (index_in_ring as f32) * (std::f32::consts::TAU / slots_per_ring as f32);
    let mut dir = (right * angle.cos() + forward * angle.sin()).normalize_or_zero();
    if dir.length_squared() <= 0.0001 {
        dir = Vec3::X;
    }
    let spacing = (object_radius * 2.0 + BUILD_UNIT_SIZE * 2.0).max(BUILD_UNIT_SIZE * 4.0);
    let radial = distance + ring as f32 * spacing;

    let mut pos = player_transform.translation + dir * radial;
    pos.x = snap_to_grid(pos.x, BUILD_GRID_SIZE);
    pos.z = snap_to_grid(pos.z, BUILD_GRID_SIZE);
    pos.y = match mobility_mode {
        Some(MobilityMode::Air) => origin_y + BUILD_UNIT_SIZE * 8.0,
        _ => origin_y,
    };

    pos.x = clamp_world_xz(pos.x, half_xz.x);
    pos.z = clamp_world_xz(pos.z, half_xz.y);

    pos
}

fn spawn_at_pick(prefab_id: u128, hit: Vec3, surface_y: f32, library: &ObjectLibrary) -> Vec3 {
    let (_size, half_xz, origin_y) = prefab_bounds(library, prefab_id, Vec3::ONE);
    let mobility_mode = library.mobility(prefab_id).map(|m| m.mode);

    let mut pos = Vec3::new(hit.x, surface_y + origin_y, hit.z);
    pos.x = snap_to_grid(pos.x, BUILD_GRID_SIZE);
    pos.z = snap_to_grid(pos.z, BUILD_GRID_SIZE);
    pos.y = match mobility_mode {
        Some(MobilityMode::Air) => surface_y + origin_y + BUILD_UNIT_SIZE * 8.0,
        _ => surface_y + origin_y,
    };

    pos.x = clamp_world_xz(pos.x, half_xz.x);
    pos.z = clamp_world_xz(pos.z, half_xz.y);

    pos
}

fn spawn_prefab_instance(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    translation: Vec3,
) -> Option<Entity> {
    if library.get(prefab_id).is_none() {
        warn!(
            "Cannot spawn prefab {}: prefab def is not loaded.",
            uuid::Uuid::from_u128(prefab_id)
        );
        return None;
    }

    let instance_id = ObjectId::new_v4();
    let transform = Transform::from_translation(translation);
    let mobility = library.mobility(prefab_id).is_some();

    let (size, half_xz, _origin_y) = prefab_bounds(library, prefab_id, transform.scale);
    let object_radius = half_xz.x.max(half_xz.y).max(0.1);

    let mut entity_commands = if mobility {
        commands.spawn((
            instance_id,
            ObjectPrefabId(prefab_id),
            Commandable,
            Collider {
                radius: object_radius,
            },
            transform,
            Visibility::Inherited,
        ))
    } else {
        commands.spawn((
            instance_id,
            ObjectPrefabId(prefab_id),
            BuildObject,
            BuildDimensions { size },
            AabbCollider {
                half_extents: half_xz,
            },
            transform,
            Visibility::Inherited,
        ))
    };

    visuals::spawn_object_visuals(
        &mut entity_commands,
        library,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        prefab_id,
        None,
    );

    Some(entity_commands.id())
}

fn draw_dashed_box(gizmos: &mut Gizmos, min: Vec3, max: Vec3, color: Color) {
    let c0 = Vec3::new(min.x, min.y, min.z);
    let c1 = Vec3::new(max.x, min.y, min.z);
    let c2 = Vec3::new(max.x, min.y, max.z);
    let c3 = Vec3::new(min.x, min.y, max.z);
    let c4 = Vec3::new(min.x, max.y, min.z);
    let c5 = Vec3::new(max.x, max.y, min.z);
    let c6 = Vec3::new(max.x, max.y, max.z);
    let c7 = Vec3::new(min.x, max.y, max.z);

    let dash = 0.12;
    let gap = 0.10;
    draw_dashed_line(gizmos, c0, c1, dash, gap, color);
    draw_dashed_line(gizmos, c1, c2, dash, gap, color);
    draw_dashed_line(gizmos, c2, c3, dash, gap, color);
    draw_dashed_line(gizmos, c3, c0, dash, gap, color);

    draw_dashed_line(gizmos, c4, c5, dash, gap, color);
    draw_dashed_line(gizmos, c5, c6, dash, gap, color);
    draw_dashed_line(gizmos, c6, c7, dash, gap, color);
    draw_dashed_line(gizmos, c7, c4, dash, gap, color);

    draw_dashed_line(gizmos, c0, c4, dash, gap, color);
    draw_dashed_line(gizmos, c1, c5, dash, gap, color);
    draw_dashed_line(gizmos, c2, c6, dash, gap, color);
    draw_dashed_line(gizmos, c3, c7, dash, gap, color);
}

fn draw_dashed_line(
    gizmos: &mut Gizmos,
    start: Vec3,
    end: Vec3,
    dash_len: f32,
    gap_len: f32,
    color: Color,
) {
    let delta = end - start;
    let length = delta.length();
    if length <= 1e-4 {
        return;
    }

    let dash_len = dash_len.max(0.005);
    let step = (dash_len + gap_len).max(0.005);
    let dir = delta / length;

    let mut dist = 0.0;
    while dist < length {
        let segment_start = start + dir * dist;
        let segment_end = start + dir * (dist + dash_len).min(length);
        gizmos.line(segment_start, segment_end, color);
        dist += step;
    }
}
