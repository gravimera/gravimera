use bevy::prelude::*;

use crate::constants::WORLD_HALF_SIZE;

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

pub(crate) fn clamp_world_xz(value: f32, half_extent: f32) -> f32 {
    clamp_world_xz_with_half_size(value, half_extent, WORLD_HALF_SIZE)
}

pub(crate) fn clamp_world_xz_with_half_size(
    value: f32,
    half_extent: f32,
    world_half_size: f32,
) -> f32 {
    if !value.is_finite() {
        return 0.0;
    }

    let world_half = if world_half_size.is_finite() {
        world_half_size.abs()
    } else {
        WORLD_HALF_SIZE
    };

    if !half_extent.is_finite() {
        return value.clamp(-world_half, world_half);
    }
    let half_extent = half_extent.abs();

    let min = -world_half + half_extent;
    let max = world_half - half_extent;
    if min.is_finite() && max.is_finite() && min <= max {
        value.clamp(min, max)
    } else {
        // `f32::clamp` panics when `min > max` or if either bound is NaN.
        value.clamp(-world_half, world_half)
    }
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

pub(crate) fn mat4_to_transform_allow_degenerate_scale(mat: Mat4) -> Option<Transform> {
    // Similar to `Mat4::to_scale_rotation_translation`, but handles degenerate (near-zero) scales
    // without producing NaNs. This is important for Gen3D-authored animations that may squash
    // parts to zero scale or mirror them with negative scale.
    const EPS: f32 = 1e-8;

    let translation = mat.w_axis.truncate();
    let col_x = mat.x_axis.truncate();
    let col_y = mat.y_axis.truncate();
    let col_z = mat.z_axis.truncate();

    if !translation.is_finite() || !col_x.is_finite() || !col_y.is_finite() || !col_z.is_finite() {
        return None;
    }

    let mut scale_x = col_x.length();
    let mut scale_y = col_y.length();
    let mut scale_z = col_z.length();
    if !scale_x.is_finite() || !scale_y.is_finite() || !scale_z.is_finite() {
        return None;
    }

    let mut r0 = if scale_x > EPS {
        col_x / scale_x
    } else {
        Vec3::ZERO
    };
    let mut r1 = if scale_y > EPS {
        col_y / scale_y
    } else {
        Vec3::ZERO
    };
    let mut r2 = if scale_z > EPS {
        col_z / scale_z
    } else {
        Vec3::ZERO
    };

    if r0.length_squared() <= EPS && r1.length_squared() > EPS && r2.length_squared() > EPS {
        r0 = r1.cross(r2);
    }
    if r1.length_squared() <= EPS && r0.length_squared() > EPS && r2.length_squared() > EPS {
        r1 = r2.cross(r0);
    }
    if r2.length_squared() <= EPS && r0.length_squared() > EPS && r1.length_squared() > EPS {
        r2 = r0.cross(r1);
    }

    fn any_perpendicular(v: Vec3) -> Vec3 {
        let a = if v.dot(Vec3::Y).abs() < 0.9 {
            Vec3::Y
        } else {
            Vec3::X
        };
        let p = a.cross(v);
        if p.length_squared() > 1e-8 {
            p.normalize()
        } else {
            Vec3::Z
        }
    }

    let mut r0 = if r0.length_squared() > EPS {
        r0.normalize()
    } else {
        Vec3::ZERO
    };
    let mut r1 = if r1.length_squared() > EPS {
        r1.normalize()
    } else {
        Vec3::ZERO
    };
    let mut r2 = if r2.length_squared() > EPS {
        r2.normalize()
    } else {
        Vec3::ZERO
    };

    // If we only have one reliable axis, construct an orthonormal basis around it.
    if r0.length_squared() <= EPS && r1.length_squared() <= EPS && r2.length_squared() <= EPS {
        r0 = Vec3::X;
        r1 = Vec3::Y;
        r2 = Vec3::Z;
    } else if r0.length_squared() <= EPS && r1.length_squared() <= EPS {
        // Only local +Z axis is reliable.
        r2 = r2.normalize();
        r0 = any_perpendicular(r2);
        r1 = r2.cross(r0).normalize();
    } else if r0.length_squared() <= EPS && r2.length_squared() <= EPS {
        // Only local +Y axis is reliable.
        r1 = r1.normalize();
        r2 = any_perpendicular(r1);
        r0 = r1.cross(r2).normalize();
    } else if r1.length_squared() <= EPS && r2.length_squared() <= EPS {
        // Only local +X axis is reliable.
        r0 = r0.normalize();
        r1 = any_perpendicular(r0);
        r2 = r0.cross(r1).normalize();
    }

    // Orthonormalize (best effort) while keeping the handedness stable.
    r0 = if r0.length_squared() > EPS {
        r0.normalize()
    } else {
        Vec3::X
    };
    r1 = r1 - r0 * r1.dot(r0);
    if r1.length_squared() <= EPS {
        r1 = any_perpendicular(r0);
    } else {
        r1 = r1.normalize();
    }
    r2 = r2 - r0 * r2.dot(r0) - r1 * r2.dot(r1);
    if r2.length_squared() <= EPS {
        r2 = r0.cross(r1);
    }
    if r2.length_squared() <= EPS {
        r2 = Vec3::Z;
    } else {
        r2 = r2.normalize();
    }

    let det_basis = r0.cross(r1).dot(r2);
    if det_basis.is_finite() && det_basis < 0.0 {
        // Encode reflection into scale by negating the largest axis and flipping the matching
        // basis column so the rotation stays a proper quaternion.
        if scale_x >= scale_y && scale_x >= scale_z {
            scale_x = -scale_x;
            r0 = -r0;
        } else if scale_y >= scale_z {
            scale_y = -scale_y;
            r1 = -r1;
        } else {
            scale_z = -scale_z;
            r2 = -r2;
        }
    }

    let rotation = Quat::from_mat3(&Mat3::from_cols(r0, r1, r2)).normalize();
    let scale = Vec3::new(scale_x, scale_y, scale_z);

    if !rotation.is_finite() || !scale.is_finite() {
        return None;
    }

    Some(Transform {
        translation,
        rotation,
        scale,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_world_xz_never_panics_on_oversized_or_invalid_half_extents() {
        let big = WORLD_HALF_SIZE * 4.0;

        assert_eq!(clamp_world_xz(100.0, big), WORLD_HALF_SIZE);
        assert_eq!(clamp_world_xz(-100.0, big), -WORLD_HALF_SIZE);
        assert_eq!(clamp_world_xz(100.0, f32::NAN), WORLD_HALF_SIZE);
        assert_eq!(clamp_world_xz(f32::NAN, 1.0), 0.0);
        assert_eq!(clamp_world_xz(f32::INFINITY, 1.0), 0.0);
    }
}
