use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, EnemyProfile, EnemyShooterProfile, EnemyVisualProfile,
    MobilityDef, MobilityMode, MuzzleProfile, ObjectDef, ObjectInteraction,
};
use crate::object::types::projectiles;

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/characters/human";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Human";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::new(HUMAN_RADIUS * 2.0, HERO_HEIGHT_WORLD, HUMAN_RADIUS * 2.0),
        ground_origin_y: None,
        collider: ColliderProfile::CircleXZ {
            radius: HUMAN_RADIUS,
        },
        interaction: ObjectInteraction {
            blocks_bullets: false,
            blocks_laser: false,
            movement_block: None,
            supports_standing: false,
        },
        aim: None,
        mobility: Some(MobilityDef {
            mode: MobilityMode::Ground,
            max_speed: HUMAN_BASE_SPEED,
        }),
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: Some(Color::srgb(0.98, 0.25, 0.25)),
        health_bar_offset_y: Some(PLAYER_HEALTH_BAR_OFFSET_Y),
        enemy: Some(EnemyProfile {
            visual: EnemyVisualProfile::Human,
            origin_y: HUMAN_ORIGIN_Y,
            base_speed: HUMAN_BASE_SPEED,
            max_health: HUMAN_HEALTH,
            stop_distance: Some(HUMAN_STOP_DISTANCE),
            shooter: Some(EnemyShooterProfile::Repeating {
                projectile_prefab: projectiles::enemy_bullet::object_id(),
                every_secs: ENEMY_FIRE_EVERY_SECS,
            }),
            turn: None,
            has_pounce: false,
        }),
        muzzle: Some(MuzzleProfile {
            gun_y: PLAYER_GUN_Y,
            torso_depth: PLAYER_TORSO_DEPTH,
            gun_offset_z: PLAYER_GUN_OFFSET_Z,
            gun_length: PLAYER_GUN_LENGTH,
            right_hand_offset: HUMAN_BULLET_RIGHT_HAND_OFFSET,
        }),
        projectile: None,
        attack: None,
    }
}
