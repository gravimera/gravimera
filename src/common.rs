use bevy::prelude::*;

use crate::constants::*;
use crate::types::*;

pub(crate) fn tick_cooldowns(time: Res<Time>, mut game: ResMut<Game>) {
    let dt = time.delta_secs();
    game.fire_cooldown_secs = (game.fire_cooldown_secs - dt).max(0.0);
}

pub(crate) fn restart_game(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut game: ResMut<Game>,
    mut camera_yaw: ResMut<CameraYaw>,
    mut camera_pitch: ResMut<CameraPitch>,
    mut camera_focus: ResMut<CameraFocus>,
    mut move_state: ResMut<MoveCommandState>,
    mut player_q: Query<(&mut Transform, Option<&mut PlayerAnimator>), With<Player>>,
    move_orders: Query<Entity, With<MoveOrder>>,
    enemies: Query<Entity, With<Enemy>>,
    bullets: Query<Entity, With<Bullet>>,
    enemy_projectiles: Query<Entity, With<EnemyProjectile>>,
    lasers: Query<Entity, With<Laser>>,
    explosions: Query<Entity, With<ExplosionParticle>>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }

    game.score = 0;
    game.health = PLAYER_MAX_HEALTH;
    game.max_health = PLAYER_MAX_HEALTH;
    game.enemy_spawn.reset();
    game.fire_cooldown_secs = 0.0;
    game.weapon = PlayerWeapon::Normal;
    game.laser_charges = 0;
    game.shotgun_charges = 0;
    game.game_over = false;
    camera_yaw.yaw = 0.0;
    camera_yaw.initialized = false;
    camera_pitch.pitch = 0.0;
    camera_focus.initialized = false;
    for entity in &move_orders {
        commands.entity(entity).remove::<MoveOrder>();
    }
    if let Some(marker) = move_state.marker.take() {
        commands.entity(marker).try_despawn();
    }

    if let Ok((mut player_transform, animator)) = player_q.single_mut() {
        *player_transform = Transform::from_xyz(0.0, PLAYER_Y, 0.0);
        if let Some(mut animator) = animator {
            animator.phase = 0.0;
            animator.last_translation = player_transform.translation;
        }
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
