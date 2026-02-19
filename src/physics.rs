use bevy::prelude::*;

use crate::constants::*;
use crate::geometry::{circle_intersects_aabb_xz, resolve_circle_against_aabbs, safe_abs_scale_y};
use crate::object::registry::{MovementBlockRule, ObjectLibrary};
use crate::types::*;

#[derive(Clone, Copy)]
struct BuildAabb {
    center: Vec2,
    half: Vec2,
    bottom_y: f32,
    top_y: f32,
    movement_block: Option<MovementBlockRule>,
    supports_standing: bool,
}

pub(crate) fn separate_enemies(
    library: Res<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        (With<BuildObject>, Without<Enemy>),
    >,
    mut enemies: Query<
        (
            &mut Transform,
            &Collider,
            &Enemy,
            &ObjectPrefabId,
            Option<&DogPounce>,
        ),
        With<Enemy>,
    >,
) {
    let mut aabbs: Vec<BuildAabb> = Vec::new();
    for (transform, collider, dimensions, prefab_id) in &objects {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        let aabb = BuildAabb {
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
        };
        aabbs.push(aabb);
    }

    {
        let mut pairs = enemies.iter_combinations_mut::<2>();
        while let Some(
            [(mut a_transform, a_collider, _a_enemy, _a_type, _a_pounce), (mut b_transform, b_collider, _b_enemy, _b_type, _b_pounce)],
        ) = pairs.fetch_next()
        {
            let a = Vec2::new(a_transform.translation.x, a_transform.translation.z);
            let b = Vec2::new(b_transform.translation.x, b_transform.translation.z);
            let delta = b - a;
            let dist2 = delta.length_squared();
            let min_dist = a_collider.radius + b_collider.radius;
            if dist2 >= min_dist * min_dist {
                continue;
            }

            let dir = if dist2 > 1e-6 {
                delta / dist2.sqrt()
            } else {
                Vec2::X
            };
            let overlap = min_dist - dist2.sqrt().max(1e-6);
            let push = dir * (overlap * 0.5);

            a_transform.translation.x -= push.x;
            a_transform.translation.z -= push.y;
            b_transform.translation.x += push.x;
            b_transform.translation.z += push.y;
        }
    }

    for (mut transform, collider, enemy, prefab_id, pounce) in &mut enemies {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = enemy.origin_y * scale_y;
        let radius = collider.radius;
        let mut pos = Vec2::new(transform.translation.x, transform.translation.z);
        let current_ground_y = if let Some(pounce) = pounce {
            (pounce.start.y - origin_y).max(0.0)
        } else {
            (transform.translation.y - origin_y).max(0.0)
        };
        let height = library
            .size(prefab_id.0)
            .map(|size| size.y * scale_y)
            .unwrap_or(HERO_HEIGHT_WORLD * scale_y);

        let mut obstacles: Vec<(Vec2, Vec2)> = Vec::with_capacity(aabbs.len());
        obstacles.extend(aabbs.iter().filter_map(|ob| {
            let Some(rule) = ob.movement_block else {
                return None;
            };
            match rule {
                MovementBlockRule::Always => Some((ob.center, ob.half)),
                MovementBlockRule::UpperBodyFraction(fraction) => {
                    let plane_y = current_ground_y + height * fraction;
                    (ob.top_y > plane_y && ob.bottom_y < plane_y).then_some((ob.center, ob.half))
                }
            }
        }));
        pos = resolve_circle_against_aabbs(pos, radius, &obstacles);

        pos.x = pos
            .x
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        pos.y = pos
            .y
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);

        let y = if pounce.is_some() {
            transform.translation.y
        } else {
            let mut ground_y = 0.0f32;
            for ob in &aabbs {
                if !ob.supports_standing {
                    continue;
                }
                let plane_y = match ob.movement_block {
                    Some(MovementBlockRule::UpperBodyFraction(fraction)) => {
                        current_ground_y + height * fraction
                    }
                    _ => f32::INFINITY,
                };
                if ob.top_y > plane_y {
                    continue;
                }
                if circle_intersects_aabb_xz(pos, radius, ob.center, ob.half) {
                    ground_y = ground_y.max(ob.top_y);
                }
            }
            ground_y + origin_y
        };
        transform.translation = Vec3::new(pos.x, y, pos.y);
    }
}

pub(crate) fn separate_commandables(
    mode: Res<State<GameMode>>,
    game: Res<Game>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        (With<BuildObject>, Without<Commandable>, Without<Enemy>),
    >,
    mut units: Query<
        (
            Entity,
            &mut Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
        ),
        (With<Commandable>, Without<BuildObject>, Without<Enemy>),
    >,
) {
    if matches!(mode.get(), GameMode::Play) && game.game_over {
        return;
    }

    let mut aabbs: Vec<BuildAabb> = Vec::new();
    for (transform, collider, dimensions, prefab_id) in &objects {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        let aabb = BuildAabb {
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
        };
        aabbs.push(aabb);
    }

    {
        let mut pairs = units.iter_combinations_mut::<2>();
        while let Some(
            [(_a_entity, mut a_transform, a_collider, _a_prefab_id, _a_player), (_b_entity, mut b_transform, b_collider, _b_prefab_id, _b_player)],
        ) = pairs.fetch_next()
        {
            let a = Vec2::new(a_transform.translation.x, a_transform.translation.z);
            let b = Vec2::new(b_transform.translation.x, b_transform.translation.z);
            let delta = b - a;
            let dist2 = delta.length_squared();

            let a_radius = if a_collider.radius.is_finite() {
                a_collider.radius.max(COMMANDABLE_MIN_SEPARATION_RADIUS)
            } else {
                COMMANDABLE_MIN_SEPARATION_RADIUS
            };
            let b_radius = if b_collider.radius.is_finite() {
                b_collider.radius.max(COMMANDABLE_MIN_SEPARATION_RADIUS)
            } else {
                COMMANDABLE_MIN_SEPARATION_RADIUS
            };
            let min_dist = a_radius + b_radius;
            if dist2 >= min_dist * min_dist {
                continue;
            }

            let dir = if dist2 > 1e-6 {
                delta / dist2.sqrt()
            } else {
                Vec2::X
            };
            let overlap = min_dist - dist2.sqrt().max(1e-6);
            let push = dir * (overlap * 0.5);

            a_transform.translation.x -= push.x;
            a_transform.translation.z -= push.y;
            b_transform.translation.x += push.x;
            b_transform.translation.z += push.y;
        }
    }

    for (_entity, mut transform, collider, prefab_id, player) in &mut units {
        let scale_y = safe_abs_scale_y(transform.scale);
        let radius = if collider.radius.is_finite() {
            collider.radius.max(COMMANDABLE_MIN_SEPARATION_RADIUS)
        } else {
            COMMANDABLE_MIN_SEPARATION_RADIUS
        };

        let (origin_y, height) = if player.is_some() {
            (PLAYER_Y, HERO_HEIGHT_WORLD)
        } else {
            (
                library.ground_origin_y_or_default(prefab_id.0) * scale_y,
                library
                    .size(prefab_id.0)
                    .map(|size| size.y * scale_y)
                    .unwrap_or(HERO_HEIGHT_WORLD * scale_y),
            )
        };

        let mobility_mode = library
            .mobility(prefab_id.0)
            .map(|mobility| mobility.mode)
            .unwrap_or(crate::object::registry::MobilityMode::Ground);

        let mut pos = Vec2::new(transform.translation.x, transform.translation.z);
        let current_ground_y = (transform.translation.y - origin_y).max(0.0);

        let mut obstacles: Vec<(Vec2, Vec2)> = Vec::with_capacity(aabbs.len());
        obstacles.extend(aabbs.iter().filter_map(|ob| {
            let Some(rule) = ob.movement_block else {
                return None;
            };
            match rule {
                MovementBlockRule::Always => Some((ob.center, ob.half)),
                MovementBlockRule::UpperBodyFraction(fraction) => {
                    let plane_y = current_ground_y + height * fraction;
                    (ob.top_y > plane_y && ob.bottom_y < plane_y).then_some((ob.center, ob.half))
                }
            }
        }));
        pos = resolve_circle_against_aabbs(pos, radius, &obstacles);

        pos.x = pos
            .x
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);
        pos.y = pos
            .y
            .clamp(-WORLD_HALF_SIZE + radius, WORLD_HALF_SIZE - radius);

        let y = match mobility_mode {
            crate::object::registry::MobilityMode::Air => transform.translation.y,
            crate::object::registry::MobilityMode::Ground => {
                let mut ground_y = 0.0f32;
                for ob in &aabbs {
                    if !ob.supports_standing {
                        continue;
                    }
                    let plane_y = match ob.movement_block {
                        Some(MovementBlockRule::UpperBodyFraction(fraction)) => {
                            current_ground_y + height * fraction
                        }
                        _ => f32::INFINITY,
                    };
                    if ob.top_y > plane_y {
                        continue;
                    }
                    if circle_intersects_aabb_xz(pos, radius, ob.center, ob.half) {
                        ground_y = ground_y.max(ob.top_y);
                    }
                }
                ground_y + origin_y
            }
        };

        transform.translation = Vec3::new(pos.x, y, pos.y);
    }
}

pub(crate) fn separate_player_from_enemies(
    library: Res<ObjectLibrary>,
    mut player_q: Query<(&mut Transform, &Collider), With<Player>>,
    enemies: Query<(&Transform, &Collider), (With<Enemy>, Without<Player>)>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        (With<BuildObject>, Without<Player>),
    >,
    game: Res<Game>,
) {
    if game.game_over {
        return;
    }

    let (mut player_transform, player_collider) = match player_q.single_mut() {
        Ok(v) => v,
        Err(_) => return,
    };

    let mut aabbs: Vec<BuildAabb> = Vec::new();
    for (transform, collider, dimensions, prefab_id) in &objects {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        let aabb = BuildAabb {
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
            movement_block: interaction.movement_block,
            supports_standing: interaction.supports_standing,
        };
        aabbs.push(aabb);
    }

    let current_ground_y = (player_transform.translation.y - PLAYER_Y).max(0.0);
    let mut blocking_obstacles: Vec<(Vec2, Vec2)> = Vec::with_capacity(aabbs.len());
    blocking_obstacles.extend(aabbs.iter().filter_map(|ob| {
        let Some(rule) = ob.movement_block else {
            return None;
        };
        match rule {
            MovementBlockRule::Always => Some((ob.center, ob.half)),
            MovementBlockRule::UpperBodyFraction(fraction) => {
                let plane_y = current_ground_y + HERO_HEIGHT_WORLD * fraction;
                (ob.top_y > plane_y && ob.bottom_y < plane_y).then_some((ob.center, ob.half))
            }
        }
    }));

    let player_radius = player_collider.radius;
    let mut player_pos = Vec2::new(
        player_transform.translation.x,
        player_transform.translation.z,
    );

    for _ in 0..6 {
        let mut moved = false;

        for (enemy_transform, enemy_collider) in &enemies {
            let enemy_pos = Vec2::new(enemy_transform.translation.x, enemy_transform.translation.z);
            let delta = player_pos - enemy_pos;
            let dist2 = delta.length_squared();
            let min_dist = player_radius + enemy_collider.radius;
            let min_dist2 = min_dist * min_dist;
            if dist2 >= min_dist2 {
                continue;
            }

            let dir = if dist2 > 1e-8 {
                delta / dist2.sqrt()
            } else {
                Vec2::X
            };

            let overlap = min_dist - dist2.sqrt().max(1e-6);
            player_pos += dir * overlap;
            moved = true;
        }

        player_pos = resolve_circle_against_aabbs(player_pos, player_radius, &blocking_obstacles);
        player_pos.x = player_pos.x.clamp(
            -WORLD_HALF_SIZE + player_radius,
            WORLD_HALF_SIZE - player_radius,
        );
        player_pos.y = player_pos.y.clamp(
            -WORLD_HALF_SIZE + player_radius,
            WORLD_HALF_SIZE - player_radius,
        );

        if !moved {
            break;
        }
    }

    let mut ground_y = 0.0f32;
    for ob in &aabbs {
        if !ob.supports_standing {
            continue;
        }
        let plane_y = match ob.movement_block {
            Some(MovementBlockRule::UpperBodyFraction(fraction)) => {
                current_ground_y + HERO_HEIGHT_WORLD * fraction
            }
            _ => f32::INFINITY,
        };
        if ob.top_y > plane_y {
            continue;
        }
        if circle_intersects_aabb_xz(player_pos, player_radius, ob.center, ob.half) {
            ground_y = ground_y.max(ob.top_y);
        }
    }
    player_transform.translation = Vec3::new(player_pos.x, ground_y + PLAYER_Y, player_pos.y);
}

#[cfg(test)]
mod tests {
    use super::*;

    use bevy::ecs::system::RunSystemOnce;

    use crate::geometry::push_circle_out_of_aabb_xz;
    use crate::object::registry::{
        ColliderProfile, MobilityDef, MobilityMode, ObjectDef, ObjectInteraction,
    };

    #[test]
    fn commandable_is_pushed_out_of_blocking_build_object() {
        let mut library = ObjectLibrary::default();

        let building_id: u128 = 0x1234;
        library.upsert(ObjectDef {
            object_id: building_id,
            label: "TestBuilding".into(),
            size: Vec3::new(2.0, 2.0, 2.0),
            ground_origin_y: None,
            collider: ColliderProfile::AabbXZ {
                half_extents: Vec2::splat(1.0),
            },
            interaction: ObjectInteraction {
                blocks_bullets: true,
                blocks_laser: true,
                movement_block: Some(MovementBlockRule::Always),
                supports_standing: false,
            },
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let unit_id: u128 = 0x5678;
        library.upsert(ObjectDef {
            object_id: unit_id,
            label: "TestUnit".into(),
            size: Vec3::new(1.0, 2.0, 1.0),
            ground_origin_y: None,
            collider: ColliderProfile::CircleXZ { radius: 0.5 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut world = World::new();
        world.insert_resource(State::<GameMode>::new(GameMode::Build));
        world.insert_resource(Game::default());
        world.insert_resource(library);

        world.spawn((
            BuildObject,
            ObjectPrefabId(building_id),
            AabbCollider {
                half_extents: Vec2::splat(1.0),
            },
            BuildDimensions {
                size: Vec3::new(2.0, 2.0, 2.0),
            },
            Transform::from_xyz(0.0, 1.0, 0.0),
        ));

        let unit_entity = world
            .spawn((
                Commandable,
                ObjectPrefabId(unit_id),
                Collider { radius: 0.5 },
                Transform::from_xyz(0.0, 1.0, 0.0),
            ))
            .id();

        let _ = world.run_system_once(separate_commandables);

        let transform = world.get::<Transform>(unit_entity).expect("unit transform");
        let pos = Vec2::new(transform.translation.x, transform.translation.z);
        assert!(
            push_circle_out_of_aabb_xz(pos, 0.5, Vec2::ZERO, Vec2::splat(1.0)).is_none(),
            "unit should not overlap the blocking build object after separation: pos={pos:?}",
        );
    }

    #[test]
    fn commandable_ground_origin_scales_with_transform() {
        let mut library = ObjectLibrary::default();

        let unit_id: u128 = 0x90ab;
        library.upsert(ObjectDef {
            object_id: unit_id,
            label: "ScaledUnit".into(),
            size: Vec3::new(1.0, 2.0, 1.0),
            ground_origin_y: None,
            collider: ColliderProfile::CircleXZ { radius: 0.5 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut world = World::new();
        world.insert_resource(State::<GameMode>::new(GameMode::Build));
        world.insert_resource(Game::default());
        world.insert_resource(library);

        let unit_entity = world
            .spawn((
                Commandable,
                ObjectPrefabId(unit_id),
                Collider { radius: 1.0 },
                Transform::from_xyz(0.0, 2.0, 0.0).with_scale(Vec3::splat(2.0)),
            ))
            .id();

        let _ = world.run_system_once(separate_commandables);

        let transform = world.get::<Transform>(unit_entity).expect("unit transform");
        assert!(
            (transform.translation.y - 2.0).abs() < 1e-4,
            "expected y≈2.0 for scale=2 (half-height), got {}",
            transform.translation.y
        );
    }

    #[test]
    fn commandable_ground_origin_override_is_used() {
        let mut library = ObjectLibrary::default();

        let unit_id: u128 = 0xBEEF;
        library.upsert(ObjectDef {
            object_id: unit_id,
            label: "GroundedUnit".into(),
            size: Vec3::new(1.0, 2.0, 1.0),
            ground_origin_y: Some(0.25),
            collider: ColliderProfile::CircleXZ { radius: 0.5 },
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 1.0,
            }),
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut world = World::new();
        world.insert_resource(State::<GameMode>::new(GameMode::Build));
        world.insert_resource(Game::default());
        world.insert_resource(library);

        let unit_entity = world
            .spawn((
                Commandable,
                ObjectPrefabId(unit_id),
                Collider { radius: 0.5 },
                Transform::from_xyz(0.0, 10.0, 0.0).with_scale(Vec3::splat(2.0)),
            ))
            .id();

        let _ = world.run_system_once(separate_commandables);

        let transform = world.get::<Transform>(unit_entity).expect("unit transform");
        assert!(
            (transform.translation.y - 0.5).abs() < 1e-4,
            "expected y≈0.5 for ground_origin_y=0.25 and scale=2.0, got {}",
            transform.translation.y
        );
    }
}
