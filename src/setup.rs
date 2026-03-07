use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::object::types::{buildings, characters};
use crate::types::*;

pub(crate) fn setup_rendered(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
    mut selection: ResMut<SelectionState>,
) {
    info!(
         "Controls: LMB selects (click/drag), RMB issues move orders.\n\
         F1 toggles Build/Play (or the Play/Build button in the top bar).\n\
         Forms: Tab cycles forms (selected), hold C to copy source current form (release to confirm; Esc cancels).\n\
         Build/Play: hold Space to fire toward the cursor (ground or an enemy).\n\
         Build: Delete remove, M duplicate unit, Ctrl/Cmd+D duplicate build.\n\
         Edit: WASD move selected (camera-relative; CapsLock = 1/3 speed), ,/. rotate builds, -/= scale selected (Shift bigger).\n\
         Motions: 1..9 and 0 force-play the selected object's animation (slot 10).\n\
         Play: Ctrl/Cmd+1/2/3 switch guns (Normal/Shotgun/Laser), R restarts, Enter command.\n\
         Camera: edge-pan (cursor near window edge), rotate with Z/X/Q/E, zoom with mouse wheel (WASD pans camera when nothing selected).\n\
         Minimap: top-right."
    );

    let ground_mesh = meshes.add(Cuboid::new(60.0, 0.1, 60.0));
    let ground_material = materials.add(Color::srgb(0.16, 0.17, 0.20));

    let player_head_mesh = meshes.add(Cuboid::new(
        PLAYER_HEAD_SIZE,
        PLAYER_HEAD_SIZE,
        PLAYER_HEAD_SIZE,
    ));
    let player_torso_mesh = meshes.add(Cuboid::new(
        PLAYER_TORSO_WIDTH,
        PLAYER_TORSO_HEIGHT,
        PLAYER_TORSO_DEPTH,
    ));
    let player_leg_mesh = meshes.add(Cuboid::new(
        PLAYER_LEG_SIZE,
        PLAYER_LEG_HEIGHT,
        PLAYER_LEG_SIZE,
    ));
    let player_arm_mesh = meshes.add(Cuboid::new(
        PLAYER_ARM_THICK,
        PLAYER_ARM_THICK,
        PLAYER_ARM_LENGTH,
    ));
    let player_gun_mesh = meshes.add(Cuboid::new(
        PLAYER_GUN_THICK * 1.4,
        PLAYER_GUN_THICK,
        PLAYER_GUN_LENGTH,
    ));

    let player_skin_material = materials.add(Color::srgb(0.95, 0.80, 0.66));
    let player_shirt_material = materials.add(Color::srgb(0.35, 0.55, 0.95));
    let player_pants_material = materials.add(Color::srgb(0.18, 0.22, 0.30));
    let player_gun_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.18, 0.18, 0.20),
        metallic: 0.65,
        perceptual_roughness: 0.35,
        ..default()
    });

    let unit_cube_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let unit_cylinder_mesh = meshes.add(Cylinder::new(0.5, 1.0));
    let unit_cone_mesh = meshes.add(Cone::new(0.5, 1.0));
    let unit_sphere_mesh = meshes.add(Sphere::new(0.5));
    let unit_plane_mesh = meshes.add(Plane3d::default());
    let unit_capsule_mesh = meshes.add(Capsule3d::new(0.25, 0.5));
    let unit_conical_frustum_mesh = meshes.add(ConicalFrustum {
        radius_top: 0.25,
        radius_bottom: 0.5,
        height: 1.0,
    });
    let unit_torus_mesh = meshes.add(Torus::new(0.25, 0.5));
    let unit_triangle_mesh = meshes.add(Triangle3d::new(
        Vec3::new(0.0, 0.0, 0.5),
        Vec3::new(-0.5, 0.0, -0.5),
        Vec3::new(0.5, 0.0, -0.5),
    ));
    let unit_tetrahedron_mesh = meshes.add(Tetrahedron::default());
    let dog_material = materials.add(Color::srgb(0.70, 0.58, 0.38));
    let human_material = materials.add(Color::srgb(0.86, 0.28, 0.28));
    let gundam_material = materials.add(Color::srgb(0.28, 0.82, 0.40));

    let enemy_gun_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.12, 0.12, 0.14),
        metallic: 0.7,
        perceptual_roughness: 0.35,
        ..default()
    });

    let enemy_bullet_mesh = meshes.add(Sphere::new(ENEMY_BULLET_MESH_RADIUS));
    let enemy_bullet_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.88, 0.20),
        emissive: Color::linear_rgb(2.4, 2.1, 0.5).into(),
        unlit: true,
        ..default()
    });
    let enemy_bullet_spot_mesh = meshes.add(Sphere::new(ENEMY_BULLET_SPOT_RADIUS));
    let enemy_bullet_spot_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.98, 0.25, 0.25),
        emissive: Color::linear_rgb(2.6, 0.5, 0.5).into(),
        unlit: true,
        ..default()
    });

    let gundam_energy_ball_mesh = meshes.add(Sphere::new(GUNDAM_ENERGY_BALL_MESH_RADIUS));
    let gundam_energy_ball_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.75),
        emissive: Color::linear_rgb(3.4, 3.4, 3.6).into(),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let gundam_energy_arc_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.55, 0.85, 1.0, 0.85),
        emissive: Color::linear_rgb(0.8, 1.4, 2.5).into(),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });
    let gundam_energy_impact_material = materials.add(StandardMaterial {
        base_color: Color::srgba(1.0, 1.0, 1.0, 0.85),
        emissive: Color::linear_rgb(3.2, 3.2, 3.5).into(),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        ..default()
    });

    let explosion_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.98, 0.55, 0.20, 0.8),
        emissive: Color::linear_rgb(2.8, 1.3, 0.4).into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let blood_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.85, 0.08, 0.08, 0.85),
        emissive: Color::linear_rgb(1.6, 0.15, 0.15).into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let health_bar_bg_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.15, 0.06, 0.06, 0.85),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });
    let health_bar_fg_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.95, 0.40),
        emissive: Color::linear_rgb(0.4, 0.9, 0.5).into(),
        unlit: true,
        ..default()
    });

    let bullet_mesh = meshes.add(Cuboid::new(0.22, 0.22, BULLET_MESH_LENGTH));
    let bullet_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.95, 0.85, 0.25),
        emissive: Color::srgb(0.95, 0.85, 0.25).into(),
        unlit: true,
        ..default()
    });

    let shotgun_pellet_mesh = meshes.add(Sphere::new(SHOTGUN_PELLET_RADIUS));
    let shotgun_pellet_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.45, 0.05, 0.05),
        emissive: Color::linear_rgb(0.3, 0.0, 0.0).into(),
        unlit: true,
        ..default()
    });

    let laser_mesh = meshes.add(Cuboid::new(0.20, 0.20, LASER_RANGE));
    let laser_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.25, 0.90, 0.98, 0.35),
        emissive: Color::linear_rgb(1.6, 1.9, 2.4).into(),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        ..default()
    });

    let build_block_materials = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.86, 0.56, 0.36),
            perceptual_roughness: 0.92,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.80, 0.42, 0.28),
            perceptual_roughness: 0.92,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.68, 0.30, 0.20),
            perceptual_roughness: 0.92,
            ..default()
        }),
    ];

    let fence_stake_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.22, 0.24, 0.27),
        metallic: 1.0,
        perceptual_roughness: 0.35,
        ..default()
    });
    let fence_stick_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.40, 0.42, 0.46),
        metallic: 1.0,
        perceptual_roughness: 0.30,
        ..default()
    });

    let tree_trunk_mesh = meshes.add(Cylinder::new(1.0, 1.0));
    let tree_cone_mesh = meshes.add(Cone::new(1.0, 1.0));
    let move_target_mesh = meshes.add(Cylinder::new(1.0, 1.0));

    let tree_trunk_materials = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.48, 0.30, 0.16),
            perceptual_roughness: 0.9,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.44, 0.28, 0.15),
            perceptual_roughness: 0.9,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.52, 0.33, 0.19),
            perceptual_roughness: 0.9,
            ..default()
        }),
    ];
    let tree_main_materials = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.11, 0.42, 0.18),
            perceptual_roughness: 0.94,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.10, 0.36, 0.22),
            perceptual_roughness: 0.94,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.14, 0.44, 0.15),
            perceptual_roughness: 0.94,
            ..default()
        }),
    ];
    let tree_crown_materials = [
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.30, 0.70, 0.34),
            perceptual_roughness: 0.92,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.26, 0.62, 0.40),
            perceptual_roughness: 0.92,
            ..default()
        }),
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.34, 0.74, 0.30),
            perceptual_roughness: 0.92,
            ..default()
        }),
    ];

    let move_target_material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.20, 0.95, 0.35, 0.70),
        emissive: Color::linear_rgb(0.45, 1.60, 0.65).into(),
        perceptual_roughness: 0.25,
        ..default()
    });

    let base_back_offset_z =
        PLAYER_TORSO_DEPTH * 0.5 + PLAYER_GUN_OFFSET_Z + PLAYER_GUN_RIG_FORWARD_OFFSET_Z
            - PLAYER_GUN_TORSO_PULLBACK_Z;
    let gun_length = PLAYER_GUN_LENGTH * HERO_GUN_MODEL_SCALE_MULT;
    commands.insert_resource(PlayerMuzzles {
        normal: base_back_offset_z + gun_length,
        shotgun: base_back_offset_z + gun_length,
        laser: base_back_offset_z + gun_length,
    });

    commands.insert_resource(SceneAssets {
        unit_cube_mesh: unit_cube_mesh.clone(),
        unit_cylinder_mesh: unit_cylinder_mesh.clone(),
        unit_cone_mesh: unit_cone_mesh.clone(),
        unit_sphere_mesh: unit_sphere_mesh.clone(),
        unit_plane_mesh: unit_plane_mesh.clone(),
        unit_capsule_mesh: unit_capsule_mesh.clone(),
        unit_conical_frustum_mesh: unit_conical_frustum_mesh.clone(),
        unit_torus_mesh: unit_torus_mesh.clone(),
        unit_triangle_mesh: unit_triangle_mesh.clone(),
        unit_tetrahedron_mesh: unit_tetrahedron_mesh.clone(),
        dog_material: dog_material.clone(),
        human_material: human_material.clone(),
        gundam_material: gundam_material.clone(),
        enemy_gun_material: enemy_gun_material.clone(),
        enemy_bullet_mesh: enemy_bullet_mesh.clone(),
        enemy_bullet_material: enemy_bullet_material.clone(),
        enemy_bullet_spot_mesh: enemy_bullet_spot_mesh.clone(),
        enemy_bullet_spot_material: enemy_bullet_spot_material.clone(),
        gundam_energy_ball_mesh: gundam_energy_ball_mesh.clone(),
        gundam_energy_ball_material: gundam_energy_ball_material.clone(),
        gundam_energy_arc_material: gundam_energy_arc_material.clone(),
        gundam_energy_impact_material: gundam_energy_impact_material.clone(),
        explosion_material: explosion_material.clone(),
        blood_material: blood_material.clone(),
        health_bar_bg_material: health_bar_bg_material.clone(),
        health_bar_fg_material: health_bar_fg_material.clone(),
        bullet_mesh: bullet_mesh.clone(),
        bullet_material: bullet_material.clone(),
        shotgun_pellet_mesh: shotgun_pellet_mesh.clone(),
        shotgun_pellet_material: shotgun_pellet_material.clone(),
        laser_mesh: laser_mesh.clone(),
        laser_material: laser_material.clone(),
        build_block_materials,
        fence_stake_material: fence_stake_material.clone(),
        fence_stick_material: fence_stick_material.clone(),
        tree_trunk_mesh: tree_trunk_mesh.clone(),
        tree_cone_mesh: tree_cone_mesh.clone(),
        tree_trunk_materials,
        tree_main_materials,
        tree_crown_materials,
        move_target_mesh: move_target_mesh.clone(),
        move_target_material: move_target_material.clone(),
    });

    commands.insert_resource(BuildPreview::default());

    let minimap_triangle = create_triangle_mask_image(&mut images);
    let minimap_floor = create_minimap_floor_tile(&mut images);
    commands.insert_resource(MinimapIcons {
        triangle: minimap_triangle,
    });

    commands.spawn((
        ObjectId::new_v4(),
        ObjectPrefabId(buildings::ground::object_id()),
        Mesh3d(ground_mesh),
        MeshMaterial3d(ground_material),
        Transform::from_xyz(0.0, -0.05, 0.0),
        Visibility::Inherited,
    ));

    let player_start = Vec3::new(0.0, PLAYER_Y, 0.0);
    let player_entity = commands
        .spawn((
            ObjectId::new_v4(),
            ObjectPrefabId(characters::hero::object_id()),
            Transform::from_translation(player_start),
            Visibility::Inherited,
            Player,
            Commandable,
            Health::new(PLAYER_MAX_HEALTH, PLAYER_MAX_HEALTH),
            LaserDamageAccum::default(),
            Collider {
                radius: PLAYER_RADIUS,
            },
            PlayerAnimator {
                phase: 0.0,
                last_translation: player_start,
            },
        ))
        .id();

    selection.selected.clear();
    selection.selected.insert(player_entity);

    let mut player_health_root = None;
    let mut player_health_fill = None;
    commands.entity(player_entity).with_children(|parent| {
        parent.spawn((
            Mesh3d(player_torso_mesh.clone()),
            MeshMaterial3d(player_shirt_material.clone()),
            Transform::from_xyz(0.0, PLAYER_TORSO_HEIGHT / 2.0, 0.0),
            Visibility::Inherited,
        ));

        parent.spawn((
            Mesh3d(player_head_mesh.clone()),
            MeshMaterial3d(player_skin_material.clone()),
            Transform::from_xyz(0.0, PLAYER_TORSO_HEIGHT + PLAYER_HEAD_SIZE / 2.0, 0.0),
            Visibility::Inherited,
        ));

        parent
            .spawn((
                Transform::from_xyz(-PLAYER_LEG_OFFSET_X, 0.0, 0.0),
                Visibility::Inherited,
                PlayerLeg { side: 1.0 },
            ))
            .with_children(|leg| {
                leg.spawn((
                    Mesh3d(player_leg_mesh.clone()),
                    MeshMaterial3d(player_pants_material.clone()),
                    Transform::from_xyz(0.0, -PLAYER_LEG_HEIGHT / 2.0, 0.0),
                    Visibility::Inherited,
                ));
            });

        parent
            .spawn((
                Transform::from_xyz(PLAYER_LEG_OFFSET_X, 0.0, 0.0),
                Visibility::Inherited,
                PlayerLeg { side: -1.0 },
            ))
            .with_children(|leg| {
                leg.spawn((
                    Mesh3d(player_leg_mesh.clone()),
                    MeshMaterial3d(player_pants_material.clone()),
                    Transform::from_xyz(0.0, -PLAYER_LEG_HEIGHT / 2.0, 0.0),
                    Visibility::Inherited,
                ));
            });

        parent
            .spawn((
                Transform::from_xyz(0.0, PLAYER_GUN_Y, 0.0),
                Visibility::Inherited,
                PlayerGunRig,
            ))
            .with_children(|rig| {
                let spawn_gun = |rig: &mut ChildSpawnerCommands, weapon: PlayerWeapon| {
                    let visibility = if weapon == PlayerWeapon::Normal {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    };

                    let translation_z = base_back_offset_z + gun_length * 0.5;
                    let transform = Transform::from_translation(Vec3::new(0.0, 0.0, translation_z))
                        .with_scale(Vec3::splat(HERO_GUN_MODEL_SCALE_MULT));

                    rig.spawn((
                        Mesh3d(player_gun_mesh.clone()),
                        MeshMaterial3d(player_gun_material.clone()),
                        transform,
                        visibility,
                        PlayerGunVisual { weapon },
                    ));
                };

                spawn_gun(rig, PlayerWeapon::Normal);
                spawn_gun(rig, PlayerWeapon::Shotgun);
                spawn_gun(rig, PlayerWeapon::Laser);

                rig.spawn((
                    Mesh3d(player_arm_mesh.clone()),
                    MeshMaterial3d(player_shirt_material.clone()),
                    Transform::from_xyz(
                        -PLAYER_ARM_OFFSET_X,
                        -PLAYER_ARM_THICK * 0.6,
                        PLAYER_ARM_LENGTH / 2.0,
                    ),
                    Visibility::Inherited,
                ));

                rig.spawn((
                    Mesh3d(player_arm_mesh.clone()),
                    MeshMaterial3d(player_shirt_material.clone()),
                    Transform::from_xyz(
                        PLAYER_ARM_OFFSET_X,
                        -PLAYER_ARM_THICK * 0.6,
                        PLAYER_ARM_LENGTH / 2.0,
                    ),
                    Visibility::Inherited,
                ));
            });

        let bar_root = parent
            .spawn((
                Transform::from_xyz(0.0, PLAYER_HEALTH_BAR_OFFSET_Y, 0.0),
                Visibility::Inherited,
            ))
            .with_children(|bar| {
                bar.spawn((
                    Mesh3d(unit_cube_mesh.clone()),
                    MeshMaterial3d(health_bar_bg_material.clone()),
                    Transform::from_scale(Vec3::new(
                        HEALTH_BAR_WIDTH,
                        HEALTH_BAR_HEIGHT,
                        HEALTH_BAR_DEPTH,
                    )),
                    Visibility::Inherited,
                ));

                player_health_fill = Some(
                    bar.spawn((
                        Mesh3d(unit_cube_mesh.clone()),
                        MeshMaterial3d(health_bar_fg_material.clone()),
                        Transform::from_translation(Vec3::new(0.0, 0.0, HEALTH_BAR_Z_OFFSET))
                            .with_scale(Vec3::new(
                                HEALTH_BAR_WIDTH,
                                HEALTH_BAR_HEIGHT * HEALTH_BAR_FILL_SCALE,
                                HEALTH_BAR_DEPTH * HEALTH_BAR_FILL_SCALE,
                            )),
                        Visibility::Inherited,
                        HealthBarFill,
                    ))
                    .id(),
                );
            })
            .id();
        player_health_root = Some(bar_root);
    });
    if let (Some(root), Some(fill)) = (player_health_root, player_health_fill) {
        commands
            .entity(player_entity)
            .try_insert(HealthBar { root, fill });
    }

    commands.spawn((
        DirectionalLight {
            shadows_enabled: false,
            illuminance: 18_000.0,
            ..default()
        },
        Transform::from_xyz(8.0, 18.0, 8.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(player_start + CAMERA_OFFSET).looking_at(player_start, Vec3::Y),
        MainCamera,
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(EDGE_SCROLL_INDICATOR_SIZE_PX),
                height: Val::Px(EDGE_SCROLL_INDICATOR_SIZE_PX),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            ZIndex(300),
            EdgeScrollIndicatorRoot,
        ))
        .with_children(|parent| {
            parent.spawn((
                Text::new(""),
                TextFont {
                    font_size: EDGE_SCROLL_INDICATOR_FONT_SIZE_PX,
                    ..default()
                },
                TextColor(Color::srgba(0.35, 1.0, 0.45, 1.0)),
                TextShadow {
                    offset: Vec2::splat(2.0),
                    color: Color::linear_rgba(0.0, 0.0, 0.0, 0.85),
                },
                EdgeScrollIndicatorText,
            ));
        });

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(8.0),
                right: Val::Px(10.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexEnd,
                row_gap: Val::Px(6.0),
                ..default()
            },
            ZIndex(280),
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.75)),
                    BorderColor::all(Color::srgba(0.25, 0.25, 0.30, 0.65)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new("Obj: -- | Prim: -- | FPS: --"),
                        TextFont {
                            font_size: 14.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.92, 0.92, 0.96)),
                        TextShadow {
                            offset: Vec2::splat(2.0),
                            color: Color::linear_rgba(0.0, 0.0, 0.0, 0.85),
                        },
                        FpsCounterText,
                    ));
                });

            parent
                .spawn((
                    Node {
                        width: Val::Px(MINIMAP_SIZE_PX),
                        height: Val::Px(MINIMAP_SIZE_PX),
                        border: UiRect::all(Val::Px(MINIMAP_BORDER_PX)),
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.75)),
                    BorderColor::all(Color::srgb(0.95, 0.85, 0.25)),
                    MinimapRoot,
                ))
                .with_children(|parent| {
                    let inner_size = (MINIMAP_SIZE_PX - MINIMAP_BORDER_PX * 2.0).max(1.0);
                    parent.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(MINIMAP_BORDER_PX),
                            top: Val::Px(MINIMAP_BORDER_PX),
                            width: Val::Px(inner_size),
                            height: Val::Px(inner_size),
                            ..default()
                        },
                        ImageNode::new(minimap_floor)
                            .with_mode(NodeImageMode::Tiled {
                                tile_x: true,
                                tile_y: true,
                                stretch_value: 0.0,
                            })
                            .with_color(Color::srgba(1.0, 1.0, 1.0, 0.75)),
                        ZIndex(-2),
                    ));
                });
        });

    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(1.0),
            height: Val::Px(1.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.25, 0.95, 0.45, 0.12)),
        BorderColor::all(Color::srgba(0.25, 0.95, 0.45, 0.85)),
        Visibility::Hidden,
        ZIndex(320),
        SelectionBoxUi,
    ));

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            Visibility::Hidden,
            CommandConsoleRoot,
        ))
        .with_children(|parent| {
            parent
                .spawn((
                    Node {
                        width: Val::Px(640.0),
                        padding: UiRect::all(Val::Px(14.0)),
                        flex_direction: FlexDirection::Column,
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.03, 0.03, 0.04, 0.92)),
                    BorderColor::all(Color::srgb(0.95, 0.85, 0.25)),
                    Outline {
                        width: Val::Px(2.0),
                        color: Color::srgb(0.95, 0.85, 0.25),
                        offset: Val::Px(0.0),
                    },
                    Visibility::Inherited,
                ))
                .with_children(|panel| {
                    panel.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 18.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.95, 0.85, 0.25)),
                        CommandConsoleText,
                    ));
                });
        });
}

fn create_triangle_mask_image(images: &mut Assets<Image>) -> Handle<Image> {
    let size: u32 = 64;
    let w = size as f32;
    let h = size as f32;

    let tip_margin = 4.0;
    let base_margin = 6.0;
    let base_half_width = w * 0.22;

    let a = Vec2::new(w * 0.5, tip_margin);
    let b = Vec2::new(w * 0.5 - base_half_width, h - base_margin);
    let c = Vec2::new(w * 0.5 + base_half_width, h - base_margin);

    let mut data = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            if point_in_triangle(p, a, b, c) {
                let idx = ((y * size + x) * 4) as usize;
                data[idx] = 255;
                data[idx + 1] = 255;
                data[idx + 2] = 255;
                data[idx + 3] = 255;
            }
        }
    }

    images.add(Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ))
}

fn point_in_triangle(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> bool {
    let v0 = c - a;
    let v1 = b - a;
    let v2 = p - a;

    let dot00 = v0.dot(v0);
    let dot01 = v0.dot(v1);
    let dot02 = v0.dot(v2);
    let dot11 = v1.dot(v1);
    let dot12 = v1.dot(v2);

    let denom = dot00 * dot11 - dot01 * dot01;
    if denom.abs() <= 1e-6 {
        return false;
    }

    let inv_denom = 1.0 / denom;
    let u = (dot11 * dot02 - dot01 * dot12) * inv_denom;
    let v = (dot00 * dot12 - dot01 * dot02) * inv_denom;
    u >= 0.0 && v >= 0.0 && (u + v) <= 1.0
}

fn create_minimap_floor_tile(images: &mut Assets<Image>) -> Handle<Image> {
    let size: u32 = 32;
    let block: u32 = 8;

    let mut data = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let cx = x / block;
            let cy = y / block;
            let parity = (cx + cy) & 1;
            let (r, g, b) = if parity == 0 {
                (20u8, 22u8, 27u8)
            } else {
                (16u8, 18u8, 22u8)
            };

            let mut a = 170u8;
            if x % block == 0 || y % block == 0 {
                a = 220u8;
            }

            let idx = ((y * size + x) * 4) as usize;
            data[idx] = r;
            data[idx + 1] = g;
            data[idx + 2] = b;
            data[idx + 3] = a;
        }
    }

    images.add(Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::default(),
    ))
}
