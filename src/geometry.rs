use bevy::prelude::*;

pub(crate) fn circles_intersect_xz(a: Vec3, ra: f32, b: Vec3, rb: f32) -> bool {
    let delta = Vec2::new(a.x - b.x, a.z - b.z);
    delta.length_squared() <= (ra + rb) * (ra + rb)
}

pub(crate) fn ray_plane_intersection_y0(ray: Ray3d) -> Option<Vec3> {
    let origin = ray.origin;
    let direction = ray.direction;
    let denom = direction.y;
    if denom.abs() < 1e-5 {
        return None;
    }

    let t = (0.0 - origin.y) / denom;
    if t < 0.0 {
        return None;
    }

    Some(origin + direction * t)
}

pub(crate) fn snap_to_grid(value: f32, grid: f32) -> f32 {
    (value / grid).round() * grid
}

pub(crate) fn safe_abs_scale_component(value: f32) -> f32 {
    if value.is_finite() && value.abs() >= 1e-4 {
        value.abs()
    } else {
        1.0
    }
}

pub(crate) fn safe_abs_scale_y(scale: Vec3) -> f32 {
    safe_abs_scale_component(scale.y)
}

pub(crate) fn normalize_flat_direction(direction: Vec3) -> Option<Vec3> {
    let flat = Vec3::new(direction.x, 0.0, direction.z);
    if flat.length_squared() < 0.0001 {
        return None;
    }
    Some(flat.normalize())
}

pub(crate) fn aabbs_intersect_xz(
    a_center: Vec2,
    a_half: Vec2,
    b_center: Vec2,
    b_half: Vec2,
) -> bool {
    let delta = a_center - b_center;
    delta.x.abs() <= (a_half.x + b_half.x) && delta.y.abs() <= (a_half.y + b_half.y)
}

pub(crate) fn point_inside_aabb_xz(point: Vec2, center: Vec2, half: Vec2) -> bool {
    let delta = point - center;
    delta.x.abs() <= half.x && delta.y.abs() <= half.y
}

pub(crate) fn circle_intersects_aabb_xz(
    circle: Vec2,
    radius: f32,
    center: Vec2,
    half: Vec2,
) -> bool {
    let delta = circle - center;
    let closest = Vec2::new(
        delta.x.clamp(-half.x, half.x),
        delta.y.clamp(-half.y, half.y),
    ) + center;
    (circle - closest).length_squared() <= radius * radius
}

pub(crate) fn resolve_circle_against_aabbs(
    mut circle: Vec2,
    radius: f32,
    aabbs: &[(Vec2, Vec2)],
) -> Vec2 {
    for _ in 0..4 {
        let mut moved = false;
        for &(center, half) in aabbs {
            let Some(push) = push_circle_out_of_aabb_xz(circle, radius, center, half) else {
                continue;
            };
            circle += push;
            moved = true;
        }
        if !moved {
            break;
        }
    }
    circle
}

pub(crate) fn push_circle_out_of_aabb_xz(
    circle: Vec2,
    radius: f32,
    center: Vec2,
    half: Vec2,
) -> Option<Vec2> {
    let delta = circle - center;
    let closest = Vec2::new(
        delta.x.clamp(-half.x, half.x),
        delta.y.clamp(-half.y, half.y),
    ) + center;
    let diff = circle - closest;
    let dist2 = diff.length_squared();
    let radius2 = radius * radius;

    if dist2 >= radius2 {
        return None;
    }

    if dist2 > 1e-10 {
        let dist = dist2.sqrt();
        let push = diff / dist * (radius - dist);
        return Some(push);
    }

    let dx = (half.x + radius) - delta.x.abs();
    let dz = (half.y + radius) - delta.y.abs();
    if dx <= 0.0 && dz <= 0.0 {
        return None;
    }

    if dx < dz {
        let sx = if delta.x >= 0.0 { 1.0 } else { -1.0 };
        Some(Vec2::new(sx * dx, 0.0))
    } else {
        let sz = if delta.y >= 0.0 { 1.0 } else { -1.0 };
        Some(Vec2::new(0.0, sz * dz))
    }
}
