use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::message::MessageReader;
use bevy::ecs::query::QueryFilter;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use rand::prelude::*;
use std::collections::HashSet;

use crate::constants::*;
use crate::object::registry::ObjectLibrary;
use crate::types::*;

const HEALTH_POPUP_TTL_SECS_ENEMY: f32 = 0.85;
const HEALTH_POPUP_TTL_SECS_HERO: f32 = 1.05;
const HEALTH_POPUP_RISE_PX_PER_SEC_ENEMY: f32 = 42.0;
const HEALTH_POPUP_RISE_PX_PER_SEC_HERO: f32 = 55.0;
const HEALTH_POPUP_HERO_FONT_SCALE: f32 = 0.7;
const HEALTH_POPUP_START_OFFSET_PX: Vec2 = Vec2::new(0.0, -14.0);
const HEALTH_POPUP_JITTER_X_PX: f32 = 14.0;
const HEALTH_POPUP_JITTER_Y_PX: f32 = 6.0;
const HEALTH_POPUP_FADE_START_T: f32 = 0.55;

#[derive(Component)]
pub(crate) struct HealthChangePopup {
    pub(crate) anchor_world: Vec3,
    pub(crate) elapsed_secs: f32,
    pub(crate) ttl_secs: f32,
    pub(crate) rise_px_per_sec: f32,
    pub(crate) base_offset_px: Vec2,
    pub(crate) fade_start_t: f32,
    pub(crate) base_color: Color,
}

pub(crate) fn update_window_title(
    mut windows: Query<&mut Window, With<PrimaryWindow>>,
    game: Res<Game>,
    build: Res<BuildState>,
    mode: Res<State<GameMode>>,
    build_scene: Res<State<BuildScene>>,
) {
    let mut window = match windows.single_mut() {
        Ok(w) => w,
        Err(_) => return,
    };

    match mode.get() {
        GameMode::Build => {
            if matches!(build_scene.get(), BuildScene::Preview) {
                window.title =
                    "Gravimera — BUILD (Preview) | drag & drop photos | Build/Stop | Scene: Realm"
                        .into();
                return;
            }
            if build.placing_active {
                window.title = format!(
                    "Gravimera — BUILD ({}) | score: {} | health: {} | B/F/T place | Esc select | F1 play | Tab forms | hold C copy",
                    build.selected.label(),
                    game.score,
                    game.health
                );
            } else {
                window.title = format!(
                    "Gravimera — BUILD | score: {} | health: {} | LMB select | B/F/T place | F1 play | Tab forms | hold C copy",
                    game.score, game.health
                );
            }
        }
        GameMode::Play => {
            if game.game_over {
                window.title = format!(
                    "Gravimera — PLAY | GAME OVER | score: {} | press R to restart",
                    game.score
                );
            } else {
                window.title = format!(
                    "Gravimera — PLAY ({}) | score: {} | health: {} | LMB select | RMB move | Space fire (hold) | Ctrl/Cmd+1/2/3 switch | 1..9/0 motions | R restart | F1 build | Tab forms | hold C copy",
                    game.weapon.label(),
                    game.score,
                    game.health,
                );
            }
        }
    }
}

pub(crate) fn update_fps_counter(
    diagnostics: Res<DiagnosticsStore>,
    mut fps_text: Query<&mut Text, With<FpsCounterText>>,
    objects: Query<(), (With<ObjectId>, With<ObjectPrefabId>)>,
    primitives: Query<(), With<Mesh3d>>,
) {
    let Ok(mut text) = fps_text.single_mut() else {
        return;
    };

    let object_count = objects.iter().count();
    let primitive_count = primitives.iter().count();
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|diag| diag.smoothed());

    match fps {
        Some(value) if value.is_finite() => {
            *text = Text::new(format!(
                "Obj: {object_count} | Prim: {primitive_count} | {value:5.1} FPS"
            ));
        }
        _ => {
            *text =
                Text::new(format!("Obj: {object_count} | Prim: {primitive_count} | FPS: --"));
        }
    }
}

pub(crate) fn update_health_bars(
    mut commands: Commands,
    game: Res<Game>,
    player_q: Query<(&Transform, &HealthBar), With<Player>>,
    camera_q: Query<&Transform, With<MainCamera>>,
    enemies_q: Query<(Entity, &Transform, &Enemy, &HealthBar), With<Enemy>>,
    mut bar_roots_q: Query<
        &mut Transform,
        (
            Without<Player>,
            Without<Enemy>,
            Without<HealthBarFill>,
            Without<MainCamera>,
        ),
    >,
    mut fills_q: Query<
        &mut Transform,
        (
            With<HealthBarFill>,
            Without<Player>,
            Without<Enemy>,
            Without<MainCamera>,
        ),
    >,
) {
    let desired_global_rotation = camera_q
        .single()
        .ok()
        .and_then(|camera_transform| {
            let mut forward = camera_transform.rotation * Vec3::NEG_Z;
            forward.y = 0.0;
            if forward.length_squared() <= 0.0001 {
                None
            } else {
                forward = forward.normalize();
                Some(Quat::from_rotation_y(forward.x.atan2(forward.z)))
            }
        })
        .unwrap_or(Quat::IDENTITY);

    if let Ok((player_transform, bar)) = player_q.single() {
        let frac = if game.max_health > 0 {
            (game.health.max(0) as f32 / game.max_health as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        if let Ok(mut root_transform) = bar_roots_q.get_mut(bar.root) {
            root_transform.rotation = player_transform.rotation.inverse() * desired_global_rotation;
        }
        if let Ok(mut transform) = fills_q.get_mut(bar.fill) {
            update_health_bar_fill_transform(&mut transform, frac);
        }
    }

    for (enemy_entity, enemy_transform, enemy, bar) in &enemies_q {
        let frac = if enemy.max_health > 0 {
            (enemy.health.max(0) as f32 / enemy.max_health as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };

        if let Ok(mut root_transform) = bar_roots_q.get_mut(bar.root) {
            root_transform.rotation = enemy_transform.rotation.inverse() * desired_global_rotation;
        }

        if let Ok(mut fill_transform) = fills_q.get_mut(bar.fill) {
            update_health_bar_fill_transform(&mut fill_transform, frac);
        } else if let Ok(mut entity_commands) = commands.get_entity(enemy_entity) {
            entity_commands.try_remove::<HealthBar>();
        }
    }
}

fn update_health_bar_fill_transform(transform: &mut Transform, frac: f32) {
    let frac = frac.clamp(0.0, 1.0);
    let width = HEALTH_BAR_WIDTH * frac;
    transform.scale.x = width;
    transform.translation.x = -HEALTH_BAR_WIDTH * 0.5 + width * 0.5;
}

pub(crate) fn spawn_health_change_popups(
    mut commands: Commands,
    mut events: MessageReader<HealthChangeEvent>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
) {
    let Ok((camera, camera_transform)) = camera_q.single() else {
        events.clear();
        return;
    };

    let mut rng = thread_rng();
    for event in events.read() {
        if event.delta == 0 {
            continue;
        }

        let magnitude = event.delta.unsigned_abs();
        let (ttl_secs, rise_px_per_sec, base_font_size) = if event.is_hero {
            (
                HEALTH_POPUP_TTL_SECS_HERO,
                HEALTH_POPUP_RISE_PX_PER_SEC_HERO,
                28.0,
            )
        } else {
            (
                HEALTH_POPUP_TTL_SECS_ENEMY,
                HEALTH_POPUP_RISE_PX_PER_SEC_ENEMY,
                20.0,
            )
        };

        let big = magnitude >= 5;
        let mut font_size = if big {
            base_font_size + if event.is_hero { 10.0 } else { 6.0 }
        } else {
            base_font_size
        };
        if event.is_hero {
            font_size *= HEALTH_POPUP_HERO_FONT_SCALE;
        }

        let base_color = if event.delta < 0 {
            if event.is_hero {
                Color::srgb(1.0, 0.35, 0.25)
            } else {
                Color::srgb(0.98, 0.25, 0.25)
            }
        } else if event.is_hero {
            Color::srgb(0.35, 1.0, 0.45)
        } else {
            Color::srgb(0.20, 0.95, 0.40)
        };

        let text = if event.delta > 0 {
            format!("+{}", event.delta)
        } else {
            event.delta.to_string()
        };

        let camera_global = GlobalTransform::from(*camera_transform);
        let Ok(mut screen_pos) = camera.world_to_viewport(&camera_global, event.world_pos) else {
            continue;
        };

        let jitter = Vec2::new(
            rng.gen_range(-HEALTH_POPUP_JITTER_X_PX..HEALTH_POPUP_JITTER_X_PX),
            rng.gen_range(-HEALTH_POPUP_JITTER_Y_PX..HEALTH_POPUP_JITTER_Y_PX),
        );
        let base_offset = HEALTH_POPUP_START_OFFSET_PX + jitter;
        screen_pos += base_offset;

        let mut entity = commands.spawn((
            Text::new(text),
            TextFont {
                font_size,
                ..default()
            },
            TextColor(base_color),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(screen_pos.x),
                top: Val::Px(screen_pos.y),
                ..default()
            },
            ZIndex(if event.is_hero { 210 } else { 200 }),
            HealthChangePopup {
                anchor_world: event.world_pos,
                elapsed_secs: 0.0,
                ttl_secs,
                rise_px_per_sec,
                base_offset_px: base_offset,
                fade_start_t: if event.is_hero {
                    HEALTH_POPUP_FADE_START_T * 0.90
                } else {
                    HEALTH_POPUP_FADE_START_T
                },
                base_color,
            },
        ));

        if event.is_hero || big {
            entity.insert(TextShadow {
                offset: if big {
                    Vec2::splat(2.0)
                } else {
                    Vec2::splat(1.0)
                },
                color: Color::linear_rgba(0.0, 0.0, 0.0, 0.85),
            });
        }
    }
}

pub(crate) fn update_health_change_popups(
    mut commands: Commands,
    time: Res<Time>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    mut popups: Query<(Entity, &mut Node, &mut TextColor, &mut HealthChangePopup)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);

    for (entity, mut node, mut color, mut popup) in &mut popups {
        popup.elapsed_secs += dt;
        if popup.elapsed_secs >= popup.ttl_secs {
            commands.entity(entity).try_despawn();
            continue;
        }

        let Ok(mut screen_pos) = camera.world_to_viewport(&camera_global, popup.anchor_world)
        else {
            continue;
        };

        let rise = popup.rise_px_per_sec * popup.elapsed_secs;
        screen_pos += popup.base_offset_px + Vec2::new(0.0, -rise);

        node.left = Val::Px(screen_pos.x);
        node.top = Val::Px(screen_pos.y);

        let t01 = (popup.elapsed_secs / popup.ttl_secs).clamp(0.0, 1.0);
        let alpha = if t01 <= popup.fade_start_t {
            1.0
        } else {
            let fade_t = ((t01 - popup.fade_start_t) / (1.0 - popup.fade_start_t)).clamp(0.0, 1.0);
            1.0 - fade_t
        };
        color.0 = popup.base_color.with_alpha(alpha);
    }
}

#[derive(Clone, Copy)]
struct MinimapContext {
    inner_size: f32,
    units_to_px: f32,
    center_px: Vec2,
    origin_world: Vec2,
    right_world: Vec2,
    forward_world: Vec2,
    zoom_t: f32,
}

impl MinimapContext {
    fn new(origin_world: Vec2, right_world: Vec2, forward_world: Vec2, zoom_t: f32) -> Self {
        let inner_size = (MINIMAP_SIZE_PX - MINIMAP_BORDER_PX * 2.0).max(1.0);
        let units_to_px = inner_size / (WORLD_HALF_SIZE * 2.0);
        let center_px = Vec2::splat(MINIMAP_BORDER_PX + inner_size * 0.5);

        Self {
            inner_size,
            units_to_px,
            center_px,
            origin_world,
            right_world,
            forward_world,
            zoom_t: zoom_t.clamp(0.0, CAMERA_ZOOM_MAX),
        }
    }
}

fn minimap_camera_axes(transform: &Transform) -> Option<(Vec2, Vec2)> {
    let mut forward = transform.rotation * Vec3::NEG_Z;
    forward.y = 0.0;
    if forward.length_squared() <= 0.0001 {
        return None;
    }
    forward = forward.normalize();

    let mut right = transform.rotation * Vec3::X;
    right.y = 0.0;
    if right.length_squared() <= 0.0001 {
        return None;
    }
    right = right.normalize();

    Some((Vec2::new(right.x, right.z), Vec2::new(forward.x, forward.z)))
}

fn wrap_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    if edge0 >= edge1 {
        return 0.0;
    }
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn apply_deadzone(angle: f32, deadzone: f32) -> f32 {
    if angle.abs() <= deadzone {
        0.0
    } else {
        angle - angle.signum() * deadzone
    }
}

fn minimap_player_marker_angle(raw_angle: f32, zoom_t: f32) -> f32 {
    let fixed_strength = smoothstep(MINIMAP_PLAYER_FIXED_START_T, CAMERA_ZOOM_MAX, zoom_t);
    if fixed_strength <= 0.0 {
        return wrap_angle(raw_angle);
    }

    let mut angle = wrap_angle(raw_angle);
    angle = apply_deadzone(angle, CAMERA_YAW_DEADZONE_RADS * fixed_strength);
    angle * (1.0 - fixed_strength)
}

pub(crate) fn update_minimap(
    mut commands: Commands,
    icons: Res<MinimapIcons>,
    zoom: Res<CameraZoom>,
    camera_q: Query<&Transform, With<MainCamera>>,
    library: Res<ObjectLibrary>,
    root_q: Query<Entity, With<MinimapRoot>>,
    player_q: Query<Entity, With<Player>>,
    enemies_q: Query<(Entity, &ObjectPrefabId), With<Enemy>>,
    buildings_q: Query<(Entity, &Transform, &AabbCollider, &ObjectPrefabId), With<BuildObject>>,
    player_transforms: Query<&Transform, With<Player>>,
    enemy_transforms: Query<&Transform, With<Enemy>>,
    mut markers_q: Query<
        (Entity, &MinimapMarker, &mut Node, &mut UiTransform),
        (Without<MinimapRoot>, Without<MinimapDirectionDot>),
    >,
    mut dir_dots_q: Query<&mut Node, With<MinimapDirectionDot>>,
    mut world_border_q: Query<
        (Entity, &MinimapWorldBorderDot, &mut Node),
        (
            Without<MinimapRoot>,
            Without<MinimapMarker>,
            Without<MinimapBuilding>,
            Without<MinimapDirectionDot>,
        ),
    >,
    mut building_markers_q: Query<
        (Entity, &MinimapBuilding, &mut Node),
        (
            Without<MinimapRoot>,
            Without<MinimapMarker>,
            Without<MinimapDirectionDot>,
        ),
    >,
) {
    let root = match root_q.single() {
        Ok(entity) => entity,
        Err(_) => return,
    };

    let zoom_t = zoom.t.clamp(0.0, CAMERA_ZOOM_MAX);
    let player_pos = player_q
        .single()
        .ok()
        .and_then(|entity| player_transforms.get(entity).ok())
        .map(|transform| Vec2::new(transform.translation.x, transform.translation.z))
        .unwrap_or(Vec2::ZERO);
    let origin_world = Vec2::ZERO.lerp(player_pos, zoom_t);
    let (right_world, forward_world) = camera_q
        .single()
        .ok()
        .and_then(minimap_camera_axes)
        .unwrap_or((Vec2::X, -Vec2::Y));
    let context = MinimapContext::new(origin_world, right_world, forward_world, zoom_t);

    update_world_border_dots(&mut commands, root, &context, &mut world_border_q);

    let mut existing_targets: HashSet<Entity> = HashSet::new();
    for (_, marker, _, _) in &markers_q {
        existing_targets.insert(marker.target);
    }

    let mut existing_buildings: HashSet<Entity> = HashSet::new();
    for (_, marker, _) in &building_markers_q {
        existing_buildings.insert(marker.target);
    }

    for (entity, transform, collider, prefab_id) in &buildings_q {
        if !existing_buildings.contains(&entity) {
            spawn_minimap_building_marker(
                &mut commands,
                root,
                entity,
                transform,
                collider,
                library
                    .minimap_color(prefab_id.0)
                    .unwrap_or(Color::srgba(0.75, 0.75, 0.78, 0.55)),
                &context,
            );
        }
    }

    if let Ok(player_entity) = player_q.single() {
        if !existing_targets.contains(&player_entity) {
            if let Ok(transform) = player_transforms.get(player_entity) {
                spawn_minimap_player_marker(
                    &mut commands,
                    &icons,
                    root,
                    player_entity,
                    transform,
                    &context,
                );
            }
        }
    }

    for (enemy_entity, prefab_id) in &enemies_q {
        if !existing_targets.contains(&enemy_entity) {
            if let Ok(transform) = enemy_transforms.get(enemy_entity) {
                spawn_minimap_enemy_marker(
                    &mut commands,
                    root,
                    enemy_entity,
                    transform,
                    library
                        .minimap_color(prefab_id.0)
                        .unwrap_or(Color::srgb(0.75, 0.75, 0.78)),
                    &context,
                );
            }
        }
    }

    for (marker_entity, marker, mut node, mut ui_transform) in &mut markers_q {
        let transform = if let Ok(transform) = player_transforms.get(marker.target) {
            transform
        } else if let Ok(transform) = enemy_transforms.get(marker.target) {
            transform
        } else {
            commands.entity(marker_entity).try_despawn();
            continue;
        };

        let (marker_left, marker_top, dir_left, dir_top, angle) =
            minimap_layout_for_transform(transform, &context);
        node.left = Val::Px(marker_left);
        node.top = Val::Px(marker_top);

        match marker.kind {
            MinimapMarkerKind::Player => {
                ui_transform.rotation = Rot2::radians(minimap_player_marker_angle(angle, zoom_t));
            }
            MinimapMarkerKind::Enemy => {
                ui_transform.rotation = Rot2::IDENTITY;
                if let Some(dir_dot) = marker.dir_dot {
                    if let Ok(mut dir_node) = dir_dots_q.get_mut(dir_dot) {
                        dir_node.left = Val::Px(dir_left);
                        dir_node.top = Val::Px(dir_top);
                    }
                }
            }
        }
    }

    for (marker_entity, marker, mut node) in &mut building_markers_q {
        let Ok((transform, collider, _object)) =
            buildings_q.get(marker.target).map(|v| (v.1, v.2, v.3))
        else {
            commands.entity(marker_entity).try_despawn();
            continue;
        };

        let (left, top, width, height) = minimap_building_layout(transform, collider, &context);
        node.left = Val::Px(left);
        node.top = Val::Px(top);
        node.width = Val::Px(width);
        node.height = Val::Px(height);
    }
}

fn update_world_border_dots<F: QueryFilter>(
    commands: &mut Commands,
    root: Entity,
    context: &MinimapContext,
    world_border_q: &mut Query<(Entity, &MinimapWorldBorderDot, &mut Node), F>,
) {
    let dot_size = MINIMAP_WORLD_BORDER_THICKNESS_PX.max(1.0);
    let dot_spacing_px = MINIMAP_WORLD_BORDER_DOT_SPACING_PX.max(dot_size);
    let step_world = dot_spacing_px / context.units_to_px.max(0.001);

    let border_points = minimap_world_border_points(step_world);
    let mut dots = vec![None; border_points.len()];

    for (entity, dot, _) in world_border_q.iter() {
        let index = dot.index as usize;
        if index < dots.len() {
            dots[index] = Some(entity);
        } else {
            commands.entity(entity).try_despawn();
        }
    }

    for (index, dot_entity) in dots.iter_mut().enumerate() {
        if dot_entity.is_some() {
            continue;
        }

        let px = minimap_world_to_px(border_points[index], context);
        let mut spawned = None;
        commands.entity(root).with_children(|parent| {
            spawned = Some(
                parent
                    .spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(px.x - dot_size * 0.5),
                            top: Val::Px(px.y - dot_size * 0.5),
                            width: Val::Px(dot_size),
                            height: Val::Px(dot_size),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.92, 0.92, 0.92, 0.55)),
                        ZIndex(0),
                        MinimapWorldBorderDot {
                            index: index as u16,
                        },
                    ))
                    .id(),
            );
        });
        *dot_entity = spawned;
    }

    for (_entity, dot, mut node) in world_border_q.iter_mut() {
        let index = dot.index as usize;
        if index >= border_points.len() {
            continue;
        }

        let px = minimap_world_to_px(border_points[index], context);
        node.left = Val::Px(px.x - dot_size * 0.5);
        node.top = Val::Px(px.y - dot_size * 0.5);
    }
}

fn minimap_world_to_px(world_pos: Vec2, context: &MinimapContext) -> Vec2 {
    let relative = world_pos - context.origin_world;
    Vec2::new(
        context.center_px.x + relative.dot(context.right_world) * context.units_to_px,
        context.center_px.y - relative.dot(context.forward_world) * context.units_to_px,
    )
}

fn minimap_world_border_points(step_world: f32) -> Vec<Vec2> {
    let half = WORLD_HALF_SIZE;
    let corners = [
        Vec2::new(-half, -half),
        Vec2::new(-half, half),
        Vec2::new(half, half),
        Vec2::new(half, -half),
    ];

    let mut points = Vec::new();
    for side in 0..corners.len() {
        let start = corners[side];
        let end = corners[(side + 1) % corners.len()];
        let length = start.distance(end);
        let steps = (length / step_world.max(0.01)).ceil() as u32;

        for i in 0..=steps {
            if side > 0 && i == 0 {
                continue;
            }
            let t = i as f32 / steps.max(1) as f32;
            points.push(start.lerp(end, t));
        }
    }

    points
}

fn spawn_minimap_player_marker(
    commands: &mut Commands,
    icons: &MinimapIcons,
    root: Entity,
    target: Entity,
    transform: &Transform,
    context: &MinimapContext,
) {
    let (left, top, _, _, angle) = minimap_layout_for_transform(transform, context);
    let angle = minimap_player_marker_angle(angle, context.zoom_t);
    let fill_size = MINIMAP_MARKER_SIZE_PX * 0.78;
    let fill_offset = (MINIMAP_MARKER_SIZE_PX - fill_size) * 0.5;

    let mut marker_entity = None;
    commands.entity(root).with_children(|parent| {
        marker_entity = Some(
            parent
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left),
                        top: Val::Px(top),
                        width: Val::Px(MINIMAP_MARKER_SIZE_PX),
                        height: Val::Px(MINIMAP_MARKER_SIZE_PX),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::radians(angle)),
                    ZIndex(2),
                ))
                .with_children(|marker| {
                    marker.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Px(MINIMAP_MARKER_SIZE_PX),
                            height: Val::Px(MINIMAP_MARKER_SIZE_PX),
                            ..default()
                        },
                        ImageNode::new(icons.triangle.clone())
                            .with_color(minimap_player_border_color()),
                    ));
                    marker.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(fill_offset),
                            top: Val::Px(fill_offset),
                            width: Val::Px(fill_size),
                            height: Val::Px(fill_size),
                            ..default()
                        },
                        ImageNode::new(icons.triangle.clone()).with_color(minimap_player_color()),
                    ));
                })
                .id(),
        );
    });

    let Some(marker_entity) = marker_entity else {
        return;
    };
    commands.entity(marker_entity).try_insert(MinimapMarker {
        target,
        kind: MinimapMarkerKind::Player,
        dir_dot: None,
    });
}

fn spawn_minimap_enemy_marker(
    commands: &mut Commands,
    root: Entity,
    target: Entity,
    transform: &Transform,
    color: Color,
    context: &MinimapContext,
) {
    let (left, top, dir_left, dir_top, _) = minimap_layout_for_transform(transform, context);

    let mut marker_entity = None;
    let mut dir_dot_entity = None;
    commands.entity(root).with_children(|parent| {
        marker_entity = Some(
            parent
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(left),
                        top: Val::Px(top),
                        width: Val::Px(MINIMAP_MARKER_SIZE_PX),
                        height: Val::Px(MINIMAP_MARKER_SIZE_PX),
                        ..default()
                    },
                    UiTransform::default(),
                    ZIndex(1),
                ))
                .with_children(|marker| {
                    marker.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px((MINIMAP_MARKER_SIZE_PX - MINIMAP_DOT_SIZE_PX) * 0.5),
                            top: Val::Px((MINIMAP_MARKER_SIZE_PX - MINIMAP_DOT_SIZE_PX) * 0.5),
                            width: Val::Px(MINIMAP_DOT_SIZE_PX),
                            height: Val::Px(MINIMAP_DOT_SIZE_PX),
                            ..default()
                        },
                        BackgroundColor(color),
                    ));

                    dir_dot_entity = Some(
                        marker
                            .spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left: Val::Px(dir_left),
                                    top: Val::Px(dir_top),
                                    width: Val::Px(MINIMAP_DIR_DOT_SIZE_PX),
                                    height: Val::Px(MINIMAP_DIR_DOT_SIZE_PX),
                                    ..default()
                                },
                                BackgroundColor(Color::srgb(0.95, 0.95, 0.95)),
                                MinimapDirectionDot,
                            ))
                            .id(),
                    );
                })
                .id(),
        );
    });

    let Some(marker_entity) = marker_entity else {
        return;
    };
    let Some(dir_dot) = dir_dot_entity else {
        commands.entity(marker_entity).try_despawn();
        return;
    };
    commands.entity(marker_entity).try_insert(MinimapMarker {
        target,
        kind: MinimapMarkerKind::Enemy,
        dir_dot: Some(dir_dot),
    });
}

fn spawn_minimap_building_marker(
    commands: &mut Commands,
    root: Entity,
    target: Entity,
    transform: &Transform,
    collider: &AabbCollider,
    color: Color,
    context: &MinimapContext,
) {
    let (left, top, width, height) = minimap_building_layout(transform, collider, context);
    let marker_entity = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                width: Val::Px(width),
                height: Val::Px(height),
                ..default()
            },
            BackgroundColor(color),
            ZIndex(-1),
            MinimapBuilding { target },
        ))
        .id();
    commands.entity(root).add_children(&[marker_entity]);
}

fn minimap_layout_for_transform(
    transform: &Transform,
    context: &MinimapContext,
) -> (f32, f32, f32, f32, f32) {
    let world_pos = Vec2::new(transform.translation.x, transform.translation.z);
    let relative = world_pos - context.origin_world;
    let px = context.center_px.x + relative.dot(context.right_world) * context.units_to_px;
    let py = context.center_px.y - relative.dot(context.forward_world) * context.units_to_px;

    let half_marker = MINIMAP_MARKER_SIZE_PX * 0.5;
    let mut left = px - half_marker;
    let mut top = py - half_marker;
    let max = MINIMAP_BORDER_PX + context.inner_size - MINIMAP_MARKER_SIZE_PX;
    left = left.clamp(MINIMAP_BORDER_PX, max);
    top = top.clamp(MINIMAP_BORDER_PX, max);

    let mut forward = transform.rotation * Vec3::Z;
    forward.y = 0.0;
    let mut forward_xz = Vec2::new(forward.x, forward.z);
    if forward_xz.length_squared() <= 0.0001 {
        forward_xz = Vec2::new(0.0, 1.0);
    } else {
        forward_xz = forward_xz.normalize();
    }

    let dir_x = forward_xz.dot(context.right_world);
    let dir_y = forward_xz.dot(context.forward_world);
    let mut dir = Vec2::new(dir_x, -dir_y);
    if dir.length_squared() <= 0.0001 {
        dir = Vec2::new(0.0, -1.0);
    } else {
        dir = dir.normalize();
    }

    let center = Vec2::splat(MINIMAP_MARKER_SIZE_PX * 0.5);
    let dir_center = center + dir * MINIMAP_DIR_OFFSET_PX;
    let half_dir = MINIMAP_DIR_DOT_SIZE_PX * 0.5;
    let mut dir_left = dir_center.x - half_dir;
    let mut dir_top = dir_center.y - half_dir;
    let dir_max = MINIMAP_MARKER_SIZE_PX - MINIMAP_DIR_DOT_SIZE_PX;
    dir_left = dir_left.clamp(0.0, dir_max);
    dir_top = dir_top.clamp(0.0, dir_max);

    let angle = dir.x.atan2(-dir.y);

    (left, top, dir_left, dir_top, angle)
}

fn minimap_building_layout(
    transform: &Transform,
    collider: &AabbCollider,
    context: &MinimapContext,
) -> (f32, f32, f32, f32) {
    let world_pos = Vec2::new(transform.translation.x, transform.translation.z);
    let relative = world_pos - context.origin_world;
    let center_u = Vec2::new(
        relative.dot(context.right_world),
        relative.dot(context.forward_world),
    );

    let hx = collider.half_extents.x;
    let hz = collider.half_extents.y;
    let corners = [
        Vec2::new(-hx, -hz),
        Vec2::new(-hx, hz),
        Vec2::new(hx, -hz),
        Vec2::new(hx, hz),
    ];

    let mut min_px = Vec2::splat(f32::INFINITY);
    let mut max_px = Vec2::splat(f32::NEG_INFINITY);
    for corner in corners {
        let offset_u = Vec2::new(
            corner.dot(context.right_world),
            corner.dot(context.forward_world),
        );
        let corner_u = center_u + offset_u;
        let px = context.center_px.x + corner_u.x * context.units_to_px;
        let py = context.center_px.y - corner_u.y * context.units_to_px;
        min_px.x = min_px.x.min(px);
        min_px.y = min_px.y.min(py);
        max_px.x = max_px.x.max(px);
        max_px.y = max_px.y.max(py);
    }

    let width = (max_px.x - min_px.x).max(2.0);
    let height = (max_px.y - min_px.y).max(2.0);

    // Buildings should not be clamped/squeezed to the minimap edge. Let them fall outside and get
    // clipped by the minimap root's overflow setting.
    let left = min_px.x;
    let top = min_px.y;

    (left, top, width, height)
}

fn minimap_player_color() -> Color {
    Color::srgb(0.95, 0.85, 0.25)
}

fn minimap_player_border_color() -> Color {
    Color::srgb(0.95, 0.15, 0.18)
}
