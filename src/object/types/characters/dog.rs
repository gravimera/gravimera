use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, EnemyProfile, EnemyVisualProfile, MobilityDef,
    MobilityMode, ObjectDef, ObjectInteraction,
};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/characters/dog";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Dog";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::new(DOG_RADIUS * 2.0, DOG_HEIGHT_WORLD, DOG_RADIUS * 2.0),
        collider: ColliderProfile::CircleXZ { radius: DOG_RADIUS },
        interaction: ObjectInteraction {
            blocks_bullets: false,
            blocks_laser: false,
            movement_block: None,
            supports_standing: false,
        },
        aim: None,
        mobility: Some(MobilityDef {
            mode: MobilityMode::Ground,
            max_speed: DOG_BASE_SPEED,
        }),
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: Some(Color::srgb(0.95, 0.65, 0.25)),
        health_bar_offset_y: Some(DOG_HEALTH_BAR_OFFSET_Y),
        enemy: Some(EnemyProfile {
            visual: EnemyVisualProfile::Dog,
            origin_y: DOG_ORIGIN_Y,
            base_speed: DOG_BASE_SPEED,
            max_health: DOG_HEALTH,
            stop_distance: None,
            shooter: None,
            turn: None,
            has_pounce: true,
        }),
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
