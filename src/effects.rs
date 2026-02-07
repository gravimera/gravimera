use bevy::prelude::*;
use rand::prelude::*;

use crate::assets::SceneAssets;
use crate::constants::*;
use crate::object::types::effects as effect_types;
use crate::types::*;

pub(crate) fn clear_killed_enemies(
    mut killed: ResMut<KilledEnemiesThisFrame>,
    mut effects: ResMut<EnemyKillEffects>,
) {
    killed.0.clear();
    effects.0.clear();
}

pub(crate) fn spawn_explosions(
    mut commands: Commands,
    assets: Res<SceneAssets>,
    effects: Res<EnemyKillEffects>,
) {
    if effects.0.is_empty() {
        return;
    }

    let mut rng = thread_rng();
    for origin in effects.0.iter().copied() {
        for _ in 0..EXPLOSION_PARTICLE_COUNT {
            let mut dir = Vec3::new(
                rng.gen_range(-1.0..1.0),
                rng.gen_range(0.15..1.25),
                rng.gen_range(-1.0..1.0),
            );
            if dir.length_squared() < 1e-4 {
                dir = Vec3::Y;
            } else {
                dir = dir.normalize();
            }

            let speed = rng.gen_range(EXPLOSION_SPEED * 0.55..EXPLOSION_SPEED * 1.15);
            let velocity = dir * speed;

            let scale = rng.gen_range(0.10..0.22);
            let initial_scale = Vec3::splat(scale);
            let jitter = Vec3::new(
                rng.gen_range(-0.25..0.25),
                rng.gen_range(0.05..0.55),
                rng.gen_range(-0.25..0.25),
            );

            commands.spawn((
                ObjectId::new_v4(),
                ObjectPrefabId(effect_types::explosion_particle::object_id()),
                Mesh3d(assets.unit_cube_mesh.clone()),
                MeshMaterial3d(assets.explosion_material.clone()),
                Transform::from_translation(origin + jitter).with_scale(initial_scale),
                Visibility::Inherited,
                ExplosionParticle {
                    velocity,
                    ttl_secs: EXPLOSION_TTL_SECS,
                    total_secs: EXPLOSION_TTL_SECS,
                    initial_scale,
                },
            ));
        }
    }
}

pub(crate) fn spawn_blood_particles(commands: &mut Commands, assets: &SceneAssets, origin: Vec3) {
    let mut rng = thread_rng();
    for _ in 0..BLOOD_PARTICLE_COUNT {
        let mut dir = Vec3::new(
            rng.gen_range(-1.0..1.0),
            rng.gen_range(0.05..1.1),
            rng.gen_range(-1.0..1.0),
        );
        if dir.length_squared() < 1e-4 {
            dir = Vec3::Y;
        } else {
            dir = dir.normalize();
        }

        let speed = rng.gen_range(BLOOD_SPEED * 0.55..BLOOD_SPEED * 1.15);
        let velocity = dir * speed;

        let scale = rng.gen_range(0.08..0.16);
        let initial_scale = Vec3::splat(scale);
        let jitter = Vec3::new(
            rng.gen_range(-0.18..0.18),
            rng.gen_range(-0.05..0.25),
            rng.gen_range(-0.18..0.18),
        );

        commands.spawn((
            ObjectId::new_v4(),
            ObjectPrefabId(effect_types::blood_particle::object_id()),
            Mesh3d(assets.unit_cube_mesh.clone()),
            MeshMaterial3d(assets.blood_material.clone()),
            Transform::from_translation(origin + jitter).with_scale(initial_scale),
            Visibility::Inherited,
            ExplosionParticle {
                velocity,
                ttl_secs: BLOOD_TTL_SECS,
                total_secs: BLOOD_TTL_SECS,
                initial_scale,
            },
        ));
    }
}

pub(crate) fn spawn_energy_impact_particles(
    commands: &mut Commands,
    assets: &SceneAssets,
    origin: Vec3,
) {
    let mut rng = thread_rng();
    for _ in 0..GUNDAM_ENERGY_IMPACT_PARTICLE_COUNT {
        let mut dir = Vec3::new(
            rng.gen_range(-1.0..1.0),
            rng.gen_range(-0.25..1.15),
            rng.gen_range(-1.0..1.0),
        );
        if dir.length_squared() < 1e-4 {
            dir = Vec3::Y;
        } else {
            dir = dir.normalize();
        }

        let speed = rng.gen_range(GUNDAM_ENERGY_IMPACT_SPEED * 0.65..GUNDAM_ENERGY_IMPACT_SPEED);
        let velocity = dir * speed;

        let scale = rng.gen_range(0.05..0.12);
        let initial_scale = Vec3::splat(scale);
        let jitter = Vec3::new(
            rng.gen_range(-0.10..0.10),
            rng.gen_range(-0.06..0.10),
            rng.gen_range(-0.10..0.10),
        );

        commands.spawn((
            ObjectId::new_v4(),
            ObjectPrefabId(effect_types::energy_impact_particle::object_id()),
            Mesh3d(assets.unit_cube_mesh.clone()),
            MeshMaterial3d(assets.gundam_energy_impact_material.clone()),
            Transform::from_translation(origin + jitter).with_scale(initial_scale),
            Visibility::Inherited,
            ExplosionParticle {
                velocity,
                ttl_secs: GUNDAM_ENERGY_IMPACT_TTL_SECS,
                total_secs: GUNDAM_ENERGY_IMPACT_TTL_SECS,
                initial_scale,
            },
        ));
    }
}

pub(crate) fn animate_energy_ball_visuals(
    time: Res<Time>,
    mut balls: Query<(&GundamEnergyBallVisual, &mut Transform), Without<GundamEnergyArcVisual>>,
    mut arcs: Query<(&GundamEnergyArcVisual, &mut Transform), Without<GundamEnergyBallVisual>>,
) {
    let t = time.elapsed_secs();

    for (visual, mut transform) in &mut balls {
        let phase = t * std::f32::consts::TAU * GUNDAM_ENERGY_BALL_PULSE_HZ + visual.phase;
        let pulse = 1.0 + 0.12 * phase.sin();
        transform.scale = Vec3::splat(pulse.max(0.01));
    }

    for (arc, mut transform) in &mut arcs {
        let axis = arc.axis.normalize_or_zero();
        if axis.length_squared() < 1e-4 {
            continue;
        }

        let reference = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        let u = axis.cross(reference).normalize_or_zero();
        let v = axis.cross(u).normalize_or_zero();

        let orbit_angle = t * 4.2 + arc.phase;
        let base = u * orbit_angle.cos() + v * orbit_angle.sin();

        let mut tangent = -u * orbit_angle.sin() + v * orbit_angle.cos();
        if tangent.length_squared() < 1e-4 {
            tangent = Vec3::Z;
        } else {
            tangent = tangent.normalize();
        }

        let jitter = (t * 7.3 + arc.phase * 1.7).sin() * GUNDAM_ENERGY_ARC_JITTER_RADIUS;
        let wobble = (t * 9.1 + arc.phase * 2.2).cos() * (GUNDAM_ENERGY_ARC_JITTER_RADIUS * 0.35);
        let radius = (GUNDAM_ENERGY_ARC_RADIUS + jitter).max(0.0);

        transform.translation = base * radius + axis * wobble;
        transform.rotation = Quat::from_rotation_arc(Vec3::Z, tangent);

        let flicker_phase = t * std::f32::consts::TAU * GUNDAM_ENERGY_ARC_FLICKER_HZ + arc.phase;
        let flicker = (flicker_phase.sin() * 0.5 + 0.5).powf(2.2);

        let thickness = 0.06;
        let length = 0.48 + 0.22 * (t * 6.1 + arc.phase).sin().abs();
        transform.scale = Vec3::new(thickness, thickness, length) * flicker;
    }
}

pub(crate) fn update_explosion_particles(
    mut commands: Commands,
    time: Res<Time>,
    mut particles: Query<(Entity, &mut Transform, &mut ExplosionParticle)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (entity, mut transform, mut particle) in &mut particles {
        particle.ttl_secs -= dt;
        if particle.ttl_secs <= 0.0 {
            commands.entity(entity).try_despawn();
            continue;
        }

        transform.translation += particle.velocity * dt;
        particle.velocity.y -= EXPLOSION_GRAVITY * dt;

        let life01 = (particle.ttl_secs / particle.total_secs).clamp(0.0, 1.0);
        let scale01 = life01.powf(1.6);
        transform.scale = particle.initial_scale * scale01.max(0.0);
    }
}
