use bevy::camera::RenderTarget;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};

use crate::assets::SceneAssets;
use crate::object::registry::{ColliderProfile, ObjectLibrary, ObjectPartKind};
use crate::object::visuals::{MaterialCache, VisualSpawnSettings};
use crate::types::{
    AnimationChannelsActive, AttackClock, ForcedAnimationChannel, GameMode, LocomotionClock,
    ObjectPrefabId,
};

use super::ai::Gen3dAiJob;
use super::state::{
    Gen3dDraft, Gen3dPreview, Gen3dPreviewAnimationDropdownButton,
    Gen3dPreviewAnimationDropdownList, Gen3dPreviewCamera, Gen3dPreviewCollisionRoot,
    Gen3dPreviewLight, Gen3dPreviewModelRoot, Gen3dPreviewPanel, Gen3dPreviewSceneRoot,
    Gen3dReviewOverlayRoot, Gen3dSidePanelRoot, Gen3dSidePanelToggleButton,
};

pub(super) fn setup_preview_scene(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    assets: &SceneAssets,
    materials: &mut Assets<StandardMaterial>,
    preview: &mut Gen3dPreview,
) -> Handle<Image> {
    let target = create_preview_render_target(images);
    let root_entity = commands
        .spawn((
            Transform::IDENTITY,
            Visibility::Inherited,
            Gen3dPreviewSceneRoot,
        ))
        .id();

    // Initialize orbit defaults.
    preview.focus = Vec3::ZERO;
    preview.yaw = super::GEN3D_PREVIEW_DEFAULT_YAW;
    preview.pitch = super::GEN3D_PREVIEW_DEFAULT_PITCH;
    preview.distance = super::GEN3D_PREVIEW_DEFAULT_DISTANCE;
    let camera_transform =
        orbit_transform(preview.yaw, preview.pitch, preview.distance, preview.focus);

    let camera_entity = commands
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(Color::srgb(0.93, 0.94, 0.96)),
                ..default()
            },
            RenderTarget::Image(target.clone().into()),
            Tonemapping::TonyMcMapface,
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
            camera_transform,
            Gen3dPreviewCamera,
        ))
        .id();

    // Preview "studio" scene: simple three-point light rig (no floor plane).

    // Key light (casts shadows).
    commands.spawn((
        DirectionalLight {
            shadows_enabled: true,
            illuminance: 16_000.0,
            color: Color::srgb(1.0, 0.97, 0.94),
            ..default()
        },
        Transform::from_xyz(10.0, 18.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
        bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
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
        bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
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
        bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
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
        bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
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
    preview.last_cursor = None;
    preview.show_collision = false;
    preview.collision_dirty = true;

    target
}

pub(crate) fn gen3d_preview_tick_selected_animation(
    mode: Res<State<GameMode>>,
    time: Res<Time>,
    mut preview: ResMut<Gen3dPreview>,
    job: Res<Gen3dAiJob>,
    library: Res<ObjectLibrary>,
    mut last_channel: Local<String>,
    mut roots: Query<
        (
            Entity,
            &mut AnimationChannelsActive,
            &mut LocomotionClock,
            &mut AttackClock,
            &mut ForcedAnimationChannel,
        ),
        With<Gen3dPreviewModelRoot>,
    >,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    // Agent-driven render/motion capture sets locomotion/attack clocks deterministically.
    // Don't overwrite them with the interactive preview ticker while capture is active.
    if job.is_capturing_motion_sheets() {
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

    for (_entity, mut channels, mut locomotion, mut attack, mut forced) in &mut roots {
        forced.channel = selected.clone();

        let wants_move =
            selected == "move" || library.channel_uses_move_driver(object_id, &selected);
        channels.moving = wants_move;
        channels.attacking_primary = selected == "attack_primary";

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
    }
}

pub(crate) fn gen3d_preview_orbit_controls(
    mode: Res<State<GameMode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    mut mouse_wheel: bevy::ecs::message::MessageReader<bevy::input::mouse::MouseWheel>,
    panel: Query<&Interaction, With<Gen3dPreviewPanel>>,
    anim_dropdown_button: Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewAnimationDropdownButton>,
    >,
    anim_dropdown_list: Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dPreviewAnimationDropdownList>,
    >,
    side_panel_root: Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dSidePanelRoot>,
    >,
    side_panel_toggle: Query<
        (&ComputedNode, &UiGlobalTransform, Option<&Visibility>),
        With<Gen3dSidePanelToggleButton>,
    >,
    mut preview: ResMut<Gen3dPreview>,
    mut cameras: Query<&mut Transform, With<Gen3dPreviewCamera>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
    let mut hovered = panel
        .iter()
        .any(|i| matches!(*i, Interaction::Hovered | Interaction::Pressed));

    let cursor_physical = window.physical_cursor_position();
    if hovered {
        if let Some(cursor) = cursor_physical {
            let mut blocked = false;

            if let Ok((node, transform, vis)) = side_panel_root.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = side_panel_toggle.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = anim_dropdown_button.single() {
                let visible = vis
                    .map(|v| !matches!(*v, Visibility::Hidden))
                    .unwrap_or(true);
                if visible && node.contains_point(*transform, cursor) {
                    blocked = true;
                }
            }

            if let Ok((node, transform, vis)) = anim_dropdown_list.single() {
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

    let dragging = hovered && mouse_buttons.pressed(MouseButton::Left);
    if dragging {
        if let (Some(prev), Some(cur)) = (preview.last_cursor, cursor) {
            let delta = cur - prev;
            let sensitivity = 0.010;
            preview.yaw = wrap_angle(preview.yaw - delta.x * sensitivity);
            preview.pitch = (preview.pitch + delta.y * sensitivity).clamp(-1.56, 1.56);
        }
    }

    preview.last_cursor = if hovered { cursor } else { None };

    let Ok(mut camera_transform) = cameras.single_mut() else {
        return;
    };
    *camera_transform =
        orbit_transform(preview.yaw, preview.pitch, preview.distance, preview.focus);
}

pub(crate) fn gen3d_apply_draft_to_preview(
    mode: Res<State<GameMode>>,
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
    existing: Query<Entity, With<Gen3dPreviewModelRoot>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
        return;
    }
    let Some(preview_root) = preview.root else {
        return;
    };

    let needs_rebuild = draft.is_changed() || existing.is_empty();
    if !needs_rebuild {
        return;
    }

    for entity in &existing {
        commands.entity(entity).try_despawn();
    }

    if draft.defs.is_empty() {
        preview.collision_dirty = true;
        return;
    }

    preview.focus = compute_draft_focus(&draft);

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
        Gen3dPreviewModelRoot,
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
        },
        AttackClock::default(),
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
            mark_parts: false,
            render_layer: Some(super::GEN3D_PREVIEW_LAYER),
        },
    );
    let model_id = model_entity.id();
    commands.entity(preview_root).add_child(model_id);

    let mut ordered = library.animation_channels_ordered(super::gen3d_draft_object_id());
    let mut channels: Vec<String> = vec![
        "idle".to_string(),
        "move".to_string(),
    ];
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
        || !preview
            .animation_channels
            .iter()
            .any(|ch| ch == selected)
    {
        preview.animation_channel = "idle".to_string();
    }

    preview.collision_dirty = true;
}

pub(crate) fn gen3d_update_collision_overlay(
    mode: Res<State<GameMode>>,
    mut commands: Commands,
    assets: Res<SceneAssets>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    draft: Res<Gen3dDraft>,
    mut preview: ResMut<Gen3dPreview>,
    existing: Query<Entity, With<Gen3dPreviewCollisionRoot>>,
) {
    if !matches!(mode.get(), GameMode::Gen3D) {
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
            bevy::camera::visibility::RenderLayers::layer(super::GEN3D_PREVIEW_LAYER),
            Gen3dPreviewCollisionRoot,
        ))
        .id();
    commands.entity(preview_root).add_child(collision_entity);
}

fn create_preview_render_target(images: &mut Assets<Image>) -> Handle<Image> {
    let mut image = Image::new_target_texture(
        super::GEN3D_PREVIEW_WIDTH_PX,
        super::GEN3D_PREVIEW_HEIGHT_PX,
        TextureFormat::bevy_default(),
        None,
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    images.add(image)
}

fn orbit_transform(yaw: f32, pitch: f32, distance: f32, focus: Vec3) -> Transform {
    let rot = Quat::from_rotation_y(yaw) * Quat::from_rotation_x(pitch);
    let pos = focus + rot * Vec3::new(0.0, 0.0, distance);
    Transform::from_translation(pos).looking_at(focus, Vec3::Y)
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
