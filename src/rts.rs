use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::collections::{HashMap, HashSet};

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::geometry::{point_inside_aabb_xz, safe_abs_scale_y, snap_to_grid};
use crate::navigation;
use crate::object::registry::ObjectLibrary;
use crate::object::types::effects as effect_types;
use crate::object::visuals;
use crate::types::*;

const SELECTION_DRAG_MIN_PX: f32 = 8.0;
const SELECTION_CLICK_RADIUS_PX: f32 = 26.0;
const SELECTION_RING_Y_OFFSET: f32 = 0.06;
const SELECTION_RING_SEGMENTS: usize = 32;
const MOVE_ENEMY_CLICK_RADIUS_PX: f32 = 30.0;

fn wrap_angle(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn turn_towards_yaw(current_yaw: f32, desired_yaw: f32, max_delta: f32) -> f32 {
    let delta = wrap_angle(desired_yaw - current_yaw);
    current_yaw + delta.clamp(-max_delta, max_delta)
}

fn world_to_screen(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    world: Vec3,
) -> Option<Vec2> {
    camera.world_to_viewport(camera_transform, world).ok()
}

fn ray_plane_intersection_y(ray: Ray3d, y: f32) -> Option<Vec3> {
    let origin = ray.origin;
    let direction = ray.direction;
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return None;
    }

    let t = (y - origin.y) / denom;
    if t < 0.0 {
        return None;
    }

    Some(origin + direction * t)
}

#[derive(Clone, Copy)]
struct MovePick {
    hit: Vec3,
    surface_y: f32,
    block_top: Option<(Vec2, Vec2)>,
}

fn cursor_move_pick(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
) -> Option<MovePick> {
    let cursor_pos = window.cursor_position()?;
    let ray = camera
        .viewport_to_world(camera_transform, cursor_pos)
        .ok()?;

    let origin = ray.origin;
    let direction = ray.direction.as_vec3();
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return None;
    }

    let mut best_t = f32::INFINITY;
    let mut pick = None;

    let t_ground = (0.0 - origin.y) / denom;
    if t_ground >= 0.0 {
        best_t = t_ground;
        pick = Some(MovePick {
            hit: origin + direction * t_ground,
            surface_y: 0.0,
            block_top: None,
        });
    }

    for (transform, collider, dimensions, prefab_id) in objects.iter() {
        if !library.interaction(prefab_id.0).supports_standing {
            continue;
        }

        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let top_y = transform.translation.y - origin_y + dimensions.size.y;
        let t = (top_y - origin.y) / denom;
        if t < 0.0 || t >= best_t {
            continue;
        }

        let hit = origin + direction * t;
        let point = Vec2::new(hit.x, hit.z);
        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if !point_inside_aabb_xz(point, center, collider.half_extents) {
            continue;
        }

        best_t = t;
        pick = Some(MovePick {
            hit,
            surface_y: top_y,
            block_top: Some((center, collider.half_extents)),
        });
    }

    pick
}

fn collect_nav_obstacles(
    objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    library: &ObjectLibrary,
) -> Vec<navigation::NavObstacle> {
    let mut obstacles = Vec::with_capacity(objects.iter().len());
    for (transform, collider, dimensions, prefab_id) in objects.iter() {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        obstacles.push(navigation::NavObstacle {
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
        });
    }
    obstacles
}

fn pick_enemy_under_cursor(
    cursor: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    enemies: &Query<(&Transform, &Enemy, &ObjectPrefabId), With<Enemy>>,
) -> Option<(Vec2, f32)> {
    let mut best: Option<(Vec2, f32)> = None;
    let mut best_d = f32::INFINITY;

    for (transform, enemy, prefab_id) in enemies.iter() {
        let scale_y = safe_abs_scale_y(transform.scale);
        let height = library
            .size(prefab_id.0)
            .map(|s| s.y * scale_y)
            .unwrap_or(HERO_HEIGHT_WORLD * scale_y);
        let world_pos = transform.translation + Vec3::Y * (height * 0.55);
        let Some(screen) = world_to_screen(camera, camera_transform, world_pos) else {
            continue;
        };
        let d = screen.distance(cursor);
        if d > MOVE_ENEMY_CLICK_RADIUS_PX {
            continue;
        }
        if d < best_d {
            best_d = d;
            let goal = Vec2::new(transform.translation.x, transform.translation.z);
            let ground_y = (transform.translation.y - enemy.origin_y * scale_y).max(0.0);
            best = Some((goal, ground_y));
        }
    }

    best
}

fn pick_enemy_entity_under_cursor(
    cursor: Vec2,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    enemies: &Query<(Entity, &Transform, &ObjectPrefabId), With<Enemy>>,
) -> Option<Entity> {
    let mut best = None;
    let mut best_d = f32::INFINITY;

    for (entity, transform, prefab_id) in enemies.iter() {
        let scale_y = safe_abs_scale_y(transform.scale);
        let height = library
            .size(prefab_id.0)
            .map(|s| s.y * scale_y)
            .unwrap_or(HERO_HEIGHT_WORLD * scale_y);
        let world_pos = transform.translation + Vec3::Y * (height * 0.55);
        let Some(screen) = world_to_screen(camera, camera_transform, world_pos) else {
            continue;
        };
        let d = screen.distance(cursor);
        if d > MOVE_ENEMY_CLICK_RADIUS_PX {
            continue;
        }
        if d < best_d {
            best_d = d;
            best = Some(entity);
        }
    }

    best
}

pub(crate) fn toggle_slow_move_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mut slow_move: ResMut<SlowMoveMode>,
) {
    if keys.just_pressed(KeyCode::CapsLock) {
        slow_move.enabled = !slow_move.enabled;
    }
}

pub(crate) fn selection_input(
    mut selection: ResMut<SelectionState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    build: Res<BuildState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    commandables: Query<
        (Entity, &Transform, Option<&Collider>, &ObjectPrefabId),
        With<Commandable>,
    >,
    build_objects: Query<(Entity, &Transform), With<BuildObject>>,
) {
    // While holding Space (fire), selection is disabled to avoid fighting the aim cursor.
    if keys.pressed(KeyCode::Space) {
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    }

    if matches!(mode.get(), GameMode::Build) && build.placing_active {
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);

    if mouse_buttons.just_pressed(MouseButton::Left) {
        selection.drag_start = Some(cursor);
        selection.drag_end = Some(cursor);
    }

    if mouse_buttons.pressed(MouseButton::Left) {
        if selection.drag_start.is_some() {
            selection.drag_end = Some(cursor);
        }
    }

    if !mouse_buttons.just_released(MouseButton::Left) {
        return;
    }

    let Some(start) = selection.drag_start.take() else {
        selection.drag_end = None;
        return;
    };
    let end = selection.drag_end.take().unwrap_or(start);

    let min = Vec2::new(start.x.min(end.x), start.y.min(end.y));
    let max = Vec2::new(start.x.max(end.x), start.y.max(end.y));
    let drag = (max - min).length();

    let mut candidates: Vec<(Entity, Vec3, bool)> = Vec::new();
    for (entity, transform, _collider, prefab_id) in &commandables {
        let height = library
            .size(prefab_id.0)
            .map(|s| s.y)
            .unwrap_or(HERO_HEIGHT_WORLD);
        candidates.push((
            entity,
            transform.translation + Vec3::Y * (height * 0.5),
            true,
        ));
    }
    if matches!(mode.get(), GameMode::Build) {
        for (entity, transform) in &build_objects {
            candidates.push((entity, transform.translation, false));
        }
    }

    if drag < SELECTION_DRAG_MIN_PX {
        let mut best_unit: Option<(Entity, f32)> = None;
        let mut best_other: Option<(Entity, f32)> = None;
        let camera_right = camera_transform.rotation * Vec3::X;

        for (entity, world_pos, is_unit) in candidates {
            let Some(screen) = world_to_screen(camera, &camera_global, world_pos) else {
                continue;
            };
            let d = screen.distance(cursor);

            if is_unit {
                let pixel_radius = commandables
                    .get(entity)
                    .ok()
                    .and_then(|(_e, transform, collider, _prefab_id)| {
                        collider.map(|c| (transform, c))
                    })
                    .and_then(|(transform, collider)| {
                        let scale = transform
                            .scale
                            .x
                            .abs()
                            .max(transform.scale.z.abs())
                            .max(1e-3);
                        let world_r = (collider.radius * scale).max(0.0);
                        if world_r <= 1e-6 {
                            return None;
                        }
                        let edge_world = world_pos + camera_right * world_r;
                        let edge_screen = world_to_screen(camera, &camera_global, edge_world)?;
                        Some(screen.distance(edge_screen).max(1.0))
                    })
                    .unwrap_or(SELECTION_CLICK_RADIUS_PX);

                if d > pixel_radius {
                    continue;
                }
                if best_unit.map(|(_, best_d)| d < best_d).unwrap_or(true) {
                    best_unit = Some((entity, d));
                }
            } else {
                if d > SELECTION_CLICK_RADIUS_PX {
                    continue;
                }
                if best_other.map(|(_, best_d)| d < best_d).unwrap_or(true) {
                    best_other = Some((entity, d));
                }
            }
        }

        if let Some((entity, _)) = best_unit {
            selection.selected.clear();
            selection.selected.insert(entity);
        } else if let Some((entity, _)) = best_other {
            selection.selected.insert(entity);
        } else {
            selection.selected.clear();
        }

        return;
    }

    let mut units: HashSet<Entity> = HashSet::new();
    let mut non_units: Vec<Entity> = Vec::new();
    for (entity, world_pos, is_unit) in candidates {
        let Some(screen) = world_to_screen(camera, &camera_global, world_pos) else {
            continue;
        };
        if screen.x >= min.x && screen.x <= max.x && screen.y >= min.y && screen.y <= max.y {
            if is_unit {
                units.insert(entity);
            } else {
                non_units.push(entity);
            }
        }
    }

    if !units.is_empty() {
        selection.selected = units;
        selection.selected.extend(non_units);
    } else if !non_units.is_empty() {
        selection.selected.extend(non_units);
    } else {
        selection.selected.clear();
    }
}

pub(crate) fn ensure_default_selection_on_enter_play(
    mut selection: ResMut<SelectionState>,
    player_q: Query<Entity, With<Player>>,
) {
    if !selection.selected.is_empty() {
        return;
    }
    let Ok(player) = player_q.single() else {
        return;
    };
    selection.selected.insert(player);
}

pub(crate) fn update_selection_box_ui(
    selection: Res<SelectionState>,
    mut q: Query<(&mut Node, &mut Visibility), With<SelectionBoxUi>>,
) {
    let Ok((mut node, mut visibility)) = q.single_mut() else {
        return;
    };

    let (Some(start), Some(end)) = (selection.drag_start, selection.drag_end) else {
        *visibility = Visibility::Hidden;
        return;
    };

    let min = Vec2::new(start.x.min(end.x), start.y.min(end.y));
    let max = Vec2::new(start.x.max(end.x), start.y.max(end.y));

    node.left = Val::Px(min.x);
    node.top = Val::Px(min.y);
    node.width = Val::Px((max.x - min.x).max(1.0));
    node.height = Val::Px((max.y - min.y).max(1.0));
    *visibility = Visibility::Inherited;
}

fn draw_circle_xz(gizmos: &mut Gizmos, center: Vec3, radius: f32, color: Color) {
    let radius = radius.max(0.01);
    let steps = SELECTION_RING_SEGMENTS.max(8);
    let mut prev = None;
    for i in 0..=steps {
        let t = (i as f32 / steps as f32) * std::f32::consts::TAU;
        let point = center + Vec3::new(t.cos() * radius, 0.0, t.sin() * radius);
        if let Some(prev) = prev {
            gizmos.line(prev, point, color);
        }
        prev = Some(point);
    }
}

pub(crate) fn draw_selected_player_gizmos(
    mut gizmos: Gizmos,
    selection: Res<SelectionState>,
    library: Res<ObjectLibrary>,
    commandables: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
        ),
        With<Commandable>,
    >,
) {
    for (entity, transform, collider, prefab_id, player) in &commandables {
        if !selection.selected.contains(&entity) {
            continue;
        }

        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = if player.is_some() {
            PLAYER_Y
        } else {
            library.ground_origin_y_or_default(prefab_id.0) * scale_y
        };
        let ground_y = (transform.translation.y - origin_y).max(0.0);
        let center = Vec3::new(
            transform.translation.x,
            ground_y + SELECTION_RING_Y_OFFSET,
            transform.translation.z,
        );
        draw_circle_xz(
            &mut gizmos,
            center,
            collider.radius * 1.45,
            Color::srgb(0.25, 0.95, 0.45),
        );
    }
}

pub(crate) fn move_command_input(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    build: Res<BuildState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    assets: Res<SceneAssets>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
    enemies: Query<(&Transform, &Enemy, &ObjectPrefabId), With<Enemy>>,
    players: Query<(), With<Player>>,
    commandables: Query<(Entity, &Transform, &Collider, &ObjectPrefabId), With<Commandable>>,
    selection: Res<SelectionState>,
    mut move_state: ResMut<MoveCommandState>,
) {
    if !mouse_buttons.just_pressed(MouseButton::Right) {
        return;
    }

    let Ok(window) = windows.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let camera_global = GlobalTransform::from(*camera_transform);

    // While placing build objects, RMB is reserved for quick-remove when clicking on an object.
    if matches!(mode.get(), GameMode::Build)
        && build.placing_active
        && !keys.pressed(KeyCode::Space)
    {
        if let Ok(ray) = camera.viewport_to_world(&camera_global, cursor) {
            if let Some(hit) = ray_plane_intersection_y(ray, 0.0) {
                let point = Vec2::new(hit.x, hit.z);
                for (transform, collider, _dimensions, _prefab) in objects.iter() {
                    let center = Vec2::new(transform.translation.x, transform.translation.z);
                    if point_inside_aabb_xz(point, center, collider.half_extents) {
                        return;
                    }
                }
            }
        }
    }

    if selection.selected.is_empty() {
        return;
    }

    let goal_pick = if matches!(mode.get(), GameMode::Play) {
        pick_enemy_under_cursor(cursor, camera, &camera_global, &library, &enemies).or_else(|| {
            cursor_move_pick(window, camera, &camera_global, &library, &objects).map(|pick| {
                let mut goal = Vec2::new(pick.hit.x, pick.hit.z);
                if let Some((center, half)) = pick.block_top {
                    let min_half = BUILD_UNIT_SIZE * 0.5;
                    if half.x > min_half && half.y > min_half {
                        goal.x = goal
                            .x
                            .clamp(center.x - half.x + min_half, center.x + half.x - min_half);
                        goal.y = goal
                            .y
                            .clamp(center.y - half.y + min_half, center.y + half.y - min_half);
                    }
                }
                (goal, pick.surface_y)
            })
        })
    } else {
        cursor_move_pick(window, camera, &camera_global, &library, &objects).map(|pick| {
            let mut goal = Vec2::new(pick.hit.x, pick.hit.z);
            if let Some((center, half)) = pick.block_top {
                let min_half = BUILD_UNIT_SIZE * 0.5;
                if half.x > min_half && half.y > min_half {
                    goal.x = goal
                        .x
                        .clamp(center.x - half.x + min_half, center.x + half.x - min_half);
                    goal.y = goal
                        .y
                        .clamp(center.y - half.y + min_half, center.y + half.y - min_half);
                }
            }
            (goal, pick.surface_y)
        })
    };

    let Some((goal, goal_ground_y)) = goal_pick else {
        return;
    };

    let obstacles = collect_nav_obstacles(&objects, &library);

    let mut any_order = false;
    for entity in selection.selected.iter().copied() {
        let Ok((_entity, transform, collider, prefab_id)) = commandables.get(entity) else {
            continue;
        };
        let Some(mobility) = library.mobility(prefab_id.0) else {
            continue;
        };

        let scale_y = safe_abs_scale_y(transform.scale);
        let radius = collider.radius.max(0.01);
        let min = Vec2::splat(-WORLD_HALF_SIZE + radius);
        let max = Vec2::splat(WORLD_HALF_SIZE - radius);
        let clamped_goal = goal.clamp(min, max);

        let start = Vec2::new(transform.translation.x, transform.translation.z);
        let origin_y = if players.contains(entity) {
            PLAYER_Y
        } else {
            library.ground_origin_y_or_default(prefab_id.0) * scale_y
        };
        let current_ground_y = (transform.translation.y - origin_y).max(0.0);
        let height = library
            .size(prefab_id.0)
            .map(|s| s.y * scale_y)
            .unwrap_or(HERO_HEIGHT_WORLD * scale_y);

        let mut order = MoveOrder::default();
        match mobility.mode {
            crate::object::registry::MobilityMode::Air => {
                order.target = Some(clamped_goal);
            }
            crate::object::registry::MobilityMode::Ground => {
                let Some(path) = navigation::find_path_height_aware(
                    start,
                    current_ground_y,
                    clamped_goal,
                    goal_ground_y,
                    radius,
                    height,
                    WORLD_HALF_SIZE,
                    NAV_GRID_SIZE,
                    &obstacles,
                ) else {
                    commands.entity(entity).remove::<MoveOrder>();
                    continue;
                };
                let path = navigation::smooth_path_height_aware(
                    start,
                    current_ground_y,
                    path,
                    radius,
                    height,
                    NAV_GRID_SIZE,
                    &obstacles,
                );
                order.path = path.into();
                order.target = Some(clamped_goal);
            }
        }

        commands.entity(entity).insert(order);
        any_order = true;
    }

    if !any_order {
        if let Some(marker) = move_state.marker.take() {
            commands.entity(marker).try_despawn();
        }
        return;
    }

    if let Some(marker) = move_state.marker.take() {
        commands.entity(marker).try_despawn();
    }

    let marker = commands
        .spawn((
            ObjectId::new_v4(),
            ObjectPrefabId(effect_types::move_target_marker::object_id()),
            Mesh3d(assets.move_target_mesh.clone()),
            MeshMaterial3d(assets.move_target_material.clone()),
            Transform::from_translation(Vec3::new(
                goal.x,
                goal_ground_y + MOVE_TARGET_MARKER_Y,
                goal.y,
            ))
            .with_scale(Vec3::new(
                MOVE_TARGET_MARKER_RADIUS,
                MOVE_TARGET_MARKER_HEIGHT,
                MOVE_TARGET_MARKER_RADIUS,
            )),
            Visibility::Inherited,
            MoveTargetMarker,
        ))
        .id();
    move_state.marker = Some(marker);
}

pub(crate) fn keyboard_move_input(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    slow_move: Res<SlowMoveMode>,
    mode: Res<State<GameMode>>,
    build: Res<BuildState>,
    mut move_state: ResMut<MoveCommandState>,
    camera_q: Query<&Transform, With<MainCamera>>,
    library: Res<ObjectLibrary>,
    selection: Res<SelectionState>,
    mut commandables: Query<
        (
            Entity,
            &mut Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&mut MoveOrder>,
        ),
        (With<Commandable>, Without<MainCamera>),
    >,
) {
    if matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    if matches!(mode.get(), GameMode::Build) && build.placing_active {
        return;
    }

    if selection.selected.is_empty() {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let Ok(camera_transform) = camera_q.single() else {
        return;
    };

    let mut camera_forward = camera_transform.rotation * Vec3::NEG_Z;
    camera_forward.y = 0.0;
    let forward_xz = if camera_forward.length_squared() > 1e-6 {
        Vec2::new(camera_forward.x, camera_forward.z).normalize()
    } else {
        Vec2::Y
    };
    let mut camera_right = camera_transform.rotation * Vec3::X;
    camera_right.y = 0.0;
    let right_xz = if camera_right.length_squared() > 1e-6 {
        Vec2::new(camera_right.x, camera_right.z).normalize()
    } else {
        Vec2::X
    };

    let mut dir = Vec2::ZERO;
    if keys.pressed(KeyCode::KeyW) {
        dir += forward_xz;
    }
    if keys.pressed(KeyCode::KeyS) {
        dir -= forward_xz;
    }
    if keys.pressed(KeyCode::KeyD) {
        dir += right_xz;
    }
    if keys.pressed(KeyCode::KeyA) {
        dir -= right_xz;
    }

    if dir.length_squared() <= 1e-6 {
        return;
    }
    let dir = dir.normalize();

    if let Some(marker) = move_state.marker.take() {
        commands.entity(marker).try_despawn();
    }

    for (entity, mut transform, collider, prefab_id, order) in &mut commandables {
        if !selection.selected.contains(&entity) {
            continue;
        }

        if let Some(mut order) = order {
            order.clear();
        }

        let Some(mobility) = library.mobility(prefab_id.0) else {
            continue;
        };
        let mut speed = mobility.max_speed.max(0.0);
        if slow_move.enabled {
            speed *= SLOW_MOVE_SPEED_MULTIPLIER;
        }
        if speed <= 0.001 {
            continue;
        }

        let step = dir * (speed * dt);
        let start_pos = Vec2::new(transform.translation.x, transform.translation.z);
        let mut pos = start_pos + step;

        let radius = collider.radius.max(0.01);
        pos.x = pos
            .x
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        pos.y = pos
            .y
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);

        transform.translation.x = pos.x;
        transform.translation.z = pos.y;

        let moved = pos - start_pos;
        if moved.length_squared() > 1e-8 {
            let moved_dir = moved.normalize();
            let desired_yaw = moved_dir.x.atan2(moved_dir.y);
            let forward = transform.rotation * Vec3::Z;
            let current_yaw = forward.x.atan2(forward.z);
            let max_delta = CLICK_MOVE_MAX_TURN_RATE_RADS_PER_SEC * dt;
            let new_yaw = turn_towards_yaw(current_yaw, desired_yaw, max_delta);
            transform.rotation = Quat::from_rotation_y(new_yaw);
        }
    }
}

pub(crate) fn unit_animation_hotkeys(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    build: Res<BuildState>,
    library: Res<ObjectLibrary>,
    selection: Res<SelectionState>,
    targets: Query<&ObjectPrefabId, Without<Player>>,
) {
    match mode.get() {
        GameMode::Play => {}
        GameMode::Build => {
            if build.placing_active {
                return;
            }
        }
        _ => return,
    }
    if selection.selected.is_empty() {
        return;
    }

    let modifier = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    if modifier {
        return;
    }

    let requested_slot =
        if keys.just_pressed(KeyCode::Digit1) || keys.just_pressed(KeyCode::Numpad1) {
            Some(0usize)
        } else if keys.just_pressed(KeyCode::Digit2) || keys.just_pressed(KeyCode::Numpad2) {
            Some(1)
        } else if keys.just_pressed(KeyCode::Digit3) || keys.just_pressed(KeyCode::Numpad3) {
            Some(2)
        } else if keys.just_pressed(KeyCode::Digit4) || keys.just_pressed(KeyCode::Numpad4) {
            Some(3)
        } else if keys.just_pressed(KeyCode::Digit5) || keys.just_pressed(KeyCode::Numpad5) {
            Some(4)
        } else if keys.just_pressed(KeyCode::Digit6) || keys.just_pressed(KeyCode::Numpad6) {
            Some(5)
        } else if keys.just_pressed(KeyCode::Digit7) || keys.just_pressed(KeyCode::Numpad7) {
            Some(6)
        } else if keys.just_pressed(KeyCode::Digit8) || keys.just_pressed(KeyCode::Numpad8) {
            Some(7)
        } else if keys.just_pressed(KeyCode::Digit9) || keys.just_pressed(KeyCode::Numpad9) {
            Some(8)
        } else if keys.just_pressed(KeyCode::Digit0) || keys.just_pressed(KeyCode::Numpad0) {
            Some(9)
        } else {
            None
        };

    let Some(slot_idx) = requested_slot else {
        return;
    };

    let wall_time = time.elapsed_secs();
    let mut channels_by_prefab: HashMap<u128, Vec<String>> = HashMap::new();

    for entity in selection.selected.iter().copied() {
        let Ok(prefab_id) = targets.get(entity) else {
            continue;
        };
        let channels = channels_by_prefab
            .entry(prefab_id.0)
            .or_insert_with(|| library.animation_channels_ordered_top10(prefab_id.0));
        let Some(channel) = channels.get(slot_idx) else {
            continue;
        };
        let channel = channel.trim();
        if channel.is_empty() {
            continue;
        }

        commands.entity(entity).insert(ForcedAnimationChannel {
            channel: channel.to_string(),
        });

        if let Some(duration_secs) = library.channel_attack_duration_secs(prefab_id.0, channel) {
            commands.entity(entity).insert(AttackClock {
                started_at_secs: wall_time,
                duration_secs,
            });
        }
    }
}

pub(crate) fn clear_forced_animation_channel_after_one_shot(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<State<GameMode>>,
    library: Res<ObjectLibrary>,
    q: Query<
        (
            Entity,
            &ObjectPrefabId,
            &ForcedAnimationChannel,
            &AttackClock,
        ),
        Without<Player>,
    >,
) {
    if !matches!(mode.get(), GameMode::Play | GameMode::Build) {
        return;
    }

    let wall_time = time.elapsed_secs();
    for (entity, prefab_id, forced, attack_clock) in &q {
        let channel = forced.channel.trim();
        if channel.is_empty() {
            continue;
        }

        if library
            .channel_attack_duration_secs(prefab_id.0, channel)
            .is_none()
        {
            continue;
        }

        let duration = attack_clock.duration_secs;
        if !duration.is_finite() || duration <= 0.0 {
            continue;
        }

        let elapsed = (wall_time - attack_clock.started_at_secs).max(0.0);
        if elapsed > duration {
            commands.entity(entity).remove::<ForcedAnimationChannel>();
        }
    }
}

pub(crate) fn build_unit_hotkeys(
    mut commands: Commands,
    mode: Res<State<GameMode>>,
    build: Res<BuildState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<SelectionState>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    library: Res<ObjectLibrary>,
    mut units: Query<
        (
            Entity,
            &mut Transform,
            &ObjectPrefabId,
            &mut Collider,
            Option<&ObjectTint>,
        ),
        (With<Commandable>, Without<Player>),
    >,
) {
    if !matches!(mode.get(), GameMode::Build) {
        return;
    }
    if build.placing_active {
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);

    let delete_pressed =
        keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace);
    if delete_pressed {
        let selected: Vec<Entity> = selection.selected.iter().copied().collect();
        for entity in selected {
            if !units.contains(entity) {
                continue;
            }
            commands.entity(entity).try_despawn();
            selection.selected.remove(&entity);
        }
        selection.drag_start = None;
        selection.drag_end = None;
    }

    let scale_step = if shift { 1.25 } else { 1.10 };
    let mut scale_mul = 1.0f32;
    if keys.just_pressed(KeyCode::Equal) {
        scale_mul *= scale_step;
    }
    if keys.just_pressed(KeyCode::Minus) {
        scale_mul /= scale_step;
    }
    if (scale_mul - 1.0).abs() > 1e-6 {
        let selected: Vec<Entity> = selection.selected.iter().copied().collect();
        for entity in selected {
            let Ok((_entity, mut transform, _prefab_id, mut collider, _tint)) =
                units.get_mut(entity)
            else {
                continue;
            };

            let mobility_mode = library
                .mobility(_prefab_id.0)
                .map(|mobility| mobility.mode)
                .unwrap_or(crate::object::registry::MobilityMode::Ground);
            let base_origin_y = library.ground_origin_y_or_default(_prefab_id.0);

            let current_scale_y = safe_abs_scale_y(transform.scale);
            let current_scale = transform
                .scale
                .x
                .abs()
                .max(transform.scale.y.abs())
                .max(transform.scale.z.abs())
                .max(1e-3);
            let new_scale = (current_scale * scale_mul).clamp(0.15, 10.0);
            let applied = new_scale / current_scale;

            transform.scale = Vec3::splat(new_scale);
            if mobility_mode == crate::object::registry::MobilityMode::Ground
                && transform.translation.y.is_finite()
            {
                let bottom_y = transform.translation.y - base_origin_y * current_scale_y;
                transform.translation.y = bottom_y + base_origin_y * new_scale;
            }

            if collider.radius.is_finite() {
                collider.radius = (collider.radius * applied).max(0.01);
            }

            let radius = collider.radius.max(0.01);
            transform.translation.x = transform
                .translation
                .x
                .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
            transform.translation.z = transform
                .translation
                .z
                .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        }
    }

    if !keys.just_pressed(KeyCode::KeyM) {
        return;
    }

    let snap_step = BUILD_GRID_SIZE.max(0.01);
    let offset_step = BUILD_UNIT_SIZE.max(snap_step);
    let offset = Vec3::new(offset_step, 0.0, offset_step);
    let mut new_selected: HashSet<Entity> = HashSet::new();
    let selected: Vec<Entity> = selection.selected.iter().copied().collect();
    for entity in selected {
        let Ok((_entity, transform, prefab_id, collider, tint)) = units.get_mut(entity) else {
            continue;
        };

        let mut new_transform = transform.clone();
        new_transform.translation += offset;
        new_transform.translation.x = snap_to_grid(new_transform.translation.x, snap_step);
        new_transform.translation.z = snap_to_grid(new_transform.translation.z, snap_step);

        let radius = collider.radius.max(0.01);
        new_transform.translation.x = new_transform
            .translation
            .x
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        new_transform.translation.z = new_transform
            .translation
            .z
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);

        let tint = tint.map(|t| t.0);
        let mut entity_commands = commands.spawn((
            ObjectId::new_v4(),
            *prefab_id,
            Commandable,
            Collider { radius },
            new_transform,
            Visibility::Inherited,
        ));
        if let Some(tint) = tint {
            entity_commands.insert(ObjectTint(tint));
        }
        visuals::spawn_object_visuals(
            &mut entity_commands,
            &library,
            &asset_server,
            &assets,
            &mut meshes,
            &mut materials,
            &mut material_cache,
            &mut mesh_cache,
            prefab_id.0,
            tint,
        );
        new_selected.insert(entity_commands.id());
    }

    if !new_selected.is_empty() {
        selection.selected = new_selected;
        selection.drag_start = None;
        selection.drag_end = None;
    }
}

pub(crate) fn execute_move_orders(
    mut commands: Commands,
    time: Res<Time>,
    slow_move: Res<SlowMoveMode>,
    mode: Res<State<GameMode>>,
    game: Res<Game>,
    library: Res<ObjectLibrary>,
    mut movers: Query<
        (
            Entity,
            &mut Transform,
            &Collider,
            &ObjectPrefabId,
            &mut MoveOrder,
        ),
        With<Commandable>,
    >,
    mut move_state: ResMut<MoveCommandState>,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let mut any_active_after = false;

    for (entity, mut transform, collider, prefab_id, mut order) in movers.iter_mut() {
        let Some(target) = order.target else {
            commands.entity(entity).remove::<MoveOrder>();
            continue;
        };

        let Some(mobility) = library.mobility(prefab_id.0) else {
            commands.entity(entity).remove::<MoveOrder>();
            continue;
        };

        let mut speed = mobility.max_speed.max(0.0);
        if slow_move.enabled {
            speed *= SLOW_MOVE_SPEED_MULTIPLIER;
        }
        if speed <= 0.001 {
            any_active_after = true;
            continue;
        }

        let start_pos = Vec2::new(transform.translation.x, transform.translation.z);
        let mut pos = start_pos;
        let mut desired_facing: Option<Vec2> = None;

        match mobility.mode {
            crate::object::registry::MobilityMode::Air => {
                let to = target - pos;
                let dist = to.length();
                if dist <= CLICK_MOVE_WAYPOINT_EPS {
                    order.clear();
                } else {
                    let step = (speed * dt).min(dist);
                    pos += to / dist * step;
                    desired_facing = Some(to / dist);
                }
            }
            crate::object::registry::MobilityMode::Ground => {
                let min_dist2 = CLICK_MOVE_WAYPOINT_EPS * CLICK_MOVE_WAYPOINT_EPS;
                for next in order.path.iter().copied() {
                    let to = next - pos;
                    if to.length_squared() > min_dist2 {
                        desired_facing = Some(to.normalize());
                        break;
                    }
                }

                let mut remaining = speed * dt;
                while remaining > 0.0 {
                    let Some(next) = order.path.front().copied() else {
                        break;
                    };

                    let to = next - pos;
                    let dist = to.length();
                    if dist <= CLICK_MOVE_WAYPOINT_EPS {
                        order.path.pop_front();
                        continue;
                    }

                    let step = remaining.min(dist);
                    pos += to / dist * step;
                    remaining -= step;

                    if step + CLICK_MOVE_WAYPOINT_EPS >= dist {
                        order.path.pop_front();
                    }
                }

                if order.path.is_empty() && order.target.is_some() {
                    order.clear();
                }
            }
        }

        let radius = collider.radius.max(0.01);
        pos.x = pos
            .x
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        pos.y = pos
            .y
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);

        transform.translation.x = pos.x;
        transform.translation.z = pos.y;

        let moved = pos - start_pos;
        let dir = if moved.length_squared() > 1e-8 {
            Some(moved.normalize())
        } else {
            desired_facing
        };

        if let Some(dir) = dir {
            if dir.length_squared() > 1e-6 {
                let desired_yaw = dir.x.atan2(dir.y);
                let forward = transform.rotation * Vec3::Z;
                let current_yaw = forward.x.atan2(forward.z);
                let max_delta = CLICK_MOVE_MAX_TURN_RATE_RADS_PER_SEC * dt;
                let yaw = turn_towards_yaw(current_yaw, desired_yaw, max_delta);
                transform.rotation = Quat::from_rotation_y(yaw);
            }
        }

        if order.target.is_some() {
            any_active_after = true;
        } else {
            commands.entity(entity).remove::<MoveOrder>();
        }
    }

    if !any_active_after {
        if let Some(marker) = move_state.marker.take() {
            commands.entity(marker).try_despawn();
        }
    }
}

pub(crate) fn update_fire_control(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    game: Res<Game>,
    selection: Res<SelectionState>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &Transform), With<MainCamera>>,
    library: Res<ObjectLibrary>,
    enemies: Query<(Entity, &Transform, &ObjectPrefabId), With<Enemy>>,
    mut fire: ResMut<FireControl>,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        fire.active = false;
        fire.target = None;
        return;
    }

    if !keys.pressed(KeyCode::Space) {
        fire.active = false;
        fire.target = None;
        return;
    }

    if selection.selected.is_empty() {
        fire.active = false;
        fire.target = None;
        return;
    }
    fire.active = true;

    fire.target = None;
    if let (Ok(window), Ok((camera, camera_transform))) = (windows.single(), camera_q.single()) {
        if let Some(cursor) = window.cursor_position() {
            let camera_global = GlobalTransform::from(*camera_transform);
            if let Some(enemy_entity) =
                pick_enemy_entity_under_cursor(cursor, camera, &camera_global, &library, &enemies)
            {
                fire.target = Some(FireTarget::Enemy(enemy_entity));
            } else if let Ok(ray) = camera.viewport_to_world(&camera_global, cursor) {
                if let Some(hit) = ray_plane_intersection_y(ray, 0.0) {
                    fire.target = Some(FireTarget::Point(Vec2::new(hit.x, hit.z)));
                }
            }
        }
    }

    if let Some(FireTarget::Enemy(enemy)) = fire.target {
        if enemies.get(enemy).is_err() {
            fire.target = None;
        }
    }
}

pub(crate) fn update_unit_aim_yaw_delta(
    mut commands: Commands,
    fire: Res<FireControl>,
    selection: Res<SelectionState>,
    library: Res<ObjectLibrary>,
    enemies: Query<&Transform, (With<Enemy>, Without<Commandable>)>,
    mut commandables: Query<
        (
            Entity,
            &Transform,
            &ObjectPrefabId,
            Option<&crate::types::AimYawDelta>,
        ),
        With<Commandable>,
    >,
) {
    let aiming = fire.active && fire.target.is_some() && !selection.selected.is_empty();

    for (entity, transform, prefab_id, existing) in commandables.iter_mut() {
        let selected = selection.selected.contains(&entity);
        if !aiming || !selected {
            if existing.is_some() {
                commands
                    .entity(entity)
                    .remove::<crate::types::AimYawDelta>();
            }
            continue;
        }

        let Some(def) = library.get(prefab_id.0) else {
            continue;
        };
        let Some(attack) = def.attack.as_ref() else {
            if existing.is_some() {
                commands
                    .entity(entity)
                    .remove::<crate::types::AimYawDelta>();
            }
            continue;
        };

        let origin = Vec2::new(transform.translation.x, transform.translation.z);
        let Some(target) = fire.target else {
            continue;
        };
        let dir2 = match target {
            FireTarget::Point(point) => point - origin,
            FireTarget::Enemy(enemy_entity) => enemies
                .get(enemy_entity)
                .ok()
                .map(|enemy_transform| {
                    Vec2::new(enemy_transform.translation.x, enemy_transform.translation.z) - origin
                })
                .unwrap_or(Vec2::ZERO),
        };

        let delta = if dir2.length_squared() <= 1e-6 {
            0.0
        } else {
            let desired_yaw = dir2.x.atan2(dir2.y);
            let forward = transform.rotation * Vec3::Z;
            let body_yaw = forward.x.atan2(forward.z);
            wrap_angle(desired_yaw - body_yaw)
        };

        let max_delta_rads = match def.aim.as_ref() {
            Some(aim) => aim
                .max_yaw_delta_degrees
                .map(|deg| deg.abs().to_radians())
                .filter(|rads| rads.is_finite()),
            None => match attack.kind {
                crate::object::registry::UnitAttackKind::RangedProjectile => None,
                crate::object::registry::UnitAttackKind::Melee => Some(120.0_f32.to_radians()),
            },
        };

        let delta = match max_delta_rads {
            None => delta,
            Some(max) => {
                let max = max.clamp(0.0, std::f32::consts::PI);
                delta.clamp(-max, max)
            }
        };

        commands
            .entity(entity)
            .insert(crate::types::AimYawDelta(delta));
    }
}
