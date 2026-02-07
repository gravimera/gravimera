use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, MobilityDef, MobilityMode, ObjectDef, ObjectInteraction,
};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/characters/hero";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Hero";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::new(PLAYER_RADIUS * 2.0, HERO_HEIGHT_WORLD, PLAYER_RADIUS * 2.0),
        collider: ColliderProfile::CircleXZ {
            radius: PLAYER_RADIUS,
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
            max_speed: PLAYER_SPEED,
        }),
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: Some(PLAYER_HEALTH_BAR_OFFSET_Y),
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
