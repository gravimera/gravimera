use bevy::ecs::message::MessageWriter;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::prelude::*;
use std::collections::HashSet;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::effects::spawn_energy_impact_particles;
use crate::geometry::{circle_intersects_aabb_xz, circles_intersect_xz, normalize_flat_direction};
use crate::object::registry::{
    AnchorRef, ColliderProfile, ObjectLibrary, ObjectPartKind, UnitAttackKind,
};
use crate::object::types::{effects, projectiles};
use crate::object::visuals;
use crate::types::*;

const SHOTGUN_PELLET_COUNT: usize = 8;

fn wrap_angle_pi(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

#[derive(SystemParam)]
pub(crate) struct ProjectileVisualSpawnParams<'w> {
    asset_server: Res<'w, AssetServer>,
    assets: Res<'w, SceneAssets>,
    meshes: ResMut<'w, Assets<Mesh>>,
    materials: ResMut<'w, Assets<StandardMaterial>>,
    material_cache: ResMut<'w, crate::object::visuals::MaterialCache>,
    mesh_cache: ResMut<'w, crate::object::visuals::PrimitiveMeshCache>,
}

fn anchor_transform(def: &crate::object::registry::ObjectDef, name: &str) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    def.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn projectile_collider_radius(def: &crate::object::registry::ObjectDef) -> f32 {
    match def.collider {
        ColliderProfile::CircleXZ { radius } => radius.max(0.01),
        ColliderProfile::AabbXZ { half_extents } => half_extents.x.max(half_extents.y).max(0.01),
        ColliderProfile::None => def.size.x.max(def.size.z).max(0.01) * 0.5,
    }
}

fn find_anchor_world_matrix(
    library: &ObjectLibrary,
    object_id: u128,
    target_object_id: u128,
    target_anchor: &str,
    object_to_world: Mat4,
    aim_object_ids: &HashSet<u128>,
    aim_quat_parent: Quat,
    ancestor_apply_aim_yaw: bool,
    stack: &mut Vec<u128>,
) -> Option<Mat4> {
    if stack.contains(&object_id) {
        return None;
    }
    let def = library.get(object_id)?;

    if object_id == target_object_id {
        let anchor_local = anchor_transform(def, target_anchor).unwrap_or(Transform::IDENTITY);
        return Some(object_to_world * anchor_local.to_matrix());
    }

    stack.push(object_id);

    for part in def.parts.iter() {
        let ObjectPartKind::ObjectRef {
            object_id: child_id,
        } = &part.kind
        else {
            continue;
        };

        let apply_aim_yaw = !ancestor_apply_aim_yaw
            && !aim_object_ids.is_empty()
            && aim_object_ids.contains(child_id);

        let mut offset = part.transform;
        if apply_aim_yaw {
            let aim_quat = if let Some(att) = part.attachment.as_ref() {
                anchor_transform(def, att.parent_anchor.as_ref())
                    .map(|anchor| {
                        let anchor_rot = if anchor.rotation.is_finite() {
                            anchor.rotation.normalize()
                        } else {
                            Quat::IDENTITY
                        };
                        let q = anchor_rot.inverse() * aim_quat_parent * anchor_rot;
                        if q.is_finite() {
                            q.normalize()
                        } else {
                            Quat::IDENTITY
                        }
                    })
                    .unwrap_or(aim_quat_parent)
            } else {
                aim_quat_parent
            };

            let q = aim_quat * offset.rotation;
            if q.is_finite() {
                offset.rotation = q.normalize();
            }
        }

        let mut child_local = offset.to_matrix();
        if let Some(att) = part.attachment.as_ref() {
            let parent_anchor = anchor_transform(def, att.parent_anchor.as_ref())?;
            let child_def = library.get(*child_id)?;
            let child_anchor = anchor_transform(child_def, att.child_anchor.as_ref())
                .unwrap_or(Transform::IDENTITY);
            child_local =
                parent_anchor.to_matrix() * offset.to_matrix() * child_anchor.to_matrix().inverse();
        }

        let child_to_world = object_to_world * child_local;
        if let Some(found) = find_anchor_world_matrix(
            library,
            *child_id,
            target_object_id,
            target_anchor,
            child_to_world,
            aim_object_ids,
            aim_quat_parent,
            ancestor_apply_aim_yaw || apply_aim_yaw,
            stack,
        ) {
            stack.pop();
            return Some(found);
        }
    }

    stack.pop();
    None
}

fn anchor_world_position(
    library: &ObjectLibrary,
    root_prefab_id: u128,
    root_transform: &Transform,
    anchor: &AnchorRef,
    aim_quat_parent: Quat,
) -> Option<Vec3> {
    let root_to_world = root_transform.to_matrix();
    let aim_object_ids = aim_object_ids_for_root(library, root_prefab_id);
    let mut stack = Vec::new();
    let mat = find_anchor_world_matrix(
        library,
        root_prefab_id,
        anchor.object_id,
        anchor.anchor.as_ref(),
        root_to_world,
        &aim_object_ids,
        aim_quat_parent,
        false,
        &mut stack,
    )?;
    Some(mat.transform_point3(Vec3::ZERO))
}

fn aim_object_ids_for_root(library: &ObjectLibrary, root_object_id: u128) -> HashSet<u128> {
    let mut out = HashSet::new();
    let Some(def) = library.get(root_object_id) else {
        return out;
    };

    if let Some(aim) = def.aim.as_ref() {
        out.extend(aim.components.iter().copied());
    }

    if out.is_empty() {
        if let Some(attack) = def.attack.as_ref() {
            if matches!(attack.kind, UnitAttackKind::RangedProjectile) {
                if let Some(ranged) = attack.ranged.as_ref() {
                    out.insert(ranged.muzzle.object_id);
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        AimProfile, AnchorDef, AttachmentDef, ObjectDef, ObjectInteraction, ObjectPartDef,
        ObjectPartKind,
    };

    #[test]
    fn anchor_world_position_applies_aim_yaw_in_parent_frame() {
        // Regression: aim yaw is defined in the parent's body frame (+Y is up). When attachments
        // use rotated join frames (common for neck/shoulder joints), applying yaw directly in the
        // join frame rotates around a horizontal axis and misplaces the muzzle when aiming.
        //
        // This test uses a rotated anchor/join frame and matching child anchor such that, without
        // the parent-frame conversion, the muzzle would incorrectly remain at +Z when yawing.
        let anchor_rot =
            Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_X, Vec3::Z, Vec3::Y)).normalize();

        let parent_id = 0x91a6_1f17_u128;
        let child_id = 0x2c1e_4caa_u128;

        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: parent_id,
            label: "parent".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: Some(AimProfile {
                max_yaw_delta_degrees: None,
                components: vec![child_id],
            }),
            mobility: None,
            anchors: vec![AnchorDef {
                name: "neck".into(),
                transform: Transform::from_rotation(anchor_rot),
            }],
            parts: vec![ObjectPartDef {
                part_id: None,
                render_priority: None,
                kind: ObjectPartKind::ObjectRef {
                    object_id: child_id,
                },
                attachment: Some(AttachmentDef {
                    parent_anchor: "neck".into(),
                    child_anchor: "neck_mount".into(),
                }),
                animations: Vec::new(),
                transform: Transform::IDENTITY,
                fallback_basis: Transform::IDENTITY,
            }],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        library.upsert(ObjectDef {
            object_id: child_id,
            label: "child".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![
                AnchorDef {
                    name: "neck_mount".into(),
                    transform: Transform::from_rotation(anchor_rot),
                },
                AnchorDef {
                    name: "muzzle".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 0.0, 1.0)),
                },
            ],
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let anchor = AnchorRef {
            object_id: child_id,
            anchor: "muzzle".into(),
        };

        let pos = anchor_world_position(
            &library,
            parent_id,
            &Transform::IDENTITY,
            &anchor,
            Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
        )
        .expect("muzzle anchor world position");

        assert!(
            (pos - Vec3::X).length() < 1e-4,
            "expected muzzle to yaw around +Y into +X; pos={:?}",
            pos
        );
    }
}

fn fire_direction_from_target(
    origin: Vec3,
    target: Option<FireTarget>,
    units: &Query<
        &Transform,
        (
            Or<(With<Commandable>, With<Enemy>)>,
            Without<Player>,
            Without<Died>,
        ),
    >,
) -> Option<Vec3> {
    match target {
        None => None,
        Some(FireTarget::Point(point)) => {
            let to = point - Vec2::new(origin.x, origin.z);
            if to.length_squared() <= 1e-6 {
                None
            } else {
                Some(Vec3::new(to.x, 0.0, to.y).normalize())
            }
        }
        Some(FireTarget::Unit(target_entity)) => units.get(target_entity).ok().and_then(|t| {
            let to = t.translation - origin;
            let flat = Vec3::new(to.x, 0.0, to.z);
            if flat.length_squared() <= 1e-6 {
                None
            } else {
                Some(flat.normalize())
            }
        }),
    }
}

fn ray_aabb_intersection_xz(origin: Vec2, dir: Vec2, center: Vec2, half: Vec2) -> Option<f32> {
    let min = center - half;
    let max = center + half;

    let mut tmin: f32 = 0.0;
    let mut tmax: f32 = f32::INFINITY;

    for (o, d, mn, mx) in [
        (origin.x, dir.x, min.x, max.x),
        (origin.y, dir.y, min.y, max.y),
    ] {
        if d.abs() < 1e-6 {
            if o < mn || o > mx {
                return None;
            }
            continue;
        }

        let inv = 1.0 / d;
        let mut t1 = (mn - o) * inv;
        let mut t2 = (mx - o) * inv;
        if t1 > t2 {
            std::mem::swap(&mut t1, &mut t2);
        }
        tmin = tmin.max(t1);
        tmax = tmax.min(t2);
        if tmin > tmax {
            return None;
        }
    }

    if tmax < 0.0 {
        return None;
    }

    Some(tmin.max(0.0))
}

fn laser_effective_range(
    origin: Vec2,
    dir: Vec2,
    obstacles: &[(Vec2, Vec2)],
    gundams: &[(Vec2, f32)],
) -> f32 {
    let mut range = LASER_RANGE;

    for &(center, half_extents) in obstacles {
        let half = half_extents + Vec2::splat(LASER_HALF_WIDTH);
        if let Some(hit) = ray_aabb_intersection_xz(origin, dir, center, half) {
            range = range.min(hit);
        }
    }

    for &(center, radius) in gundams {
        let r = radius + LASER_HALF_WIDTH;
        let m = origin - center;
        let b = m.dot(dir);
        let c = m.dot(m) - r * r;

        // Circle is behind the ray origin and we're pointing away from it.
        if c > 0.0 && b > 0.0 {
            continue;
        }

        let disc = b * b - c;
        if disc < 0.0 {
            continue;
        }

        let hit = (-b - disc.sqrt()).max(0.0);
        range = range.min(hit);
    }

    range.max(0.0)
}

pub(crate) fn switch_player_weapon(keys: Res<ButtonInput<KeyCode>>, mut game: ResMut<Game>) {
    // Digit keys are reserved for unit animation hotkeys. Use Ctrl/Cmd + digit for weapon switching.
    let modifier = keys.pressed(KeyCode::ControlLeft)
        || keys.pressed(KeyCode::ControlRight)
        || keys.pressed(KeyCode::SuperLeft)
        || keys.pressed(KeyCode::SuperRight);
    if !modifier {
        return;
    }

    let requested = if keys.just_pressed(KeyCode::Digit1) || keys.just_pressed(KeyCode::Numpad1) {
        Some(PlayerWeapon::Normal)
    } else if keys.just_pressed(KeyCode::Digit2) || keys.just_pressed(KeyCode::Numpad2) {
        Some(PlayerWeapon::Shotgun)
    } else if keys.just_pressed(KeyCode::Digit3) || keys.just_pressed(KeyCode::Numpad3) {
        Some(PlayerWeapon::Laser)
    } else {
        None
    };

    let Some(weapon) = requested else {
        return;
    };

    if weapon.is_available(game.shotgun_charges, game.laser_charges) {
        game.weapon = weapon;
    }
}

pub(crate) fn player_muzzle_position(
    player_transform: &Transform,
    direction: Vec3,
    muzzle_forward: f32,
) -> Vec3 {
    player_transform.translation + Vec3::new(0.0, PLAYER_GUN_Y, 0.0) + direction * muzzle_forward
}

pub(crate) fn update_lasers(
    mut commands: Commands,
    time: Res<Time>,
    muzzles: Res<PlayerMuzzles>,
    mut player_q: Query<&mut Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &ObjectPrefabId),
        (With<BuildObject>, Without<Laser>, Without<Player>),
    >,
    enemies: Query<
        (&Transform, &Collider, &ObjectPrefabId),
        (With<Enemy>, Without<Laser>, Without<Player>),
    >,
    mut lasers: Query<(Entity, &mut Transform, &mut Laser), Without<Player>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    if lasers.is_empty() {
        return;
    }

    let Ok(mut player_transform) = player_q.single_mut() else {
        return;
    };

    let obstacles: Vec<(Vec2, Vec2)> = objects
        .iter()
        .filter_map(|(transform, collider, prefab_id)| {
            library.interaction(prefab_id.0).blocks_laser.then_some((
                Vec2::new(transform.translation.x, transform.translation.z),
                collider.half_extents,
            ))
        })
        .collect();
    let gundams: Vec<(Vec2, f32)> = enemies
        .iter()
        .filter_map(|(transform, collider, prefab_id)| {
            library.interaction(prefab_id.0).blocks_laser.then_some((
                Vec2::new(transform.translation.x, transform.translation.z),
                collider.radius,
            ))
        })
        .collect();

    for (entity, mut transform, mut laser) in &mut lasers {
        let Some(direction) = normalize_flat_direction(laser.direction) else {
            laser.ttl_secs -= dt;
            if laser.ttl_secs <= 0.0 {
                commands.entity(entity).try_despawn();
            }
            continue;
        };
        laser.direction = direction;

        player_transform.rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));

        let muzzle = player_muzzle_position(
            &player_transform,
            direction,
            muzzles.for_weapon(PlayerWeapon::Laser),
        );
        let origin = Vec2::new(muzzle.x, muzzle.z);
        let dir2 = Vec2::new(direction.x, direction.z);
        let range = laser_effective_range(origin, dir2, &obstacles, &gundams);
        let scale_z = (range / LASER_RANGE).clamp(0.001, 1.0);
        let yaw = direction.x.atan2(direction.z);
        let center = muzzle + direction * (range * 0.5);
        let rotation = Quat::from_rotation_y(yaw);

        transform.translation = center;
        transform.rotation = rotation;
        transform.scale = Vec3::new(1.0, 1.0, scale_z);

        laser.ttl_secs -= dt;
        if laser.ttl_secs <= 0.0 {
            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn laser_kill_enemies(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut effects: ResMut<EnemyKillEffects>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    muzzles: Res<PlayerMuzzles>,
    mut player_q: Query<(Entity, &Transform, &mut Health), With<Player>>,
    library: Res<ObjectLibrary>,
    objects: Query<(&Transform, &AabbCollider, &ObjectPrefabId), With<BuildObject>>,
    lasers: Query<&Laser>,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Enemy>,
            &mut Health,
            &mut LaserDamageAccum,
            Option<&Died>,
        ),
        (Without<Player>, Or<(With<Commandable>, With<Enemy>)>),
    >,
) {
    if game.game_over {
        return;
    }
    let Some(laser) = lasers.iter().next() else {
        return;
    };

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    let Some(direction) = normalize_flat_direction(laser.direction) else {
        return;
    };
    let Ok((player_entity, player_transform, mut player_health)) = player_q.single_mut() else {
        return;
    };

    let muzzle = player_muzzle_position(
        player_transform,
        direction,
        muzzles.for_weapon(PlayerWeapon::Laser),
    );
    let origin = Vec2::new(muzzle.x, muzzle.z);
    let dir2 = Vec2::new(direction.x, direction.z);
    let obstacles: Vec<(Vec2, Vec2)> = objects
        .iter()
        .filter_map(|(transform, collider, prefab_id)| {
            library.interaction(prefab_id.0).blocks_laser.then_some((
                Vec2::new(transform.translation.x, transform.translation.z),
                collider.half_extents,
            ))
        })
        .collect();
    let laser_blockers: Vec<(Vec2, f32)> = units
        .iter()
        .filter_map(
            |(entity, transform, collider, prefab_id, _enemy, health, _accum, died)| {
                if entity == player_entity {
                    return None;
                }
                if died.is_some() || health.current <= 0 {
                    return None;
                }
                library.interaction(prefab_id.0).blocks_laser.then_some((
                    Vec2::new(transform.translation.x, transform.translation.z),
                    collider.radius,
                ))
            },
        )
        .collect();
    let range = laser_effective_range(origin, dir2, &obstacles, &laser_blockers);

    let mut kills = 0u32;
    let damage = dt * LASER_DAMAGE_PER_SEC;
    let wall_time = time.elapsed_secs();

    for (entity, transform, collider, prefab_id, enemy, mut health, mut accum, died) in &mut units {
        if died.is_some() || health.current <= 0 {
            continue;
        }

        let pos = Vec2::new(transform.translation.x, transform.translation.z);
        let to = pos - origin;
        let proj = to.dot(dir2);
        let r = LASER_HALF_WIDTH + collider.radius;
        let perp2 = (to - dir2 * proj).length_squared();
        if perp2 > r * r {
            continue;
        }

        let h = (r * r - perp2).sqrt();
        let entry = proj - h;
        let exit = proj + h;
        if exit < 0.0 || entry > range {
            continue;
        }

        accum.0 += damage;
        let whole = accum.0.floor() as i32;
        if whole <= 0 {
            continue;
        }
        accum.0 -= whole as f32;

        let popup_offset_y = library
            .health_bar_offset_y(prefab_id.0)
            .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
        let popup_pos = transform.translation + Vec3::Y * popup_offset_y;
        health_events.write(HealthChangeEvent {
            world_pos: popup_pos,
            delta: -whole,
            is_hero: false,
        });

        health.current = (health.current - whole).max(0);
        if health.current <= 0 {
            crate::unit_health::start_die_motion(
                &mut commands,
                wall_time,
                &library,
                entity,
                prefab_id.0,
                transform,
            );
            if enemy.is_some() {
                effects.0.push(transform.translation);
                kills += 1;
            }
        }
    }

    let health_gains = crate::enemies::apply_kill_rewards(&mut game, &mut player_health, kills);
    if health_gains > 0 {
        let popup_pos = player_transform.translation + Vec3::Y * PLAYER_HEALTH_BAR_OFFSET_Y;
        for _ in 0..health_gains {
            health_events.write(HealthChangeEvent {
                world_pos: popup_pos,
                delta: 1,
                is_hero: true,
            });
        }
    }
}

pub(crate) fn player_fire(
    mut commands: Commands,
    mode: Res<State<GameMode>>,
    fire: Res<FireControl>,
    selection: Res<SelectionState>,
    mut game: ResMut<Game>,
    assets: Res<SceneAssets>,
    muzzles: Res<PlayerMuzzles>,
    mut player_q: Query<(Entity, &mut Transform, Option<&LocomotionClock>), With<Player>>,
    units: Query<
        &Transform,
        (
            Or<(With<Commandable>, With<Enemy>)>,
            Without<Player>,
            Without<Died>,
        ),
    >,
    lasers: Query<Entity, With<Laser>>,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        return;
    }

    if !fire.active {
        return;
    }
    if game.fire_cooldown_secs > 0.0 {
        return;
    }

    let Ok((player_entity, mut player_transform, locomotion)) = player_q.single_mut() else {
        return;
    };
    if !selection.selected.contains(&player_entity) {
        return;
    }
    let direction = locomotion
        .map(|clock| clock.last_move_dir_xz)
        .filter(|dir| dir.length_squared() > 1e-6)
        .map(|dir| Vec3::new(dir.x, 0.0, dir.y))
        .and_then(normalize_flat_direction)
        .or_else(|| normalize_flat_direction(player_transform.rotation * Vec3::Z))
        .or_else(|| fire_direction_from_target(player_transform.translation, fire.target, &units))
        .unwrap_or(Vec3::Z);
    let yaw = direction.x.atan2(direction.z);
    player_transform.rotation = Quat::from_rotation_y(yaw);

    match game.weapon {
        PlayerWeapon::Normal => {
            let muzzle = player_muzzle_position(
                &player_transform,
                direction,
                muzzles.for_weapon(PlayerWeapon::Normal),
            );
            let spawn_pos = muzzle + direction * (BULLET_MESH_LENGTH * 0.5);
            let velocity = direction * BULLET_SPEED;
            let rotation = Quat::from_rotation_y(direction.x.atan2(direction.z));

            let bullet_entity = commands
                .spawn((
                    ObjectId::new_v4(),
                    ObjectPrefabId(projectiles::player_bullet::object_id()),
                    ProjectileOwner(player_entity),
                    Transform::from_translation(spawn_pos).with_rotation(rotation),
                    Visibility::Inherited,
                    Bullet {
                        velocity,
                        ttl_secs: BULLET_TTL_SECS,
                    },
                    Collider {
                        radius: BULLET_RADIUS,
                    },
                ))
                .id();

            commands.entity(bullet_entity).with_children(|parent| {
                parent.spawn((
                    Mesh3d(assets.bullet_mesh.clone()),
                    MeshMaterial3d(assets.bullet_material.clone()),
                    BulletVisual,
                    BulletTrailVisual,
                ));
            });

            game.fire_cooldown_secs = FIRE_COOLDOWN_SECS;
        }
        PlayerWeapon::Shotgun => {
            if game.shotgun_charges == 0 {
                game.weapon = PlayerWeapon::Normal;
                return;
            }
            game.shotgun_charges -= 1;

            let muzzle = player_muzzle_position(
                &player_transform,
                direction,
                muzzles.for_weapon(PlayerWeapon::Shotgun),
            );
            let mut rng = thread_rng();
            for _ in 0..SHOTGUN_PELLET_COUNT {
                let angle = rng
                    .gen_range(-SHOTGUN_ARC_HALF_ANGLE_DEGREES..SHOTGUN_ARC_HALF_ANGLE_DEGREES)
                    .to_radians();
                let pellet_dir = (Quat::from_rotation_y(angle) * direction).normalize();

                let spawn_pos = muzzle + pellet_dir * SHOTGUN_PELLET_RADIUS;
                let velocity = pellet_dir * SHOTGUN_PELLET_SPEED;
                let rotation = Quat::from_rotation_y(pellet_dir.x.atan2(pellet_dir.z));

                let bullet_entity = commands
                    .spawn((
                        ObjectId::new_v4(),
                        ObjectPrefabId(projectiles::player_shotgun_pellet::object_id()),
                        ProjectileOwner(player_entity),
                        Transform::from_translation(spawn_pos).with_rotation(rotation),
                        Visibility::Inherited,
                        Bullet {
                            velocity,
                            ttl_secs: BULLET_TTL_SECS,
                        },
                        Collider {
                            radius: SHOTGUN_PELLET_RADIUS,
                        },
                    ))
                    .id();

                commands.entity(bullet_entity).with_children(|parent| {
                    parent.spawn((
                        Mesh3d(assets.shotgun_pellet_mesh.clone()),
                        MeshMaterial3d(assets.shotgun_pellet_material.clone()),
                        BulletVisual,
                    ));
                });
            }

            if game.shotgun_charges == 0 {
                game.weapon = PlayerWeapon::Normal;
            }
            game.fire_cooldown_secs = SHOTGUN_FIRE_COOLDOWN_SECS;
        }
        PlayerWeapon::Laser => {
            if game.laser_charges == 0 {
                game.weapon = PlayerWeapon::Normal;
                return;
            }
            game.laser_charges -= 1;

            for entity in &lasers {
                commands.entity(entity).try_despawn();
            }

            let muzzle = player_muzzle_position(
                &player_transform,
                direction,
                muzzles.for_weapon(PlayerWeapon::Laser),
            );
            let center = muzzle + direction * (LASER_RANGE * 0.5);

            commands.spawn((
                ObjectId::new_v4(),
                ObjectPrefabId(effects::laser::object_id()),
                Mesh3d(assets.laser_mesh.clone()),
                MeshMaterial3d(assets.laser_material.clone()),
                Transform::from_translation(center).with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::Inherited,
                Laser {
                    ttl_secs: LASER_DURATION_SECS,
                    direction,
                },
            ));

            if game.laser_charges == 0 {
                game.weapon = PlayerWeapon::Normal;
            }
            game.fire_cooldown_secs = LASER_DURATION_SECS;
        }
    }
}

pub(crate) fn ensure_attack_cooldowns(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    q: Query<(Entity, &ObjectPrefabId), (With<Commandable>, Without<AttackCooldown>)>,
) {
    for (entity, prefab_id) in &q {
        if library.attack(prefab_id.0).is_some() {
            commands.entity(entity).insert(AttackCooldown::default());
        }
    }
}

pub(crate) fn tick_attack_cooldowns(time: Res<Time>, mut q: Query<&mut AttackCooldown>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    for mut cooldown in &mut q {
        cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);
    }
}

pub(crate) fn unit_attack_execute(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<State<GameMode>>,
    fire: Res<FireControl>,
    selection: Res<SelectionState>,
    mut visuals_spawn: ProjectileVisualSpawnParams,
    library: Res<ObjectLibrary>,
    mut game: ResMut<Game>,
    mut effects: ResMut<EnemyKillEffects>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    player_entity_q: Query<Entity, With<Player>>,
    player_transform_q: Query<&Transform, With<Player>>,
    mut commandables: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&LocomotionClock>,
            Option<&AimYawDelta>,
            &mut AttackCooldown,
        ),
        (With<Commandable>, Without<Player>, Without<Died>),
    >,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Enemy>,
            Option<&Player>,
            &mut Health,
            Option<&Died>,
        ),
        Or<(With<Commandable>, With<Enemy>)>,
    >,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        return;
    }
    if !fire.active || fire.target.is_none() {
        return;
    }
    if selection.selected.is_empty() {
        return;
    }

    let wall_time = time.elapsed_secs();
    let mut melee_kills: u32 = 0;

    for entity in selection.selected.iter().copied() {
        let Ok((entity, transform, _collider, prefab_id, locomotion, aim_delta, mut cooldown)) =
            commandables.get_mut(entity)
        else {
            continue;
        };
        let Some(def) = library.get(prefab_id.0) else {
            continue;
        };
        let Some(attack) = def.attack.as_ref() else {
            continue;
        };
        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        // Don't attack while dead (health can hit 0 before the `Died` marker is applied).
        if let Ok((_e, _t, _c, _p, _enemy, _player, health, died)) = units.get(entity) {
            if died.is_some() || health.current <= 0 {
                continue;
            }
        }

        let duration_secs = attack.anim_window_secs.max(0.0);
        if duration_secs > 0.0 {
            commands.entity(entity).insert(AttackClock {
                started_at_secs: wall_time,
                duration_secs,
            });
        }
        cooldown.remaining_secs = attack.cooldown_secs.max(0.0);

        match attack.kind {
            UnitAttackKind::Melee => {
                let Some(melee) = attack.melee.as_ref() else {
                    continue;
                };
                let damage = attack.damage.max(0);
                if damage == 0 {
                    continue;
                }

                let aim_rot = Quat::from_rotation_y(aim_delta.copied().unwrap_or_default().0);
                let direction = normalize_flat_direction((transform.rotation * aim_rot) * Vec3::Z)
                    .unwrap_or(Vec3::Z);
                let origin = transform.translation;
                let forward2 = Vec2::new(direction.x, direction.z).normalize_or_zero();
                let cos_min = if melee.arc_degrees >= 360.0 {
                    -1.0
                } else {
                    let half =
                        (melee.arc_degrees.to_radians() * 0.5).clamp(0.0, std::f32::consts::PI);
                    half.cos()
                };

                // If player clicked a unit, prefer that target for melee.
                let mut did_hit_target = false;
                if let Some(FireTarget::Unit(target_entity)) = fire.target {
                    if target_entity != entity {
                        if let Ok((
                            target_entity,
                            target_transform,
                            target_collider,
                            target_prefab,
                            target_enemy,
                            target_player,
                            mut target_health,
                            target_died,
                        )) = units.get_mut(target_entity)
                        {
                            if target_died.is_none() && target_health.current > 0 {
                                let to = target_transform.translation - origin;
                                let to2 = Vec2::new(to.x, to.z);
                                let dist = to2.length();
                                let within =
                                    dist <= melee.range + melee.radius + target_collider.radius;
                                let dir_ok = if to2.length_squared() <= 1e-6 {
                                    true
                                } else {
                                    forward2.dot(to2.normalize()) >= cos_min
                                };
                                if within && dir_ok {
                                    let popup_offset_y = library
                                        .health_bar_offset_y(target_prefab.0)
                                        .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
                                    health_events.write(HealthChangeEvent {
                                        world_pos: target_transform.translation
                                            + Vec3::Y * popup_offset_y,
                                        delta: -damage,
                                        is_hero: target_player.is_some(),
                                    });

                                    target_health.current = (target_health.current - damage).max(0);
                                    did_hit_target = true;

                                    if target_health.current <= 0 {
                                        crate::unit_health::start_die_motion(
                                            &mut commands,
                                            wall_time,
                                            &library,
                                            target_entity,
                                            target_prefab.0,
                                            target_transform,
                                        );
                                        if target_enemy.is_some() {
                                            effects.0.push(target_transform.translation);
                                            melee_kills = melee_kills.saturating_add(1);
                                        } else if target_player.is_some() {
                                            game.game_over = true;
                                            info!(
                                                "GAME OVER. Final score: {}. Press R to restart.",
                                                game.score
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                if did_hit_target {
                    continue;
                }

                // Otherwise, hit the closest unit in the forward arc.
                let mut best: Option<(Entity, f32)> = None;
                for (
                    target_entity,
                    target_transform,
                    target_collider,
                    _prefab_id,
                    _enemy,
                    _player,
                    health,
                    died,
                ) in units.iter()
                {
                    if target_entity == entity {
                        continue;
                    }
                    if died.is_some() || health.current <= 0 {
                        continue;
                    }
                    let to = target_transform.translation - origin;
                    let to2 = Vec2::new(to.x, to.z);
                    if to2.length_squared() <= 1e-6 {
                        continue;
                    }
                    let dist = to2.length();
                    if dist > melee.range + melee.radius + target_collider.radius {
                        continue;
                    }
                    if forward2.dot(to2.normalize()) < cos_min {
                        continue;
                    }
                    best = Some(match best {
                        None => (target_entity, dist),
                        Some((best_entity, best_distance)) => {
                            if dist < best_distance {
                                (target_entity, dist)
                            } else {
                                (best_entity, best_distance)
                            }
                        }
                    });
                }

                let Some((target_entity, _dist)) = best else {
                    continue;
                };
                if let Ok((
                    target_entity,
                    target_transform,
                    _target_collider,
                    target_prefab,
                    target_enemy,
                    target_player,
                    mut target_health,
                    target_died,
                )) = units.get_mut(target_entity)
                {
                    if target_died.is_some() || target_health.current <= 0 {
                        continue;
                    }
                    let popup_offset_y = library
                        .health_bar_offset_y(target_prefab.0)
                        .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
                    health_events.write(HealthChangeEvent {
                        world_pos: target_transform.translation + Vec3::Y * popup_offset_y,
                        delta: -damage,
                        is_hero: target_player.is_some(),
                    });

                    target_health.current = (target_health.current - damage).max(0);
                    if target_health.current <= 0 {
                        crate::unit_health::start_die_motion(
                            &mut commands,
                            wall_time,
                            &library,
                            target_entity,
                            target_prefab.0,
                            target_transform,
                        );
                        if target_enemy.is_some() {
                            effects.0.push(target_transform.translation);
                            melee_kills = melee_kills.saturating_add(1);
                        } else if target_player.is_some() {
                            game.game_over = true;
                            info!(
                                "GAME OVER. Final score: {}. Press R to restart.",
                                game.score
                            );
                        }
                    }
                }
            }
            UnitAttackKind::RangedProjectile => {
                let Some(ranged) = attack.ranged.as_ref() else {
                    continue;
                };
                let Some(projectile_profile) = library.projectile(ranged.projectile_prefab) else {
                    continue;
                };
                let Some(projectile_def) = library.get(ranged.projectile_prefab) else {
                    continue;
                };

                let fire_direction = locomotion
                    .map(|clock| clock.last_move_dir_xz)
                    .filter(|dir| dir.length_squared() > 1e-6)
                    .map(|dir| Vec3::new(dir.x, 0.0, dir.y))
                    .and_then(normalize_flat_direction)
                    .unwrap_or_else(|| {
                        normalize_flat_direction(transform.rotation * Vec3::Z).unwrap_or(Vec3::Z)
                    });

                let dir2 = Vec2::new(fire_direction.x, fire_direction.z).normalize_or_zero();
                let desired_yaw = dir2.x.atan2(dir2.y);
                let forward = transform.rotation * Vec3::Z;
                let body_yaw = forward.x.atan2(forward.z);
                let delta = wrap_angle_pi(desired_yaw - body_yaw);
                let aim_rot = Quat::from_rotation_y(delta);
                commands.entity(entity).insert(AimYawDelta(delta));

                let muzzle_pos = anchor_world_position(
                    &library,
                    prefab_id.0,
                    transform,
                    &ranged.muzzle,
                    aim_rot,
                )
                .unwrap_or_else(|| transform.translation + Vec3::Y * 1.0);

                let radius = projectile_collider_radius(projectile_def);
                let spawn_pos = muzzle_pos + fire_direction * (radius * 1.05 + 0.01);
                let velocity = fire_direction * projectile_profile.speed;
                let yaw = fire_direction.x.atan2(fire_direction.z);
                let rotation = Quat::from_rotation_y(yaw);

                let mut bullet_entity = commands.spawn((
                    ObjectId::new_v4(),
                    ObjectPrefabId(ranged.projectile_prefab),
                    ProjectileOwner(entity),
                    Transform::from_translation(spawn_pos).with_rotation(rotation),
                    Visibility::Inherited,
                    Bullet {
                        velocity,
                        ttl_secs: projectile_profile.ttl_secs,
                    },
                    Collider { radius },
                ));
                visuals::spawn_object_visuals(
                    &mut bullet_entity,
                    &library,
                    &visuals_spawn.asset_server,
                    &visuals_spawn.assets,
                    &mut visuals_spawn.meshes,
                    &mut visuals_spawn.materials,
                    &mut visuals_spawn.material_cache,
                    &mut visuals_spawn.mesh_cache,
                    ranged.projectile_prefab,
                    None,
                );
            }
        }
    }

    if melee_kills > 0 {
        let Ok(player_entity) = player_entity_q.single() else {
            return;
        };
        let Ok((_e, _t, _c, _p, _enemy, _player, mut player_health, _died)) =
            units.get_mut(player_entity)
        else {
            return;
        };
        let health_gains =
            crate::enemies::apply_kill_rewards(&mut game, &mut player_health, melee_kills);
        if health_gains > 0 {
            if let Ok(player_transform) = player_transform_q.single() {
                let popup_pos = player_transform.translation + Vec3::Y * PLAYER_HEALTH_BAR_OFFSET_Y;
                for _ in 0..health_gains {
                    health_events.write(HealthChangeEvent {
                        world_pos: popup_pos,
                        delta: 1,
                        is_hero: true,
                    });
                }
            }
        }
    }
}

pub(crate) fn brain_attack_execute(
    mut commands: Commands,
    time: Res<Time>,
    mode: Res<State<GameMode>>,
    mut visuals_spawn: ProjectileVisualSpawnParams,
    library: Res<ObjectLibrary>,
    mut game: ResMut<Game>,
    mut effects: ResMut<EnemyKillEffects>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    player_entity_q: Query<Entity, With<Player>>,
    player_transform_q: Query<&Transform, With<Player>>,
    mut attackers: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&LocomotionClock>,
            &mut AttackCooldown,
            &BrainAttackOrder,
        ),
        (With<Commandable>, With<AttackCooldown>, Without<Died>),
    >,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Enemy>,
            Option<&Player>,
            &mut Health,
            Option<&Died>,
        ),
        Or<(With<Commandable>, With<Enemy>)>,
    >,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        return;
    }

    let tick_index = (time.elapsed_secs_f64() * 60.0).floor().max(0.0) as u64;
    let wall_time = time.elapsed_secs();
    let mut melee_kills: u32 = 0;

    for (entity, transform, _collider, prefab_id, locomotion, mut cooldown, order) in
        attackers.iter_mut()
    {
        if let Some(valid_until_tick) = order.valid_until_tick {
            if tick_index > valid_until_tick {
                commands.entity(entity).remove::<BrainAttackOrder>();
                continue;
            }
        }

        // Don't attack while dead (health can hit 0 before the `Died` marker is applied).
        if let Ok((_e, _t, _c, _p, _enemy, _player, health, died)) = units.get_mut(entity) {
            if died.is_some() || health.current <= 0 {
                continue;
            }
        }

        let Some(def) = library.get(prefab_id.0) else {
            commands.entity(entity).remove::<BrainAttackOrder>();
            continue;
        };
        let Some(attack) = def.attack.as_ref() else {
            commands.entity(entity).remove::<BrainAttackOrder>();
            continue;
        };

        let Ok((
            target_entity,
            target_transform,
            target_collider,
            target_prefab,
            target_enemy,
            target_player,
            mut target_health,
            target_died,
        )) = units.get_mut(order.target)
        else {
            commands.entity(entity).remove::<BrainAttackOrder>();
            continue;
        };

        if target_entity == entity {
            commands.entity(entity).remove::<BrainAttackOrder>();
            continue;
        }
        if target_died.is_some() || target_health.current <= 0 {
            commands.entity(entity).remove::<BrainAttackOrder>();
            continue;
        }

        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        let to_target = target_transform.translation - transform.translation;
        let Some(direction) = normalize_flat_direction(to_target) else {
            continue;
        };

        let duration_secs = attack.anim_window_secs.max(0.0);
        if duration_secs > 0.0 {
            commands.entity(entity).insert(AttackClock {
                started_at_secs: wall_time,
                duration_secs,
            });
        }
        cooldown.remaining_secs = attack.cooldown_secs.max(0.0);

        let damage = attack.damage.max(0);
        match attack.kind {
            UnitAttackKind::Melee => {
                let Some(melee) = attack.melee.as_ref() else {
                    continue;
                };
                if damage == 0 {
                    continue;
                }

                let to2 = Vec2::new(to_target.x, to_target.z);
                let dist = to2.length();
                if dist > melee.range + melee.radius + target_collider.radius {
                    continue;
                }

                let popup_offset_y = library
                    .health_bar_offset_y(target_prefab.0)
                    .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
                health_events.write(HealthChangeEvent {
                    world_pos: target_transform.translation + Vec3::Y * popup_offset_y,
                    delta: -damage,
                    is_hero: target_player.is_some(),
                });

                target_health.current = (target_health.current - damage).max(0);
                if target_health.current <= 0 {
                    crate::unit_health::start_die_motion(
                        &mut commands,
                        wall_time,
                        &library,
                        target_entity,
                        target_prefab.0,
                        target_transform,
                    );
                    if target_enemy.is_some() {
                        effects.0.push(target_transform.translation);
                        melee_kills = melee_kills.saturating_add(1);
                    } else if target_player.is_some() {
                        game.game_over = true;
                        info!(
                            "GAME OVER. Final score: {}. Press R to restart.",
                            game.score
                        );
                    }
                }
            }
            UnitAttackKind::RangedProjectile => {
                let Some(ranged) = attack.ranged.as_ref() else {
                    continue;
                };
                let Some(projectile_profile) = library.projectile(ranged.projectile_prefab) else {
                    continue;
                };
                let Some(projectile_def) = library.get(ranged.projectile_prefab) else {
                    continue;
                };

                let fire_direction = locomotion
                    .map(|clock| clock.last_move_dir_xz)
                    .filter(|dir| dir.length_squared() > 1e-6)
                    .map(|dir| Vec3::new(dir.x, 0.0, dir.y))
                    .and_then(normalize_flat_direction)
                    .unwrap_or(direction);

                let dir2 = Vec2::new(fire_direction.x, fire_direction.z).normalize_or_zero();
                let desired_yaw = dir2.x.atan2(dir2.y);
                let forward = transform.rotation * Vec3::Z;
                let body_yaw = forward.x.atan2(forward.z);
                let delta = wrap_angle_pi(desired_yaw - body_yaw);

                let aim_rot = Quat::from_rotation_y(delta);
                commands.entity(entity).insert(AimYawDelta(delta));
                let aim_direction =
                    normalize_flat_direction((transform.rotation * aim_rot) * Vec3::Z)
                        .unwrap_or(fire_direction);
                let muzzle_pos = anchor_world_position(
                    &library,
                    prefab_id.0,
                    transform,
                    &ranged.muzzle,
                    aim_rot,
                )
                .unwrap_or_else(|| transform.translation + Vec3::Y * 1.0);

                let radius = projectile_collider_radius(projectile_def);
                let spawn_pos = muzzle_pos + aim_direction * (radius * 1.05 + 0.01);
                let velocity = aim_direction * projectile_profile.speed;
                let yaw = aim_direction.x.atan2(aim_direction.z);
                let rotation = Quat::from_rotation_y(yaw);

                let mut bullet_entity = commands.spawn((
                    ObjectId::new_v4(),
                    ObjectPrefabId(ranged.projectile_prefab),
                    ProjectileOwner(entity),
                    Transform::from_translation(spawn_pos).with_rotation(rotation),
                    Visibility::Inherited,
                    Bullet {
                        velocity,
                        ttl_secs: projectile_profile.ttl_secs,
                    },
                    Collider { radius },
                ));
                visuals::spawn_object_visuals(
                    &mut bullet_entity,
                    &library,
                    &visuals_spawn.asset_server,
                    &visuals_spawn.assets,
                    &mut visuals_spawn.meshes,
                    &mut visuals_spawn.materials,
                    &mut visuals_spawn.material_cache,
                    &mut visuals_spawn.mesh_cache,
                    ranged.projectile_prefab,
                    None,
                );
            }
        }
    }

    if melee_kills > 0 {
        let Ok(player_entity) = player_entity_q.single() else {
            return;
        };
        let Ok((_e, _t, _c, _p, _enemy, _player, mut player_health, _died)) =
            units.get_mut(player_entity)
        else {
            return;
        };
        let health_gains =
            crate::enemies::apply_kill_rewards(&mut game, &mut player_health, melee_kills);
        if health_gains > 0 {
            if let Ok(player_transform) = player_transform_q.single() {
                let popup_pos = player_transform.translation + Vec3::Y * PLAYER_HEALTH_BAR_OFFSET_Y;
                for _ in 0..health_gains {
                    health_events.write(HealthChangeEvent {
                        world_pos: popup_pos,
                        delta: 1,
                        is_hero: true,
                    });
                }
            }
        }
    }
}

pub(crate) fn move_bullets(
    time: Res<Time>,
    mut bullets: Query<(&mut Transform, &Bullet, Option<&Children>), Without<BulletVisual>>,
    mut bullet_visuals: Query<&mut Transform, (With<BulletTrailVisual>, Without<Bullet>)>,
) {
    let dt = time.delta_secs();
    for (mut transform, bullet, children) in &mut bullets {
        let previous = transform.translation;
        transform.translation += bullet.velocity * dt;

        let Some(children) = children else {
            continue;
        };

        let travel = transform.translation.distance(previous);
        let travel_for_trail = travel.min(bullet.velocity.length() * BULLET_TRAIL_MAX_SECS);
        let stretched_length = BULLET_MESH_LENGTH + travel_for_trail;
        let scale_z = stretched_length / BULLET_MESH_LENGTH;
        let offset_z = -travel_for_trail / 2.0;

        for child in children.iter() {
            let Ok(mut child_transform) = bullet_visuals.get_mut(child) else {
                continue;
            };
            child_transform.translation = Vec3::new(0.0, 0.0, offset_z);
            child_transform.scale = Vec3::new(1.0, 1.0, scale_z);
        }
    }
}

pub(crate) fn despawn_expired_bullets(
    mut commands: Commands,
    time: Res<Time>,
    mut bullets: Query<(Entity, &Transform, &mut Bullet)>,
) {
    let dt = time.delta_secs();
    for (entity, transform, mut bullet) in &mut bullets {
        bullet.ttl_secs -= dt;
        let pos = transform.translation;
        let out_of_bounds =
            pos.x.abs() > WORLD_HALF_SIZE * 1.4 || pos.z.abs() > WORLD_HALF_SIZE * 1.4;
        if bullet.ttl_secs <= 0.0 || out_of_bounds {
            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn bullet_object_collisions(
    mut commands: Commands,
    bullets: Query<(Entity, &Transform, &Collider), With<Bullet>>,
    projectile_prefabs: Query<&ObjectPrefabId, With<Bullet>>,
    library: Res<ObjectLibrary>,
    assets: Option<Res<SceneAssets>>,
    objects: Query<(&Transform, &AabbCollider, &ObjectPrefabId), With<BuildObject>>,
) {
    let assets = assets.as_deref();
    let bullet_blockers: Vec<(Vec2, Vec2)> = objects
        .iter()
        .filter_map(|(transform, collider, prefab_id)| {
            library.interaction(prefab_id.0).blocks_bullets.then_some((
                Vec2::new(transform.translation.x, transform.translation.z),
                collider.half_extents,
            ))
        })
        .collect();
    let laser_blockers: Vec<(Vec2, Vec2)> = objects
        .iter()
        .filter_map(|(transform, collider, prefab_id)| {
            library.interaction(prefab_id.0).blocks_laser.then_some((
                Vec2::new(transform.translation.x, transform.translation.z),
                collider.half_extents,
            ))
        })
        .collect();

    for (entity, transform, collider) in &bullets {
        let prefab_id = projectile_prefabs.get(entity).ok();
        let obstacle_rule = prefab_id
            .and_then(|prefab_id| library.projectile(prefab_id.0))
            .map(|profile| profile.obstacle_rule)
            .unwrap_or(crate::object::registry::ProjectileObstacleRule::BulletsBlockers);
        let spawn_energy_impact = prefab_id
            .and_then(|prefab_id| library.projectile(prefab_id.0))
            .map(|profile| profile.spawn_energy_impact)
            .unwrap_or(false);

        let obstacles = match obstacle_rule {
            crate::object::registry::ProjectileObstacleRule::BulletsBlockers => &bullet_blockers,
            crate::object::registry::ProjectileObstacleRule::LaserBlockers => &laser_blockers,
        };
        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if obstacles.iter().any(|(ob_center, ob_half)| {
            circle_intersects_aabb_xz(center, collider.radius, *ob_center, *ob_half)
        }) {
            if spawn_energy_impact {
                if let Some(assets) = assets {
                    spawn_energy_impact_particles(&mut commands, assets, transform.translation);
                }
            }
            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn bullet_enemy_collisions(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut effects: ResMut<EnemyKillEffects>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    player_entity_q: Query<Entity, With<Player>>,
    player_transform_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    assets: Option<Res<SceneAssets>>,
    bullets: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&ProjectileOwner>,
        ),
        With<Bullet>,
    >,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Enemy>,
            Option<&Player>,
            &mut Health,
            Option<&Died>,
        ),
        Or<(With<Commandable>, With<Enemy>)>,
    >,
) {
    let assets = assets.as_deref();
    if game.game_over {
        return;
    }

    let wall_time = time.elapsed_secs();
    let bullet_data: Vec<(Entity, Vec3, f32, i32, bool, Option<Entity>)> = bullets
        .iter()
        .map(|(entity, transform, collider, prefab_id, owner)| {
            let (damage, spawn_energy_impact) = library
                .projectile(prefab_id.0)
                .map(|profile| (profile.damage, profile.spawn_energy_impact))
                .unwrap_or((BULLET_DAMAGE, false));
            (
                entity,
                transform.translation,
                collider.radius,
                damage,
                spawn_energy_impact,
                owner.map(|o| o.0),
            )
        })
        .collect();

    let mut bullets_to_despawn: HashSet<Entity> = HashSet::default();
    let mut enemy_kills: u32 = 0;

    for (
        target_entity,
        target_transform,
        target_collider,
        prefab_id,
        enemy,
        player,
        mut health,
        died,
    ) in &mut units
    {
        if died.is_some() || health.current <= 0 {
            continue;
        }

        for (bullet_entity, bullet_pos, bullet_radius, bullet_damage, spawn_energy_impact, owner) in
            &bullet_data
        {
            if bullets_to_despawn.contains(bullet_entity) {
                continue;
            }
            if owner.is_some_and(|owner| owner == target_entity) {
                continue;
            }
            if circles_intersect_xz(
                *bullet_pos,
                *bullet_radius,
                target_transform.translation,
                target_collider.radius,
            ) {
                bullets_to_despawn.insert(*bullet_entity);
                if *spawn_energy_impact {
                    if let Some(assets) = assets {
                        spawn_energy_impact_particles(&mut commands, assets, *bullet_pos);
                    }
                }

                let popup_offset_y = library
                    .health_bar_offset_y(prefab_id.0)
                    .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
                let popup_pos = target_transform.translation + Vec3::Y * popup_offset_y;
                health_events.write(HealthChangeEvent {
                    world_pos: popup_pos,
                    delta: -*bullet_damage,
                    is_hero: player.is_some(),
                });

                health.current = (health.current - *bullet_damage).max(0);
                if health.current <= 0 {
                    crate::unit_health::start_die_motion(
                        &mut commands,
                        wall_time,
                        &library,
                        target_entity,
                        prefab_id.0,
                        target_transform,
                    );
                    if enemy.is_some() {
                        effects.0.push(target_transform.translation);
                        enemy_kills = enemy_kills.saturating_add(1);
                    } else if player.is_some() {
                        game.game_over = true;
                        info!(
                            "GAME OVER. Final score: {}. Press R to restart.",
                            game.score
                        );
                    }
                    break;
                }
            }
        }
    }

    if enemy_kills > 0 {
        let Ok(player_entity) = player_entity_q.single() else {
            return;
        };
        let Ok((_e, _t, _c, _p, _enemy, _player, mut player_health, _died)) =
            units.get_mut(player_entity)
        else {
            return;
        };
        let health_gains =
            crate::enemies::apply_kill_rewards(&mut game, &mut player_health, enemy_kills);
        if health_gains > 0 {
            if let Ok(player_transform) = player_transform_q.single() {
                let popup_pos = player_transform.translation + Vec3::Y * PLAYER_HEALTH_BAR_OFFSET_Y;
                for _ in 0..health_gains {
                    health_events.write(HealthChangeEvent {
                        world_pos: popup_pos,
                        delta: 1,
                        is_hero: true,
                    });
                }
            }
        }
    }

    for entity in bullets_to_despawn {
        commands.entity(entity).try_despawn();
    }
}
