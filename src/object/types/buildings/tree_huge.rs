use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectPartDef,
};
use crate::object::types::buildings::atoms;

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/buildings/tree_huge";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "TreeHuge";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    let variant = 2usize;
    let scale = BUILD_TREE_VARIANT_SCALES[variant];
    let size = BUILD_TREE_BASE_SIZE * scale;
    let trunk_radius = BUILD_UNIT_SIZE * 0.55 * scale;
    let half_height = size.y * 0.5;
    let bottom_y = -half_height;

    let trunk_height = BUILD_UNIT_SIZE * 1.8 * scale;
    let main_height = BUILD_UNIT_SIZE * 2.4 * scale;
    let main_radius = BUILD_UNIT_SIZE * 1.45 * scale;
    let crown_height = BUILD_UNIT_SIZE * 0.8 * scale;
    let crown_radius = BUILD_UNIT_SIZE * 1.05 * scale;
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size,
        collider: ColliderProfile::AabbXZ {
            half_extents: Vec2::splat(trunk_radius),
        },
        interaction: ObjectInteraction {
            blocks_bullets: true,
            blocks_laser: true,
            movement_block: Some(MovementBlockRule::Always),
            supports_standing: false,
        },
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![
            ObjectPartDef::object_ref(
                atoms::tree_trunk_2::object_id(),
                Transform::from_translation(Vec3::new(0.0, bottom_y + trunk_height * 0.5, 0.0))
                    .with_scale(Vec3::new(trunk_radius, trunk_height, trunk_radius)),
            ),
            ObjectPartDef::object_ref(
                atoms::tree_main_2::object_id(),
                Transform::from_translation(Vec3::new(
                    0.0,
                    bottom_y + trunk_height + main_height * 0.5,
                    0.0,
                ))
                .with_scale(Vec3::new(main_radius, main_height, main_radius)),
            ),
            ObjectPartDef::object_ref(
                atoms::tree_crown_2::object_id(),
                Transform::from_translation(Vec3::new(
                    0.0,
                    bottom_y + trunk_height + main_height + crown_height * 0.5,
                    0.0,
                ))
                .with_scale(Vec3::new(crown_radius, crown_height, crown_radius)),
            ),
        ],
        minimap_color: Some(Color::srgba(0.18, 0.55, 0.28, 0.55)),
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
