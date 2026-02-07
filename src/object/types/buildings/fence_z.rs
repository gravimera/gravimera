use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, MovementBlockRule, ObjectDef, ObjectInteraction,
    ObjectPartDef,
};
use crate::object::types::buildings::atoms;
use crate::types::FenceAxis;

fn fence_basis(axis: FenceAxis, along: f32, y: f32, across: f32) -> Vec3 {
    match axis {
        FenceAxis::X => Vec3::new(along, y, across),
        FenceAxis::Z => Vec3::new(across, y, along),
    }
}

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/buildings/fence_z";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "FenceZ";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    let axis = FenceAxis::Z;
    let size = Vec3::new(BUILD_FENCE_WIDTH, BUILD_FENCE_HEIGHT, BUILD_FENCE_LENGTH);
    let stake_thick = BUILD_FENCE_WIDTH * 0.85;
    let stick_thick_y = BUILD_UNIT_SIZE * 0.20;
    let stick_thick_across = BUILD_FENCE_WIDTH * 0.35;
    let stake_offset = BUILD_FENCE_LENGTH * 0.5 - BUILD_GRID_SIZE * 0.5;
    let stake_scale = fence_basis(axis, stake_thick, BUILD_FENCE_HEIGHT, stake_thick);
    let stick_length = (BUILD_FENCE_LENGTH - stake_thick * 2.0).max(BUILD_GRID_SIZE);
    let stick_scale = fence_basis(axis, stick_length, stick_thick_y, stick_thick_across);
    let bottom_y = -BUILD_FENCE_HEIGHT * 0.5 + BUILD_UNIT_SIZE * 0.60;
    let top_y = -BUILD_FENCE_HEIGHT * 0.5 + BUILD_UNIT_SIZE * 2.00;
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size,
        collider: ColliderProfile::AabbXZ {
            half_extents: Vec2::new(size.x * 0.5, size.z * 0.5),
        },
        interaction: ObjectInteraction {
            blocks_bullets: false,
            blocks_laser: true,
            movement_block: Some(MovementBlockRule::UpperBodyFraction(
                CROSS_FENCE_BLOCKING_HEIGHT_FRACTION,
            )),
            supports_standing: false,
        },
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![
            ObjectPartDef::object_ref(
                atoms::fence_stake::object_id(),
                Transform::from_translation(fence_basis(axis, -stake_offset, 0.0, 0.0))
                    .with_scale(stake_scale),
            ),
            ObjectPartDef::object_ref(
                atoms::fence_stake::object_id(),
                Transform::from_translation(fence_basis(axis, stake_offset, 0.0, 0.0))
                    .with_scale(stake_scale),
            ),
            ObjectPartDef::object_ref(
                atoms::fence_stick::object_id(),
                Transform::from_translation(fence_basis(axis, 0.0, bottom_y, 0.0))
                    .with_scale(stick_scale),
            ),
            ObjectPartDef::object_ref(
                atoms::fence_stick::object_id(),
                Transform::from_translation(fence_basis(axis, 0.0, top_y, 0.0))
                    .with_scale(stick_scale),
            ),
        ],
        minimap_color: Some(Color::srgba(0.55, 0.58, 0.62, 0.55)),
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
