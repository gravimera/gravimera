use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectPartDef,
};
use crate::object::types::buildings::atoms;

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/buildings/block";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "Block";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    let size = BUILD_BLOCK_SIZE;
    let slice_size = Vec3::new(BUILD_UNIT_SIZE, BUILD_BLOCK_SIZE.y, BUILD_BLOCK_SIZE.z);
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size,
        collider: ColliderProfile::AabbXZ {
            half_extents: Vec2::new(size.x * 0.5, size.z * 0.5),
        },
        interaction: ObjectInteraction {
            blocks_bullets: true,
            blocks_laser: true,
            movement_block: Some(MovementBlockRule::UpperBodyFraction(
                CROSS_BLOCK_BLOCKING_HEIGHT_FRACTION,
            )),
            supports_standing: true,
        },
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![
            ObjectPartDef::object_ref(
                atoms::block_slice_0::object_id(),
                Transform::from_translation(Vec3::new(-BUILD_UNIT_SIZE, 0.0, 0.0))
                    .with_scale(slice_size),
            ),
            ObjectPartDef::object_ref(
                atoms::block_slice_1::object_id(),
                Transform::from_translation(Vec3::ZERO).with_scale(slice_size),
            ),
            ObjectPartDef::object_ref(
                atoms::block_slice_2::object_id(),
                Transform::from_translation(Vec3::new(BUILD_UNIT_SIZE, 0.0, 0.0))
                    .with_scale(slice_size),
            ),
        ],
        minimap_color: Some(Color::srgba(0.82, 0.42, 0.28, 0.55)),
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
