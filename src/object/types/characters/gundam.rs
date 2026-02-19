use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, EnemyProfile, EnemyShooterProfile, EnemyVisualProfile,
    MobilityDef, MobilityMode, MuzzleProfile, ObjectDef, ObjectInteraction, TurnProfile,
};
use crate::object::types::projectiles;

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/characters/gundam";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Gundam";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::new(
            GUNDAM_RADIUS * 2.0,
            GUNDAM_HEIGHT_WORLD,
            GUNDAM_RADIUS * 2.0,
        ),
        ground_origin_y: None,
        collider: ColliderProfile::CircleXZ {
            radius: GUNDAM_RADIUS,
        },
        interaction: ObjectInteraction {
            blocks_bullets: false,
            blocks_laser: true,
            movement_block: None,
            supports_standing: false,
        },
        aim: None,
        mobility: Some(MobilityDef {
            mode: MobilityMode::Ground,
            max_speed: GUNDAM_BASE_SPEED,
        }),
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: Some(Color::srgb(0.35, 0.98, 0.55)),
        health_bar_offset_y: Some(GUNDAM_HEALTH_BAR_OFFSET_Y),
        enemy: Some(EnemyProfile {
            visual: EnemyVisualProfile::Gundam,
            origin_y: GUNDAM_ORIGIN_Y,
            base_speed: GUNDAM_BASE_SPEED,
            max_health: GUNDAM_HEALTH,
            stop_distance: Some(GUNDAM_STOP_DISTANCE),
            shooter: Some(EnemyShooterProfile::Burst {
                projectile_prefab: projectiles::gundam_energy_ball::object_id(),
                shots_per_burst: GUNDAM_BURST_SHOTS,
                shot_interval_secs: GUNDAM_BURST_SHOT_INTERVAL_SECS,
                charge_secs: GUNDAM_BURST_CHARGE_SECS,
            }),
            turn: Some(TurnProfile {
                max_turn_rate_rads_per_sec: GUNDAM_MAX_TURN_RATE_RADS_PER_SEC,
                turn_to_move_threshold_rads: GUNDAM_TURN_TO_MOVE_THRESHOLD_RADS,
            }),
            has_pounce: false,
        }),
        muzzle: Some(MuzzleProfile {
            gun_y: GUNDAM_GUN_Y,
            torso_depth: GUNDAM_TORSO_DEPTH,
            gun_offset_z: GUNDAM_GUN_OFFSET_Z,
            gun_length: GUNDAM_GUN_LENGTH,
            right_hand_offset: 0.0,
        }),
        projectile: None,
        attack: None,
    }
}
