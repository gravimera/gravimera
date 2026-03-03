use bevy::prelude::*;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::object::registry::ObjectLibrary;
use crate::types::*;

pub(crate) const DEFAULT_UNIT_MAX_HEALTH: i32 = 100;

fn default_max_health_for_prefab(library: &ObjectLibrary, prefab_id: u128) -> i32 {
    library
        .enemy(prefab_id)
        .map(|profile| profile.max_health)
        .unwrap_or(DEFAULT_UNIT_MAX_HEALTH)
        .max(1)
}

pub(crate) fn start_die_motion(
    commands: &mut Commands,
    wall_time: f32,
    library: &ObjectLibrary,
    entity: Entity,
    prefab_id: u128,
    transform: &Transform,
) {
    let restore_transform = transform.clone();

    let scale_y = transform.scale.y.abs().max(0.001);
    let height = library
        .size(prefab_id)
        .map(|size| size.y.abs() * scale_y)
        .unwrap_or(HERO_HEIGHT_WORLD * scale_y);

    let sink = (height * UNIT_DIE_SINK_FRACTION_HEIGHT).clamp(0.0, 1.25);

    let end = Transform {
        translation: transform.translation - Vec3::Y * sink,
        rotation: transform.rotation * Quat::from_rotation_x(UNIT_DIE_PITCH_RADS),
        scale: transform.scale,
    };

    commands.entity(entity).insert(Died { restore_transform });
    commands.entity(entity).insert(DieMotion {
        started_at_secs: wall_time,
        duration_secs: UNIT_DIE_MOTION_SECS,
        start: transform.clone(),
        end,
    });
}

pub(crate) fn ensure_health_for_commandables(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    q: Query<(Entity, &ObjectPrefabId), (With<Commandable>, Without<Health>)>,
) {
    for (entity, prefab_id) in &q {
        let max_health = default_max_health_for_prefab(&library, prefab_id.0);
        commands
            .entity(entity)
            .insert(Health::new(max_health, max_health));
    }
}

pub(crate) fn ensure_laser_damage_accum_for_units(
    mut commands: Commands,
    q: Query<Entity, (With<Health>, Without<LaserDamageAccum>)>,
) {
    for entity in &q {
        commands.entity(entity).insert(LaserDamageAccum::default());
    }
}

pub(crate) fn ensure_health_bars_for_units(
    mut commands: Commands,
    assets: Res<SceneAssets>,
    library: Res<ObjectLibrary>,
    units: Query<
        (Entity, &ObjectPrefabId),
        (
            With<Health>,
            Without<HealthBar>,
            Or<(With<Commandable>, With<Enemy>)>,
        ),
    >,
) {
    for (entity, prefab_id) in &units {
        let offset_y = library
            .health_bar_offset_y(prefab_id.0)
            .unwrap_or(PLAYER_HEALTH_BAR_OFFSET_Y);

        let mut root = None;
        let mut fill = None;

        commands.entity(entity).with_children(|parent| {
            let bar_root = parent
                .spawn((
                    Transform::from_xyz(0.0, offset_y, 0.0),
                    Visibility::Inherited,
                ))
                .with_children(|bar| {
                    bar.spawn((
                        Mesh3d(assets.unit_cube_mesh.clone()),
                        MeshMaterial3d(assets.health_bar_bg_material.clone()),
                        Transform::from_scale(Vec3::new(
                            HEALTH_BAR_WIDTH,
                            HEALTH_BAR_HEIGHT,
                            HEALTH_BAR_DEPTH,
                        )),
                        Visibility::Inherited,
                    ));

                    fill = Some(
                        bar.spawn((
                            Mesh3d(assets.unit_cube_mesh.clone()),
                            MeshMaterial3d(assets.health_bar_fg_material.clone()),
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

            root = Some(bar_root);
        });

        if let (Some(root), Some(fill)) = (root, fill) {
            commands.entity(entity).insert(HealthBar { root, fill });
        }
    }
}

pub(crate) fn update_die_motions(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Transform, &DieMotion)>,
) {
    fn safe_unit_quat(q: Quat) -> Quat {
        if !q.is_finite() {
            return Quat::IDENTITY;
        }
        let n = q.normalize();
        if n.is_finite() {
            n
        } else {
            Quat::IDENTITY
        }
    }

    let wall_time = time.elapsed_secs();
    for (entity, mut transform, motion) in &mut q {
        let duration = motion.duration_secs.max(0.001);
        let t = ((wall_time - motion.started_at_secs) / duration).clamp(0.0, 1.0);
        let t_smooth = t * t * (3.0 - 2.0 * t);

        transform.translation = motion
            .start
            .translation
            .lerp(motion.end.translation, t_smooth);
        transform.scale = motion.start.scale.lerp(motion.end.scale, t_smooth);
        transform.rotation = safe_unit_quat(motion.start.rotation)
            .slerp(safe_unit_quat(motion.end.rotation), t_smooth);

        if t >= 1.0 {
            *transform = motion.end.clone();
            commands.entity(entity).remove::<DieMotion>();
        }
    }
}

pub(crate) fn recover_health_on_enter_build_mode(
    mut commands: Commands,
    mut game: ResMut<Game>,
    mut units: Query<(
        Entity,
        &mut Transform,
        &mut Health,
        Option<&mut LaserDamageAccum>,
        Option<&Died>,
    )>,
) {
    game.game_over = false;

    for (entity, mut transform, mut health, accum, died) in &mut units {
        health.max = health.max.max(1);
        health.current = health.max;
        if let Some(mut accum) = accum {
            accum.0 = 0.0;
        }

        if let Some(died) = died {
            *transform = died.restore_transform.clone();
            commands.entity(entity).remove::<Died>();
            commands.entity(entity).remove::<DieMotion>();
        }
    }
}
