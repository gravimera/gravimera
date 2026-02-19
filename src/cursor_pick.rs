use bevy::prelude::*;

use crate::geometry::{point_inside_aabb_xz, safe_abs_scale_y};
use crate::object::registry::ObjectLibrary;
use crate::types::{AabbCollider, BuildDimensions, BuildObject, ObjectPrefabId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfacePick {
    pub(crate) hit: Vec3,
    pub(crate) surface_y: f32,
    pub(crate) block_top: Option<(Vec2, Vec2)>,
}

pub(crate) fn cursor_surface_pick(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    objects: &Query<(&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId), With<BuildObject>>,
) -> Option<SurfacePick> {
    let cursor_pos = window.cursor_position()?;
    let ray = camera
        .viewport_to_world(camera_transform, cursor_pos)
        .ok()?;

    let origin = ray.origin;
    let direction = ray.direction.as_vec3();
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return None;
    }

    let mut best_t = f32::INFINITY;
    let mut pick = None;

    let t_ground = (0.0 - origin.y) / denom;
    if t_ground >= 0.0 {
        best_t = t_ground;
        pick = Some(SurfacePick {
            hit: origin + direction * t_ground,
            surface_y: 0.0,
            block_top: None,
        });
    }

    for (transform, collider, dimensions, prefab_id) in objects.iter() {
        if !library.interaction(prefab_id.0).supports_standing {
            continue;
        }

        let scale_y = safe_abs_scale_y(transform.scale);
        let origin_y = library.ground_origin_y_or_default(prefab_id.0) * scale_y;
        let top_y = transform.translation.y - origin_y + dimensions.size.y;
        let t = (top_y - origin.y) / denom;
        if t < 0.0 || t >= best_t {
            continue;
        }

        let hit = origin + direction * t;
        let point = Vec2::new(hit.x, hit.z);
        let center = Vec2::new(transform.translation.x, transform.translation.z);
        if !point_inside_aabb_xz(point, center, collider.half_extents) {
            continue;
        }

        best_t = t;
        pick = Some(SurfacePick {
            hit,
            surface_y: top_y,
            block_top: Some((center, collider.half_extents)),
        });
    }

    pick
}

