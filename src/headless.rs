use bevy::ecs::message::MessageWriter;
use bevy::prelude::*;

use crate::constants::*;
use crate::object::types::characters;
use crate::object::types::projectiles;
use crate::types::*;

#[derive(Resource)]
pub(crate) struct HeadlessExit {
    pub(crate) timer: Option<Timer>,
}

pub(crate) fn setup_headless(mut commands: Commands, exit: Res<HeadlessExit>) {
    match &exit.timer {
        Some(timer) => println!(
            "Headless mode: running simulation for {:.1}s (use `--headless-seconds 0` to run forever).",
            timer.duration().as_secs_f32()
        ),
        None => println!("Headless mode: running simulation until the process is stopped."),
    }

    commands.spawn((
        ObjectId::new_v4(),
        ObjectPrefabId(characters::hero::object_id()),
        Transform::from_xyz(0.0, PLAYER_Y, 0.0),
        Player,
        Commandable,
        Health::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH),
        LaserDamageAccum::default(),
        Collider {
            radius: PLAYER_RADIUS,
        },
    ));
}

pub(crate) fn headless_move_player(
    time: Res<Time>,
    mut phase: Local<f32>,
    mut player_q: Query<&mut Transform, With<Player>>,
    game: Res<Game>,
) {
    if game.game_over {
        return;
    }

    let mut player_transform = match player_q.single_mut() {
        Ok(t) => t,
        Err(_) => return,
    };

    let dt = time.delta_secs();
    *phase += dt * 0.8;
    let dir = Vec3::new(phase.cos(), 0.0, phase.sin());

    player_transform.translation += dir * (PLAYER_SPEED * 0.35) * dt;
    player_transform.translation.x = player_transform
        .translation
        .x
        .clamp(-WORLD_HALF_SIZE, WORLD_HALF_SIZE);
    player_transform.translation.z = player_transform
        .translation
        .z
        .clamp(-WORLD_HALF_SIZE, WORLD_HALF_SIZE);
    player_transform.translation.y = PLAYER_Y;
}

pub(crate) fn headless_aim_at_nearest_enemy(
    mut aim: ResMut<Aim>,
    mut player_q: Query<&mut Transform, With<Player>>,
    enemies: Query<&Transform, (With<Enemy>, Without<Player>)>,
    game: Res<Game>,
) {
    if game.game_over {
        return;
    }

    let mut player_transform = match player_q.single_mut() {
        Ok(t) => t,
        Err(_) => return,
    };

    let mut best_flat = None;
    let mut best_d2 = f32::INFINITY;

    for enemy_transform in &enemies {
        let to_enemy = enemy_transform.translation - player_transform.translation;
        let flat = Vec3::new(to_enemy.x, 0.0, to_enemy.z);
        let d2 = flat.length_squared();
        if d2 < best_d2 {
            best_d2 = d2;
            best_flat = Some(flat);
        }
    }

    let Some(flat) = best_flat else {
        aim.direction = Vec3::Z;
        return;
    };
    if flat.length_squared() < 0.0001 {
        return;
    }

    aim.direction = flat.normalize();
    let yaw = aim.direction.x.atan2(aim.direction.z);
    player_transform.rotation = Quat::from_rotation_y(yaw);
}

pub(crate) fn headless_shooting(
    mut commands: Commands,
    aim: Res<Aim>,
    mut game: ResMut<Game>,
    muzzles: Res<PlayerMuzzles>,
    player_q: Query<&Transform, With<Player>>,
) {
    if game.game_over {
        return;
    }
    if game.fire_cooldown_secs > 0.0 {
        return;
    }

    let player_transform = match player_q.single() {
        Ok(t) => t,
        Err(_) => return,
    };

    let mut direction = aim.direction;
    direction.y = 0.0;
    if direction.length_squared() < 0.0001 {
        return;
    }
    direction = direction.normalize();

    let muzzle_forward = muzzles.for_weapon(PlayerWeapon::Normal);
    let muzzle = player_transform.translation
        + Vec3::new(0.0, PLAYER_GUN_Y, 0.0)
        + direction * muzzle_forward;
    let spawn_pos = muzzle + direction * (BULLET_MESH_LENGTH * 0.5);
    let velocity = direction * BULLET_SPEED;

    commands.spawn((
        ObjectId::new_v4(),
        ObjectPrefabId(projectiles::player_bullet::object_id()),
        Transform::from_translation(spawn_pos)
            .with_rotation(Quat::from_rotation_y(direction.x.atan2(direction.z))),
        Bullet {
            velocity,
            ttl_secs: BULLET_TTL_SECS,
        },
        Collider {
            radius: BULLET_RADIUS,
        },
    ));

    game.fire_cooldown_secs = FIRE_COOLDOWN_SECS;
}

pub(crate) fn headless_exit_after_timer(
    time: Res<Time<bevy::time::Real>>,
    mut exit: MessageWriter<AppExit>,
    mut headless: ResMut<HeadlessExit>,
    game: Res<Game>,
    player_health: Query<&Health, With<Player>>,
) {
    let Some(timer) = headless.timer.as_mut() else {
        return;
    };

    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }

    let health = player_health.single().ok().map(|h| h.current).unwrap_or(0);
    println!(
        "Headless simulation finished. score: {} | health: {}",
        game.score, health
    );
    exit.write(AppExit::Success);
}
