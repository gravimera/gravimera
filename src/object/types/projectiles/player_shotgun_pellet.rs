use bevy::prelude::*;

use crate::constants::*;
use crate::object::registry::{
    builtin_object_id, ColliderProfile, ObjectDef, ObjectInteraction, ProjectileObstacleRule,
    ProjectileProfile,
};

pub(crate) const OBJECT_KEY: &str = "gravimera/builtin/projectiles/player_shotgun_pellet";
#[allow(dead_code)]
pub(crate) const LABEL: &str = "PlayerShotgunPellet";

pub(crate) fn object_id() -> u128 {
    builtin_object_id(OBJECT_KEY)
}

pub(crate) fn def() -> ObjectDef {
    ObjectDef {
        object_id: object_id(),
        label: LABEL.into(),
        size: Vec3::splat(SHOTGUN_PELLET_RADIUS * 2.0),
        collider: ColliderProfile::CircleXZ {
            radius: SHOTGUN_PELLET_RADIUS,
        },
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: Vec::new(),
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: Some(ProjectileProfile {
            obstacle_rule: ProjectileObstacleRule::BulletsBlockers,
            speed: SHOTGUN_PELLET_SPEED,
            ttl_secs: BULLET_TTL_SECS,
            damage: BULLET_DAMAGE,
            spawn_energy_impact: false,
        }),
        attack: None,
    }
}
