use bevy::prelude::*;

use crate::object::registry::ObjectLibrary;
use crate::types::{
    ActionClock, AnimationChannelsActive, AttackClock, Commandable, LocomotionClock, MoveOrder,
    ObjectPrefabId,
};

pub(crate) fn ensure_animation_channels_active(
    mut commands: Commands,
    q: Query<Entity, (With<ObjectPrefabId>, Without<AnimationChannelsActive>)>,
) {
    for entity in &q {
        commands
            .entity(entity)
            // Use `try_insert` to avoid panicking if another system despawns this entity
            // earlier in the same schedule tick (common for short-lived projectiles).
            .try_insert(AnimationChannelsActive::default());
    }
}

pub(crate) fn update_animation_channels_active(
    time: Res<Time>,
    mut q: Query<(Entity, Option<&MoveOrder>, &mut AnimationChannelsActive), With<Commandable>>,
    locomotion: Query<&LocomotionClock>,
    attacks: Query<&AttackClock>,
    actions: Query<&ActionClock>,
) {
    let wall_time = time.elapsed_secs();
    for (entity, order, mut channels) in &mut q {
        let speed_mps = locomotion
            .get(entity)
            .ok()
            .map(|c| c.speed_mps)
            .unwrap_or(0.0);
        let order_active = order.and_then(|o| o.target).is_some();
        channels.moving = speed_mps > 0.05 || order_active;
        channels.attacking_primary = attacks
            .get(entity)
            .map(|clock| {
                clock.duration_secs > 0.0
                    && (wall_time - clock.started_at_secs).max(0.0) <= clock.duration_secs
            })
            .unwrap_or(false);
        channels.acting = actions
            .get(entity)
            .map(|clock| {
                clock.duration_secs > 0.0
                    && (wall_time - clock.started_at_secs).max(0.0) <= clock.duration_secs
            })
            .unwrap_or(false);
    }
}

pub(crate) fn ensure_locomotion_clocks(
    mut commands: Commands,
    library: Res<ObjectLibrary>,
    q: Query<(Entity, &Transform, &ObjectPrefabId), Without<LocomotionClock>>,
) {
    for (entity, transform, prefab_id) in &q {
        if library.mobility(prefab_id.0).is_none() {
            continue;
        }

        // `try_insert` avoids panicking if the entity was despawned earlier in the tick.
        commands.entity(entity).try_insert(LocomotionClock {
            t: 0.0,
            distance_m: 0.0,
            signed_distance_m: 0.0,
            speed_mps: 0.0,
            last_translation: transform.translation,
        });
    }
}

pub(crate) fn update_locomotion_clocks(
    time: Res<Time>,
    library: Res<ObjectLibrary>,
    mut q: Query<(&Transform, &ObjectPrefabId, &mut LocomotionClock)>,
) {
    let dt = time.delta_secs();
    if dt <= 0.0 {
        return;
    }

    for (transform, prefab_id, mut clock) in &mut q {
        clock.speed_mps = 0.0;
        if library.mobility(prefab_id.0).is_none() {
            clock.last_translation = transform.translation;
            continue;
        }

        let delta = transform.translation - clock.last_translation;
        clock.last_translation = transform.translation;

        let delta_xz = Vec2::new(delta.x, delta.z);
        let dist = delta_xz.length();
        if !dist.is_finite() || dist <= 1e-6 {
            continue;
        }

        clock.distance_m += dist;
        if !clock.distance_m.is_finite() {
            clock.distance_m = 0.0;
        }

        let mut forward = transform.rotation * Vec3::Z;
        forward.y = 0.0;
        let forward_xz = Vec2::new(forward.x, forward.z);
        let signed_step = if forward_xz.length_squared() > 1e-6 {
            delta_xz.dot(forward_xz.normalize())
        } else {
            dist
        };
        if signed_step.is_finite() {
            clock.signed_distance_m += signed_step;
        }
        if !clock.signed_distance_m.is_finite() {
            clock.signed_distance_m = 0.0;
        }

        let speed = dist / dt;
        if speed.is_finite() {
            clock.speed_mps = speed.max(0.0);
        }

        // `t` is a generic movement "phase" driver. Advancing it in meters avoids coupling gait
        // animation speed to `mobility.max_speed` and yields a stable cycles-per-meter mapping.
        clock.t += dist;
        if !clock.t.is_finite() {
            clock.t = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        ColliderProfile, MobilityDef, MobilityMode, ObjectDef, ObjectInteraction,
    };
    use std::time::Duration;

    #[test]
    fn locomotion_phase_advances_in_meters() {
        let prefab_id = 0xD15E_A5E_u128;

        let mut library = ObjectLibrary::default();
        library.upsert(ObjectDef {
            object_id: prefab_id,
            label: "test_prefab".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: Some(MobilityDef {
                mode: MobilityMode::Ground,
                max_speed: 10.0,
            }),
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });

        let mut app = App::new();
        app.insert_resource(Time::<()>::default());
        app.insert_resource(library);
        app.add_systems(Update, update_locomotion_clocks);

        let entity = app
            .world_mut()
            .spawn((
                Transform::from_translation(Vec3::ZERO),
                ObjectPrefabId(prefab_id),
                LocomotionClock {
                    t: 0.0,
                    distance_m: 0.0,
                    signed_distance_m: 0.0,
                    speed_mps: 0.0,
                    last_translation: Vec3::ZERO,
                },
            ))
            .id();

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(Duration::from_secs_f32(0.5));
        app.world_mut()
            .entity_mut(entity)
            .get_mut::<Transform>()
            .unwrap()
            .translation
            .z = 1.0;
        app.update();

        let clock = app.world().get::<LocomotionClock>(entity).unwrap();
        assert!((clock.distance_m - 1.0).abs() < 1e-5);
        assert!((clock.t - 1.0).abs() < 1e-5);
        assert!((clock.signed_distance_m - 1.0).abs() < 1e-5);
        assert!((clock.speed_mps - 2.0).abs() < 1e-5);
    }
}
