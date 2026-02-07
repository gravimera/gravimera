use bevy::ecs::hierarchy::ChildSpawnerCommands;
use bevy::prelude::*;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::types::EnemyLeg;

pub(crate) fn spawn_scaled_cube(
    parent: &mut ChildSpawnerCommands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    translation: Vec3,
    scale: Vec3,
) {
    parent.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        Transform::from_translation(translation).with_scale(scale),
    ));
}

pub(crate) fn spawn_leg_pivot(
    parent: &mut ChildSpawnerCommands,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    pivot: Vec3,
    size: Vec3,
    visual_offset: Vec3,
    group: f32,
) {
    parent
        .spawn((
            Transform::from_translation(pivot),
            Visibility::Inherited,
            EnemyLeg { group },
        ))
        .with_children(|leg| {
            spawn_scaled_cube(leg, mesh, material, visual_offset, size);
        });
}

pub(crate) fn spawn_dog_model(parent: &mut ChildSpawnerCommands, assets: &SceneAssets) {
    let mesh = &assets.unit_cube_mesh;
    let material = &assets.dog_material;

    spawn_scaled_cube(
        parent,
        mesh,
        material,
        Vec3::new(0.0, DOG_BODY_HEIGHT * 0.5, 0.0),
        Vec3::new(DOG_BODY_WIDTH, DOG_BODY_HEIGHT, DOG_BODY_LENGTH),
    );

    spawn_scaled_cube(
        parent,
        mesh,
        material,
        Vec3::new(
            0.0,
            DOG_BODY_HEIGHT + DOG_HEAD_SIZE * 0.5,
            DOG_BODY_LENGTH * 0.5 + DOG_HEAD_SIZE * 0.35,
        ),
        Vec3::splat(DOG_HEAD_SIZE),
    );

    let leg_size = Vec3::new(DOG_LEG_THICK, DOG_LEG_HEIGHT, DOG_LEG_THICK);
    let leg_offset = Vec3::new(0.0, -DOG_LEG_HEIGHT * 0.5, 0.0);

    // Diagonal gait: front-left + back-right swing together.
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(-DOG_LEG_OFFSET_X, 0.0, DOG_LEG_OFFSET_Z),
        leg_size,
        leg_offset,
        1.0,
    );
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(DOG_LEG_OFFSET_X, 0.0, DOG_LEG_OFFSET_Z),
        leg_size,
        leg_offset,
        -1.0,
    );
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(-DOG_LEG_OFFSET_X, 0.0, -DOG_LEG_OFFSET_Z),
        leg_size,
        leg_offset,
        -1.0,
    );
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(DOG_LEG_OFFSET_X, 0.0, -DOG_LEG_OFFSET_Z),
        leg_size,
        leg_offset,
        1.0,
    );
}

pub(crate) fn spawn_enemy_human_model(parent: &mut ChildSpawnerCommands, assets: &SceneAssets) {
    let mesh = &assets.unit_cube_mesh;

    spawn_scaled_cube(
        parent,
        mesh,
        &assets.human_material,
        Vec3::new(0.0, PLAYER_TORSO_HEIGHT * 0.5, 0.0),
        Vec3::new(PLAYER_TORSO_WIDTH, PLAYER_TORSO_HEIGHT, PLAYER_TORSO_DEPTH),
    );

    spawn_scaled_cube(
        parent,
        mesh,
        &assets.human_material,
        Vec3::new(0.0, PLAYER_TORSO_HEIGHT + PLAYER_HEAD_SIZE * 0.5, 0.0),
        Vec3::splat(PLAYER_HEAD_SIZE),
    );

    let leg_size = Vec3::new(PLAYER_LEG_SIZE, PLAYER_LEG_HEIGHT, PLAYER_LEG_SIZE);
    let leg_offset = Vec3::new(0.0, -PLAYER_LEG_HEIGHT * 0.5, 0.0);

    spawn_leg_pivot(
        parent,
        mesh,
        &assets.human_material,
        Vec3::new(-PLAYER_LEG_OFFSET_X, 0.0, 0.0),
        leg_size,
        leg_offset,
        1.0,
    );
    spawn_leg_pivot(
        parent,
        mesh,
        &assets.human_material,
        Vec3::new(PLAYER_LEG_OFFSET_X, 0.0, 0.0),
        leg_size,
        leg_offset,
        -1.0,
    );

    parent
        .spawn((
            Transform::from_xyz(
                0.0,
                PLAYER_GUN_Y,
                PLAYER_TORSO_DEPTH * 0.5 + PLAYER_GUN_OFFSET_Z,
            ),
            Visibility::Inherited,
        ))
        .with_children(|rig| {
            spawn_scaled_cube(
                rig,
                mesh,
                &assets.enemy_gun_material,
                Vec3::new(0.0, 0.0, PLAYER_GUN_LENGTH * 0.5),
                Vec3::new(PLAYER_GUN_THICK * 1.4, PLAYER_GUN_THICK, PLAYER_GUN_LENGTH),
            );

            spawn_scaled_cube(
                rig,
                mesh,
                &assets.human_material,
                Vec3::new(
                    -PLAYER_ARM_OFFSET_X,
                    -PLAYER_ARM_THICK * 0.6,
                    PLAYER_ARM_LENGTH * 0.5,
                ),
                Vec3::new(PLAYER_ARM_THICK, PLAYER_ARM_THICK, PLAYER_ARM_LENGTH),
            );
            spawn_scaled_cube(
                rig,
                mesh,
                &assets.human_material,
                Vec3::new(
                    PLAYER_ARM_OFFSET_X,
                    -PLAYER_ARM_THICK * 0.6,
                    PLAYER_ARM_LENGTH * 0.5,
                ),
                Vec3::new(PLAYER_ARM_THICK, PLAYER_ARM_THICK, PLAYER_ARM_LENGTH),
            );
        });
}

pub(crate) fn spawn_gundam_model(parent: &mut ChildSpawnerCommands, assets: &SceneAssets) {
    let mesh = &assets.unit_cube_mesh;
    let material = &assets.gundam_material;

    spawn_scaled_cube(
        parent,
        mesh,
        material,
        Vec3::new(0.0, GUNDAM_TORSO_HEIGHT * 0.5, 0.0),
        Vec3::new(GUNDAM_TORSO_WIDTH, GUNDAM_TORSO_HEIGHT, GUNDAM_TORSO_DEPTH),
    );

    spawn_scaled_cube(
        parent,
        mesh,
        material,
        Vec3::new(0.0, GUNDAM_TORSO_HEIGHT + GUNDAM_HEAD_SIZE * 0.5, 0.0),
        Vec3::splat(GUNDAM_HEAD_SIZE),
    );

    let leg_size = Vec3::new(GUNDAM_LEG_SIZE, GUNDAM_LEG_HEIGHT, GUNDAM_LEG_SIZE);
    let leg_offset = Vec3::new(0.0, -GUNDAM_LEG_HEIGHT * 0.5, 0.0);
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(-GUNDAM_LEG_OFFSET_X, 0.0, 0.0),
        leg_size,
        leg_offset,
        1.0,
    );
    spawn_leg_pivot(
        parent,
        mesh,
        material,
        Vec3::new(GUNDAM_LEG_OFFSET_X, 0.0, 0.0),
        leg_size,
        leg_offset,
        -1.0,
    );

    parent
        .spawn((
            Transform::from_xyz(
                0.0,
                GUNDAM_GUN_Y,
                GUNDAM_TORSO_DEPTH * 0.5 + GUNDAM_GUN_OFFSET_Z,
            ),
            Visibility::Inherited,
        ))
        .with_children(|rig| {
            spawn_scaled_cube(
                rig,
                mesh,
                &assets.enemy_gun_material,
                Vec3::new(0.0, 0.0, GUNDAM_GUN_LENGTH * 0.5),
                Vec3::new(GUNDAM_GUN_THICK * 1.2, GUNDAM_GUN_THICK, GUNDAM_GUN_LENGTH),
            );

            spawn_scaled_cube(
                rig,
                mesh,
                material,
                Vec3::new(
                    -GUNDAM_ARM_OFFSET_X,
                    -GUNDAM_ARM_THICK * 0.6,
                    GUNDAM_ARM_LENGTH * 0.5,
                ),
                Vec3::new(GUNDAM_ARM_THICK, GUNDAM_ARM_THICK, GUNDAM_ARM_LENGTH),
            );
            spawn_scaled_cube(
                rig,
                mesh,
                material,
                Vec3::new(
                    GUNDAM_ARM_OFFSET_X,
                    -GUNDAM_ARM_THICK * 0.6,
                    GUNDAM_ARM_LENGTH * 0.5,
                ),
                Vec3::new(GUNDAM_ARM_THICK, GUNDAM_ARM_THICK, GUNDAM_ARM_LENGTH),
            );
        });
}
