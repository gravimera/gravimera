use bevy::prelude::*;

use crate::genfloor::{floor_half_size, sample_floor_point, ActiveWorldFloor};
use crate::geometry::{point_inside_aabb_xz, safe_abs_scale_y};
use crate::object::registry::ObjectLibrary;
use crate::types::{AabbCollider, BuildDimensions, BuildObject, ObjectPrefabId};

#[derive(Clone, Copy, Debug)]
pub(crate) struct SurfacePick {
    pub(crate) hit: Vec3,
    pub(crate) surface_y: f32,
    pub(crate) block_top: Option<(Vec2, Vec2)>,
    #[allow(dead_code)]
    pub(crate) floor_is_water: bool,
}

fn ray_floor_intersection(
    active_floor: &ActiveWorldFloor,
    origin: Vec3,
    direction: Vec3,
) -> Option<(Vec3, f32, bool)> {
    let mut t_min = 0.0f32;
    let mut t_max = f32::INFINITY;
    let half = floor_half_size(active_floor);
    let min_x = -half.x;
    let max_x = half.x;
    let min_z = -half.y;
    let max_z = half.y;

    for (o, d, min, max) in [
        (origin.x, direction.x, min_x, max_x),
        (origin.z, direction.z, min_z, max_z),
    ] {
        if d.abs() < 1e-6 {
            if o < min || o > max {
                return None;
            }
            continue;
        }
        let t1 = (min - o) / d;
        let t2 = (max - o) / d;
        let axis_min = t1.min(t2);
        let axis_max = t1.max(t2);
        t_min = t_min.max(axis_min);
        t_max = t_max.min(axis_max);
    }

    if t_max < t_min || t_max < 0.0 {
        return None;
    }
    t_min = t_min.max(0.0);

    let mesh = &active_floor.def.mesh;
    let cell_x = mesh.size_m[0].abs() / mesh.subdiv[0].max(1) as f32;
    let cell_z = mesh.size_m[1].abs() / mesh.subdiv[1].max(1) as f32;
    let cell = cell_x.min(cell_z).max(0.5);
    let distance = (t_max - t_min).max(0.0) * direction.length().max(1e-6);
    let steps = ((distance / cell).ceil() as usize).clamp(32, 512);

    let mut prev_t = t_min;
    let prev_pos = origin + direction * prev_t;
    let prev_sample = sample_floor_point(active_floor, prev_pos.x, prev_pos.z);
    let mut prev_f = prev_pos.y - prev_sample.height;
    if prev_f.abs() <= 1e-3 {
        let mut hit = prev_pos;
        hit.y = prev_sample.height;
        return Some((hit, prev_t, prev_sample.is_water));
    }

    let mut hit_lo: Option<(f32, f32)> = None;
    let mut hit_hi: Option<(f32, f32)> = None;

    for step in 1..=steps {
        let t = t_min + (t_max - t_min) * (step as f32 / steps as f32);
        let pos = origin + direction * t;
        let sample = sample_floor_point(active_floor, pos.x, pos.z);
        let f = pos.y - sample.height;
        if f.abs() <= 1e-3 {
            let mut hit = pos;
            hit.y = sample.height;
            return Some((hit, t, sample.is_water));
        }
        if (prev_f > 0.0 && f < 0.0) || (prev_f < 0.0 && f > 0.0) {
            hit_lo = Some((prev_t, prev_f));
            hit_hi = Some((t, f));
            break;
        }
        prev_t = t;
        prev_f = f;
    }

    let (mut lo_t, mut lo_f) = hit_lo?;
    let mut hi_t = hit_hi?.0;

    for _ in 0..20 {
        let mid_t = (lo_t + hi_t) * 0.5;
        let mid_pos = origin + direction * mid_t;
        let mid_sample = sample_floor_point(active_floor, mid_pos.x, mid_pos.z);
        let mid_f = mid_pos.y - mid_sample.height;
        if mid_f.abs() <= 1e-3 {
            let mut hit = mid_pos;
            hit.y = mid_sample.height;
            return Some((hit, mid_t, mid_sample.is_water));
        }
        if (lo_f > 0.0 && mid_f > 0.0) || (lo_f < 0.0 && mid_f < 0.0) {
            lo_t = mid_t;
            lo_f = mid_f;
        } else {
            hi_t = mid_t;
        }
    }

    let t_hit = (lo_t + hi_t) * 0.5;
    let mut hit = origin + direction * t_hit;
    let hit_sample = sample_floor_point(active_floor, hit.x, hit.z);
    hit.y = hit_sample.height;
    Some((hit, t_hit, hit_sample.is_water))
}

pub(crate) fn cursor_surface_pick(
    window: &Window,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    library: &ObjectLibrary,
    active_floor: &ActiveWorldFloor,
    objects: &Query<
        (&Transform, &AabbCollider, &BuildDimensions, &ObjectPrefabId),
        With<BuildObject>,
    >,
) -> Option<SurfacePick> {
    let cursor_pos = window.cursor_position()?;
    let ray = camera
        .viewport_to_world(camera_transform, cursor_pos)
        .ok()?;

    let origin = ray.origin;
    let direction = ray.direction.as_vec3();
    let denom = direction.y;
    let allow_block_top = denom.abs() >= 1e-5;

    let mut best_t = f32::INFINITY;
    let mut pick = None;

    if let Some((hit, t_ground, is_water)) = ray_floor_intersection(active_floor, origin, direction)
    {
        best_t = t_ground;
        pick = Some(SurfacePick {
            hit,
            surface_y: hit.y,
            block_top: None,
            floor_is_water: is_water,
        });
    }

    if !allow_block_top {
        return pick;
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
            floor_is_water: false,
        });
    }

    pick
}
