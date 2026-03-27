use bevy::prelude::*;

use crate::object::registry::PartAnimationSlot;

const LEGACY_INTERNAL_BASE_CHANNEL: &str = "__base";

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

pub(super) fn normalize_attachment_motion(
    fallback_basis: &mut Transform,
    animations: &mut Vec<PartAnimationSlot>,
) {
    let mut legacy_base_basis: Option<Transform> = None;
    animations.retain(|slot| {
        let channel = slot.channel.as_ref().trim();
        if channel == LEGACY_INTERNAL_BASE_CHANNEL {
            if legacy_base_basis.is_none() {
                legacy_base_basis = Some(sanitize_basis(slot.spec.basis));
            }
            false
        } else {
            true
        }
    });

    let has_non_empty_channel = animations
        .iter()
        .any(|slot| !slot.channel.as_ref().trim().is_empty());
    if !has_non_empty_channel {
        *fallback_basis = Transform::IDENTITY;
        return;
    }

    let sanitized = sanitize_basis(*fallback_basis);
    if transforms_equal(&sanitized, &Transform::IDENTITY) {
        if let Some(legacy) = legacy_base_basis {
            *fallback_basis = legacy;
        } else {
            *fallback_basis = sanitized;
        }
    } else {
        *fallback_basis = sanitized;
    }
}

pub(super) fn rebase_bases_for_offset_change(
    old_offset: Transform,
    new_offset: Transform,
    fallback_basis: &mut Transform,
    animations: &mut [PartAnimationSlot],
) {
    if transforms_equal(&old_offset, &new_offset) {
        return;
    }
    let has_non_empty_channel = animations
        .iter()
        .any(|slot| !slot.channel.as_ref().trim().is_empty());
    if !has_non_empty_channel {
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

    fn rebase_basis(delta_mat: Mat4, basis: Transform) -> Transform {
        let basis_old = sanitize_basis(basis);
        let basis_mat = basis_old.to_matrix();
        if !basis_mat.is_finite() {
            return Transform::IDENTITY;
        }

        let mat = delta_mat * basis_mat;
        let Some(mut rebased) = crate::geometry::mat4_to_transform_allow_degenerate_scale(mat)
        else {
            return basis_old;
        };

        if !rebased.translation.is_finite()
            || !rebased.rotation.is_finite()
            || !rebased.scale.is_finite()
        {
            rebased = Transform::IDENTITY;
        }
        rebased
    }

    *fallback_basis = rebase_basis(delta_mat, *fallback_basis);

    for slot in animations.iter_mut() {
        slot.spec.basis = rebase_basis(delta_mat, slot.spec.basis);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSpec,
    };

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

    fn base_slot_with_basis(basis: Transform) -> PartAnimationSlot {
        PartAnimationSlot {
            channel: LEGACY_INTERNAL_BASE_CHANNEL.into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::Always,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    }],
                },
            },
        }
    }

    #[test]
    fn normalize_attachment_motion_migrates_legacy_base_slot() {
        let mut fallback_basis = Transform::IDENTITY;
        let mut animations = vec![
            PartAnimationSlot {
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
            },
            base_slot_with_basis(Transform::from_translation(Vec3::new(0.0, 0.1, 0.0))),
        ];

        normalize_attachment_motion(&mut fallback_basis, &mut animations);
        assert!(
            (fallback_basis.translation - Vec3::new(0.0, 0.1, 0.0)).length_squared() < 1e-6,
            "expected legacy __base slot to seed fallback_basis"
        );
        assert!(
            !animations
                .iter()
                .any(|slot| slot.channel.as_ref().trim() == LEGACY_INTERNAL_BASE_CHANNEL),
            "expected legacy __base slot to be removed"
        );
    }

    #[test]
    fn normalize_attachment_motion_resets_without_channels() {
        let mut fallback_basis = Transform::from_translation(Vec3::new(0.0, 0.2, 0.0));
        let mut animations = vec![base_slot_with_basis(Transform::from_translation(
            Vec3::new(0.0, 0.1, 0.0),
        ))];

        normalize_attachment_motion(&mut fallback_basis, &mut animations);
        assert!(
            transforms_equal(&fallback_basis, &Transform::IDENTITY),
            "expected fallback_basis to reset when there are no non-empty channels"
        );
        assert!(
            animations.is_empty(),
            "expected legacy base slot to be removed"
        );
    }

    #[test]
    fn rebase_preserves_composed_offset_basis() {
        let old_offset = Transform::from_translation(Vec3::new(0.0, 0.0, 0.1))
            .with_rotation(Quat::from_rotation_y(0.3));
        let new_offset = Transform::from_translation(Vec3::new(0.0, 0.0, 0.1))
            .with_rotation(Quat::from_rotation_y(-0.8));

        let basis_old = Transform::from_translation(Vec3::new(0.0, 0.05, 0.0))
            .with_rotation(Quat::from_rotation_x(0.2));
        let fallback_old = Transform::from_translation(Vec3::new(0.02, 0.0, 0.0))
            .with_rotation(Quat::from_rotation_z(0.1));

        let mut fallback_basis = fallback_old;
        let mut animations = vec![PartAnimationSlot {
            channel: "idle".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::Always,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                basis: basis_old,
                clip: PartAnimationDef::Loop {
                    duration_secs: 1.0,
                    keyframes: vec![PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    }],
                },
            },
        }];

        rebase_bases_for_offset_change(
            old_offset,
            new_offset,
            &mut fallback_basis,
            &mut animations,
        );

        let composed_expected = old_offset.to_matrix() * basis_old.to_matrix();
        let composed_got = new_offset.to_matrix() * animations[0].spec.basis.to_matrix();
        let expected =
            crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_expected).unwrap();
        let got = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_got).unwrap();

        assert!(
            approx_eq_transform(got, expected, 1e-4),
            "expected rebased slot basis to preserve composed transform: got={:?} expected={:?}",
            got,
            expected
        );

        let composed_expected = old_offset.to_matrix() * fallback_old.to_matrix();
        let composed_got = new_offset.to_matrix() * fallback_basis.to_matrix();
        let expected =
            crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_expected).unwrap();
        let got = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed_got).unwrap();

        assert!(
            approx_eq_transform(got, expected, 1e-4),
            "expected rebased fallback_basis to preserve composed transform: got={:?} expected={:?}",
            got,
            expected
        );
    }
}
