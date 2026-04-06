use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::ecs::hierarchy::ChildOf;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::assets::SceneAssets;
use crate::object::registry::{ColliderProfile, ObjectDef, ObjectLibrary, ObjectPartKind};
use crate::object::visuals::{
    MaterialCache, VisualObjectRefRoot, VisualPartId, VisualSpawnSettings,
};
use crate::types::{
    ActionClock, AnimationChannelsActive, AttackClock, BuildScene, ForcedAnimationChannel,
    LocomotionClock, ObjectPrefabId,
};

use super::ai::Gen3dAiJob;
use super::state::{
    Gen3dDraft, Gen3dManualTweakState, Gen3dPreview, Gen3dPreviewAnimationDropdownButton,
    Gen3dPreviewAnimationDropdownList, Gen3dPreviewCamera, Gen3dPreviewCollisionRoot,
    Gen3dPreviewComponentLabel, Gen3dPreviewComponentLabelText, Gen3dPreviewExplodeToggleButton,
    Gen3dPreviewExportButton, Gen3dPreviewHoverFrame, Gen3dPreviewHoverInfoCard,
    Gen3dPreviewHoverInfoText, Gen3dPreviewLight, Gen3dPreviewModelRoot, Gen3dPreviewPanel,
    Gen3dPreviewSceneRoot, Gen3dPreviewUiModelRoot, Gen3dReviewOverlayRoot, Gen3dSidePanelRoot,
    Gen3dSidePanelToggleButton, Gen3dWorkshop,
};
use super::task_queue::Gen3dTaskQueue;

pub(crate) fn gen3d_update_preview_camera_render_layers(
    build_scene: Res<State<BuildScene>>,
    mut cameras: Query<&mut bevy::camera::visibility::RenderLayers, With<Gen3dPreviewCamera>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let desired = bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_UI_LAYER);

    for mut layers in &mut cameras {
        if *layers != desired {
            *layers = desired.clone();
        }
    }
}

#[derive(SystemParam)]
pub(crate) struct Gen3dPreviewOrbitUi<'w, 's> {
    windows: Query<'w, 's, &'static mut Window, With<bevy::window::PrimaryWindow>>,
    panel: Query<
        'w,
        's,
        (
            &'static Interaction,
            &'static ComputedNode,
            &'static UiGlobalTransform,
        ),
        With<Gen3dPreviewPanel>,
    >,
    anim_dropdown_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewAnimationDropdownButton>,
    >,
    explode_toggle_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewExplodeToggleButton>,
    >,
    export_button: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewExportButton>,
    >,
    anim_dropdown_list: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dPreviewAnimationDropdownList>,
    >,
    side_panel_root: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dSidePanelRoot>,
    >,
    side_panel_toggle: Query<
        'w,
        's,
        (
            &'static ComputedNode,
            &'static UiGlobalTransform,
            Option<&'static Visibility>,
        ),
        With<Gen3dSidePanelToggleButton>,
    >,
}

#[derive(Default)]
pub(crate) struct Gen3dPreviewOrbitDragState {
    lmb_started_on_ffd_handle: bool,
}

#[derive(SystemParam)]
pub(crate) struct Gen3dPreviewFocusWorld<'w, 's> {
    library: Res<'w, ObjectLibrary>,
    ui_roots: Query<'w, 's, Entity, With<Gen3dPreviewUiModelRoot>>,
    preview_components: Query<
        'w,
        's,
        (
            Entity,
            &'static VisualObjectRefRoot,
            &'static Transform,
            Option<&'static ChildOf>,
        ),
        (With<VisualObjectRefRoot>, Without<Gen3dPreviewCamera>),
    >,
}

pub(super) fn setup_preview_scene(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    assets: &SceneAssets,
    materials: &mut Assets<StandardMaterial>,
    preview: &mut Gen3dPreview,
) -> Handle<Image> {
    let target = crate::orbit_capture::create_render_target(
        images,
        super::GEN3D_PREVIEW_WIDTH_PX,
        super::GEN3D_PREVIEW_HEIGHT_PX,
    );
    let root_entity = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Gen3dPreviewSceneRoot,
        ))
        .id();

    // Initialize orbit defaults.
    preview.draft_focus = Vec3::ZERO;
    preview.view_pan = Vec3::ZERO;
    preview.yaw = super::GEN3D_PREVIEW_DEFAULT_YAW;
    preview.pitch = super::GEN3D_PREVIEW_DEFAULT_PITCH;
    preview.distance = super::GEN3D_PREVIEW_DEFAULT_DISTANCE;
    let camera_transform = crate::orbit_capture::orbit_transform(
        preview.yaw,
        preview.pitch,
        preview.distance,
        preview.draft_focus,
    );

    let aspect =
        super::GEN3D_PREVIEW_WIDTH_PX.max(1) as f32 / super::GEN3D_PREVIEW_HEIGHT_PX.max(1) as f32;
    let mut projection = bevy::camera::PerspectiveProjection::default();
    projection.aspect_ratio = aspect;

    let camera_entity = commands
        .spawn((
            Camera3d::default(),
            bevy::camera::Projection::Perspective(projection),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            Tonemapping::TonyMcMapface,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_UI_LAYER),
            camera_transform,
            Gen3dPreviewCamera,
        ))
        .id();

    // Preview "studio" scene: simple three-point light rig (no floor plane).

    let preview_layers = bevy::camera::visibility::RenderLayers::from_layers(&[
        super::GEN3D_PREVIEW_UI_LAYER,
        super::GEN3D_PREVIEW_LAYER,
    ]);

    // Key light (casts shadows).
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 16_000.0,
            color: Color::srgb(1.0, 0.97, 0.94),
            ..default()
        },
        Transform::from_xyz(10.0, 18.0, -8.0).looking_at(Vec3::ZERO, Vec3::Y),
        preview_layers.clone(),
        Gen3dPreviewLight,
    ));
    // Fill light (soft, no shadows).
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 6_500.0,
            color: Color::srgb(0.90, 0.95, 1.0),
            ..default()
        },
        Transform::from_xyz(-10.0, 10.0, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
        preview_layers.clone(),
        Gen3dPreviewLight,
    ));
    // Rim light (adds edge highlights).
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 4_000.0,
            color: Color::srgb(1.0, 1.0, 1.0),
            ..default()
        },
        Transform::from_xyz(0.0, 12.0, -12.0).looking_at(Vec3::ZERO, Vec3::Y),
        preview_layers.clone(),
        Gen3dPreviewLight,
    ));
    // Under light (brightens underside for bottom views; no shadows).
    commands.spawn((
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 4_500.0,
            color: Color::srgb(0.96, 0.97, 1.0),
            ..default()
        },
        Transform::from_xyz(0.0, -14.0, 0.0).looking_at(Vec3::ZERO, Vec3::Z),
        preview_layers,
        Gen3dPreviewLight,
    ));

    // Axis + grid overlay used for AI auto-review screenshots (rendered on a separate layer so
    // it doesn't clutter the user-visible preview panel).
    let axis_x_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.18, 0.18),
        unlit: true,
        ..default()
    });
    let axis_y_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.95, 0.35),
        unlit: true,
        ..default()
    });
    let axis_z_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.42, 0.95),
        unlit: true,
        ..default()
    });
    let grid_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.35, 0.37, 0.40, 0.45),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let overlay_root = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
            Gen3dReviewOverlayRoot,
        ))
        .id();
    commands.entity(root_entity).add_child(overlay_root);

    // Axes: red=X, green=Y, blue=Z. Grid lines are on the XZ plane.
    commands.entity(overlay_root).with_children(|parent| {
        let axis_thickness = 0.015;
        let axis_len = 1.6;
        let axis_y = 0.012;

        // X axis (+X to the right).
        parent.spawn((
            Mesh3d(assets.unit_cube_mesh.clone()),
            MeshMaterial3d(axis_x_material.clone()),
            Transform::from_translation(Vec3::new(axis_len * 0.5, axis_y, 0.0))
                .with_scale(Vec3::new(axis_len, axis_thickness, axis_thickness)),
            Visibility::Inherited,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
        ));
        // Z axis (+Z forward).
        parent.spawn((
            Mesh3d(assets.unit_cube_mesh.clone()),
            MeshMaterial3d(axis_z_material.clone()),
            Transform::from_translation(Vec3::new(0.0, axis_y, axis_len * 0.5))
                .with_scale(Vec3::new(axis_thickness, axis_thickness, axis_len)),
            Visibility::Inherited,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
        ));
        // Y axis (+Y up).
        parent.spawn((
            Mesh3d(assets.unit_cube_mesh.clone()),
            MeshMaterial3d(axis_y_material.clone()),
            Transform::from_translation(Vec3::new(0.0, axis_len * 0.5, 0.0)).with_scale(Vec3::new(
                axis_thickness,
                axis_len,
                axis_thickness,
            )),
            Visibility::Inherited,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
        ));

        // Grid (small, subtle).
        let grid_extent = 1.5;
        let grid_step = 0.5;
        let grid_thickness = 0.006;
        let mut v = -grid_extent;
        while v <= grid_extent + 1e-4 {
            // Lines parallel to X (vary Z).
            parent.spawn((
                Mesh3d(assets.unit_cube_mesh.clone()),
                MeshMaterial3d(grid_material.clone()),
                Transform::from_translation(Vec3::new(0.0, axis_y * 0.5, v)).with_scale(Vec3::new(
                    grid_extent * 2.0,
                    grid_thickness,
                    grid_thickness,
                )),
                Visibility::Inherited,
                bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
            ));
            // Lines parallel to Z (vary X).
            parent.spawn((
                Mesh3d(assets.unit_cube_mesh.clone()),
                MeshMaterial3d(grid_material.clone()),
                Transform::from_translation(Vec3::new(v, axis_y * 0.5, 0.0)).with_scale(Vec3::new(
                    grid_thickness,
                    grid_thickness,
                    grid_extent * 2.0,
                )),
                Visibility::Inherited,
                bevy::camera::visibility::RenderLayers::layer(super::GEN3D_REVIEW_LAYER),
            ));
            v += grid_step;
        }
    });

    preview.target = Some(target.clone());
    preview.camera = Some(camera_entity);
    preview.root = Some(root_entity);
    preview.capture_root = None;
    preview.last_cursor = None;
    preview.show_collision = false;
    preview.collision_dirty = true;
    preview.ui_applied_session_id = None;
    preview.ui_applied_assembly_rev = None;
    preview.capture_applied_session_id = None;
    preview.capture_applied_assembly_rev = None;
    preview.explode_components = false;
    preview.hovered_component = None;

    target
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub(crate) struct Gen3dPreviewAppliedExplodeOffset(pub(crate) Vec3);

#[derive(Clone, Copy, Debug)]
pub(crate) struct PreviewImageLayout {
    pub(crate) panel_bounds_physical: Rect,
    pub(crate) panel_size_logical: Vec2,
    pub(crate) panel_inverse_scale: f32,
    pub(crate) image_bounds_physical: Rect,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PreviewProjectedComponent {
    pub(crate) frame_panel_logical: Rect,
    pub(crate) label_anchor_panel_logical: Vec2,
}

#[derive(Clone, Debug)]
pub(crate) struct PreviewComponentOverlayInfo {
    pub(crate) entity: Entity,
    pub(crate) parent_object_id: u128,
    pub(crate) object_id: u128,
    pub(crate) label: String,
    pub(crate) depth: usize,
    pub(crate) order: usize,
    pub(crate) stable_order: usize,
    pub(crate) projected: Option<PreviewProjectedComponent>,
    pub(crate) ray_t: Option<f32>,
    pub(crate) applied_explode_offset_local: Vec3,
}

fn aspect_fit_size(content_w_px: f32, content_h_px: f32, aspect: f32) -> Vec2 {
    let content_w_px = content_w_px.max(1.0);
    let content_h_px = content_h_px.max(1.0);
    let aspect = aspect.clamp(0.05, 20.0);

    let box_aspect = (content_w_px / content_h_px).max(0.05);
    if aspect >= box_aspect {
        Vec2::new(content_w_px, (content_w_px / aspect).max(1.0))
    } else {
        Vec2::new((content_h_px * aspect).max(1.0), content_h_px)
    }
}

fn ui_node_bounds_physical(node: &ComputedNode, transform: UiGlobalTransform) -> Option<Rect> {
    if !node.size.is_finite() || node.size.x <= 0.0 || node.size.y <= 0.0 {
        return None;
    }
    let half = node.size * 0.5;
    let corners = [
        Vec2::new(-half.x, -half.y),
        Vec2::new(half.x, -half.y),
        Vec2::new(half.x, half.y),
        Vec2::new(-half.x, half.y),
    ];
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    for corner in corners {
        let point = transform.transform_point2(corner);
        min = min.min(point);
        max = max.max(point);
    }
    if !min.is_finite() || !max.is_finite() {
        return None;
    }
    Some(Rect { min, max })
}

pub(crate) fn preview_image_layout(
    panel_node: &ComputedNode,
    panel_transform: UiGlobalTransform,
) -> Option<PreviewImageLayout> {
    let panel_bounds_physical = ui_node_bounds_physical(panel_node, panel_transform)?;
    let panel_inverse_scale = panel_node.inverse_scale_factor();
    let panel_size_logical = panel_node.size * panel_inverse_scale;

    let content_size_physical = Vec2::new(
        (panel_node.size.x
            - panel_node.border.min_inset.x
            - panel_node.border.max_inset.x
            - panel_node.padding.min_inset.x
            - panel_node.padding.max_inset.x)
            .max(0.0),
        (panel_node.size.y
            - panel_node.border.min_inset.y
            - panel_node.border.max_inset.y
            - panel_node.padding.min_inset.y
            - panel_node.padding.max_inset.y)
            .max(0.0),
    );
    if content_size_physical.x < 1.0 || content_size_physical.y < 1.0 {
        return None;
    }

    let image_size_physical = aspect_fit_size(
        content_size_physical.x,
        content_size_physical.y,
        super::GEN3D_PREVIEW_WIDTH_PX.max(1) as f32 / super::GEN3D_PREVIEW_HEIGHT_PX.max(1) as f32,
    );
    let content_min_physical = Vec2::new(
        panel_bounds_physical.min.x
            + panel_node.border.min_inset.x
            + panel_node.padding.min_inset.x,
        panel_bounds_physical.min.y
            + panel_node.border.min_inset.y
            + panel_node.padding.min_inset.y,
    );
    let image_min_physical =
        content_min_physical + (content_size_physical - image_size_physical) * 0.5;

    Some(PreviewImageLayout {
        panel_bounds_physical,
        panel_size_logical,
        panel_inverse_scale,
        image_bounds_physical: Rect {
            min: image_min_physical,
            max: image_min_physical + image_size_physical,
        },
    })
}

pub(crate) fn preview_cursor_to_target(
    cursor_physical: Vec2,
    image_bounds_physical: Rect,
) -> Option<Vec2> {
    let image_size = image_bounds_physical.max - image_bounds_physical.min;
    if image_size.x <= 0.0 || image_size.y <= 0.0 {
        return None;
    }
    if cursor_physical.x < image_bounds_physical.min.x
        || cursor_physical.x > image_bounds_physical.max.x
        || cursor_physical.y < image_bounds_physical.min.y
        || cursor_physical.y > image_bounds_physical.max.y
    {
        return None;
    }

    let uv = (cursor_physical - image_bounds_physical.min) / image_size;
    Some(Vec2::new(
        uv.x * super::GEN3D_PREVIEW_WIDTH_PX as f32,
        uv.y * super::GEN3D_PREVIEW_HEIGHT_PX as f32,
    ))
}

fn preview_local_center(size: Vec3, ground_origin_y: Option<f32>) -> Vec3 {
    let size = size.abs();
    let ground_origin_y = ground_origin_y
        .filter(|value| value.is_finite() && *value >= 0.0)
        .unwrap_or(size.y * 0.5);
    Vec3::new(0.0, size.y * 0.5 - ground_origin_y, 0.0)
}

fn preview_orbit_rotation(yaw: f32, pitch: f32) -> Quat {
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch)
}

fn preview_camera_basis(yaw: f32, pitch: f32) -> (Vec3, Vec3) {
    let rotation = preview_orbit_rotation(yaw, pitch);
    let mut view_dir = -(rotation * Vec3::Z);
    if !view_dir.is_finite() || view_dir.length_squared() <= 1e-6 {
        view_dir = -Vec3::Z;
    } else {
        view_dir = view_dir.normalize();
    }

    let mut right = view_dir.cross(Vec3::Y);
    if !right.is_finite() || right.length_squared() <= 1e-6 {
        right = Vec3::X;
    } else {
        right = right.normalize();
    }

    let mut up = right.cross(view_dir);
    if !up.is_finite() || up.length_squared() <= 1e-6 {
        up = Vec3::Y;
    } else {
        up = up.normalize();
    }

    (right, up)
}

fn preview_pan_step_scale(distance: f32) -> f32 {
    (distance.abs() * 0.22).clamp(0.12, 6.0)
}

pub(crate) fn preview_pan_delta_world(yaw: f32, pitch: f32, distance: f32, pan: Vec2) -> Vec3 {
    if !pan.is_finite() || pan.length_squared() <= 1e-8 {
        return Vec3::ZERO;
    }

    let (right, up) = preview_camera_basis(yaw, pitch);
    let scale = preview_pan_step_scale(distance);
    (right * pan.x + up * pan.y) * scale
}

fn explode_direction(delta: Vec3, order: usize) -> Vec3 {
    if delta.is_finite() && delta.length_squared() > 1e-4 {
        return delta.normalize();
    }

    let angle = order as f32 * 2.399_963_1;
    Vec3::new(
        angle.cos(),
        if order % 2 == 0 { 0.28 } else { -0.22 },
        angle.sin(),
    )
    .normalize()
}

fn explode_offset(delta: Vec3, size: Vec3, order: usize) -> Vec3 {
    let size = size.abs();
    let distance = (size.max_element() * 0.75).max(0.35) + 0.18;
    explode_direction(delta, order) * distance
}

fn ray_intersects_local_aabb(origin: Vec3, direction: Vec3, half: Vec3) -> Option<f32> {
    let mut t_min = f32::NEG_INFINITY;
    let mut t_max = f32::INFINITY;

    for axis in 0..3 {
        let o = origin[axis];
        let d = direction[axis];
        let min = -half[axis];
        let max = half[axis];

        if d.abs() < 1e-6 {
            if o < min || o > max {
                return None;
            }
            continue;
        }

        let inv_d = 1.0 / d;
        let mut t0 = (min - o) * inv_d;
        let mut t1 = (max - o) * inv_d;
        if t0 > t1 {
            std::mem::swap(&mut t0, &mut t1);
        }
        t_min = t_min.max(t0);
        t_max = t_max.min(t1);
        if t_max < t_min {
            return None;
        }
    }

    if t_max < 0.0 {
        None
    } else {
        Some(t_min.max(0.0))
    }
}

fn ray_intersects_component(ray: Ray3d, world_from_box: Mat4, half: Vec3) -> Option<f32> {
    let inverse = world_from_box.inverse();
    let origin_local = inverse.transform_point3(ray.origin);
    let direction_local = inverse.transform_vector3(ray.direction.into());
    ray_intersects_local_aabb(origin_local, direction_local, half)
}

fn project_component_bounds(
    camera: &Camera,
    camera_transform: &GlobalTransform,
    world_from_box: Mat4,
    layout: PreviewImageLayout,
) -> Option<PreviewProjectedComponent> {
    let half = Vec3::new(0.5, 0.5, 0.5);
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(half.x, half.y, half.z),
    ];
    let mut min = Vec2::splat(f32::INFINITY);
    let mut max = Vec2::splat(f32::NEG_INFINITY);
    let mut any = false;

    for corner in corners {
        let world = world_from_box.transform_point3(corner);
        let Ok(viewport) = camera.world_to_viewport(camera_transform, world) else {
            continue;
        };
        min = min.min(viewport);
        max = max.max(viewport);
        any = true;
    }
    if !any || !min.is_finite() || !max.is_finite() {
        return None;
    }

    let center_world = world_from_box.transform_point3(Vec3::ZERO);
    let center_viewport = camera
        .world_to_viewport(camera_transform, center_world)
        .ok()?;
    let image_size = layout.image_bounds_physical.max - layout.image_bounds_physical.min;
    if image_size.x <= 0.0 || image_size.y <= 0.0 {
        return None;
    }

    Some(PreviewProjectedComponent {
        frame_panel_logical: Rect {
            min: preview_target_to_panel_logical(min, layout),
            max: preview_target_to_panel_logical(max, layout),
        },
        label_anchor_panel_logical: preview_target_to_panel_logical(center_viewport, layout),
    })
}

pub(crate) fn preview_target_to_panel_logical(point: Vec2, layout: PreviewImageLayout) -> Vec2 {
    let image_size = layout.image_bounds_physical.max - layout.image_bounds_physical.min;
    let uv = Vec2::new(
        point.x / super::GEN3D_PREVIEW_WIDTH_PX.max(1) as f32,
        point.y / super::GEN3D_PREVIEW_HEIGHT_PX.max(1) as f32,
    );
    let physical = layout.image_bounds_physical.min + uv * image_size;
    (physical - layout.panel_bounds_physical.min) * layout.panel_inverse_scale
}

pub(crate) fn preview_panel_logical_to_target(
    point: Vec2,
    layout: PreviewImageLayout,
) -> Option<Vec2> {
    if !point.is_finite()
        || point.x < 0.0
        || point.y < 0.0
        || point.x > layout.panel_size_logical.x
        || point.y > layout.panel_size_logical.y
    {
        return None;
    }

    let physical = layout.panel_bounds_physical.min + point / layout.panel_inverse_scale;
    preview_cursor_to_target(physical, layout.image_bounds_physical)
}

fn component_box_world_from_entity_matrix(world_from_entity: Mat4, def: &ObjectDef) -> Mat4 {
    let scale = def.size.abs().max(Vec3::splat(0.01));
    let local_center = preview_local_center(def.size, def.ground_origin_y);
    world_from_entity
        * Transform {
            translation: local_center,
            rotation: Quat::IDENTITY,
            scale,
        }
        .to_matrix()
}

fn component_box_world_from_box(global_transform: &GlobalTransform, def: &ObjectDef) -> Mat4 {
    component_box_world_from_entity_matrix(global_transform.to_matrix(), def)
}

fn expand_world_bounds_for_box(world_from_box: Mat4, min: &mut Vec3, max: &mut Vec3) -> bool {
    let half = Vec3::splat(0.5);
    let corners = [
        Vec3::new(-half.x, -half.y, -half.z),
        Vec3::new(-half.x, -half.y, half.z),
        Vec3::new(-half.x, half.y, -half.z),
        Vec3::new(-half.x, half.y, half.z),
        Vec3::new(half.x, -half.y, -half.z),
        Vec3::new(half.x, -half.y, half.z),
        Vec3::new(half.x, half.y, -half.z),
        Vec3::new(half.x, half.y, half.z),
    ];
    let mut any = false;
    for corner in corners {
        let point = world_from_box.transform_point3(corner);
        if !point.is_finite() {
            continue;
        }
        *min = min.min(point);
        *max = max.max(point);
        any = true;
    }
    any
}

fn finite_bounds_center(min: Vec3, max: Vec3) -> Option<Vec3> {
    if !min.is_finite() || !max.is_finite() {
        return None;
    }

    let center = (min + max) * 0.5;
    if center.is_finite() {
        Some(center)
    } else {
        None
    }
}

fn preview_component_sort_key(
    meta: &VisualObjectRefRoot,
    entity: Entity,
) -> (usize, u128, usize, u128, u64) {
    (
        meta.depth,
        meta.parent_object_id,
        meta.order,
        meta.object_id,
        entity.to_bits(),
    )
}

fn component_chain_world_affine(
    entity: Entity,
    component_chain: &std::collections::HashMap<Entity, (Option<Entity>, Transform)>,
    cache: &mut std::collections::HashMap<Entity, Mat4>,
) -> Mat4 {
    if let Some(existing) = cache.get(&entity) {
        return *existing;
    }

    let Some((parent_entity, local_transform)) = component_chain.get(&entity).copied() else {
        return Mat4::IDENTITY;
    };
    let parent_world = parent_entity
        .filter(|parent| component_chain.contains_key(parent))
        .map(|parent| component_chain_world_affine(parent, component_chain, cache))
        .unwrap_or(Mat4::IDENTITY);
    let world = parent_world * local_transform.to_matrix();
    cache.insert(entity, world);
    world
}

pub(crate) fn compute_preview_component_bounds_center_from_transforms<'a>(
    library: &ObjectLibrary,
    ui_root: Entity,
    components: impl IntoIterator<
        Item = (
            Entity,
            &'a VisualObjectRefRoot,
            &'a Transform,
            Option<Entity>,
        ),
    >,
) -> Option<Vec3> {
    let mut ordered_components = Vec::new();
    let mut component_chain =
        std::collections::HashMap::<Entity, (Option<Entity>, Transform)>::new();

    for (entity, meta, transform, parent_entity) in components {
        if meta.root_entity != ui_root {
            continue;
        }
        component_chain.insert(entity, (parent_entity, *transform));
        ordered_components.push((entity, *meta));
    }

    if ordered_components.is_empty() {
        return None;
    }

    ordered_components.sort_by_key(|(entity, meta)| preview_component_sort_key(meta, *entity));

    let mut world_cache = std::collections::HashMap::<Entity, Mat4>::new();
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;

    for (entity, meta) in ordered_components {
        let Some(def) = library.get(meta.object_id) else {
            continue;
        };
        let world_from_entity =
            component_chain_world_affine(entity, &component_chain, &mut world_cache);
        let world_from_box = component_box_world_from_entity_matrix(world_from_entity, def);
        any |= expand_world_bounds_for_box(world_from_box, &mut min, &mut max);
    }

    if !any {
        return None;
    }
    finite_bounds_center(min, max)
}

pub(crate) fn effective_preview_camera_focus(
    preview: &Gen3dPreview,
    exploded_center: Option<Vec3>,
) -> Vec3 {
    let base_focus = if preview.explode_components {
        exploded_center.unwrap_or(preview.draft_focus)
    } else {
        preview.draft_focus
    };
    let focus = base_focus + preview.view_pan;
    if focus.is_finite() {
        focus
    } else {
        base_focus
    }
}

pub(crate) fn collect_preview_component_overlays<'a>(
    library: &ObjectLibrary,
    camera: &Camera,
    camera_transform: &GlobalTransform,
    layout: PreviewImageLayout,
    ui_root: Entity,
    ray: Option<Ray3d>,
    components: impl IntoIterator<Item = (Entity, &'a VisualObjectRefRoot, &'a GlobalTransform, Vec3)>,
) -> Vec<PreviewComponentOverlayInfo> {
    let mut overlays = Vec::new();

    for (entity, meta, global_transform, applied_explode_offset_local) in components {
        if meta.root_entity != ui_root {
            continue;
        }
        let Some(def) = library.get(meta.object_id) else {
            continue;
        };

        let world_from_box = component_box_world_from_box(global_transform, def);
        let projected = project_component_bounds(camera, camera_transform, world_from_box, layout);
        let ray_t = ray
            .as_ref()
            .and_then(|ray| ray_intersects_component(*ray, world_from_box, Vec3::splat(0.5)));

        overlays.push(PreviewComponentOverlayInfo {
            entity,
            parent_object_id: meta.parent_object_id,
            object_id: meta.object_id,
            label: component_label_text(def, meta.order),
            depth: meta.depth,
            order: meta.order,
            stable_order: 0,
            projected,
            ray_t,
            applied_explode_offset_local,
        });
    }

    overlays.sort_by_key(|overlay| {
        (
            overlay.depth,
            overlay.parent_object_id,
            overlay.order,
            overlay.object_id,
            overlay.entity.to_bits(),
        )
    });
    for (stable_order, overlay) in overlays.iter_mut().enumerate() {
        overlay.stable_order = stable_order;
    }
    overlays
}

fn rect_contains_point(rect: Rect, point: Vec2) -> bool {
    point.x >= rect.min.x && point.x <= rect.max.x && point.y >= rect.min.y && point.y <= rect.max.y
}

fn projected_frame_area(projected: PreviewProjectedComponent) -> f32 {
    let size = (projected.frame_panel_logical.max - projected.frame_panel_logical.min).abs();
    (size.x * size.y).max(0.0)
}

pub(crate) fn pick_hovered_preview_component(
    overlays: &[PreviewComponentOverlayInfo],
    cursor_panel_logical: Option<Vec2>,
) -> Option<usize> {
    if let Some(cursor_panel_logical) = cursor_panel_logical {
        let mut best: Option<(usize, usize, f32, f32, usize)> = None;
        for (index, overlay) in overlays.iter().enumerate() {
            let Some(projected) = overlay.projected else {
                continue;
            };
            if !rect_contains_point(projected.frame_panel_logical, cursor_panel_logical) {
                continue;
            }

            let depth_rank = usize::MAX.saturating_sub(overlay.depth);
            let area = projected_frame_area(projected);
            let ray_t = overlay.ray_t.unwrap_or(f32::INFINITY);
            let candidate = (depth_rank, index, area, ray_t, overlay.stable_order);

            let replace = match best {
                None => true,
                Some((best_depth_rank, best_index, best_area, best_ray_t, best_stable_order)) => {
                    depth_rank < best_depth_rank
                        || (depth_rank == best_depth_rank
                            && (area < best_area
                                || (area == best_area
                                    && (ray_t < best_ray_t
                                        || (ray_t == best_ray_t
                                            && (overlay.stable_order < best_stable_order
                                                || (overlay.stable_order == best_stable_order
                                                    && index < best_index)))))))
                }
            };
            if replace {
                best = Some(candidate);
            }
        }

        if let Some((_, index, ..)) = best {
            return Some(index);
        }
    }

    overlays
        .iter()
        .enumerate()
        .filter_map(|(index, overlay)| {
            overlay
                .ray_t
                .zip(overlay.projected)
                .map(|(ray_t, projected)| {
                    (
                        ray_t,
                        overlay.depth,
                        projected_frame_area(projected),
                        overlay.stable_order,
                        index,
                    )
                })
        })
        .min_by(|a, b| {
            a.0.partial_cmp(&b.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.1.cmp(&a.1))
                .then_with(|| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.3.cmp(&b.3))
        })
        .map(|(.., index)| index)
}

fn component_label_text(def: &ObjectDef, order: usize) -> String {
    let label = def.label.trim();
    if !label.is_empty() {
        label.to_string()
    } else {
        format!("Component {}", order + 1)
    }
}

fn component_info_text(library: &ObjectLibrary, def: &ObjectDef, label: &str) -> String {
    let primitive_parts = def
        .parts
        .iter()
        .filter(|part| matches!(part.kind, ObjectPartKind::Primitive { .. }))
        .count();
    let child_components = def
        .parts
        .iter()
        .filter(|part| matches!(part.kind, ObjectPartKind::ObjectRef { .. }))
        .count();
    let mut channels = library.animation_channels_ordered(def.object_id);
    channels.retain(|channel| !channel.trim().is_empty());
    let channels_text = if channels.is_empty() {
        "none".to_string()
    } else if channels.len() <= 4 {
        channels.join(", ")
    } else {
        format!("{}, +{} more", channels[..4].join(", "), channels.len() - 4)
    };

    format!(
        "{label}\nsize: {:.2} x {:.2} x {:.2}\nparts: {primitive_parts} primitive | {child_components} child\nchannels: {channels_text}",
        def.size.x.abs(),
        def.size.y.abs(),
        def.size.z.abs(),
    )
}

pub(crate) fn gen3d_preview_tick_selected_animation(
    build_scene: Res<State<BuildScene>>,
    time: Res<Time>,
    mut preview: ResMut<Gen3dPreview>,
    library: Res<ObjectLibrary>,
    mut last_channel: Local<String>,
    mut roots: Query<
        (
            Entity,
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
            &mut ActionClock,
            &mut ForcedAnimationChannel,
        ),
        With<Gen3dPreviewUiModelRoot>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let dt = time.delta_secs();
    let wall_time = time.elapsed_secs();
    let object_id = super::gen3d_draft_object_id();

    let mut selected = preview.animation_channel.trim().to_string();
    if selected.is_empty() {
        selected = "idle".to_string();
    }

    let channel_changed = selected != *last_channel;
    if channel_changed {
        *last_channel = selected.clone();
    }

    for (_entity, mut channels, mut locomotion, mut attack, mut action, mut forced) in &mut roots {
        forced.channel = selected.clone();

        let wants_move =
            selected == "move" || library.channel_uses_move_driver(object_id, &selected);
        let wants_attack = library
            .channel_attack_duration_secs(object_id, &selected)
            .is_some();
        channels.moving = wants_move;
        channels.attacking_primary = wants_attack;
        let wants_action = selected == "action"
            || library
                .channel_action_duration_secs(object_id, &selected)
                .is_some();
        channels.acting = wants_action;

        let speed_mps = library
            .mobility(object_id)
            .map(|m| m.max_speed.abs())
            .filter(|v| v.is_finite())
            .unwrap_or(1.0)
            .max(0.25);

        if wants_move && dt.is_finite() && dt > 0.0 {
            locomotion.speed_mps = speed_mps;
            locomotion.t += speed_mps * dt;
            locomotion.distance_m += speed_mps * dt;
            locomotion.signed_distance_m += speed_mps * dt;
            if !locomotion.t.is_finite() {
                locomotion.t = 0.0;
            }
            if !locomotion.distance_m.is_finite() {
                locomotion.distance_m = 0.0;
            }
            if !locomotion.signed_distance_m.is_finite() {
                locomotion.signed_distance_m = 0.0;
            }
        } else {
            locomotion.speed_mps = 0.0;
        }

        if let Some(duration_secs) = library.channel_attack_duration_secs(object_id, &selected) {
            if channel_changed || attack.duration_secs <= 0.0 {
                attack.started_at_secs = wall_time;
                attack.duration_secs = duration_secs;
            }

            let elapsed = (wall_time - attack.started_at_secs).max(0.0);
            if attack.duration_secs > 0.0 && elapsed > attack.duration_secs {
                preview.animation_channel = "idle".to_string();
            }
        } else {
            attack.duration_secs = 0.0;
        }

        if let Some(duration_secs) = library.channel_action_duration_secs(object_id, &selected) {
            if channel_changed || action.duration_secs <= 0.0 {
                action.started_at_secs = wall_time;
                action.duration_secs = duration_secs;
            }

            let elapsed = (wall_time - action.started_at_secs).max(0.0);
            if action.duration_secs > 0.0 && elapsed > action.duration_secs {
                preview.animation_channel = "idle".to_string();
            }
        } else {
            action.duration_secs = 0.0;
        }
    }
}

pub(crate) fn gen3d_preview_orbit_controls(
    build_scene: Res<State<BuildScene>>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut workshop: ResMut<Gen3dWorkshop>,
    tweak: Res<Gen3dManualTweakState>,
    mut orbit_ui: Gen3dPreviewOrbitUi,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    draft: Res<Gen3dDraft>,
    parts: Query<(&GlobalTransform, &VisualPartId)>,
    focus_world: Gen3dPreviewFocusWorld,
    mut preview: ResMut<Gen3dPreview>,
    preview_camera_meta: Query<(&Camera, &GlobalTransform), With<Gen3dPreviewCamera>>,
    mut cameras: Query<&mut Transform, With<Gen3dPreviewCamera>>,
    mut drag_state: Local<Gen3dPreviewOrbitDragState>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let Ok(mut window) = orbit_ui.windows.single_mut() else {
        return;
    };

    let cursor_physical = window.physical_cursor_position();
    let Ok((_interaction, panel_node, panel_transform)) = orbit_ui.panel.single() else {
        // Drain wheel events so we don't build up.
        for _ in mouse_wheel.read() {}
        return;
    };

    let mut hovered = cursor_physical
        .map(|cursor| panel_node.contains_point(*panel_transform, cursor))
        .unwrap_or(false);

    if hovered {
        if let Some(cursor) = cursor_physical {
            let mut blocked = false;

            if let Ok((node, transform, vis)) = orbit_ui.side_panel_root.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = orbit_ui.side_panel_toggle.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = orbit_ui.anim_dropdown_button.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = orbit_ui.explode_toggle_button.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = orbit_ui.export_button.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = orbit_ui.anim_dropdown_list.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if blocked {
                hovered = false;
            }
        }
    }

    if mouse_buttons.just_released(MouseButton::Left) {
        drag_state.lmb_started_on_ffd_handle = false;
    }
    if mouse_buttons.just_pressed(MouseButton::Left) {
        drag_state.lmb_started_on_ffd_handle = false;

        if hovered && !tweak.color_picker_open && tweak.enabled && tweak.deform_mode {
            let picked_handle = cursor_physical.and_then(|cursor_physical| {
                let Some(part_id) = tweak.selected_part_id else {
                    return None;
                };
                let Some((_component, _before_transform, primitive)) =
                    super::manual_tweak::find_selected_primitive_part(&draft, part_id)
                else {
                    return None;
                };

                let part_transform = parts
                    .iter()
                    .find(|(_t, id)| id.0 == part_id)
                    .map(|(t, _id)| t.to_matrix());
                let Some(part_from_local) = part_transform else {
                    return None;
                };
                let inv_local_from_part = part_from_local.inverse();

                let Ok((_interaction, panel_node, panel_transform)) = orbit_ui.panel.single()
                else {
                    return None;
                };
                let Some(layout) = preview_image_layout(panel_node, *panel_transform) else {
                    return None;
                };
                let Some(cursor_target) =
                    preview_cursor_to_target(cursor_physical, layout.image_bounds_physical)
                else {
                    return None;
                };
                let Ok((camera, camera_transform)) = preview_camera_meta.single() else {
                    return None;
                };
                let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_target) else {
                    return None;
                };

                let ray_origin_local = inv_local_from_part.transform_point3(ray.origin);
                let ray_dir_local = inv_local_from_part.transform_vector3(ray.direction.into());
                super::manual_tweak::gen3d_ffd_pick_control_point_index_for_primitive(
                    &primitive,
                    ray_origin_local,
                    ray_dir_local,
                )
            });

            if picked_handle.is_some() {
                drag_state.lmb_started_on_ffd_handle = true;
            }
        }
    }

    if hovered && mouse_buttons.just_pressed(MouseButton::Left) && workshop.prompt_focused {
        workshop.prompt_focused = false;
        window.ime_enabled = false;
    }

    let cursor = window.cursor_position();

    if hovered {
        let mut scroll = 0.0f32;
        for ev in mouse_wheel.read() {
            let delta = match ev.unit {
                bevy::input::mouse::MouseScrollUnit::Line => ev.y,
                bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 120.0,
            };
            scroll += delta;
        }
        if scroll.abs() > 1e-4 {
            preview.distance = (preview.distance - scroll * 0.6).clamp(0.5, 250.0);
        }
    } else {
        // Drain wheel events so we don't build up.
        for _ in mouse_wheel.read() {}
    }

    let orbit_blocked = tweak.enabled && tweak.deform_mode && drag_state.lmb_started_on_ffd_handle;
    let dragging = hovered
        && mouse_buttons.pressed(MouseButton::Left)
        && !orbit_blocked
        && !tweak.color_picker_open;
    if dragging {
        if let (Some(prev), Some(cur)) = (preview.last_cursor, cursor) {
            let delta = cur - prev;
            let sensitivity = 0.010;
            preview.yaw = wrap_angle(preview.yaw - delta.x * sensitivity);
            preview.pitch = (preview.pitch + delta.y * sensitivity).clamp(-1.56, 1.56);
        }
    }

    if hovered && !workshop.prompt_focused && !tweak.color_picker_rgb_focused {
        let allow_arrow_pan = !tweak.enabled;
        let x = (keys.pressed(KeyCode::KeyD)
            || (allow_arrow_pan && keys.pressed(KeyCode::ArrowRight))) as i8
            - (keys.pressed(KeyCode::KeyA) || (allow_arrow_pan && keys.pressed(KeyCode::ArrowLeft)))
                as i8;
        let y = (keys.pressed(KeyCode::KeyW) || (allow_arrow_pan && keys.pressed(KeyCode::ArrowUp)))
            as i8
            - (keys.pressed(KeyCode::KeyS) || (allow_arrow_pan && keys.pressed(KeyCode::ArrowDown)))
                as i8;
        let mut pan_input = Vec2::new(x as f32, y as f32);
        if pan_input.length_squared() > 1.0 {
            pan_input = pan_input.normalize();
        }
        if pan_input.length_squared() > 1e-6 {
            let pan_units = pan_input * time.delta_secs() * 3.0;
            let yaw = preview.yaw;
            let pitch = preview.pitch;
            let distance = preview.distance;
            preview.view_pan += preview_pan_delta_world(yaw, pitch, distance, pan_units);
            if !preview.view_pan.is_finite() {
                preview.view_pan = Vec3::ZERO;
            }
        }
    }

    preview.last_cursor = if hovered { cursor } else { None };

    let exploded_center = focus_world.ui_roots.iter().next().and_then(|ui_root| {
        compute_preview_component_bounds_center_from_transforms(
            &focus_world.library,
            ui_root,
            focus_world
                .preview_components
                .iter()
                .map(|(entity, meta, transform, child_of)| {
                    (
                        entity,
                        meta,
                        transform,
                        child_of.map(|child_of| child_of.parent()),
                    )
                }),
        )
    });
    let focus = effective_preview_camera_focus(&preview, exploded_center);

    let Ok(mut camera_transform) = cameras.single_mut() else {
        return;
    };
    *camera_transform =
        crate::orbit_capture::orbit_transform(preview.yaw, preview.pitch, preview.distance, focus);
}

pub(crate) fn gen3d_clear_preview_component_explode_offsets(
    build_scene: Res<State<BuildScene>>,
    ui_roots: Query<Entity, With<Gen3dPreviewUiModelRoot>>,
    mut components: Query<
        (
            &VisualObjectRefRoot,
            &mut Transform,
            Option<&Gen3dPreviewAppliedExplodeOffset>,
        ),
        With<VisualObjectRefRoot>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let Some(ui_root) = ui_roots.iter().next() else {
        return;
    };

    for (meta, mut transform, applied_offset) in &mut components {
        if meta.root_entity != ui_root {
            continue;
        }
        let offset = applied_offset.map(|offset| offset.0).unwrap_or(Vec3::ZERO);
        if offset.length_squared() > 1e-8 {
            transform.translation -= offset;
        }
    }
}

pub(crate) fn gen3d_apply_preview_component_explode_offsets(
    build_scene: Res<State<BuildScene>>,
    preview: Res<Gen3dPreview>,
    library: Res<ObjectLibrary>,
    ui_roots: Query<Entity, With<Gen3dPreviewUiModelRoot>>,
    mut components: ParamSet<(
        Query<(
            Entity,
            &VisualObjectRefRoot,
            &Transform,
            Option<&ChildOf>,
            Option<&Gen3dPreviewAppliedExplodeOffset>,
        )>,
        Query<&mut Transform, With<VisualObjectRefRoot>>,
    )>,
    mut commands: Commands,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    let Some(ui_root) = ui_roots.iter().next() else {
        return;
    };

    let mut ordered_components = Vec::new();
    let mut component_chain =
        std::collections::HashMap::<Entity, (Option<Entity>, Transform)>::new();
    {
        let components_read = components.p0();
        for (entity, meta, transform, child_of, applied_offset) in &components_read {
            if meta.root_entity != ui_root {
                continue;
            }

            let parent_entity = child_of.map(|child_of| child_of.parent());
            let last_offset_local = applied_offset.map(|offset| offset.0).unwrap_or(Vec3::ZERO);
            component_chain.insert(entity, (parent_entity, *transform));
            ordered_components.push((
                entity,
                *meta,
                parent_entity,
                applied_offset.is_some(),
                last_offset_local,
            ));
        }
    }
    ordered_components.sort_by_key(|(entity, meta, ..)| preview_component_sort_key(meta, *entity));
    let mut world_cache = std::collections::HashMap::<Entity, Mat4>::new();
    let mut transforms = components.p1();

    for (stable_order, (entity, meta, parent_entity, had_applied_offset, last_offset_local)) in
        ordered_components.into_iter().enumerate()
    {
        let Ok(mut transform) = transforms.get_mut(entity) else {
            continue;
        };

        let new_offset_local = if preview.explode_components {
            if let Some(def) = library.get(meta.object_id) {
                let world_from_entity =
                    component_chain_world_affine(entity, &component_chain, &mut world_cache);
                let parent_world = parent_entity
                    .filter(|parent| component_chain.contains_key(parent))
                    .map(|parent| {
                        component_chain_world_affine(parent, &component_chain, &mut world_cache)
                    })
                    .unwrap_or(Mat4::IDENTITY);
                let current_center = world_from_entity
                    .transform_point3(preview_local_center(def.size, def.ground_origin_y));
                let desired_world_offset =
                    explode_offset(current_center - preview.draft_focus, def.size, stable_order);
                parent_world
                    .inverse()
                    .transform_vector3(desired_world_offset)
            } else {
                Vec3::ZERO
            }
        } else {
            Vec3::ZERO
        };

        transform.translation += new_offset_local;
        if !had_applied_offset || last_offset_local != new_offset_local {
            commands
                .entity(entity)
                .insert(Gen3dPreviewAppliedExplodeOffset(new_offset_local));
        }
    }
}

pub(crate) fn gen3d_update_preview_component_overlay(
    build_scene: Res<State<BuildScene>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    library: Res<ObjectLibrary>,
    mut preview: ResMut<Gen3dPreview>,
    cameras: Query<(&Camera, &GlobalTransform), With<Gen3dPreviewCamera>>,
    panels: Query<(&ComputedNode, &UiGlobalTransform), With<Gen3dPreviewPanel>>,
    ui_roots: Query<Entity, With<Gen3dPreviewUiModelRoot>>,
    components: Query<(
        Entity,
        &VisualObjectRefRoot,
        &GlobalTransform,
        Option<&Gen3dPreviewAppliedExplodeOffset>,
    )>,
    mut overlay_nodes: Query<
        (
            Option<&Gen3dPreviewHoverFrame>,
            Option<&Gen3dPreviewHoverInfoCard>,
            Option<&Gen3dPreviewComponentLabel>,
            &mut Node,
            &mut Visibility,
        ),
        Or<(
            With<Gen3dPreviewHoverFrame>,
            With<Gen3dPreviewHoverInfoCard>,
            With<Gen3dPreviewComponentLabel>,
        )>,
    >,
    mut overlay_texts: Query<
        (
            Option<&Gen3dPreviewHoverInfoText>,
            Option<&Gen3dPreviewComponentLabelText>,
            &mut Text,
        ),
        Or<(
            With<Gen3dPreviewHoverInfoText>,
            With<Gen3dPreviewComponentLabelText>,
        )>,
    >,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }

    let hide_overlay = |overlay_nodes: &mut Query<
        (
            Option<&Gen3dPreviewHoverFrame>,
            Option<&Gen3dPreviewHoverInfoCard>,
            Option<&Gen3dPreviewComponentLabel>,
            &mut Node,
            &mut Visibility,
        ),
        Or<(
            With<Gen3dPreviewHoverFrame>,
            With<Gen3dPreviewHoverInfoCard>,
            With<Gen3dPreviewComponentLabel>,
        )>,
    >,
                        overlay_texts: &mut Query<
        (
            Option<&Gen3dPreviewHoverInfoText>,
            Option<&Gen3dPreviewComponentLabelText>,
            &mut Text,
        ),
        Or<(
            With<Gen3dPreviewHoverInfoText>,
            With<Gen3dPreviewComponentLabelText>,
        )>,
    >| {
        for (_frame_marker, _card_marker, _label_marker, mut node, mut vis) in
            overlay_nodes.iter_mut()
        {
            node.display = Display::None;
            *vis = Visibility::Hidden;
        }
        for (_hover_info_marker, _label_marker, mut text) in overlay_texts.iter_mut() {
            **text = "".into();
        }
    };

    let context = windows
        .single()
        .ok()
        .zip(cameras.single().ok())
        .zip(panels.single().ok())
        .and_then(
            |((window, (camera, camera_transform)), (panel_node, panel_transform))| {
                preview_image_layout(panel_node, *panel_transform)
                    .map(|layout| (window, camera, camera_transform, layout))
            },
        )
        .and_then(|(window, camera, camera_transform, layout)| {
            ui_roots
                .iter()
                .next()
                .map(|ui_root| (window, camera, camera_transform, layout, ui_root))
        });
    let Some((window, camera, camera_transform, layout, ui_root)) = context else {
        hide_overlay(&mut overlay_nodes, &mut overlay_texts);
        preview.hovered_component = None;
        return;
    };

    let cursor_target = window
        .physical_cursor_position()
        .and_then(|cursor| preview_cursor_to_target(cursor, layout.image_bounds_physical));
    let cursor_panel_logical =
        cursor_target.map(|target| preview_target_to_panel_logical(target, layout));
    let ray =
        cursor_target.and_then(|cursor| camera.viewport_to_world(camera_transform, cursor).ok());
    let overlays = collect_preview_component_overlays(
        &library,
        camera,
        camera_transform,
        layout,
        ui_root,
        ray,
        components
            .iter()
            .map(|(entity, meta, global_transform, applied_offset)| {
                (
                    entity,
                    meta,
                    global_transform,
                    applied_offset.map(|offset| offset.0).unwrap_or(Vec3::ZERO),
                )
            }),
    );

    let hovered = pick_hovered_preview_component(&overlays, cursor_panel_logical)
        .and_then(|index| overlays.get(index).map(|overlay| (index, overlay)));

    let hovered_frame = hovered.and_then(|(_index, hovered)| {
        hovered.projected.map(|projected| {
            let frame_min = projected.frame_panel_logical.min.max(Vec2::ZERO);
            let frame_max = projected
                .frame_panel_logical
                .max
                .min(layout.panel_size_logical);
            (hovered, frame_min, frame_max)
        })
    });

    for (frame_marker, card_marker, label_marker, mut node, mut vis) in &mut overlay_nodes {
        if let Some(marker) = label_marker {
            if !preview.explode_components {
                node.display = Display::None;
                *vis = Visibility::Hidden;
                continue;
            }
            let Some((_, projected)) = overlays
                .get(marker.index())
                .and_then(|overlay| overlay.projected.map(|projected| (overlay, projected)))
            else {
                node.display = Display::None;
                *vis = Visibility::Hidden;
                continue;
            };
            let left = (projected.label_anchor_panel_logical.x + 10.0)
                .clamp(4.0, (layout.panel_size_logical.x - 96.0).max(4.0));
            let top = (projected.label_anchor_panel_logical.y - 10.0)
                .clamp(4.0, (layout.panel_size_logical.y - 28.0).max(4.0));
            node.left = Val::Px(left);
            node.top = Val::Px(top);
            node.display = Display::Flex;
            *vis = Visibility::Visible;
            continue;
        }

        if frame_marker.is_some() {
            let Some((_hovered, frame_min, frame_max)) = hovered_frame else {
                node.display = Display::None;
                *vis = Visibility::Hidden;
                continue;
            };
            let frame_size = (frame_max - frame_min).max(Vec2::splat(4.0));
            node.left = Val::Px(frame_min.x);
            node.top = Val::Px(frame_min.y);
            node.width = Val::Px(frame_size.x);
            node.height = Val::Px(frame_size.y);
            node.display = Display::Flex;
            *vis = Visibility::Visible;
            continue;
        }

        if card_marker.is_some() {
            let Some((_hovered, frame_min, frame_max)) = hovered_frame else {
                node.display = Display::None;
                *vis = Visibility::Hidden;
                continue;
            };
            let card_left =
                (frame_max.x + 10.0).clamp(8.0, (layout.panel_size_logical.x - 220.0).max(8.0));
            let card_top = frame_min
                .y
                .clamp(8.0, (layout.panel_size_logical.y - 92.0).max(8.0));
            node.left = Val::Px(card_left);
            node.top = Val::Px(card_top);
            node.display = Display::Flex;
            *vis = Visibility::Visible;
        }
    }

    let hover_info_text = hovered
        .and_then(|(_index, hovered)| {
            library
                .get(hovered.object_id)
                .map(|def| component_info_text(&library, def, &hovered.label))
        })
        .unwrap_or_default();

    for (hover_info_marker, label_marker, mut text) in &mut overlay_texts {
        if hover_info_marker.is_some() {
            **text = hover_info_text.clone().into();
            continue;
        }
        let value = if preview.explode_components {
            label_marker
                .and_then(|marker| overlays.get(marker.index()))
                .map(|overlay| overlay.label.as_str())
                .unwrap_or("")
        } else {
            ""
        };
        **text = value.into();
    }

    let Some((_index, hovered)) = hovered else {
        preview.hovered_component = None;
        return;
    };

    if library.get(hovered.object_id).is_none() {
        preview.hovered_component = Some(super::state::Gen3dPreviewHoveredComponent {
            entity: hovered.entity,
            object_id: hovered.object_id,
            label: hovered.label.clone(),
        });
        return;
    }

    preview.hovered_component = Some(super::state::Gen3dPreviewHoveredComponent {
        entity: hovered.entity,
        object_id: hovered.object_id,
        label: hovered.label.clone(),
    });
}

pub(crate) fn gen3d_apply_draft_to_preview(
    build_scene: Res<State<BuildScene>>,
    job: Res<Gen3dAiJob>,
    task_queue: Res<Gen3dTaskQueue>,
    tweak: Res<Gen3dManualTweakState>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    assets: Res<SceneAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cache: ResMut<MaterialCache>,
    mut mesh_cache: ResMut<crate::object::visuals::PrimitiveMeshCache>,
    mut library: ResMut<ObjectLibrary>,
    draft: Res<Gen3dDraft>,
    mut preview: ResMut<Gen3dPreview>,
    existing_ui: Query<Entity, With<Gen3dPreviewUiModelRoot>>,
    existing_capture: Query<Entity, With<Gen3dPreviewModelRoot>>,
) {
    fn remap_capture_id(map: &std::collections::HashMap<u128, u128>, object_id: u128) -> u128 {
        map.get(&object_id).copied().unwrap_or(object_id)
    }

    fn remap_defs_for_capture(
        draft: &Gen3dDraft,
        session_id: uuid::Uuid,
    ) -> (Vec<ObjectDef>, u128) {
        let salt = session_id.as_u128();
        let mut map = std::collections::HashMap::new();
        for def in &draft.defs {
            map.insert(def.object_id, def.object_id ^ salt);
        }

        let mut defs = draft.defs.clone();
        for def in &mut defs {
            def.object_id = remap_capture_id(&map, def.object_id);
            for part in &mut def.parts {
                if let ObjectPartKind::ObjectRef { object_id } = &mut part.kind {
                    *object_id = remap_capture_id(&map, *object_id);
                }
            }
            if let Some(aim) = def.aim.as_mut() {
                for object_id in &mut aim.components {
                    *object_id = remap_capture_id(&map, *object_id);
                }
            }
            if let Some(attack) = def.attack.as_mut() {
                if let Some(ranged) = attack.ranged.as_mut() {
                    ranged.projectile_prefab = remap_capture_id(&map, ranged.projectile_prefab);
                    ranged.muzzle.object_id = remap_capture_id(&map, ranged.muzzle.object_id);
                }
            }
        }

        let root_id = remap_capture_id(&map, super::gen3d_draft_object_id());
        (defs, root_id)
    }

    let in_preview = super::gen3d_ui_scene(build_scene.get());

    let mut running_session = None;
    if job.is_running() {
        running_session = Some(task_queue.active_session_id);
    } else if let Some(id) = task_queue.running_session_id {
        if id != task_queue.active_session_id {
            if let Some(state) = task_queue.inactive_states.get(&id) {
                if state.job.is_running() {
                    running_session = Some(id);
                }
            }
        }
    }

    if !in_preview && running_session.is_none() {
        return;
    }
    let Some(preview_root) = preview.root else {
        return;
    };

    if in_preview {
        let session_id = task_queue.active_session_id;
        let mark_parts = tweak.enabled;
        let needs_rebuild = existing_ui.is_empty()
            || preview.ui_applied_session_id != Some(session_id)
            || preview.ui_applied_assembly_rev != Some(job.assembly_rev())
            || preview.ui_applied_mark_parts != mark_parts;
        if needs_rebuild {
            for entity in &existing_ui {
                commands.entity(entity).try_despawn();
            }

            preview.ui_applied_session_id = Some(session_id);
            preview.ui_applied_assembly_rev = Some(job.assembly_rev());
            preview.ui_applied_mark_parts = mark_parts;

            if draft.defs.is_empty() {
                preview.draft_focus = Vec3::ZERO;
                preview.view_pan = Vec3::ZERO;
                preview.collision_dirty = true;
            } else {
                preview.draft_focus = compute_draft_focus(&draft);
                preview.view_pan = Vec3::ZERO;

                for mut def in draft.defs.clone() {
                    if def.object_id == super::gen3d_draft_object_id() {
                        def.object_id = super::gen3d_draft_object_id();
                        def.label = "gen3d_draft".into();
                    }
                    library.upsert(def);
                }

                let mut model_entity = commands.spawn((
                    Transform::IDENTITY,
                    Visibility::Inherited,
                    Gen3dPreviewUiModelRoot,
                    ObjectPrefabId(super::gen3d_draft_object_id()),
                    ForcedAnimationChannel {
                        channel: preview.animation_channel.clone(),
                    },
                    AnimationChannelsActive::default(),
                    LocomotionClock {
                        t: 0.0,
                        distance_m: 0.0,
                        signed_distance_m: 0.0,
                        speed_mps: 0.0,
                        last_translation: Vec3::ZERO,
                        last_move_dir_xz: Vec2::ZERO,
                    },
                    AttackClock::default(),
                    ActionClock::default(),
                ));
                crate::object::visuals::spawn_object_visuals_with_settings(
                    &mut model_entity,
                    &library,
                    &asset_server,
                    &assets,
                    &mut meshes,
                    &mut materials,
                    &mut cache,
                    &mut mesh_cache,
                    super::gen3d_draft_object_id(),
                    None,
                    VisualSpawnSettings {
                        mark_parts,
                        render_layer: Some(super::GEN3D_PREVIEW_UI_LAYER),
                    },
                );
                let model_id = model_entity.id();
                commands.entity(preview_root).add_child(model_id);

                let mut ordered =
                    library.animation_channels_ordered(super::gen3d_draft_object_id());
                let mut channels: Vec<String> =
                    vec!["idle".to_string(), "move".to_string(), "action".to_string()];
                for ch in ordered.drain(..) {
                    let trimmed = ch.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    if channels.iter().any(|existing| existing == trimmed) {
                        continue;
                    }
                    channels.push(trimmed.to_string());
                }
                preview.animation_channels = channels;

                let selected = preview.animation_channel.trim();
                if selected.is_empty()
                    || !preview.animation_channels.iter().any(|ch| ch == selected)
                {
                    preview.animation_channel = "idle".to_string();
                }

                preview.collision_dirty = true;
            }
        }
    }

    match running_session {
        None => {
            for entity in &existing_capture {
                commands.entity(entity).try_despawn();
            }
            preview.capture_root = None;
            preview.capture_applied_session_id = None;
            preview.capture_applied_assembly_rev = None;
        }
        Some(id) => {
            let (job_ref, draft_ref) = if id == task_queue.active_session_id {
                (&*job, &*draft)
            } else {
                let Some(state) = task_queue.inactive_states.get(&id) else {
                    return;
                };
                (&state.job, &state.draft)
            };

            let needs_rebuild = existing_capture.is_empty()
                || preview.capture_applied_session_id != Some(id)
                || preview.capture_applied_assembly_rev != Some(job_ref.assembly_rev());
            if !needs_rebuild {
                return;
            }

            for entity in &existing_capture {
                commands.entity(entity).try_despawn();
            }
            preview.capture_root = None;
            preview.capture_applied_session_id = Some(id);
            preview.capture_applied_assembly_rev = Some(job_ref.assembly_rev());

            if draft_ref.defs.is_empty() {
                return;
            }

            let (capture_defs, capture_root_id) = remap_defs_for_capture(draft_ref, id);
            for def in capture_defs {
                library.upsert(def);
            }

            let mut model_entity = commands.spawn((
                Transform::IDENTITY,
                Visibility::Inherited,
                Gen3dPreviewModelRoot,
                ObjectPrefabId(capture_root_id),
                ForcedAnimationChannel {
                    channel: preview.animation_channel.clone(),
                },
                AnimationChannelsActive::default(),
                LocomotionClock {
                    t: 0.0,
                    distance_m: 0.0,
                    signed_distance_m: 0.0,
                    speed_mps: 0.0,
                    last_translation: Vec3::ZERO,
                    last_move_dir_xz: Vec2::ZERO,
                },
                AttackClock::default(),
                ActionClock::default(),
            ));
            crate::object::visuals::spawn_object_visuals_with_settings(
                &mut model_entity,
                &library,
                &asset_server,
                &assets,
                &mut meshes,
                &mut materials,
                &mut cache,
                &mut mesh_cache,
                capture_root_id,
                None,
                VisualSpawnSettings {
                    mark_parts: false,
                    render_layer: Some(super::GEN3D_PREVIEW_LAYER),
                },
            );
            let model_id = model_entity.id();
            commands.entity(preview_root).add_child(model_id);
            preview.capture_root = Some(model_id);
        }
    }
}

pub(crate) fn gen3d_update_collision_overlay(
    build_scene: Res<State<BuildScene>>,
    mut commands: Commands,
    assets: Res<SceneAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    draft: Res<Gen3dDraft>,
    mut preview: ResMut<Gen3dPreview>,
    existing: Query<Entity, With<Gen3dPreviewCollisionRoot>>,
) {
    if !super::gen3d_ui_scene(build_scene.get()) {
        return;
    }
    if !preview.collision_dirty {
        return;
    }
    preview.collision_dirty = false;

    for entity in &existing {
        commands.entity(entity).try_despawn();
    }

    if !preview.show_collision {
        return;
    }
    let Some(preview_root) = preview.root else {
        return;
    };
    let Some(def) = draft.root_def() else {
        return;
    };

    let collider = def.collider;
    let (mesh, scale) = match collider {
        ColliderProfile::None => return,
        ColliderProfile::CircleXZ { radius } => (
            assets.unit_cylinder_mesh.clone(),
            Vec3::new((radius * 2.0).max(0.01), 0.02, (radius * 2.0).max(0.01)),
        ),
        ColliderProfile::AabbXZ { half_extents } => (
            assets.unit_cube_mesh.clone(),
            Vec3::new(
                (half_extents.x * 2.0).max(0.01),
                0.02,
                (half_extents.y * 2.0).max(0.01),
            ),
        ),
    };

    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.15, 0.95, 0.35, 0.28),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let collision_entity = commands
        .spawn((
            Mesh3d(mesh),
            MeshMaterial3d(material),
            Transform::from_translation(Vec3::new(0.0, 0.01, 0.0)).with_scale(scale),
            Visibility::Inherited,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_UI_LAYER),
            Gen3dPreviewCollisionRoot,
        ))
        .id();
    commands.entity(preview_root).add_child(collision_entity);
}

fn wrap_angle(mut v: f32) -> f32 {
    while v > std::f32::consts::PI {
        v -= std::f32::consts::TAU;
    }
    while v < -std::f32::consts::PI {
        v += std::f32::consts::TAU;
    }
    v
}

fn rotated_half_extents(half: Vec3, rotation: Quat) -> Vec3 {
    let abs = Mat3::from_quat(rotation).abs();
    abs * half
}

pub(super) fn compute_draft_focus(draft: &Gen3dDraft) -> Vec3 {
    let Some(root) = draft.root_def() else {
        return Vec3::ZERO;
    };
    if root.parts.is_empty() {
        return Vec3::ZERO;
    }

    let mut sizes = std::collections::HashMap::<u128, Vec3>::new();
    sizes.reserve(draft.defs.len());
    for def in &draft.defs {
        sizes.insert(def.object_id, def.size);
    }

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for part in root.parts.iter() {
        let (half, center, rot) = match &part.kind {
            ObjectPartKind::ObjectRef { object_id } => {
                let size = sizes.get(object_id).copied().unwrap_or(Vec3::ONE);
                (
                    size.abs() * 0.5,
                    part.transform.translation,
                    part.transform.rotation,
                )
            }
            ObjectPartKind::Primitive { .. } => (
                part.transform.scale.abs() * 0.5,
                part.transform.translation,
                part.transform.rotation,
            ),
            ObjectPartKind::Model { .. } => continue,
        };

        let ext = rotated_half_extents(half, rot);
        min = min.min(center - ext);
        max = max.max(center + ext);
    }

    if !min.x.is_finite() || !max.x.is_finite() {
        return Vec3::ZERO;
    }

    let center = (min + max) * 0.5;
    if !center.x.is_finite() || !center.y.is_finite() || !center.z.is_finite() {
        Vec3::ZERO
    } else {
        center
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        ColliderProfile, MeshKey, ObjectDef, ObjectInteraction, ObjectPartDef, PartAnimationDef,
        PartAnimationDriver, PartAnimationFamily, PartAnimationKeyframeDef, PartAnimationSlot,
        PartAnimationSpec, PrimitiveVisualDef,
    };
    use crate::types::{BuildScene, ForcedAnimationChannel};
    use std::time::Duration;

    #[test]
    fn preview_local_center_respects_ground_origin() {
        let centered = preview_local_center(Vec3::new(2.0, 4.0, 6.0), None);
        assert!((centered - Vec3::ZERO).length() < 1e-4);

        let feet_origin = preview_local_center(Vec3::new(2.0, 4.0, 6.0), Some(0.0));
        assert!((feet_origin - Vec3::new(0.0, 2.0, 0.0)).length() < 1e-4);

        let above_center = preview_local_center(Vec3::new(2.0, 4.0, 6.0), Some(3.5));
        assert!((above_center - Vec3::new(0.0, -1.5, 0.0)).length() < 1e-4);
    }

    #[test]
    fn explode_direction_uses_delta_when_available() {
        let delta = Vec3::new(3.0, 4.0, 0.0);
        let direction = explode_direction(delta, 2);
        assert!((direction - delta.normalize()).length() < 1e-4);
    }

    #[test]
    fn explode_direction_falls_back_to_stable_vector() {
        let a = explode_direction(Vec3::ZERO, 0);
        let b = explode_direction(Vec3::ZERO, 1);

        assert!(a.length() > 0.99 && a.length() < 1.01);
        assert!(b.length() > 0.99 && b.length() < 1.01);
        assert!(
            (a - b).length() > 0.1,
            "fallback directions should differ by order"
        );
    }

    #[test]
    fn preview_cursor_to_target_maps_displayed_image_space() {
        let image_bounds = Rect {
            min: Vec2::new(100.0, 50.0),
            max: Vec2::new(580.0, 320.0),
        };
        let mapped = preview_cursor_to_target(Vec2::new(340.0, 185.0), image_bounds)
            .expect("cursor inside image");

        assert!((mapped.x - crate::gen3d::GEN3D_PREVIEW_WIDTH_PX as f32 * 0.5).abs() < 1e-3);
        assert!((mapped.y - crate::gen3d::GEN3D_PREVIEW_HEIGHT_PX as f32 * 0.5).abs() < 1e-3);
        assert!(preview_cursor_to_target(Vec2::new(90.0, 185.0), image_bounds).is_none());
    }

    #[test]
    fn ray_intersects_local_aabb_returns_entry_distance() {
        let origin = Vec3::new(-2.0, 0.0, 0.0);
        let direction = Vec3::X;
        let half = Vec3::splat(0.5);
        let t = ray_intersects_local_aabb(origin, direction, half).expect("ray should hit");
        assert!((t - 1.5).abs() < 1e-4);

        let miss = ray_intersects_local_aabb(Vec3::new(-2.0, 2.0, 0.0), direction, half);
        assert!(miss.is_none());
    }

    #[test]
    fn preview_pan_delta_uses_camera_screen_axes() {
        let delta = preview_pan_delta_world(0.0, 0.0, 5.0, Vec2::new(1.0, 0.0));
        assert!(delta.x > 0.9, "expected positive right pan, got {delta:?}");
        assert!(delta.y.abs() < 1e-4, "unexpected vertical drift: {delta:?}");
        assert!(delta.z.abs() < 1e-4, "unexpected depth drift: {delta:?}");
    }

    #[test]
    fn effective_preview_focus_uses_exploded_center_plus_pan() {
        let preview = Gen3dPreview {
            draft_focus: Vec3::new(1.0, 2.0, 3.0),
            view_pan: Vec3::new(0.5, -0.25, 1.5),
            explode_components: true,
            ..Default::default()
        };
        let focus = effective_preview_camera_focus(&preview, Some(Vec3::new(4.0, 5.0, 6.0)));
        assert!((focus - Vec3::new(4.5, 4.75, 7.5)).length() < 1e-4);

        let assembled_preview = Gen3dPreview {
            explode_components: false,
            ..preview
        };
        let assembled_focus =
            effective_preview_camera_focus(&assembled_preview, Some(Vec3::new(40.0, 50.0, 60.0)));
        assert!((assembled_focus - Vec3::new(1.5, 1.75, 4.5)).length() < 1e-4);
    }

    #[test]
    fn hover_prefers_deeper_smaller_projected_component() {
        let overlays = vec![
            PreviewComponentOverlayInfo {
                entity: Entity::from_bits(1),
                parent_object_id: 10,
                object_id: 11,
                label: "torso".into(),
                depth: 1,
                order: 0,
                stable_order: 0,
                projected: Some(PreviewProjectedComponent {
                    frame_panel_logical: Rect {
                        min: Vec2::new(0.0, 0.0),
                        max: Vec2::new(100.0, 100.0),
                    },
                    label_anchor_panel_logical: Vec2::new(50.0, 50.0),
                }),
                ray_t: Some(1.0),
                applied_explode_offset_local: Vec3::ZERO,
            },
            PreviewComponentOverlayInfo {
                entity: Entity::from_bits(2),
                parent_object_id: 11,
                object_id: 12,
                label: "head".into(),
                depth: 2,
                order: 0,
                stable_order: 1,
                projected: Some(PreviewProjectedComponent {
                    frame_panel_logical: Rect {
                        min: Vec2::new(25.0, 25.0),
                        max: Vec2::new(75.0, 75.0),
                    },
                    label_anchor_panel_logical: Vec2::new(50.0, 50.0),
                }),
                ray_t: Some(1.2),
                applied_explode_offset_local: Vec3::ZERO,
            },
        ];

        let hovered =
            pick_hovered_preview_component(&overlays, Some(Vec2::new(50.0, 50.0))).unwrap();
        assert_eq!(
            hovered, 1,
            "nested component should win over enclosing parent"
        );
    }

    #[test]
    fn preview_tick_keeps_ui_preview_animating_during_motion_capture() {
        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(State::new(BuildScene::Preview));
        app.insert_resource(Gen3dPreview {
            animation_channel: "move".to_string(),
            ..Default::default()
        });
        let mut job = Gen3dAiJob::default();
        job.set_motion_capture_active_for_tests(true);
        app.insert_resource(job);
        app.insert_resource(ObjectLibrary::default());
        app.add_systems(Update, gen3d_preview_tick_selected_animation);

        let root = app
            .world_mut()
            .spawn((
                Gen3dPreviewUiModelRoot,
                AnimationChannelsActive::default(),
                LocomotionClock {
                    t: 0.0,
                    distance_m: 0.0,
                    signed_distance_m: 0.0,
                    speed_mps: 0.0,
                    last_translation: Vec3::ZERO,
                    last_move_dir_xz: Vec2::ZERO,
                },
                AttackClock::default(),
                ActionClock::default(),
                ForcedAnimationChannel::default(),
            ))
            .id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.5));
        app.update();

        let locomotion = app
            .world()
            .get::<LocomotionClock>(root)
            .expect("preview root locomotion clock");
        let forced = app
            .world()
            .get::<ForcedAnimationChannel>(root)
            .expect("preview root forced channel");
        assert!(
            locomotion.distance_m > 0.0,
            "expected preview locomotion clock to advance"
        );
        assert_eq!(forced.channel, "move");
    }

    #[test]
    fn preview_tick_marks_custom_attack_driven_channel_as_attacking() {
        let mut library = ObjectLibrary::default();
        let root_id = super::super::gen3d_draft_object_id();
        let attack_slot = PartAnimationSlot {
            channel: "lunge".into(),
            family: PartAnimationFamily::Base,
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::AttackTime,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)),
                    }],
                },
            },
        };
        library.upsert(ObjectDef {
            object_id: root_id,
            label: "draft".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: vec![ObjectPartDef::primitive(
                PrimitiveVisualDef::Primitive {
                    mesh: MeshKey::UnitCube,
                    params: None,
                    color: Color::WHITE,
                    unlit: false,
                    deform: None,
                },
                Transform::IDENTITY,
            )
            .with_animation_slot(
                attack_slot.channel,
                attack_slot.family,
                attack_slot.spec,
            )],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(State::new(BuildScene::Preview));
        app.insert_resource(Gen3dPreview {
            animation_channel: "lunge".to_string(),
            ..Default::default()
        });
        app.insert_resource(Gen3dAiJob::default());
        app.insert_resource(library);
        app.add_systems(Update, gen3d_preview_tick_selected_animation);

        let root = app
            .world_mut()
            .spawn((
                Gen3dPreviewUiModelRoot,
                AnimationChannelsActive::default(),
                LocomotionClock {
                    t: 0.0,
                    distance_m: 0.0,
                    signed_distance_m: 0.0,
                    speed_mps: 0.0,
                    last_translation: Vec3::ZERO,
                    last_move_dir_xz: Vec2::ZERO,
                },
                AttackClock::default(),
                ActionClock::default(),
                ForcedAnimationChannel::default(),
            ))
            .id();

        app.update();

        let channels = app
            .world()
            .get::<AnimationChannelsActive>(root)
            .copied()
            .expect("preview root channels");
        let attack = app
            .world()
            .get::<AttackClock>(root)
            .copied()
            .expect("preview root attack clock");
        assert!(channels.attacking_primary, "channels={channels:?}");
        assert!(attack.duration_secs > 0.0, "attack={attack:?}");
    }
}
