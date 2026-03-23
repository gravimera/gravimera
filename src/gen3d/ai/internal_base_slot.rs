use bevy::prelude::*;

use crate::object::registry::{
    PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
    PartAnimationSpec, PART_ANIMATION_INTERNAL_BASE_CHANNEL,
};

fn sanitize_basis(basis: Transform) -> Transform {
    if !basis.translation.is_finite() || !basis.rotation.is_finite() || !basis.scale.is_finite() {
        Transform::IDENTITY
    } else {
        basis
    }
}

fn transforms_equal(a: &Transform, b: &Transform) -> bool {
    a.translation == b.translation && a.rotation == b.rotation && a.scale == b.scale
}

fn internal_base_clip() -> PartAnimationDef {
    PartAnimationDef::Loop {
        duration_secs: 1.0,
        keyframes: vec![PartAnimationKeyframeDef {
            time_secs: 0.0,
            delta: Transform::IDENTITY,
        }],
    }
}

fn internal_base_slot() -> PartAnimationSlot {
    PartAnimationSlot {
        channel: PART_ANIMATION_INTERNAL_BASE_CHANNEL.into(),
        spec: PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            basis: Transform::IDENTITY,
            clip: internal_base_clip(),
        },
    }
}

fn canonicalize_internal_base_slot(slot: &mut PartAnimationSlot) {
    slot.channel = PART_ANIMATION_INTERNAL_BASE_CHANNEL.into();
    let basis = sanitize_basis(slot.spec.basis);
    slot.spec.driver = PartAnimationDriver::Always;
    slot.spec.speed_scale = 1.0;
    slot.spec.time_offset_units = 0.0;
    slot.spec.basis = basis;
    slot.spec.clip = internal_base_clip();
}

pub(super) fn normalize_internal_base_slot(animations: &mut Vec<PartAnimationSlot>) {
    let has_non_base_slot = animations.iter().any(|slot| {
        let channel = slot.channel.as_ref().trim();
        !channel.is_empty() && channel != PART_ANIMATION_INTERNAL_BASE_CHANNEL
    });

    if !has_non_base_slot {
        animations.retain(|slot| {
            slot.channel.as_ref().trim() != PART_ANIMATION_INTERNAL_BASE_CHANNEL
        });
        return;
    }

    let first_base_idx = animations
        .iter()
        .position(|slot| slot.channel.as_ref().trim() == PART_ANIMATION_INTERNAL_BASE_CHANNEL);
    match first_base_idx {
        Some(idx) => {
            canonicalize_internal_base_slot(&mut animations[idx]);
            for i in (0..animations.len()).rev() {
                if i != idx
                    && animations[i].channel.as_ref().trim() == PART_ANIMATION_INTERNAL_BASE_CHANNEL
                {
                    animations.remove(i);
                }
            }
        }
        None => {
            animations.push(internal_base_slot());
        }
    }
}

pub(super) fn rebase_slot_bases_for_offset_change(
    old_offset: Transform,
    new_offset: Transform,
    animations: &mut [PartAnimationSlot],
) {
    if animations.is_empty() || transforms_equal(&old_offset, &new_offset) {
        return;
    }

    let old_mat = old_offset.to_matrix();
    let inv_new = new_offset.to_matrix().inverse();
    if !old_mat.is_finite() || !inv_new.is_finite() {
        return;
    }

    let delta_mat = inv_new * old_mat;
    if !delta_mat.is_finite() {
        return;
    }

    for slot in animations.iter_mut() {
        let basis_old = sanitize_basis(slot.spec.basis);
        let basis_mat = basis_old.to_matrix();
        if !basis_mat.is_finite() {
            slot.spec.basis = Transform::IDENTITY;
            continue;
        }

        let mat = delta_mat * basis_mat;
        let Some(mut rebased) = crate::geometry::mat4_to_transform_allow_degenerate_scale(mat)
        else {
            slot.spec.basis = basis_old;
            continue;
        };

        if !rebased.translation.is_finite()
            || !rebased.rotation.is_finite()
            || !rebased.scale.is_finite()
        {
            rebased = Transform::IDENTITY;
        }
        slot.spec.basis = rebased;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx_eq_f32(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    fn approx_eq_vec3(a: Vec3, b: Vec3, eps: f32) -> bool {
        approx_eq_f32(a.x, b.x, eps) && approx_eq_f32(a.y, b.y, eps) && approx_eq_f32(a.z, b.z, eps)
    }

    fn approx_eq_quat(a: Quat, b: Quat, eps: f32) -> bool {
        // Quats are equivalent up to sign.
        let a = a.normalize();
        let b = b.normalize();
        let d1 = (a.x - b.x).abs() + (a.y - b.y).abs() + (a.z - b.z).abs() + (a.w - b.w).abs();
        let d2 = (a.x + b.x).abs() + (a.y + b.y).abs() + (a.z + b.z).abs() + (a.w + b.w).abs();
        d1.min(d2) <= eps * 4.0
    }

    fn approx_eq_transform(a: Transform, b: Transform, eps: f32) -> bool {
        approx_eq_vec3(a.translation, b.translation, eps)
            && approx_eq_vec3(a.scale, b.scale, eps)
            && approx_eq_quat(a.rotation, b.rotation, eps)
    }

    #[test]
    fn normalize_internal_base_slot_adds_and_removes_base() {
        let mut animations = vec![PartAnimationSlot {
            channel: "move".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MovePhase,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: Transform::IDENTITY,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    }],
                },
            },
        }];

        normalize_internal_base_slot(&mut animations);
        assert!(
            animations
                .iter()
                .any(|s| s.channel.as_ref() == PART_ANIMATION_INTERNAL_BASE_CHANNEL),
            "expected normalize to add an internal base slot"
        );

        let mut base_only = vec![internal_base_slot()];
        normalize_internal_base_slot(&mut base_only);
        assert!(
            base_only.is_empty(),
            "expected normalize to remove __base when it is the only slot"
        );
    }

    #[test]
    fn rebase_preserves_composed_offset_basis() {
        let old_offset =
            Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)).with_rotation(Quat::from_rotation_y(0.3));
        let new_offset =
            Transform::from_translation(Vec3::new(0.0, 0.0, 0.1)).with_rotation(Quat::from_rotation_y(-0.8));

        let basis_old =
            Transform::from_translation(Vec3::new(0.0, 0.05, 0.0)).with_rotation(Quat::from_rotation_x(0.2));

        let mut animations = vec![PartAnimationSlot {
            channel: "idle".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::Always,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: basis_old,
                clip: internal_base_clip(),
            },
        }];

        rebase_slot_bases_for_offset_change(old_offset, new_offset, &mut animations);

        let composed_expected = old_offset.to_matrix() * basis_old.to_matrix();
        let composed_got = new_offset.to_matrix() * animations[0].spec.basis.to_matrix();
        let expected =
            crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_expected).unwrap();
        let got = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_got).unwrap();

        assert!(
            approx_eq_transform(got, expected, 1e-4),
            "expected rebased basis to preserve composed transform: got={:?} expected={:?}",
            got,
            expected
        );
    }
}
