use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{builtin_object_id, ColliderProfile, ObjectDef, ObjectInteraction};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/buildings/ground";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Ground";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::new(WORLD_HALF_SIZE * 2.4, 0.1, WORLD_HALF_SIZE * 2.4),
        collider: ColliderProfile::None,
        interaction: ObjectInteraction::none(),
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
    }
}
