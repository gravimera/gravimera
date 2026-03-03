use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;
use rand::prelude::*;
use std::time::Duration;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::effects::{spawn_blood_particles, spawn_energy_impact_particles};
use crate::geometry::{
    circle_intersects_aabb_xz, circles_intersect_xz, clamp_world_xz, normalize_flat_direction,
    resolve_circle_against_aabbs, safe_abs_scale_y,
};
use crate::models::{spawn_dog_model, spawn_enemy_human_model, spawn_gundam_model};
use crate::object::registry::{
    ColliderProfile, EnemyShooterProfile, EnemyVisualProfile, MovementBlockRule, ObjectLibrary,
    ProjectileObstacleRule,
};
use crate::object::types::characters;
use crate::types::*;

fn choose_enemy_prefab_id(rng: &mut impl Rng, ratios: &SpawnRatios) -> u128 {
    let roll: f32 = rng.gen();
    if roll < ratios.dog {
        characters::dog::object_id()
    } else if roll < ratios.dog + ratios.human {
        characters::human::object_id()
    } else {
        characters::gundam::object_id()
    }
}

fn wrap_angle_pi(angle: f32) -> f32 {
    (angle + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
}

fn attach_energy_ball_visuals(
    projectile: &mut EntityCommands<'_>,
    assets: &SceneAssets,
    rng: &mut impl Rng,
) {
    projectile.insert((
        Mesh3d(assets.gundam_energy_ball_mesh.clone()),
        MeshMaterial3d(assets.gundam_energy_ball_material.clone()),
        GundamEnergyBallVisual {
            phase: rng.gen_range(0.0..std::f32::consts::TAU),
        },
    ));

    projectile.with_children(|parent| {
        for _ in 0..GUNDAM_ENERGY_ARC_COUNT {
            let mut axis = Vec3::new(
                rng.gen_range(-1.0..1.0),
                rng.gen_range(-1.0..1.0),
                rng.gen_range(-1.0..1.0),
            );
            if axis.length_squared() < 1e-4 {
                axis = Vec3::Y;
            } else {
                axis = axis.normalize();
            }

            parent.spawn((
                Mesh3d(assets.unit_cube_mesh.clone()),
                MeshMaterial3d(assets.gundam_energy_arc_material.clone()),
                Transform::from_scale(Vec3::splat(0.001)),
                Visibility::Inherited,
                GundamEnergyArcVisual {
                    axis,
                    phase: rng.gen_range(0.0..std::f32::consts::TAU),
                },
            ));
        }
    });
}

fn circle_collider_radius(library: &ObjectLibrary, prefab_id: u128) -> f32 {
    match library.collider(prefab_id) {
        Some(ColliderProfile::CircleXZ { radius }) => radius,
        Some(ColliderProfile::AabbXZ { half_extents }) => half_extents.length(),
        _ => 0.5,
    }
}

fn spawn_enemy_common(
    commands: &mut Commands,
    library: &ObjectLibrary,
    enemy_prefab_id: u128,
    center: Vec2,
    speed: f32,
    rng: &mut impl Rng,
) -> Option<Entity> {
    let profile = library.enemy(enemy_prefab_id)?;
    let radius = circle_collider_radius(library, enemy_prefab_id);
    let spawn_pos = Vec3::new(center.x, profile.origin_y, center.y);

    let mut enemy_commands = commands.spawn((
        ObjectId::new_v4(),
        ObjectPrefabId(enemy_prefab_id),
        Transform::from_translation(spawn_pos),
        Visibility::Inherited,
        Enemy {
            speed,
            origin_y: profile.origin_y,
        },
        Health::new(profile.max_health, profile.max_health),
        LaserDamageAccum::default(),
        Collider { radius },
        EnemyAnimator {
            phase: 0.0,
            last_translation: spawn_pos,
        },
    ));

    if profile.has_pounce {
        enemy_commands.try_insert(DogPounceCooldown {
            remaining_secs: 0.0,
            was_in_range: false,
        });
        enemy_commands.try_insert(DogBiteCooldown {
            remaining_secs: rng.gen_range(0.0..DOG_BITE_EVERY_SECS),
        });
    }

    match profile.shooter {
        Some(EnemyShooterProfile::Repeating {
            projectile_prefab,
            every_secs,
        }) => {
            let mut timer = Timer::from_seconds(every_secs, TimerMode::Repeating);
            timer.set_elapsed(Duration::from_secs_f32(
                rng.gen_range(0.0..every_secs.max(0.01)),
            ));
            enemy_commands.try_insert(EnemyShooter {
                timer,
                projectile_prefab,
            });
        }
        Some(EnemyShooterProfile::Burst {
            projectile_prefab: _,
            shots_per_burst,
            shot_interval_secs,
            charge_secs: _,
        }) => {
            enemy_commands.try_insert(GundamShooter {
                cooldown_secs: rng.gen_range(0.0..shot_interval_secs.max(0.01)),
                shots_left: shots_per_burst.max(1),
            });
        }
        None => {}
    }

    Some(enemy_commands.id())
}

fn spawn_enemy_rendered(
    commands: &mut Commands,
    assets: &SceneAssets,
    library: &ObjectLibrary,
    enemy_prefab_id: u128,
    center: Vec2,
) {
    let mut rng = thread_rng();
    let Some(profile) = library.enemy(enemy_prefab_id) else {
        return;
    };
    let speed = profile.base_speed * rng.gen_range(0.85..1.15);
    let Some(entity) =
        spawn_enemy_common(commands, library, enemy_prefab_id, center, speed, &mut rng)
    else {
        return;
    };

    let mut health_root = None;
    let mut health_fill = None;
    commands.entity(entity).with_children(|parent| {
        match profile.visual {
            EnemyVisualProfile::Dog => {
                spawn_dog_model(parent, assets);
            }
            EnemyVisualProfile::Human => {
                spawn_enemy_human_model(parent, assets);
            }
            EnemyVisualProfile::Gundam => {
                spawn_gundam_model(parent, assets);
            }
        }

        let offset_y = library
            .health_bar_offset_y(enemy_prefab_id)
            .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);

        let bar_root = parent
            .spawn((
                Transform::from_xyz(0.0, offset_y, 0.0),
                Visibility::Inherited,
            ))
            .with_children(|bar| {
                bar.spawn((
                    Mesh3d(assets.unit_cube_mesh.clone()),
                    MeshMaterial3d(assets.health_bar_bg_material.clone()),
                    Transform::from_scale(Vec3::new(
                        HEALTH_BAR_WIDTH,
                        HEALTH_BAR_HEIGHT,
                        HEALTH_BAR_DEPTH,
                    )),
                    Visibility::Inherited,
                ));

                health_fill = Some(
                    bar.spawn((
                        Mesh3d(assets.unit_cube_mesh.clone()),
                        MeshMaterial3d(assets.health_bar_fg_material.clone()),
                        Transform::from_translation(Vec3::new(0.0, 0.0, HEALTH_BAR_Z_OFFSET))
                            .with_scale(Vec3::new(
                                HEALTH_BAR_WIDTH,
                                HEALTH_BAR_HEIGHT * HEALTH_BAR_FILL_SCALE,
                                HEALTH_BAR_DEPTH * HEALTH_BAR_FILL_SCALE,
                            )),
                        Visibility::Inherited,
                        HealthBarFill,
                    ))
                    .id(),
                );
            })
            .id();
        health_root = Some(bar_root);
    });

    if let (Some(root), Some(fill)) = (health_root, health_fill) {
        commands.entity(entity).try_insert(HealthBar { root, fill });
    }
}

fn spawn_enemy_headless(
    commands: &mut Commands,
    library: &ObjectLibrary,
    enemy_prefab_id: u128,
    center: Vec2,
) {
    let Some(profile) = library.enemy(enemy_prefab_id) else {
        return;
    };
    let mut rng = thread_rng();
    let speed = profile.base_speed * rng.gen_range(0.85..1.15);
    let _ = spawn_enemy_common(commands, library, enemy_prefab_id, center, speed, &mut rng);
}

pub(crate) fn apply_kill_rewards(game: &mut Game, player_health: &mut Health, kills: u32) -> u32 {
    let mut health_gains = 0u32;
    for _ in 0..kills {
        game.score = game.score.saturating_add(1);
        if game.score % 10 == 0 {
            player_health.max = player_health.max.saturating_add(1).max(1);
            player_health.current =
                (player_health.current.saturating_add(1)).min(player_health.max);
            health_gains += 1;
        }
        if game.score % 5 == 0 {
            game.shotgun_charges = game.shotgun_charges.saturating_add(1);
        }
        if game.score % 20 == 0 {
            game.laser_charges = game.laser_charges.saturating_add(1);
        }
    }

    health_gains
}

pub(crate) fn spawn_enemies(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    assets: Res<SceneAssets>,
    ratios: Res<SpawnRatios>,
    player_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
) {
    if game.game_over {
        return;
    }

    game.enemy_spawn.tick(time.delta());
    if !game.enemy_spawn.just_finished() {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let mut rng = thread_rng();
    let enemy_prefab_id = choose_enemy_prefab_id(&mut rng, &ratios);
    let radius = circle_collider_radius(&library, enemy_prefab_id);
    let height = library
        .size(enemy_prefab_id)
        .map(|size| size.y)
        .unwrap_or(HERO_HEIGHT_WORLD);
    let ground_y = 0.0f32;
    let obstacles: Vec<(Vec2, Vec2)> = objects
        .iter()
        .filter_map(|(transform, collider, dimensions, prefab_id)| {
            let interaction = library.interaction(prefab_id.0);
            let Some(rule) = interaction.movement_block else {
                return None;
            };

            let scale_y = safe_abs_scale_y(transform.scale);
            let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
            let bottom_y = transform.translation.y - origin_y;
            let top_y = bottom_y + dimensions.size.y;
            match rule {
                MovementBlockRule::Always => Some((
                    Vec2::new(transform.translation.x, transform.translation.z),
                    collider.half_extents,
                )),
                MovementBlockRule::UpperBodyFraction(fraction) => {
                    let plane_y = ground_y + height * fraction;
                    (top_y > plane_y && bottom_y < plane_y).then_some((
                        Vec2::new(transform.translation.x, transform.translation.z),
                        collider.half_extents,
                    ))
                }
            }
        })
        .collect();
    let player_center = Vec2::new(
        player_transform.translation.x,
        player_transform.translation.z,
    );
    let mut spawn_center = None;

    for _ in 0..16 {
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let dir = Vec2::new(angle.cos(), angle.sin());
        let mut candidate = player_center + dir * ENEMY_SPAWN_RADIUS;
        candidate.x = clamp_world_xz(candidate.x, radius);
        candidate.y = clamp_world_xz(candidate.y, radius);

        if obstacles
            .iter()
            .any(|(center, half)| circle_intersects_aabb_xz(candidate, radius, *center, *half))
        {
            continue;
        }

        spawn_center = Some(candidate);
        break;
    }

    let spawn_center = spawn_center.unwrap_or_else(|| {
        let angle = rng.gen_range(0.0..std::f32::consts::TAU);
        let dir = Vec2::new(angle.cos(), angle.sin());
        player_center + dir * ENEMY_SPAWN_RADIUS
    });

    let spawn_center = Vec2::new(
        clamp_world_xz(spawn_center.x, radius),
        clamp_world_xz(spawn_center.y, radius),
    );

    spawn_enemy_rendered(
        &mut commands,
        &assets,
        &library,
        enemy_prefab_id,
        spawn_center,
    );
}

pub(crate) fn spawn_enemies_headless(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    ratios: Res<SpawnRatios>,
    player_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
) {
    if game.game_over {
        return;
    }

    game.enemy_spawn.tick(time.delta());
    if !game.enemy_spawn.just_finished() {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let mut rng = thread_rng();
    let enemy_prefab_id = choose_enemy_prefab_id(&mut rng, &ratios);
    let radius = circle_collider_radius(&library, enemy_prefab_id);
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    let dir = Vec3::new(angle.cos(), 0.0, angle.sin());
    let spawn_pos = player_transform.translation + dir * ENEMY_SPAWN_RADIUS;
    let center = Vec2::new(
        clamp_world_xz(spawn_pos.x, radius),
        clamp_world_xz(spawn_pos.z, radius),
    );

    spawn_enemy_headless(&mut commands, &library, enemy_prefab_id, center);
}

pub(crate) fn move_enemies(
    time: Res<Time>,
    game: Res<Game>,
    player_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    mut enemies: Query<
        (
            &mut Transform,
            &Enemy,
            &Collider,
            &ObjectPrefabId,
            Option<&Health>,
            Option<&Died>,
        ),
        (Without<Player>, Without<DogPounce>),
    >,
) {
    if game.game_over {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let dt = time.delta_secs();
    for (mut enemy_transform, enemy, enemy_collider, prefab_id, health, died) in &mut enemies {
        if died.is_some() || health.is_some_and(|health| health.current <= 0) {
            continue;
        }

        let Some(profile) = library.enemy(prefab_id.0) else {
            continue;
        };

        let to_player = player_transform.translation - enemy_transform.translation;
        let flat = Vec3::new(to_player.x, 0.0, to_player.z);
        if flat.length_squared() < 0.0001 {
            continue;
        }
        let dir = flat.normalize();
        let dist = flat.length();

        let stop_dist = profile.stop_distance;
        if let Some(turn_profile) = profile.turn {
            let desired_yaw = dir.x.atan2(dir.z);
            let forward = enemy_transform.rotation * Vec3::Z;
            let current_yaw = forward.x.atan2(forward.z);
            let yaw_error = wrap_angle_pi(desired_yaw - current_yaw);

            let max_step = turn_profile.max_turn_rate_rads_per_sec * dt;
            let turn_step = yaw_error.clamp(-max_step, max_step);
            let turning = yaw_error.abs() > turn_profile.turn_to_move_threshold_rads;

            let stop = stop_dist.is_some_and(|stop| dist <= stop);
            if stop || turning {
                enemy_transform.rotation = Quat::from_rotation_y(current_yaw + turn_step);
                continue;
            }
        } else if stop_dist.is_some_and(|stop| dist <= stop) {
            enemy_transform.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
            continue;
        }

        let radius = enemy_collider.radius;
        let mut pos = Vec2::new(enemy_transform.translation.x, enemy_transform.translation.z);
        let step_dir = if profile.turn.is_some() {
            let forward = enemy_transform.rotation * Vec3::Z;
            let flat_forward = Vec3::new(forward.x, 0.0, forward.z);
            if flat_forward.length_squared() < 1e-6 {
                dir
            } else {
                flat_forward.normalize()
            }
        } else {
            dir
        };

        pos += Vec2::new(step_dir.x, step_dir.z) * enemy.speed * dt;
        pos.x = clamp_world_xz(pos.x, radius);
        pos.y = clamp_world_xz(pos.y, radius);

        enemy_transform.translation.x = pos.x;
        enemy_transform.translation.z = pos.y;
        if profile.turn.is_none() {
            enemy_transform.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
        }
    }
}

pub(crate) fn animate_enemy_models(
    time: Res<Time>,
    mut enemies_q: Query<(&Transform, &Enemy, &mut EnemyAnimator, Option<&Children>), With<Enemy>>,
    mut legs_q: Query<(&EnemyLeg, &mut Transform), Without<Enemy>>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (enemy_transform, enemy, mut animator, children) in &mut enemies_q {
        let delta = enemy_transform.translation - animator.last_translation;
        animator.last_translation = enemy_transform.translation;

        let speed = Vec2::new(delta.x, delta.z).length() / dt;
        let denom = enemy.speed.max(0.01);
        let speed01 = (speed / denom).clamp(0.0, 1.0);

        animator.phase =
            (animator.phase + dt * ENEMY_LEG_SWING_RADS_PER_SEC * speed01) % std::f32::consts::TAU;
        let swing = animator.phase.sin() * ENEMY_LEG_SWING_MAX_RADS * speed01;

        let Some(children) = children else {
            continue;
        };

        for child in children.iter() {
            let Ok((leg, mut leg_transform)) = legs_q.get_mut(child) else {
                continue;
            };
            leg_transform.rotation = Quat::from_rotation_x(swing * leg.group);
        }
    }
}

pub(crate) fn tick_dog_pounce_cooldowns(time: Res<Time>, mut dogs: Query<&mut DogPounceCooldown>) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for mut cooldown in &mut dogs {
        cooldown.remaining_secs = (cooldown.remaining_secs - dt).max(0.0);
    }
}

pub(crate) fn dog_try_start_pounce(
    mut commands: Commands,
    game: Res<Game>,
    player_q: Query<(&Transform, &Collider), With<Player>>,
    mut dogs: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &Enemy,
            &mut DogPounceCooldown,
            Option<&Health>,
            Option<&Died>,
        ),
        (With<Enemy>, Without<DogPounce>),
    >,
) {
    if game.game_over {
        return;
    }

    let (player_transform, player_collider) = match player_q.single() {
        Ok(v) => v,
        Err(_) => return,
    };

    let player_pos = player_transform.translation;
    let player_radius = player_collider.radius;
    let mut rng = thread_rng();

    for (entity, transform, collider, enemy, mut cooldown, health, died) in &mut dogs {
        if died.is_some() || health.is_some_and(|health| health.current <= 0) {
            continue;
        }

        let delta = Vec2::new(
            player_pos.x - transform.translation.x,
            player_pos.z - transform.translation.z,
        );
        let dist = delta.length();
        if dist > DOG_POUNCE_TRIGGER_RANGE {
            cooldown.was_in_range = false;
            continue;
        }

        if cooldown.was_in_range {
            continue;
        }
        cooldown.was_in_range = true;

        if cooldown.remaining_secs > 0.0 {
            continue;
        }

        let min_separation = player_radius + collider.radius + 0.08;
        if dist <= min_separation {
            continue;
        }

        if rng.gen::<f32>() >= DOG_POUNCE_CHANCE {
            cooldown.remaining_secs = DOG_POUNCE_FAIL_COOLDOWN_SECS;
            continue;
        }

        let start = transform.translation;
        let player_ground_y = (player_pos.y - PLAYER_Y).max(0.0);
        let end = Vec3::new(player_pos.x, player_ground_y + enemy.origin_y, player_pos.z);
        let distance = Vec2::new(end.x - start.x, end.z - start.z).length();
        let duration = (distance / DOG_POUNCE_SPEED)
            .clamp(DOG_POUNCE_MIN_DURATION_SECS, DOG_POUNCE_MAX_DURATION_SECS);
        let arc_height = DOG_POUNCE_HEIGHT_BASE + distance * DOG_POUNCE_HEIGHT_PER_METER;

        commands.entity(entity).try_insert(DogPounce {
            start,
            end,
            elapsed_secs: 0.0,
            duration_secs: duration,
            arc_height,
            did_damage: false,
        });
        cooldown.remaining_secs = DOG_POUNCE_COOLDOWN_SECS;
    }
}

pub(crate) fn update_dog_pounces(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    assets: Option<Res<SceneAssets>>,
    library: Res<ObjectLibrary>,
    objects: Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        (With<BuildObject>, Without<Enemy>),
    >,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
            Option<&Enemy>,
            &mut Health,
            Option<&Died>,
        ),
        (Or<(With<Commandable>, With<Enemy>)>, Without<DogPounce>),
    >,
    mut pounces: Query<
        (
            Entity,
            &mut Transform,
            &Collider,
            &Enemy,
            &ObjectPrefabId,
            &mut DogPounce,
            Option<&Health>,
            Option<&Died>,
        ),
        (With<Enemy>, Without<Player>),
    >,
) {
    if game.game_over {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let wall_time = time.elapsed_secs();

    #[derive(Clone, Copy)]
    struct ObstacleAabb {
        center: Vec2,
        half: Vec2,
        bottom_y: f32,
        top_y: f32,
        movement_block: Option<MovementBlockRule>,
    }

    let mut all_obstacles: Vec<ObstacleAabb> = Vec::new();
    all_obstacles.reserve(objects.iter().len());
    for (transform, collider, dimensions, prefab_id) in &objects {
        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let bottom_y = transform.translation.y - origin_y;
        let top_y = bottom_y + dimensions.size.y;
        let interaction = library.interaction(prefab_id.0);
        all_obstacles.push(ObstacleAabb {
            center: Vec2::new(transform.translation.x, transform.translation.z),
            half: collider.half_extents,
            bottom_y,
            top_y,
            movement_block: interaction.movement_block,
        });
    }

    let assets = assets.as_deref();
    let mut finished: Vec<Entity> = Vec::new();

    for (entity, mut transform, collider, enemy, prefab_id, mut pounce, health, died) in
        &mut pounces
    {
        if died.is_some() || health.is_some_and(|health| health.current <= 0) {
            commands.entity(entity).try_remove::<DogPounce>();
            continue;
        }

        pounce.elapsed_secs += dt;
        let t = (pounce.elapsed_secs / pounce.duration_secs).clamp(0.0, 1.0);
        let t_smooth = t * t * (3.0 - 2.0 * t);

        let travel = pounce.end - pounce.start;
        let pos = pounce.start + travel * t_smooth;
        let base_y = pos.y;

        let jump = (std::f32::consts::PI * t).sin() * pounce.arc_height;
        let mut xz = Vec2::new(pos.x, pos.z);
        let current_ground_y = (pounce.start.y - enemy.origin_y).max(0.0);
        let height = library
            .size(prefab_id.0)
            .map(|size| size.y)
            .unwrap_or(DOG_HEIGHT_WORLD);
        let blocking_obstacles: Vec<(Vec2, Vec2)> = all_obstacles
            .iter()
            .filter_map(|ob| {
                let Some(rule) = ob.movement_block else {
                    return None;
                };
                match rule {
                    MovementBlockRule::Always => Some((ob.center, ob.half)),
                    MovementBlockRule::UpperBodyFraction(fraction) => {
                        let plane_y = current_ground_y + height * fraction;
                        (ob.top_y > plane_y && ob.bottom_y < plane_y)
                            .then_some((ob.center, ob.half))
                    }
                }
            })
            .collect();
        xz = resolve_circle_against_aabbs(xz, collider.radius, &blocking_obstacles);
        xz.x = xz.x.clamp(
            -WORLD_HALF_SIZE + collider.radius,
            WORLD_HALF_SIZE - collider.radius,
        );
        xz.y = xz.y.clamp(
            -WORLD_HALF_SIZE + collider.radius,
            WORLD_HALF_SIZE - collider.radius,
        );

        transform.translation = Vec3::new(xz.x, base_y + jump, xz.y);

        let dir = Vec3::new(travel.x, 0.0, travel.z);
        if dir.length_squared() > 1e-6 {
            transform.rotation = Quat::from_rotation_y(dir.x.atan2(dir.z));
        }

        if !pounce.did_damage {
            for (
                target_entity,
                target_transform,
                target_collider,
                target_prefab,
                target_player,
                _target_enemy,
                mut target_health,
                target_died,
            ) in &mut units
            {
                if target_entity == entity {
                    continue;
                }
                if target_died.is_some() || target_health.current <= 0 {
                    continue;
                }
                if !circles_intersect_xz(
                    target_transform.translation,
                    target_collider.radius,
                    transform.translation,
                    collider.radius,
                ) {
                    continue;
                }

                pounce.did_damage = true;

                let popup_offset_y = library
                    .health_bar_offset_y(target_prefab.0)
                    .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
                health_events.write(HealthChangeEvent {
                    world_pos: target_transform.translation + Vec3::Y * popup_offset_y,
                    delta: -DOG_POUNCE_DAMAGE,
                    is_hero: target_player.is_some(),
                });

                target_health.current = (target_health.current - DOG_POUNCE_DAMAGE).max(0);

                if let Some(assets) = assets {
                    let hit = target_transform.translation + Vec3::new(0.0, PLAYER_GUN_Y, 0.0);
                    spawn_blood_particles(&mut commands, assets, hit);
                }

                if target_health.current <= 0 {
                    crate::unit_health::start_die_motion(
                        &mut commands,
                        wall_time,
                        &library,
                        target_entity,
                        target_prefab.0,
                        target_transform,
                    );
                    if target_player.is_some() {
                        game.game_over = true;
                        info!(
                            "GAME OVER. Final score: {}. Press R to restart.",
                            game.score
                        );
                    }
                }

                transform.translation.y = base_y;
                commands.entity(entity).try_remove::<DogPounce>();
                break;
            }
        }

        if t >= 1.0 {
            finished.push(entity);
        }
    }

    for entity in finished {
        commands.entity(entity).try_remove::<DogPounce>();
    }
}

pub(crate) fn dog_bite_attack(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    library: Res<ObjectLibrary>,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
            Option<&mut DogBiteCooldown>,
            Option<&DogPounce>,
            &mut Health,
            Option<&Died>,
        ),
        Or<(With<Commandable>, With<Enemy>)>,
    >,
) {
    if game.game_over {
        return;
    }

    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }
    let wall_time = time.elapsed_secs();

    let mut live_targets: Vec<(Entity, Vec3, f32, i32)> = Vec::new();
    let mut biters: Vec<(Entity, Vec3, f32)> = Vec::new();

    for (entity, transform, collider, _prefab, _player, bite, pounce, health, died) in
        units.iter_mut()
    {
        if died.is_none() && health.current > 0 {
            live_targets.push((
                entity,
                transform.translation,
                collider.radius,
                health.current,
            ));
        }

        let Some(mut bite) = bite else {
            continue;
        };
        if pounce.is_some() {
            continue;
        }
        if died.is_some() || health.current <= 0 {
            continue;
        }

        bite.remaining_secs -= dt;
        if bite.remaining_secs > 0.0 {
            continue;
        }

        biters.push((entity, transform.translation, collider.radius));
    }

    for (dog_entity, dog_pos, dog_radius) in biters {
        let mut best: Option<(Entity, f32)> = None;
        for (target_entity, target_pos, target_radius, target_health) in &live_targets {
            if *target_entity == dog_entity {
                continue;
            }
            if *target_health <= 0 {
                continue;
            }

            let delta = Vec2::new(target_pos.x - dog_pos.x, target_pos.z - dog_pos.z);
            let bite_range = target_radius + dog_radius + DOG_BITE_RANGE_PADDING;
            let d2 = delta.length_squared();
            if d2 > bite_range * bite_range {
                continue;
            }

            best = Some(match best {
                None => (*target_entity, d2),
                Some((best_entity, best_d2)) => {
                    if d2 < best_d2 {
                        (*target_entity, d2)
                    } else {
                        (best_entity, best_d2)
                    }
                }
            });
        }

        let Some((target_entity, _d2)) = best else {
            continue;
        };

        let mut player_died = false;
        {
            let Ok((
                _target_entity,
                target_transform,
                _target_collider,
                target_prefab,
                target_player,
                _target_bite,
                _target_pounce,
                mut target_health,
                target_died,
            )) = units.get_mut(target_entity)
            else {
                continue;
            };
            if target_died.is_some() || target_health.current <= 0 {
                continue;
            }

            let popup_offset_y = library
                .health_bar_offset_y(target_prefab.0)
                .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
            health_events.write(HealthChangeEvent {
                world_pos: target_transform.translation + Vec3::Y * popup_offset_y,
                delta: -DOG_BITE_DAMAGE,
                is_hero: target_player.is_some(),
            });

            target_health.current = (target_health.current - DOG_BITE_DAMAGE).max(0);
            if target_health.current <= 0 {
                crate::unit_health::start_die_motion(
                    &mut commands,
                    wall_time,
                    &library,
                    target_entity,
                    target_prefab.0,
                    target_transform,
                );

                if target_player.is_some() {
                    player_died = true;
                    game.game_over = true;
                    info!(
                        "GAME OVER. Final score: {}. Press R to restart.",
                        game.score
                    );
                }
            }
        }

        if let Ok((_e, _t, _c, _p, _pl, bite, _pounce, _health, _died)) = units.get_mut(dog_entity)
        {
            if let Some(mut bite) = bite {
                bite.remaining_secs = DOG_BITE_EVERY_SECS;
            }
        }

        if player_died {
            break;
        }
    }
}

pub(crate) fn enemy_shooting(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    assets: Option<Res<SceneAssets>>,
    player_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    mut enemies: Query<
        (
            &Transform,
            &ObjectPrefabId,
            &mut EnemyShooter,
            Option<&Health>,
            Option<&Died>,
        ),
        With<Enemy>,
    >,
) {
    if game.game_over {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let assets = assets.as_deref();
    let dt = time.delta();
    let player_pos = player_transform.translation;
    let mut rng = thread_rng();

    for (enemy_transform, enemy_type, mut shooter, health, died) in &mut enemies {
        if died.is_some() || health.is_some_and(|health| health.current <= 0) {
            continue;
        }

        shooter.timer.tick(dt);
        if !shooter.timer.just_finished() {
            continue;
        }

        let to_player = player_pos - enemy_transform.translation;
        let Some(direction) = normalize_flat_direction(to_player) else {
            continue;
        };

        let yaw = direction.x.atan2(direction.z);
        let Some(muzzle) = library.muzzle(enemy_type.0) else {
            continue;
        };
        let muzzle_pos = muzzle.world_muzzle_position(enemy_transform, direction);

        let projectile_prefab = shooter.projectile_prefab;
        let Some(projectile_profile) = library.projectile(projectile_prefab) else {
            continue;
        };
        let radius = circle_collider_radius(&library, projectile_prefab);
        let spawn_pos = muzzle_pos + direction * radius;
        let velocity = direction * projectile_profile.speed;

        let mut projectile = commands.spawn((
            ObjectId::new_v4(),
            ObjectPrefabId(projectile_prefab),
            Transform::from_translation(spawn_pos).with_rotation(Quat::from_rotation_y(yaw)),
            Visibility::Inherited,
            EnemyProjectile {
                velocity,
                ttl_secs: projectile_profile.ttl_secs,
            },
            Collider { radius },
        ));

        let Some(assets) = assets else {
            continue;
        };

        if projectile_profile.spawn_energy_impact {
            attach_energy_ball_visuals(&mut projectile, assets, &mut rng);
        } else {
            projectile.insert((
                Mesh3d(assets.enemy_bullet_mesh.clone()),
                MeshMaterial3d(assets.enemy_bullet_material.clone()),
            ));

            projectile.with_children(|parent| {
                parent.spawn((
                    Mesh3d(assets.enemy_bullet_spot_mesh.clone()),
                    MeshMaterial3d(assets.enemy_bullet_spot_material.clone()),
                    Transform::from_translation(Vec3::new(
                        ENEMY_BULLET_MESH_RADIUS * 0.55,
                        ENEMY_BULLET_MESH_RADIUS * 0.25,
                        ENEMY_BULLET_MESH_RADIUS * 0.55,
                    )),
                    Visibility::Inherited,
                ));
            });
        }
    }
}

pub(crate) fn gundam_shooting(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    assets: Option<Res<SceneAssets>>,
    player_q: Query<&Transform, With<Player>>,
    library: Res<ObjectLibrary>,
    mut gundams: Query<
        (
            &Transform,
            &ObjectPrefabId,
            &mut GundamShooter,
            Option<&Health>,
            Option<&Died>,
        ),
        With<Enemy>,
    >,
) {
    if game.game_over {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let assets = assets.as_deref();
    let dt = time.delta_secs();
    let player_pos = player_transform.translation;
    let mut rng = thread_rng();

    for (enemy_transform, enemy_type, mut shooter, health, died) in &mut gundams {
        if died.is_some() || health.is_some_and(|health| health.current <= 0) {
            continue;
        }

        let Some(profile) = library.enemy(enemy_type.0) else {
            continue;
        };
        let Some(muzzle) = library.muzzle(enemy_type.0) else {
            continue;
        };
        let Some(EnemyShooterProfile::Burst {
            projectile_prefab,
            shots_per_burst,
            shot_interval_secs,
            charge_secs,
        }) = profile.shooter
        else {
            continue;
        };

        shooter.cooldown_secs -= dt;

        let to_player = player_pos - enemy_transform.translation;

        let mut safety = 0;
        while shooter.cooldown_secs <= 0.0 && safety < 8 {
            safety += 1;

            let Some(direction) = normalize_flat_direction(to_player) else {
                shooter.cooldown_secs = 0.1;
                break;
            };

            let yaw = direction.x.atan2(direction.z);
            let muzzle_pos = muzzle.world_muzzle_position(enemy_transform, direction);
            let Some(projectile_profile) = library.projectile(projectile_prefab) else {
                break;
            };
            let radius = circle_collider_radius(&library, projectile_prefab);
            let spawn_pos = muzzle_pos + direction * radius;
            let velocity = direction * projectile_profile.speed;

            let mut projectile = commands.spawn((
                ObjectId::new_v4(),
                ObjectPrefabId(projectile_prefab),
                Transform::from_translation(spawn_pos).with_rotation(Quat::from_rotation_y(yaw)),
                Visibility::Inherited,
                EnemyProjectile {
                    velocity,
                    ttl_secs: projectile_profile.ttl_secs,
                },
                Collider { radius },
            ));

            if let Some(assets) = assets {
                if projectile_profile.spawn_energy_impact {
                    attach_energy_ball_visuals(&mut projectile, assets, &mut rng);
                } else {
                    projectile.insert((
                        Mesh3d(assets.enemy_bullet_mesh.clone()),
                        MeshMaterial3d(assets.enemy_bullet_material.clone()),
                    ));
                }
            }

            shooter.shots_left = shooter.shots_left.saturating_sub(1);
            if shooter.shots_left == 0 {
                shooter.shots_left = shots_per_burst.max(1);
                shooter.cooldown_secs += charge_secs;
            } else {
                shooter.cooldown_secs += shot_interval_secs;
            }
        }
    }
}

pub(crate) fn move_enemy_projectiles(
    mut commands: Commands,
    time: Res<Time>,
    game: Res<Game>,
    mut projectiles: Query<(Entity, &mut Transform, &mut EnemyProjectile)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    if game.game_over {
        for (entity, _transform, _projectile) in &mut projectiles {
            commands.entity(entity).try_despawn();
        }
        return;
    }

    for (entity, mut transform, mut projectile) in &mut projectiles {
        projectile.ttl_secs -= dt;
        if projectile.ttl_secs <= 0.0 {
            commands.entity(entity).try_despawn();
            continue;
        }

        transform.translation += projectile.velocity * dt;

        let pos = transform.translation;
        let out_of_bounds =
            pos.x.abs() > WORLD_HALF_SIZE * 1.6 || pos.z.abs() > WORLD_HALF_SIZE * 1.6;
        if out_of_bounds {
            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn enemy_projectile_object_collisions(
    mut commands: Commands,
    assets: Option<Res<SceneAssets>>,
    projectiles: Query<(
        Entity,
        &Transform,
        &Collider,
        &EnemyProjectile,
        &ObjectPrefabId,
    )>,
    library: Res<ObjectLibrary>,
    objects: Query<(&Transform, &AabbCollider, &ObjectPrefabId), With<BuildObject>>,
) {
    let mut bullet_obstacles: Vec<(Vec2, Vec2)> = Vec::new();
    let mut energy_obstacles: Vec<(Vec2, Vec2)> = Vec::new();
    for (transform, collider, prefab_id) in &objects {
        let interaction = library.interaction(prefab_id.0);
        let entry = (
            Vec2::new(transform.translation.x, transform.translation.z),
            collider.half_extents,
        );
        if interaction.blocks_bullets {
            bullet_obstacles.push(entry);
        }
        if interaction.blocks_laser {
            energy_obstacles.push(entry);
        }
    }

    let assets = assets.as_deref();
    for (entity, transform, collider, projectile, prefab_id) in &projectiles {
        if projectile.ttl_secs <= 0.0 {
            continue;
        }

        let Some(projectile_profile) = library.projectile(prefab_id.0) else {
            continue;
        };

        let obstacles = match projectile_profile.obstacle_rule {
            ProjectileObstacleRule::BulletsBlockers => &bullet_obstacles,
            ProjectileObstacleRule::LaserBlockers => &energy_obstacles,
        };
        if obstacles.is_empty() {
            continue;
        }

        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if obstacles.iter().any(|(ob_center, ob_half)| {
            circle_intersects_aabb_xz(center, collider.radius, *ob_center, *ob_half)
        }) {
            if projectile_profile.spawn_energy_impact {
                if let Some(assets) = assets {
                    spawn_energy_impact_particles(&mut commands, assets, transform.translation);
                }
            }

            commands.entity(entity).try_despawn();
        }
    }
}

pub(crate) fn enemy_projectile_player_collisions(
    mut commands: Commands,
    time: Res<Time>,
    mut game: ResMut<Game>,
    mut health_events: MessageWriter<HealthChangeEvent>,
    assets: Option<Res<SceneAssets>>,
    library: Res<ObjectLibrary>,
    projectiles: Query<(
        Entity,
        &Transform,
        &Collider,
        &EnemyProjectile,
        &ObjectPrefabId,
    )>,
    mut units: Query<
        (
            Entity,
            &Transform,
            &Collider,
            &ObjectPrefabId,
            Option<&Player>,
            Option<&Enemy>,
            &mut Health,
            Option<&Died>,
        ),
        Or<(With<Commandable>, With<Enemy>)>,
    >,
) {
    if game.game_over {
        return;
    }

    let wall_time = time.elapsed_secs();
    let assets = assets.as_deref();
    for (entity, transform, collider, projectile, prefab_id) in &projectiles {
        if projectile.ttl_secs <= 0.0 {
            continue;
        }

        let Some(projectile_profile) = library.projectile(prefab_id.0) else {
            continue;
        };

        let mut hit = None;
        for (
            target_entity,
            target_transform,
            target_collider,
            target_prefab,
            target_player,
            _target_enemy,
            mut health,
            died,
        ) in &mut units
        {
            if died.is_some() || health.current <= 0 {
                continue;
            }
            if !circles_intersect_xz(
                target_transform.translation,
                target_collider.radius,
                transform.translation,
                collider.radius,
            ) {
                continue;
            }

            let popup_offset_y = library
                .health_bar_offset_y(target_prefab.0)
                .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);
            health_events.write(HealthChangeEvent {
                world_pos: target_transform.translation + Vec3::Y * popup_offset_y,
                delta: -projectile_profile.damage,
                is_hero: target_player.is_some(),
            });

            health.current = (health.current - projectile_profile.damage).max(0);
            if health.current <= 0 {
                crate::unit_health::start_die_motion(
                    &mut commands,
                    wall_time,
                    &library,
                    target_entity,
                    target_prefab.0,
                    target_transform,
                );
                if target_player.is_some() {
                    game.game_over = true;
                    info!(
                        "GAME OVER. Final score: {}. Press R to restart.",
                        game.score
                    );
                }
            }

            hit = Some(target_entity);
            break;
        }

        if hit.is_none() {
            continue;
        }

        commands.entity(entity).try_despawn();
        if projectile_profile.spawn_energy_impact {
            if let Some(assets) = assets {
                spawn_energy_impact_particles(&mut commands, assets, transform.translation);
            }
        }

        if game.game_over {
            break;
        }
    }
}
