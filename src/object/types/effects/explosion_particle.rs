use bevy::prelude::*;

use crate::object::registry::{builtin_object_id, ColliderProfile, ObjectDef, ObjectInteraction};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/effects/explosion_particle";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "ExplosionParticle";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::splat(0.2),
        ground_origin_y: None,
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
