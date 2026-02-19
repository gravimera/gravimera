use bevy::prelude::*;
use std::collections::HashSet;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::geometry::{
    aabbs_intersect_xz, circle_intersects_aabb_xz, point_inside_aabb_xz, snap_to_grid,
};
use crate::object::registry::{ColliderProfile, ObjectLibrary};
use crate::object::types::buildings;
use crate::object::visuals;
use crate::types::*;

fn build_spec(build: &BuildState) -> BuildPreviewSpec {
    BuildPreviewSpec {
        kind: build.selected,
        fence_axis: build.fence_axis,
        tree_variant: if build.selected == BuildObjectKind::Tree {
            build.tree_variant
        } else {
            0
        },
    }
}

fn build_object_size(spec: BuildPreviewSpec) -> Vec3 {
    match spec.kind {
        BuildObjectKind::Block => BUILD_BLOCK_SIZE,
        BuildObjectKind::Fence => match spec.fence_axis {
            FenceAxis::X => Vec3::new(BUILD_FENCE_LENGTH, BUILD_FENCE_HEIGHT, BUILD_FENCE_WIDTH),
            FenceAxis::Z => Vec3::new(BUILD_FENCE_WIDTH, BUILD_FENCE_HEIGHT, BUILD_FENCE_LENGTH),
        },
        BuildObjectKind::Tree => {
            let index = spec.tree_variant as usize % BUILD_TREE_VARIANT_SCALES.len();
            BUILD_TREE_BASE_SIZE * BUILD_TREE_VARIANT_SCALES[index]
        }
    }
}

fn build_object_collider_half_xz(spec: BuildPreviewSpec, size: Vec3) -> Vec2 {
    match spec.kind {
        BuildObjectKind::Tree => {
            let index = spec.tree_variant as usize % BUILD_TREE_VARIANT_SCALES.len();
            let scale = BUILD_TREE_VARIANT_SCALES[index];
            let trunk_radius = BUILD_UNIT_SIZE * 0.55 * scale;
            Vec2::splat(trunk_radius)
        }
        _ => Vec2::new(size.x * 0.5, size.z * 0.5),
    }
}

fn snapped_center_xz(cursor_hit: Vec3, half: Vec2) -> Vec2 {
    let mut x = snap_to_grid(cursor_hit.x, BUILD_GRID_SIZE);
    let mut z = snap_to_grid(cursor_hit.z, BUILD_GRID_SIZE);
    x = x.clamp(-WORLD_HALF_SIZE + half.x, WORLD_HALF_SIZE - half.x);
    z = z.clamp(-WORLD_HALF_SIZE + half.y, WORLD_HALF_SIZE - half.y);
    Vec2::new(x, z)
}

fn aabbs_intersect_3d(a_center: Vec3, a_half: Vec3, b_center: Vec3, b_half: Vec3) -> bool {
    const EPS: f32 = 1e-4;
    let delta = a_center - b_center;
    delta.x.abs() < (a_half.x + b_half.x) - EPS
        && delta.y.abs() < (a_half.y + b_half.y) - EPS
        && delta.z.abs() < (a_half.z + b_half.z) - EPS
}

#[derive(Clone, Copy)]
struct ExistingBuildObject {
    kind: BuildObjectKind,
    center: Vec3,
    half_xz: Vec2,
    half_y: f32,
    fence_axis: Option<FenceAxis>,
}

fn is_space_free(center: Vec3, half: Vec3, existing: &[ExistingBuildObject]) -> bool {
    for object in existing {
        let other_half = Vec3::new(object.half_xz.x, object.half_y, object.half_xz.y);
        if aabbs_intersect_3d(center, half, object.center, other_half) {
            return false;
        }
    }
    true
}

fn find_block_center_y(
    center_xz: Vec2,
    half_xz: Vec2,
    size_y: f32,
    existing: &[ExistingBuildObject],
) -> Option<f32> {
    let mut support_levels: Vec<f32> = vec![0.0];
    for object in existing {
        if object.kind != BuildObjectKind::Block {
            continue;
        }
        let other_center = Vec2::new(object.center.x, object.center.z);
        if !aabbs_intersect_xz(center_xz, half_xz, other_center, object.half_xz) {
            continue;
        }
        support_levels.push(object.center.y + object.half_y);
    }

    support_levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    support_levels.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);

    let half_y = size_y * 0.5;
    let half = Vec3::new(half_xz.x, half_y, half_xz.y);

    for support_y in support_levels.into_iter().rev() {
        let center_y = support_y + half_y;
        let center = Vec3::new(center_xz.x, center_y, center_xz.y);
        if !is_space_free(center, half, existing) {
            continue;
        }
        if support_y <= 1e-4 {
            return Some(center_y);
        }

        let mut supported_cells = 0u8;
        let mut support_blocks = 0u16;
        for object in existing {
            if object.kind != BuildObjectKind::Block {
                continue;
            }
            let top_y = object.center.y + object.half_y;
            if (top_y - support_y).abs() > 1e-4 {
                continue;
            }
            support_blocks += 1;
        }
        if support_blocks == 0 {
            continue;
        }

        for dx in [-1.0, 0.0, 1.0] {
            for dz in [-1.0, 0.0, 1.0] {
                let point = center_xz + Vec2::new(dx * BUILD_GRID_SIZE, dz * BUILD_GRID_SIZE);
                let mut supported = false;
                for object in existing {
                    if object.kind != BuildObjectKind::Block {
                        continue;
                    }
                    let top_y = object.center.y + object.half_y;
                    if (top_y - support_y).abs() > 1e-4 {
                        continue;
                    }
                    let other_center = Vec2::new(object.center.x, object.center.z);
                    if point_inside_aabb_xz(point, other_center, object.half_xz) {
                        supported = true;
                        break;
                    }
                }
                if supported {
                    supported_cells += 1;
                }
            }
        }

        if supported_cells >= 4 {
            return Some(center_y);
        }
    }

    None
}

fn fence_basis(axis: FenceAxis, along: f32, y: f32, across: f32) -> Vec3 {
    match axis {
        FenceAxis::X => Vec3::new(along, y, across),
        FenceAxis::Z => Vec3::new(across, y, along),
    }
}

fn find_fence_center_y(
    center_xz: Vec2,
    axis: FenceAxis,
    size_xz: Vec2,
    size_y: f32,
    existing: &[ExistingBuildObject],
) -> Option<f32> {
    let half_y = size_y * 0.5;
    let half = Vec3::new(size_xz.x * 0.5, half_y, size_xz.y * 0.5);

    let stake_thick = BUILD_FENCE_WIDTH * 0.85;
    let stake_offset = BUILD_FENCE_LENGTH * 0.5 - stake_thick * 0.5;
    let stake_left = center_xz
        + match axis {
            FenceAxis::X => Vec2::new(-stake_offset, 0.0),
            FenceAxis::Z => Vec2::new(0.0, -stake_offset),
        };
    let stake_right = center_xz
        + match axis {
            FenceAxis::X => Vec2::new(stake_offset, 0.0),
            FenceAxis::Z => Vec2::new(0.0, stake_offset),
        };

    let mut aligned_fence_levels: Vec<f32> = Vec::new();
    for object in existing {
        if object.kind != BuildObjectKind::Fence {
            continue;
        }
        if object.fence_axis != Some(axis) {
            continue;
        }
        let other = Vec2::new(object.center.x, object.center.z);
        if (other - center_xz).length_squared() > 1e-6 {
            continue;
        }
        aligned_fence_levels.push(object.center.y + object.half_y);
    }

    let mut stake_left_levels: HashSet<i32> = HashSet::new();
    let mut stake_right_levels: HashSet<i32> = HashSet::new();
    stake_left_levels.insert(0);
    stake_right_levels.insert(0);

    for object in existing {
        if object.kind != BuildObjectKind::Block {
            continue;
        }
        let top_y = object.center.y + object.half_y;
        let units = (top_y / BUILD_GRID_SIZE).round() as i32;
        let other = Vec2::new(object.center.x, object.center.z);
        if point_inside_aabb_xz(stake_left, other, object.half_xz) {
            stake_left_levels.insert(units);
        }
        if point_inside_aabb_xz(stake_right, other, object.half_xz) {
            stake_right_levels.insert(units);
        }
    }

    let mut candidates: Vec<f32> = vec![0.0];
    for level in aligned_fence_levels {
        candidates.push(level);
    }
    for units in stake_left_levels.intersection(&stake_right_levels) {
        candidates.push(*units as f32 * BUILD_GRID_SIZE);
    }

    candidates.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    candidates.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);

    for support_y in candidates.into_iter().rev() {
        let center_y = support_y + half_y;
        let center = Vec3::new(center_xz.x, center_y, center_xz.y);
        if !is_space_free(center, half, existing) {
            continue;
        }
        return Some(center_y);
    }

    None
}

fn find_tree_center_y(
    center_xz: Vec2,
    stump_half_xz: Vec2,
    size_y: f32,
    existing: &[ExistingBuildObject],
) -> Option<f32> {
    let radius = stump_half_xz.x.max(stump_half_xz.y);
    let half_y = size_y * 0.5;
    let half = Vec3::new(stump_half_xz.x, half_y, stump_half_xz.y);

    let mut support_levels: Vec<f32> = vec![0.0];
    for object in existing {
        if object.kind != BuildObjectKind::Block {
            continue;
        }

        let other_center = Vec2::new(object.center.x, object.center.z);
        if circle_intersects_aabb_xz(center_xz, radius, other_center, object.half_xz) {
            support_levels.push(object.center.y + object.half_y);
        }
    }

    support_levels.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    support_levels.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);

    for support_y in support_levels.into_iter().rev() {
        let center_y = support_y + half_y;
        let center = Vec3::new(center_xz.x, center_y, center_xz.y);
        if !is_space_free(center, half, existing) {
            continue;
        }

        return Some(center_y);
    }

    None
}

pub(crate) fn spawn_build_object_from_spec(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    spec: BuildPreviewSpec,
    transform: Transform,
    object_id: ObjectId,
) -> Entity {
    let prefab_id = buildings::prefab_id_from_build_spec(spec);
    let size = library
        .size(prefab_id)
        .unwrap_or_else(|| build_object_size(spec));
    let collider_half_xz = match library.collider(prefab_id) {
        Some(ColliderProfile::AabbXZ { half_extents }) => half_extents,
        Some(ColliderProfile::CircleXZ { radius }) => Vec2::splat(radius),
        _ => build_object_collider_half_xz(spec, size),
    };

    spawn_build_object_with_collider_half_xz(
        commands,
        asset_server,
        assets,
        meshes,
        materials,
        material_cache,
        mesh_cache,
        library,
        prefab_id,
        size,
        collider_half_xz,
        transform,
        object_id,
        None,
    )
}

fn spawn_build_object_with_collider_half_xz(
    commands: &mut Commands,
    asset_server: &AssetServer,
    assets: &SceneAssets,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    material_cache: &mut visuals::MaterialCache,
    mesh_cache: &mut visuals::PrimitiveMeshCache,
    library: &ObjectLibrary,
    prefab_id: u128,
    size: Vec3,
    collider_half_xz: Vec2,
    transform: Transform,
    object_id: ObjectId,
    tint: Option<Color>,
) -> Entity {
    let mut entity_commands = commands.spawn((
        object_id,
        ObjectPrefabId(prefab_id),
        BuildObject,
        BuildDimensions { size },
        AabbCollider {
            half_extents: collider_half_xz,
        },
        transform,
        Visibility::Inherited,
    ));
    if let Some(tint) = tint {
        entity_commands.insert(ObjectTint(tint));
    }
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
        tint,
    );
    entity_commands.id()
}

pub(crate) fn toggle_game_mode(
    keys: Res<ButtonInput<KeyCode>>,
    mode: Res<State<GameMode>>,
    mut next_mode: ResMut<NextState<GameMode>>,
) {
    if keys.just_pressed(KeyCode::Tab) {
        match mode.get() {
            GameMode::Build => next_mode.set(GameMode::Play),
            GameMode::Play => next_mode.set(GameMode::Build),
        }
    }
}

pub(crate) fn enter_build_mode(
    mut commands: Commands,
    enemies: Query<Entity, With<Enemy>>,
    bullets: Query<Entity, With<Bullet>>,
    enemy_projectiles: Query<Entity, With<EnemyProjectile>>,
    lasers: Query<Entity, With<Laser>>,
    explosions: Query<Entity, With<ExplosionParticle>>,
    mut build: ResMut<BuildState>,
    mut selection: ResMut<SelectionState>,
    mut preview: ResMut<BuildPreview>,
    player: Query<Entity, With<Player>>,
) {
    build.placing_active = false;
    selection.clear();
    preview.visible = false;
    preview.spec = None;
    if let Ok(entity) = player.single() {
        selection.selected.insert(entity);
    }

    for entity in &enemies {
        commands.entity(entity).try_despawn();
    }
    for entity in &bullets {
        commands.entity(entity).try_despawn();
    }
    for entity in &enemy_projectiles {
        commands.entity(entity).try_despawn();
    }
    for entity in &lasers {
        commands.entity(entity).try_despawn();
    }
    for entity in &explosions {
        commands.entity(entity).try_despawn();
    }
}

pub(crate) fn enter_play_mode(
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut next_build_scene: ResMut<NextState<BuildScene>>,
    lasers: Query<Entity, With<Laser>>,
    enemy_projectiles: Query<Entity, With<EnemyProjectile>>,
    explosions: Query<Entity, With<ExplosionParticle>>,
    mut selection: ResMut<SelectionState>,
    mut preview: ResMut<BuildPreview>,
    player: Query<Entity, With<Player>>,
) {
    next_build_scene.set(BuildScene::Realm);

    game.enemy_spawn.reset();
    game.fire_cooldown_secs = 0.0;

    selection.clear();
    preview.visible = false;
    preview.spec = None;
    if let Ok(entity) = player.single() {
        selection.selected.insert(entity);
    }
    for entity in &lasers {
        commands.entity(entity).try_despawn();
    }
    for entity in &enemy_projectiles {
        commands.entity(entity).try_despawn();
    }
    for entity in &explosions {
        commands.entity(entity).try_despawn();
    }
}

pub(crate) fn build_cancel_preview_and_clear_selection(
    keys: Res<ButtonInput<KeyCode>>,
    mut build: ResMut<BuildState>,
    mut selection: ResMut<SelectionState>,
) {
    if !keys.just_pressed(KeyCode::Escape) {
        return;
    }

    build.placing_active = false;
    selection.clear();
}

pub(crate) fn build_select_object(
    keys: Res<ButtonInput<KeyCode>>,
    mut build: ResMut<BuildState>,
    mut selection: ResMut<SelectionState>,
    player: Query<Entity, With<Player>>,
) {
    if keys.just_pressed(KeyCode::KeyB) {
        build.selected = BuildObjectKind::Block;
        build.placing_active = true;
        selection.drag_start = None;
        selection.drag_end = None;
    } else if keys.just_pressed(KeyCode::KeyF) {
        build.selected = BuildObjectKind::Fence;
        build.placing_active = true;
        selection.drag_start = None;
        selection.drag_end = None;
    } else if keys.just_pressed(KeyCode::KeyT) {
        build.selected = BuildObjectKind::Tree;
        build.placing_active = true;
        selection.drag_start = None;
        selection.drag_end = None;
    } else {
        return;
    }

    if let Ok(entity) = player.single() {
        selection.selected.insert(entity);
    }
}

pub(crate) fn build_toggle_fence_axis(
    keys: Res<ButtonInput<KeyCode>>,
    mut build: ResMut<BuildState>,
) {
    if !keys.just_pressed(KeyCode::KeyG) {
        return;
    }

    match build.selected {
        BuildObjectKind::Fence => {
            build.fence_axis = build.fence_axis.toggle();
        }
        BuildObjectKind::Tree => {
            // Roll-robin: medium -> small -> huge.
            build.tree_variant = match build.tree_variant {
                1 => 0,
                0 => 2,
                _ => 1,
            };
        }
        _ => {}
    }
}

pub(crate) fn build_place_object(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    aim: Res<Aim>,
    build: Res<BuildState>,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material_cache: ResMut<visuals::MaterialCache>,
    mut mesh_cache: ResMut<visuals::PrimitiveMeshCache>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (
            Entity,
            &Transform,
            &AabbCollider,
            &BuildDimensions,
            &ObjectPrefabId,
        ),
        (
            With<BuildObject>,
            Without<BuildPreviewMarker>,
            Without<Player>,
        ),
    >,
    mut player: Query<(&mut Transform, &Collider), With<Player>>,
    enemies: Query<(&Transform, &Collider), (With<Enemy>, Without<Player>)>,
) {
    if !build.placing_active {
        return;
    }
    // While firing (Space held), don't place objects.
    if keys.pressed(KeyCode::Space) {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Left) {
        return;
    }
    if !aim.has_cursor_hit {
        return;
    }

    let spec = build_spec(&build);
    let prefab_id = buildings::prefab_id_from_build_spec(spec);
    let size = library
        .size(prefab_id)
        .unwrap_or_else(|| build_object_size(spec));
    let clamp_half_xz = Vec2::new(size.x * 0.5, size.z * 0.5);
    let collider_half_xz = match library.collider(prefab_id) {
        Some(ColliderProfile::AabbXZ { half_extents }) => half_extents,
        Some(ColliderProfile::CircleXZ { radius }) => Vec2::splat(radius),
        _ => build_object_collider_half_xz(spec, size),
    };
    let center_xz = snapped_center_xz(aim.cursor_hit, clamp_half_xz);

    let mut existing: Vec<ExistingBuildObject> = Vec::new();
    existing.reserve(objects.iter().len());
    for (_entity, transform, collider, dimensions, prefab_id) in &objects {
        let kind = buildings::build_spec_from_prefab_id(prefab_id.0)
            .map(|spec| spec.kind)
            .unwrap_or(BuildObjectKind::Block);
        let fence_axis = buildings::fence_axis_from_prefab_id(prefab_id.0);
        existing.push(ExistingBuildObject {
            kind,
            center: transform.translation,
            half_xz: collider.half_extents,
            half_y: dimensions.size.y * 0.5,
            fence_axis,
        });
    }

    let center_y = match spec.kind {
        BuildObjectKind::Block => {
            find_block_center_y(center_xz, collider_half_xz, size.y, &existing)
        }
        BuildObjectKind::Fence => find_fence_center_y(
            center_xz,
            spec.fence_axis,
            Vec2::new(size.x, size.z),
            size.y,
            &existing,
        ),
        BuildObjectKind::Tree => find_tree_center_y(center_xz, collider_half_xz, size.y, &existing),
    };
    let Some(center_y) = center_y else {
        return;
    };

    let center = Vec3::new(center_xz.x, center_y, center_xz.y);
    let half = Vec3::new(collider_half_xz.x, size.y * 0.5, collider_half_xz.y);
    if !is_space_free(center, half, &existing) {
        return;
    }

    if let Ok((player_transform, player_collider)) = player.single_mut() {
        let player_center = Vec2::new(
            player_transform.translation.x,
            player_transform.translation.z,
        );
        if circle_intersects_aabb_xz(
            player_center,
            player_collider.radius,
            center_xz,
            collider_half_xz,
        ) {
            return;
        }
    }
    for (enemy_transform, enemy_collider) in &enemies {
        let enemy_center = Vec2::new(enemy_transform.translation.x, enemy_transform.translation.z);
        if circle_intersects_aabb_xz(
            enemy_center,
            enemy_collider.radius,
            center_xz,
            collider_half_xz,
        ) {
            return;
        }
    }

    let object_id = ObjectId::new_v4();
    let _entity = spawn_build_object_from_spec(
        &mut commands,
        &asset_server,
        &assets,
        &mut meshes,
        &mut materials,
        &mut material_cache,
        &mut mesh_cache,
        &library,
        spec,
        Transform::from_translation(center),
        object_id,
    );

    // Tree type is selected via F key; placing does not auto-cycle.

    if let Ok((mut player_transform, _collider)) = player.single_mut() {
        let to = Vec3::new(
            center.x - player_transform.translation.x,
            0.0,
            center.z - player_transform.translation.z,
        );
        if to.length_squared() > 1e-6 {
            player_transform.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
        }
    }
}

pub(crate) fn build_remove_object(
    mut commands: Commands,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    aim: Res<Aim>,
    build: Res<BuildState>,
    objects: Query<(Entity, &Transform, &AabbCollider), (With<BuildObject>, Without<Player>)>,
    mut player_q: Query<&mut Transform, With<Player>>,
) {
    if !build.placing_active {
        return;
    }
    // While firing (Space held), RMB remains a move-command and should not quick-remove buildings.
    if keys.pressed(KeyCode::Space) {
        return;
    }
    if !mouse_buttons.just_pressed(MouseButton::Right) {
        return;
    }
    if !aim.has_cursor_hit {
        return;
    }

    let point = Vec2::new(aim.cursor_hit.x, aim.cursor_hit.z);
    let mut best: Option<(Entity, f32, f32, Vec3)> = None;

    for (entity, transform, collider) in &objects {
        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if !point_inside_aabb_xz(point, center, collider.half_extents) {
            continue;
        }

        let d2 = (point - center).length_squared();
        let y = transform.translation.y;
        let choose = match best {
            None => true,
            Some((_best_entity, best_d2, best_y, _best_pos)) => {
                d2 < best_d2 - 1e-6 || ((d2 - best_d2).abs() <= 1e-6 && y > best_y)
            }
        };
        if choose {
            best = Some((entity, d2, y, transform.translation));
        }
    }

    if let Some((entity, _d2, _y, pos)) = best {
        commands.entity(entity).try_despawn();
        if let Ok(mut player_transform) = player_q.single_mut() {
            let to = Vec3::new(
                pos.x - player_transform.translation.x,
                0.0,
                pos.z - player_transform.translation.z,
            );
            if to.length_squared() > 1e-6 {
                player_transform.rotation = Quat::from_rotation_y(to.x.atan2(to.z));
            }
        }
    }
}

pub(crate) fn build_update_preview(
    build: Res<BuildState>,
    aim: Res<Aim>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (
            Entity,
            &Transform,
            &AabbCollider,
            &BuildDimensions,
            &ObjectPrefabId,
        ),
        (With<BuildObject>, Without<BuildPreviewMarker>),
    >,
    mut preview: ResMut<BuildPreview>,
) {
    if !build.placing_active {
        preview.visible = false;
        preview.spec = None;
        return;
    }

    let spec = build_spec(&build);
    let prefab_id = buildings::prefab_id_from_build_spec(spec);

    let Some(hit) = aim.has_cursor_hit.then(|| aim.cursor_hit) else {
        preview.visible = false;
        return;
    };

    let size = library
        .size(prefab_id)
        .unwrap_or_else(|| build_object_size(spec));
    let clamp_half_xz = Vec2::new(size.x * 0.5, size.z * 0.5);
    let collider_half_xz = match library.collider(prefab_id) {
        Some(ColliderProfile::AabbXZ { half_extents }) => half_extents,
        Some(ColliderProfile::CircleXZ { radius }) => Vec2::splat(radius),
        _ => build_object_collider_half_xz(spec, size),
    };
    let center_xz = snapped_center_xz(hit, clamp_half_xz);

    let mut existing: Vec<ExistingBuildObject> = Vec::new();
    existing.reserve(objects.iter().len());
    for (_entity, transform, collider, dimensions, prefab_id) in &objects {
        let kind = buildings::build_spec_from_prefab_id(prefab_id.0)
            .map(|spec| spec.kind)
            .unwrap_or(BuildObjectKind::Block);
        let fence_axis = buildings::fence_axis_from_prefab_id(prefab_id.0);
        existing.push(ExistingBuildObject {
            kind,
            center: transform.translation,
            half_xz: collider.half_extents,
            half_y: dimensions.size.y * 0.5,
            fence_axis,
        });
    }

    let center_y = match spec.kind {
        BuildObjectKind::Block => {
            find_block_center_y(center_xz, collider_half_xz, size.y, &existing)
        }
        BuildObjectKind::Fence => find_fence_center_y(
            center_xz,
            spec.fence_axis,
            Vec2::new(size.x, size.z),
            size.y,
            &existing,
        ),
        BuildObjectKind::Tree => find_tree_center_y(center_xz, collider_half_xz, size.y, &existing),
    };
    let Some(center_y) = center_y else {
        preview.visible = false;
        return;
    };

    preview.spec = Some(spec);
    preview.center = Vec3::new(center_xz.x, center_y, center_xz.y);
    preview.visible = true;
}

fn draw_box_outline(gizmos: &mut Gizmos, center: Vec3, half: Vec3, color: Color) {
    let min = center - half;
    let max = center + half;

    let c0 = Vec3::new(min.x, min.y, min.z);
    let c1 = Vec3::new(max.x, min.y, min.z);
    let c2 = Vec3::new(max.x, min.y, max.z);
    let c3 = Vec3::new(min.x, min.y, max.z);
    let c4 = Vec3::new(min.x, max.y, min.z);
    let c5 = Vec3::new(max.x, max.y, min.z);
    let c6 = Vec3::new(max.x, max.y, max.z);
    let c7 = Vec3::new(min.x, max.y, max.z);

    gizmos.line(c0, c1, color);
    gizmos.line(c1, c2, color);
    gizmos.line(c2, c3, color);
    gizmos.line(c3, c0, color);

    gizmos.line(c4, c5, color);
    gizmos.line(c5, c6, color);
    gizmos.line(c6, c7, color);
    gizmos.line(c7, c4, color);

    gizmos.line(c0, c4, color);
    gizmos.line(c1, c5, color);
    gizmos.line(c2, c6, color);
    gizmos.line(c3, c7, color);
}

pub(crate) fn build_draw_preview_gizmos(
    time: Res<Time>,
    build: Res<BuildState>,
    preview: Res<BuildPreview>,
    mut gizmos: Gizmos,
) {
    if !build.placing_active {
        return;
    }
    if !preview.visible {
        return;
    }

    let Some(spec) = preview.spec else {
        return;
    };

    let wave = (time.elapsed_secs() * 6.0).sin() * 0.5 + 0.5;
    let alpha = 0.35 + wave * 0.45;
    let color = Color::srgba(0.95, 0.85, 0.25, alpha);

    match spec.kind {
        BuildObjectKind::Block => {
            draw_box_outline(&mut gizmos, preview.center, BUILD_BLOCK_SIZE * 0.5, color);
        }
        BuildObjectKind::Fence => {
            let axis = spec.fence_axis;
            let stake_thick = BUILD_FENCE_WIDTH * 0.85;
            let stick_thick_y = BUILD_UNIT_SIZE * 0.20;
            let stick_thick_across = BUILD_FENCE_WIDTH * 0.35;

            let stake_offset = BUILD_FENCE_LENGTH * 0.5 - stake_thick * 0.5;
            let stake_size = fence_basis(axis, stake_thick, BUILD_FENCE_HEIGHT, stake_thick);
            let stake_half = stake_size * 0.5;
            draw_box_outline(
                &mut gizmos,
                preview.center + fence_basis(axis, -stake_offset, 0.0, 0.0),
                stake_half,
                color,
            );
            draw_box_outline(
                &mut gizmos,
                preview.center + fence_basis(axis, stake_offset, 0.0, 0.0),
                stake_half,
                color,
            );

            let stick_length = (BUILD_FENCE_LENGTH - stake_thick * 2.0).max(BUILD_UNIT_SIZE);
            let stick_size = fence_basis(axis, stick_length, stick_thick_y, stick_thick_across);
            let stick_half = stick_size * 0.5;
            let bottom_y = -BUILD_FENCE_HEIGHT * 0.5 + BUILD_UNIT_SIZE * 0.60;
            let top_y = -BUILD_FENCE_HEIGHT * 0.5 + BUILD_UNIT_SIZE * 2.00;
            draw_box_outline(
                &mut gizmos,
                preview.center + fence_basis(axis, 0.0, bottom_y, 0.0),
                stick_half,
                color,
            );
            draw_box_outline(
                &mut gizmos,
                preview.center + fence_basis(axis, 0.0, top_y, 0.0),
                stick_half,
                color,
            );
        }
        BuildObjectKind::Tree => {
            let variant = spec.tree_variant as usize % BUILD_TREE_VARIANT_SCALES.len();
            let scale = BUILD_TREE_VARIANT_SCALES[variant];
            let size = BUILD_TREE_BASE_SIZE * scale;
            let half_height = size.y * 0.5;
            let bottom_y = -half_height;

            let trunk_height = BUILD_UNIT_SIZE * 1.8 * scale;
            let trunk_radius = BUILD_UNIT_SIZE * 0.55 * scale;
            let main_height = BUILD_UNIT_SIZE * 2.4 * scale;
            let main_radius = BUILD_UNIT_SIZE * 1.45 * scale;
            let crown_height = BUILD_UNIT_SIZE * 0.8 * scale;
            let crown_radius = BUILD_UNIT_SIZE * 1.05 * scale;

            let trunk_size = Vec3::new(trunk_radius * 2.0, trunk_height, trunk_radius * 2.0);
            let trunk_center = preview.center + Vec3::new(0.0, bottom_y + trunk_height * 0.5, 0.0);
            draw_box_outline(&mut gizmos, trunk_center, trunk_size * 0.5, color);

            let main_size = Vec3::new(main_radius * 2.0, main_height, main_radius * 2.0);
            let main_center =
                preview.center + Vec3::new(0.0, bottom_y + trunk_height + main_height * 0.5, 0.0);
            draw_box_outline(&mut gizmos, main_center, main_size * 0.5, color);

            let crown_size = Vec3::new(crown_radius * 2.0, crown_height, crown_radius * 2.0);
            let crown_center = preview.center
                + Vec3::new(
                    0.0,
                    bottom_y + trunk_height + main_height + crown_height * 0.5,
                    0.0,
                );
            draw_box_outline(&mut gizmos, crown_center, crown_size * 0.5, color);
        }
    }
}

pub(crate) fn build_remove_selected_objects(
    mut commands: Commands,
    build: Res<BuildState>,
    keys: Res<ButtonInput<KeyCode>>,
    mut selection: ResMut<SelectionState>,
    objects: Query<(), With<BuildObject>>,
) {
    if build.placing_active {
        return;
    }
    if !(keys.just_pressed(KeyCode::Delete) || keys.just_pressed(KeyCode::Backspace)) {
        return;
    }

    let selected: Vec<Entity> = selection.selected.iter().copied().collect();
    for entity in selected {
        if !objects.contains(entity) {
            continue;
        }
        commands.entity(entity).try_despawn();
        selection.selected.remove(&entity);
    }
    selection.drag_start = None;
    selection.drag_end = None;
}

fn build_instance_bounds(
    library: &ObjectLibrary,
    prefab_id: u128,
    rotation: Quat,
    scale: Vec3,
) -> (Vec3, Vec2) {
    let base_size = library
        .size(prefab_id)
        .unwrap_or_else(|| Vec3::splat(DEFAULT_OBJECT_SIZE_M));

    let mut scale = scale;
    if !scale.x.is_finite() || scale.x.abs() < 1e-4 {
        scale.x = 1.0;
    }
    if !scale.y.is_finite() || scale.y.abs() < 1e-4 {
        scale.y = 1.0;
    }
    if !scale.z.is_finite() || scale.z.abs() < 1e-4 {
        scale.z = 1.0;
    }

    let (yaw, _pitch, _roll) = rotation.to_euler(EulerRot::YXZ);
    let c = yaw.cos().abs();
    let s = yaw.sin().abs();

    let half_unrot = match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => {
            let r = radius.max(0.01) * scale.x.abs().max(scale.z.abs()).max(0.01);
            Vec2::splat(r)
        }
        Some(ColliderProfile::AabbXZ { half_extents }) => Vec2::new(
            half_extents.x.abs().max(0.01) * scale.x.abs().max(0.01),
            half_extents.y.abs().max(0.01) * scale.z.abs().max(0.01),
        ),
        _ => Vec2::new(
            (base_size.x * 0.5).abs().max(0.01) * scale.x.abs().max(0.01),
            (base_size.z * 0.5).abs().max(0.01) * scale.z.abs().max(0.01),
        ),
    };

    let half = Vec2::new(
        c * half_unrot.x + s * half_unrot.y,
        s * half_unrot.x + c * half_unrot.y,
    );

    let size = Vec3::new(
        (half.x * 2.0).max(0.01),
        (base_size.y * scale.y.abs()).max(0.01),
        (half.y * 2.0).max(0.01),
    );

    (size, half)
}

pub(crate) fn build_edit_selected_objects(
    mut commands: Commands,
    time: Res<Time>,
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
    camera_q: Query<&Transform, (With<MainCamera>, Without<BuildObject>)>,
    mut wasd_repeat: Local<BuildObjectWasdRepeat>,
    mut objects: Query<
        (
            Entity,
            &mut Transform,
            &mut AabbCollider,
            &mut BuildDimensions,
            &ObjectPrefabId,
            Option<&ObjectTint>,
        ),
        With<BuildObject>,
    >,
) {
    if build.placing_active {
        wasd_repeat.dir = Vec2::ZERO;
        wasd_repeat.cooldown_secs = 0.0;
        return;
    }
    if selection.selected.is_empty() {
        wasd_repeat.dir = Vec2::ZERO;
        wasd_repeat.cooldown_secs = 0.0;
        return;
    }

    let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let step = if shift {
        BUILD_GRID_SIZE * 5.0
    } else {
        BUILD_GRID_SIZE
    };

    let modifier = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);

    if modifier && keys.just_pressed(KeyCode::KeyD) {
        let offset = Vec3::new(step, 0.0, step);
        let mut new_selected: HashSet<Entity> = HashSet::new();
        let selected: Vec<Entity> = selection.selected.iter().copied().collect();
        for entity in selected {
            let Ok((_entity, transform, collider, dimensions, prefab_id, tint)) =
                objects.get_mut(entity)
            else {
                continue;
            };
            let prefab_id = prefab_id.0;
            let size = dimensions.size;
            let half_extents = collider.half_extents;
            let tint = tint.map(|t| t.0);

            let mut transform = transform.clone();
            transform.translation += offset;
            transform.translation.x = snap_to_grid(transform.translation.x, BUILD_GRID_SIZE);
            transform.translation.z = snap_to_grid(transform.translation.z, BUILD_GRID_SIZE);
            transform.translation.x = transform.translation.x.clamp(
                -WORLD_HALF_SIZE + half_extents.x,
                WORLD_HALF_SIZE - half_extents.x,
            );
            transform.translation.z = transform.translation.z.clamp(
                -WORLD_HALF_SIZE + half_extents.y,
                WORLD_HALF_SIZE - half_extents.y,
            );

            let new_entity = spawn_build_object_with_collider_half_xz(
                &mut commands,
                &asset_server,
                &assets,
                &mut meshes,
                &mut materials,
                &mut material_cache,
                &mut mesh_cache,
                &library,
                prefab_id,
                size,
                half_extents,
                transform,
                ObjectId::new_v4(),
                tint,
            );
            new_selected.insert(new_entity);
        }
        if new_selected.is_empty() {
            return;
        }
        selection.selected = new_selected;
        selection.drag_start = None;
        selection.drag_end = None;
        return;
    }

    let mut move_delta = Vec3::ZERO;
    const REPEAT_INITIAL_DELAY_SECS: f32 = 0.28;
    const REPEAT_INTERVAL_SECS: f32 = 0.06;

    let mut camera_forward = camera_q
        .single()
        .map(|transform| transform.rotation * Vec3::NEG_Z)
        .unwrap_or(Vec3::Z);
    camera_forward.y = 0.0;
    let forward_xz = if camera_forward.length_squared() > 1e-6 {
        Vec2::new(camera_forward.x, camera_forward.z).normalize()
    } else {
        Vec2::Y
    };

    let mut camera_right = camera_q
        .single()
        .map(|transform| transform.rotation * Vec3::X)
        .unwrap_or(Vec3::X);
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

    let mut step_dir = Vec2::ZERO;
    if dir.length_squared() > 1e-6 {
        let dir = dir.normalize();
        if dir.x.abs() >= 0.5 {
            step_dir.x = dir.x.signum();
        }
        if dir.y.abs() >= 0.5 {
            step_dir.y = dir.y.signum();
        }
    }
    let dt = time.delta_secs().max(0.0);
    if step_dir.length_squared() <= 1e-6 {
        wasd_repeat.dir = Vec2::ZERO;
        wasd_repeat.cooldown_secs = 0.0;
    } else if step_dir != wasd_repeat.dir {
        wasd_repeat.dir = step_dir;
        wasd_repeat.cooldown_secs = REPEAT_INITIAL_DELAY_SECS;
        move_delta.x += step * step_dir.x;
        move_delta.z += step * step_dir.y;
    } else {
        wasd_repeat.cooldown_secs -= dt;
        if wasd_repeat.cooldown_secs <= 0.0 {
            wasd_repeat.cooldown_secs = REPEAT_INTERVAL_SECS;
            move_delta.x += step * step_dir.x;
            move_delta.z += step * step_dir.y;
        }
    }
    if keys.just_pressed(KeyCode::ArrowLeft) {
        move_delta.x -= step;
    }
    if keys.just_pressed(KeyCode::ArrowRight) {
        move_delta.x += step;
    }
    if keys.just_pressed(KeyCode::ArrowUp) {
        move_delta.z += step;
    }
    if keys.just_pressed(KeyCode::ArrowDown) {
        move_delta.z -= step;
    }

    let rot_step_deg = if shift { 45.0 } else { 15.0 };
    let mut yaw_delta_deg = 0.0f32;
    if keys.just_pressed(KeyCode::Comma) {
        yaw_delta_deg -= rot_step_deg;
    }
    if keys.just_pressed(KeyCode::Period) {
        yaw_delta_deg += rot_step_deg;
    }

    let scale_step = if shift { 1.25 } else { 1.10 };
    let mut scale_mul = 1.0f32;
    if keys.just_pressed(KeyCode::Equal) {
        scale_mul *= scale_step;
    }
    if keys.just_pressed(KeyCode::Minus) {
        scale_mul /= scale_step;
    }

    let has_transform = move_delta.length_squared() > 1e-6
        || yaw_delta_deg.abs() > 1e-6
        || (scale_mul - 1.0).abs() > 1e-6;
    if !has_transform {
        return;
    }

    let yaw_delta = Quat::from_rotation_y(yaw_delta_deg.to_radians());
    let selected: Vec<Entity> = selection.selected.iter().copied().collect();
    for entity in selected {
        let Ok((_entity, mut transform, mut collider, mut dimensions, prefab_id, _tint)) =
            objects.get_mut(entity)
        else {
            continue;
        };

        let base_size_y = library
            .size(prefab_id.0)
            .map(|size| size.y.abs())
            .unwrap_or(dimensions.size.y.abs());
        let scale_y = if base_size_y.is_finite() && base_size_y > 1e-6 {
            (dimensions.size.y / base_size_y).abs()
        } else {
            1.0
        };
        let base_origin_y = library.ground_origin_y_or_default(prefab_id.0);
        let bottom_y = transform.translation.y - base_origin_y * scale_y;

        transform.translation += move_delta;
        if yaw_delta_deg.abs() > 1e-6 {
            transform.rotation = (yaw_delta * transform.rotation).normalize();
        }
        if (scale_mul - 1.0).abs() > 1e-6 {
            transform.scale *= Vec3::splat(scale_mul);
            transform.scale.x = transform.scale.x.clamp(0.15, 10.0);
            transform.scale.y = transform.scale.y.clamp(0.15, 10.0);
            transform.scale.z = transform.scale.z.clamp(0.15, 10.0);
        }

        let (new_size, new_half) =
            build_instance_bounds(&library, prefab_id.0, transform.rotation, transform.scale);
        if (scale_mul - 1.0).abs() > 1e-6 {
            let new_scale_y = if base_size_y.is_finite() && base_size_y > 1e-6 {
                (new_size.y / base_size_y).abs()
            } else {
                1.0
            };
            transform.translation.y = bottom_y + base_origin_y * new_scale_y;
        }

        transform.translation.x = snap_to_grid(transform.translation.x, BUILD_GRID_SIZE);
        transform.translation.z = snap_to_grid(transform.translation.z, BUILD_GRID_SIZE);
        transform.translation.x = transform
            .translation
            .x
            .clamp(-WORLD_HALF_SIZE + new_half.x, WORLD_HALF_SIZE - new_half.x);
        transform.translation.z = transform
            .translation
            .z
            .clamp(-WORLD_HALF_SIZE + new_half.y, WORLD_HALF_SIZE - new_half.y);

        dimensions.size = new_size;
        collider.half_extents = new_half;
    }
}

#[derive(Default)]
pub(crate) struct BuildObjectWasdRepeat {
    dir: Vec2,
    cooldown_secs: f32,
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

pub(crate) fn build_draw_selection_gizmos(
    mut gizmos: Gizmos,
    build: Res<BuildState>,
    selection: Res<SelectionState>,
    objects: Query<(Entity, &Transform, &AabbCollider, &BuildDimensions), With<BuildObject>>,
) {
    if build.placing_active {
        return;
    }

    let edge_color = Color::srgb(0.25, 0.95, 0.85);
    for (entity, transform, collider, dimensions) in &objects {
        if !selection.selected.contains(&entity) {
            continue;
        }

        let y_half = dimensions.size.y * 0.5;
        let min = Vec3::new(
            transform.translation.x - collider.half_extents.x,
            transform.translation.y - y_half,
            transform.translation.z - collider.half_extents.y,
        );
        let max = Vec3::new(
            transform.translation.x + collider.half_extents.x,
            transform.translation.y + y_half,
            transform.translation.z + collider.half_extents.y,
        );

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
        draw_dashed_line(&mut gizmos, c0, c1, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c1, c2, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c2, c3, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c3, c0, dash, gap, edge_color);

        draw_dashed_line(&mut gizmos, c4, c5, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c5, c6, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c6, c7, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c7, c4, dash, gap, edge_color);

        draw_dashed_line(&mut gizmos, c0, c4, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c1, c5, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c2, c6, dash, gap, edge_color);
        draw_dashed_line(&mut gizmos, c3, c7, dash, gap, edge_color);
    }
}
