use bevy::prelude::*;

use crate::object::registry::{
    builtin_object_id, ColliderProfile, MaterialKey, MeshKey, ObjectDef, ObjectInteraction,
    ObjectPartDef, PrimitiveVisualDef,
};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/buildings/atoms/block_slice_1";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "BlockSlice1";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::ONE,
        collider: ColliderProfile::None,
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![ObjectPartDef::primitive(
            PrimitiveVisualDef::Mesh {
                mesh: MeshKey::UnitCube,
                material: MaterialKey::BuildBlock { index: 1 },
            },
            Transform::IDENTITY,
        )],
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    }
}
