use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};

pub(crate) fn create_render_target(
    images: &mut Assets<Image>,
    width_px: u32,
    height_px: u32,
) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        width_px.max(1),
        height_px.max(1),
        TextureFormat::bevy_default(),
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(image)
}

pub(crate) fn orbit_transform(yaw: f32, pitch: f32, distance: f32, focus: Vec3) -> Transform {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let pos = focus + rot * Vec3::new(0.0, 0.0, distance);
    Transform::from_translation(pos).looking_at(focus, Vec3::Y)
}

pub(crate) fn required_distance_for_view(
    half_extents: Vec3,
    yaw: f32,
    pitch: f32,
    fov_y: f32,
    aspect: f32,
    near: f32,
) -> f32 {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let mut view_dir = -rot * Vec3::Z;
    if !view_dir.is_finite() || view_dir.length_squared() <= 1e-6 {
        view_dir = -Vec3::Z;
    } else {
        view_dir = view_dir.normalize();
    }

    let mut right = Vec3::Y.cross(view_dir);
    if !right.is_finite() || right.length_squared() <= 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }
    let mut up = view_dir.cross(right);
    if !up.is_finite() || up.length_squared() <= 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    let extent_right = half_extents.x * right.x.abs()
        + half_extents.y * right.y.abs()
        + half_extents.z * right.z.abs();
    let extent_up =
        half_extents.x * up.x.abs() + half_extents.y * up.y.abs() + half_extents.z * up.z.abs();
    let extent_forward = half_extents.x * view_dir.x.abs()
        + half_extents.y * view_dir.y.abs()
        + half_extents.z * view_dir.z.abs();

    let tan_y = (fov_y * 0.5).tan().max(1e-4);
    let tan_x = (tan_y * aspect).max(1e-4);
    let dist_y = extent_up / tan_y;
    let dist_x = extent_right / tan_x;

    // Ensure the near plane won't clip the bounds.
    dist_x.max(dist_y).max(extent_forward + near + 0.05)
}
