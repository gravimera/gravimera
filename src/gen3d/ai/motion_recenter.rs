use bevy::prelude::*;
use serde::Deserialize;
use std::collections::HashSet;

use crate::object::registry::{
    PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef, PartAnimationSlot,
};

use super::agent_parsing::resolve_component_index_by_name_hint;
use super::artifacts::{
    append_gen3d_jsonl_artifact, write_gen3d_assembly_snapshot, write_gen3d_json_artifact,
};
use super::convert;
use super::schema::AiJointKindJson;
use super::{Gen3dAiJob, Gen3dPlannedAttachment, Gen3dPlannedComponent};

use super::super::state::Gen3dDraft;

const DEFAULT_CYCLE_M: f32 = 1.0;
const SAMPLE_COUNT: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RestBiasTarget {
    Warn,
    Error,
}

impl RestBiasTarget {
    fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "warn" | "warning" => Some(Self::Warn),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct JointRestBiasThresholds {
    warn_min_angle_deg: f32,
    error_min_angle_deg: f32,
    max_span_deg: f32,
}

impl JointRestBiasThresholds {
    fn default() -> Self {
        Self {
            warn_min_angle_deg: 50.0,
            error_min_angle_deg: 75.0,
            max_span_deg: 70.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JointRestBiasSeverity {
    Warn,
    Error,
}

impl JointRestBiasSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    fn rank(self) -> u8 {
        match self {
            Self::Warn => 1,
            Self::Error => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct JointRestBiasStats {
    min_angle_deg: f32,
    min_phase_01: f32,
    max_angle_deg: f32,
    max_phase_01: f32,
    span_deg: f32,
}

impl JointRestBiasStats {
    fn severity(self, thresholds: JointRestBiasThresholds) -> Option<JointRestBiasSeverity> {
        if !self.min_angle_deg.is_finite() || !self.max_angle_deg.is_finite() {
            return None;
        }
        if self.min_angle_deg <= thresholds.warn_min_angle_deg
            || self.span_deg > thresholds.max_span_deg
        {
            return None;
        }
        if self.min_angle_deg >= thresholds.error_min_angle_deg {
            Some(JointRestBiasSeverity::Error)
        } else {
            Some(JointRestBiasSeverity::Warn)
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecenterAttachmentMotionArgsV1 {
    #[serde(default)]
    version: u32,
    child_components: Vec<ComponentRefJsonV1>,
    #[serde(default)]
    channels: Option<Vec<String>>,
    #[serde(default)]
    target: Option<String>,
    #[serde(default)]
    dry_run: bool,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ComponentRefJsonV1 {
    Name(String),
    Index(u64),
}

fn quat_angle_deg(q: Quat) -> f32 {
    let w = q.w.clamp(-1.0, 1.0).abs();
    (2.0 * w.acos()).to_degrees()
}

fn lerp_transform(a: &Transform, b: &Transform, alpha: f32) -> Transform {
    let translation = a.translation.lerp(b.translation, alpha);
    let rotation = a.rotation.slerp(b.rotation, alpha).normalize();
    let scale = a.scale.lerp(b.scale, alpha);
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn mul_transform(a: &Transform, b: &Transform) -> Transform {
    let composed = a.to_matrix() * b.to_matrix();
    crate::geometry::mat4_to_transform_allow_degenerate_scale(composed).unwrap_or(*b)
}

fn sample_keyframes_loop(
    duration_secs: f32,
    keyframes: &[PartAnimationKeyframeDef],
    time_secs: f32,
) -> Transform {
    let duration = duration_secs.max(1e-6);
    let mut t = if time_secs.is_finite() {
        time_secs
    } else {
        0.0
    };
    t = t.rem_euclid(duration);

    if keyframes.is_empty() {
        return Transform::IDENTITY;
    }
    if keyframes.len() == 1 {
        return keyframes[0].delta;
    }

    let mut prev = &keyframes[0];
    for next in &keyframes[1..] {
        if t < next.time_secs {
            let dt = (next.time_secs - prev.time_secs).max(1e-6);
            let alpha = ((t - prev.time_secs) / dt).clamp(0.0, 1.0);
            return lerp_transform(&prev.delta, &next.delta, alpha);
        }
        prev = next;
    }

    // Wrap around (last -> first).
    let first = &keyframes[0];
    let last = prev;
    let t0 = last.time_secs;
    let t1 = duration + first.time_secs;
    let dt = (t1 - t0).max(1e-6);
    let alpha = ((t - t0) / dt).clamp(0.0, 1.0);
    lerp_transform(&last.delta, &first.delta, alpha)
}

fn sample_keyframes_clamped(
    duration_secs: f32,
    keyframes: &[PartAnimationKeyframeDef],
    time_secs: f32,
) -> Transform {
    let duration = duration_secs.max(1e-6);
    let mut t = if time_secs.is_finite() {
        time_secs
    } else {
        0.0
    };
    t = t.clamp(0.0, duration);

    if keyframes.is_empty() {
        return Transform::IDENTITY;
    }
    if keyframes.len() == 1 {
        return keyframes[0].delta;
    }

    if t <= keyframes[0].time_secs {
        return keyframes[0].delta;
    }

    let mut prev = &keyframes[0];
    for next in &keyframes[1..] {
        if t < next.time_secs {
            let dt = (next.time_secs - prev.time_secs).max(1e-6);
            let alpha = ((t - prev.time_secs) / dt).clamp(0.0, 1.0);
            return lerp_transform(&prev.delta, &next.delta, alpha);
        }
        prev = next;
    }

    prev.delta
}

fn sample_part_animation(animation: &PartAnimationDef, time_secs: f32) -> Transform {
    match animation {
        PartAnimationDef::Loop {
            duration_secs,
            keyframes,
        } => sample_keyframes_loop(*duration_secs, keyframes, time_secs),
        PartAnimationDef::Once {
            duration_secs,
            keyframes,
        } => sample_keyframes_clamped(*duration_secs, keyframes, time_secs),
        PartAnimationDef::PingPong {
            duration_secs,
            keyframes,
        } => {
            let duration = (*duration_secs).max(1e-6);
            let mut t = if time_secs.is_finite() {
                time_secs
            } else {
                0.0
            };
            let period = duration * 2.0;
            t = t.rem_euclid(period);
            if t > duration {
                t = period - t;
            }
            sample_keyframes_clamped(duration, keyframes, t)
        }
        PartAnimationDef::Spin {
            axis,
            radians_per_unit,
            ..
        } => {
            let axis = if axis.length_squared() > 1e-6 {
                axis.normalize()
            } else {
                Vec3::Y
            };
            let angle = if time_secs.is_finite() && radians_per_unit.is_finite() {
                time_secs * *radians_per_unit
            } else {
                0.0
            };
            Transform {
                translation: Vec3::ZERO,
                rotation: Quat::from_axis_angle(axis, angle),
                scale: Vec3::ONE,
            }
        }
    }
}

fn sample_animation_slot_delta(
    slot: &PartAnimationSlot,
    sample_t_m: f32,
    sample_phase_01: f32,
) -> Transform {
    let driver_t = match slot.spec.driver {
        PartAnimationDriver::Always => match &slot.spec.clip {
            PartAnimationDef::Loop { duration_secs, .. }
            | PartAnimationDef::Once { duration_secs, .. }
                if duration_secs.is_finite() && *duration_secs > 0.0 =>
            {
                sample_phase_01 * *duration_secs
            }
            PartAnimationDef::PingPong { duration_secs, .. }
                if duration_secs.is_finite() && *duration_secs > 0.0 =>
            {
                sample_phase_01 * (*duration_secs * 2.0)
            }
            _ => sample_t_m,
        },
        PartAnimationDriver::MovePhase => sample_phase_01,
        PartAnimationDriver::MoveDistance => sample_t_m,
        PartAnimationDriver::AttackTime => sample_phase_01,
    };

    let mut t = if driver_t.is_finite() { driver_t } else { 0.0 };
    t *= slot.spec.speed_scale.max(0.0);
    if slot.spec.time_offset_units.is_finite() {
        t += slot.spec.time_offset_units;
    }
    sample_part_animation(&slot.spec.clip, t)
}

fn infer_cycle_m(rig_move_cycle_m: Option<f32>, components: &[Gen3dPlannedComponent]) -> f32 {
    if let Some(v) = rig_move_cycle_m
        .filter(|v| v.is_finite())
        .map(|v| v.abs())
        .filter(|v| *v > 1e-3)
    {
        return v;
    }

    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(slot) = att.animations.iter().find(|s| s.channel.as_ref() == "move") else {
            continue;
        };
        if !matches!(
            slot.spec.driver,
            PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
        ) {
            continue;
        }
        let (duration_secs, repeats) = match &slot.spec.clip {
            PartAnimationDef::Loop { duration_secs, .. }
            | PartAnimationDef::Once { duration_secs, .. } => (*duration_secs, 1.0),
            PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0),
            PartAnimationDef::Spin { .. } => continue,
        };
        if !duration_secs.is_finite() || duration_secs <= 0.0 {
            continue;
        }
        let speed_scale = slot.spec.speed_scale.max(1e-6);
        let effective = (repeats * duration_secs / speed_scale).abs();
        if effective.is_finite() && effective > 1e-3 {
            return effective;
        }
    }

    DEFAULT_CYCLE_M
}

fn hinge_signed_angle_and_off_axis_deg(q_delta: Quat, axis_join: Vec3) -> (f32, f32) {
    let axis = axis_join.normalize();
    let q = if q_delta.is_finite() {
        q_delta.normalize()
    } else {
        Quat::IDENTITY
    };

    // Twist around the hinge axis.
    let v = Vec3::new(q.x, q.y, q.z);
    let v_proj = axis * v.dot(axis);
    let mut twist = Quat::from_xyzw(v_proj.x, v_proj.y, v_proj.z, q.w);
    if twist.length_squared() > 1e-8 {
        twist = twist.normalize();
    } else {
        twist = Quat::IDENTITY;
    }
    // Shortest representation.
    if twist.w < 0.0 {
        twist = -twist;
    }
    let twist_vec = Vec3::new(twist.x, twist.y, twist.z);
    let sin_half = twist_vec.length();
    let angle = 2.0 * sin_half.atan2(twist.w);
    let sign = if twist_vec.dot(axis) >= 0.0 {
        1.0
    } else {
        -1.0
    };
    let signed_deg = (angle * sign).to_degrees();

    // Swing is the remaining rotation not explained by the hinge twist.
    let swing = (q * twist.inverse()).normalize();
    let swing_w = swing.w.clamp(-1.0, 1.0).abs();
    let swing_angle = 2.0 * swing_w.acos();
    let off_axis_deg = swing_angle.to_degrees().abs();

    (signed_deg, off_axis_deg)
}

fn joint_rest_bias_stats(
    base_offset: Transform,
    slot: &PartAnimationSlot,
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
) -> JointRestBiasStats {
    let mut max_angle_deg: f32 = 0.0;
    let mut max_angle_phase: f32 = 0.0;
    let mut min_angle_deg: f32 = f32::INFINITY;
    let mut min_angle_phase: f32 = 0.0;

    for (i, &sample_t_m) in samples_t_m.iter().enumerate() {
        let sample_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
        let delta = sample_animation_slot_delta(slot, sample_t_m, sample_phase_01);
        let animated_offset = mul_transform(&base_offset, &delta);
        let q_delta = (base_offset.rotation.inverse() * animated_offset.rotation).normalize();
        let angle_deg = quat_angle_deg(q_delta).abs();

        if angle_deg.is_finite() && angle_deg > max_angle_deg {
            max_angle_deg = angle_deg;
            max_angle_phase = sample_phase_01;
        }
        if angle_deg.is_finite() && angle_deg < min_angle_deg {
            min_angle_deg = angle_deg;
            min_angle_phase = sample_phase_01;
        }
    }

    let span_deg = if min_angle_deg.is_finite() && max_angle_deg.is_finite() {
        (max_angle_deg - min_angle_deg).max(0.0)
    } else {
        f32::NAN
    };

    JointRestBiasStats {
        min_angle_deg,
        min_phase_01: min_angle_phase,
        max_angle_deg,
        max_phase_01: max_angle_phase,
        span_deg,
    }
}

fn joint_rest_bias_stats_with_bias(
    base_offset: Transform,
    slot: &PartAnimationSlot,
    bias_rotation: Quat,
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
) -> JointRestBiasStats {
    let bias = Transform {
        translation: Vec3::ZERO,
        rotation: if bias_rotation.is_finite() {
            bias_rotation.normalize()
        } else {
            Quat::IDENTITY
        },
        scale: Vec3::ONE,
    };
    let bias_inv = Transform {
        translation: Vec3::ZERO,
        rotation: bias.rotation.inverse(),
        scale: Vec3::ONE,
    };
    let base_offset = mul_transform(&base_offset, &bias);

    let mut max_angle_deg: f32 = 0.0;
    let mut max_angle_phase: f32 = 0.0;
    let mut min_angle_deg: f32 = f32::INFINITY;
    let mut min_angle_phase: f32 = 0.0;

    for (i, &sample_t_m) in samples_t_m.iter().enumerate() {
        let sample_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
        let delta = sample_animation_slot_delta(slot, sample_t_m, sample_phase_01);
        let delta = mul_transform(&bias_inv, &delta);
        let animated_offset = mul_transform(&base_offset, &delta);
        let q_delta = (base_offset.rotation.inverse() * animated_offset.rotation).normalize();
        let angle_deg = quat_angle_deg(q_delta).abs();

        if angle_deg.is_finite() && angle_deg > max_angle_deg {
            max_angle_deg = angle_deg;
            max_angle_phase = sample_phase_01;
        }
        if angle_deg.is_finite() && angle_deg < min_angle_deg {
            min_angle_deg = angle_deg;
            min_angle_phase = sample_phase_01;
        }
    }

    let span_deg = if min_angle_deg.is_finite() && max_angle_deg.is_finite() {
        (max_angle_deg - min_angle_deg).max(0.0)
    } else {
        f32::NAN
    };

    JointRestBiasStats {
        min_angle_deg,
        min_phase_01: min_angle_phase,
        max_angle_deg,
        max_phase_01: max_angle_phase,
        span_deg,
    }
}

fn bias_candidate_rotations_for_attachment(
    attachment: &Gen3dPlannedAttachment,
    joint_kind: Option<AiJointKindJson>,
    axis_join: Option<Vec3>,
    channels_for_bias: &HashSet<String>,
    thresholds: JointRestBiasThresholds,
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
) -> Vec<(Quat, Option<f32>)> {
    let mut candidates: Vec<(Quat, Option<f32>)> = Vec::new();
    let mut hinge_angles_deg: Vec<f32> = Vec::new();
    let mut sample_quats: Vec<Quat> = Vec::new();
    let mut sample_quat_angles: Vec<(f32, Quat)> = Vec::new();

    for slot in attachment.animations.iter() {
        if !channels_for_bias.contains(slot.channel.as_ref()) {
            continue;
        }
        for (i, &sample_t_m) in samples_t_m.iter().enumerate() {
            let sample_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
            let delta = sample_animation_slot_delta(slot, sample_t_m, sample_phase_01);
            let animated_offset = mul_transform(&attachment.offset, &delta);
            let q_delta =
                (attachment.offset.rotation.inverse() * animated_offset.rotation).normalize();
            if !q_delta.is_finite() {
                continue;
            }
            let q_delta = q_delta.normalize();
            if let (Some(AiJointKindJson::Hinge), Some(axis)) = (joint_kind, axis_join) {
                let (hinge_deg, _) = hinge_signed_angle_and_off_axis_deg(q_delta, axis);
                if hinge_deg.is_finite() {
                    hinge_angles_deg.push(hinge_deg);
                }
            } else {
                let q = if q_delta.w < 0.0 { -q_delta } else { q_delta };
                sample_quats.push(q);
                sample_quat_angles.push((quat_angle_deg(q), q));
            }
        }
    }

    if matches!(joint_kind, Some(AiJointKindJson::Hinge)) && axis_join.is_some() {
        fn normalize_bias_deg(deg: f32) -> f32 {
            if !deg.is_finite() {
                return 0.0;
            }
            (deg + 180.0).rem_euclid(360.0) - 180.0
        }

        let axis = axis_join.unwrap();
        let mut keys: HashSet<i32> = HashSet::new();

        let mut add_deg = |deg: f32, candidates: &mut Vec<(Quat, Option<f32>)>| {
            let deg = normalize_bias_deg(deg);
            let key = (deg * 10.0).round() as i32;
            if keys.insert(key) {
                candidates.push((Quat::from_axis_angle(axis, deg.to_radians()), Some(deg)));
            }
        };

        add_deg(0.0, &mut candidates);
        for deg in hinge_angles_deg.iter().copied() {
            add_deg(deg, &mut candidates);
            add_deg(deg - thresholds.warn_min_angle_deg, &mut candidates);
            add_deg(deg + thresholds.warn_min_angle_deg, &mut candidates);
            add_deg(deg - thresholds.error_min_angle_deg, &mut candidates);
            add_deg(deg + thresholds.error_min_angle_deg, &mut candidates);
        }
        return candidates;
    }

    // Non-hinge: use representative samples + quaternion mean.
    if !sample_quat_angles.is_empty() {
        sample_quat_angles
            .sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        let (_min_angle, q_min) = sample_quat_angles[0];
        candidates.push((q_min, None));
        let (_max_angle, q_max) = sample_quat_angles[sample_quat_angles.len() - 1];
        if (q_max.x - q_min.x).abs() > 1e-5
            || (q_max.y - q_min.y).abs() > 1e-5
            || (q_max.z - q_min.z).abs() > 1e-5
            || (q_max.w - q_min.w).abs() > 1e-5
        {
            candidates.push((q_max, None));
        }
    }

    if !sample_quats.is_empty() {
        let mut sum = Vec4::ZERO;
        for q in &sample_quats {
            let q = q.normalize();
            sum += Vec4::new(q.x, q.y, q.z, q.w);
        }
        if sum.length_squared() > 1e-8 {
            let mut mean = Quat::from_xyzw(sum.x, sum.y, sum.z, sum.w).normalize();
            if mean.w < 0.0 {
                mean = -mean;
            }
            candidates.push((mean, None));
        }
    }

    if candidates.is_empty() {
        candidates.push((Quat::IDENTITY, None));
    }

    // Dedup by coarse quaternion quantization.
    let mut seen: HashSet<(i16, i16, i16, i16)> = HashSet::new();
    candidates
        .into_iter()
        .filter(|(q, _)| q.is_finite())
        .filter(|(q, _)| {
            let q = q.normalize();
            let key = (
                (q.x * 1000.0).round() as i16,
                (q.y * 1000.0).round() as i16,
                (q.z * 1000.0).round() as i16,
                (q.w * 1000.0).round() as i16,
            );
            seen.insert(key)
        })
        .collect()
}

fn spin_slot_supported_for_hinge_bias(slot: &PartAnimationSlot, axis_join: Vec3) -> bool {
    let PartAnimationDef::Spin {
        axis,
        radians_per_unit,
        axis_space,
    } = &slot.spec.clip
    else {
        return true;
    };
    if *axis_space != crate::object::registry::PartAnimationSpinAxisSpace::Join {
        return false;
    }
    if !radians_per_unit.is_finite() || radians_per_unit.abs() <= 1e-6 {
        return false;
    }
    let axis = if axis.length_squared() > 1e-6 {
        axis.normalize()
    } else {
        Vec3::Y
    };
    let join = axis_join.normalize();
    axis.dot(join).abs() >= 0.999
}

fn apply_bias_to_attachment_in_place(
    attachment: &mut Gen3dPlannedAttachment,
    bias_rotation: Quat,
    hinge_bias_deg: Option<f32>,
    hinge_axis_join: Option<Vec3>,
) {
    let bias = Transform {
        translation: Vec3::ZERO,
        rotation: bias_rotation.normalize(),
        scale: Vec3::ONE,
    };
    let bias_inv = Transform {
        translation: Vec3::ZERO,
        rotation: bias.rotation.inverse(),
        scale: Vec3::ONE,
    };

    attachment.offset = mul_transform(&attachment.offset, &bias);

    for slot in attachment.animations.iter_mut() {
        match &mut slot.spec.clip {
            PartAnimationDef::Loop { keyframes, .. }
            | PartAnimationDef::Once { keyframes, .. }
            | PartAnimationDef::PingPong { keyframes, .. } => {
                for kf in keyframes.iter_mut() {
                    kf.delta = mul_transform(&bias_inv, &kf.delta);
                    if kf.delta.rotation.is_finite() {
                        kf.delta.rotation = kf.delta.rotation.normalize();
                    } else {
                        kf.delta.rotation = Quat::IDENTITY;
                    }
                }
            }
            PartAnimationDef::Spin {
                axis,
                radians_per_unit,
                axis_space,
            } => {
                let (Some(bias_deg), Some(axis_join)) = (hinge_bias_deg, hinge_axis_join) else {
                    // Unsupported; leave unchanged (caller should have prevented applying).
                    continue;
                };
                if *axis_space != crate::object::registry::PartAnimationSpinAxisSpace::Join {
                    continue;
                }
                let omega = *radians_per_unit;
                if !omega.is_finite() || omega.abs() <= 1e-6 {
                    continue;
                }
                let axis = if axis.length_squared() > 1e-6 {
                    axis.normalize()
                } else {
                    Vec3::Y
                };
                let join = axis_join.normalize();
                let sign = if axis.dot(join) >= 0.0 { 1.0 } else { -1.0 };
                let delta_offset = -(sign * bias_deg.to_radians()) / omega;
                if delta_offset.is_finite() {
                    slot.spec.time_offset_units += delta_offset;
                }
            }
        }
    }

    if let (Some(bias_deg), Some(axis_join)) = (hinge_bias_deg, hinge_axis_join) {
        if let Some(joint) = attachment.joint.as_mut() {
            if joint.kind == AiJointKindJson::Hinge {
                // Ensure our bias is actually a hinge twist. (Axis alignment checked by caller.)
                if axis_join.is_finite() && axis_join.length_squared() > 1e-6 {
                    if let Some([min_deg, max_deg]) = joint.limits_degrees.as_mut() {
                        if min_deg.is_finite() && max_deg.is_finite() && bias_deg.is_finite() {
                            *min_deg -= bias_deg;
                            *max_deg -= bias_deg;
                        }
                    }
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum AttachmentRecenterDecision {
    Noop {
        reason: &'static str,
    },
    Refuse {
        reason: &'static str,
    },
    Apply {
        bias_rot: Quat,
        hinge_bias_deg: Option<f32>,
        before_stats: Vec<(JointRestBiasStats, Option<JointRestBiasSeverity>)>,
        after_stats: Vec<(JointRestBiasStats, Option<JointRestBiasSeverity>)>,
    },
}

fn plan_recenter_for_attachment(
    attachment: &Gen3dPlannedAttachment,
    joint_kind: Option<AiJointKindJson>,
    hinge_axis_join: Option<Vec3>,
    channels_for_bias: &HashSet<String>,
    target: RestBiasTarget,
    thresholds: JointRestBiasThresholds,
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
) -> AttachmentRecenterDecision {
    // Before stats (per slot).
    let mut before_stats: Vec<(JointRestBiasStats, Option<JointRestBiasSeverity>)> = Vec::new();
    for slot in attachment.animations.iter() {
        let stats = joint_rest_bias_stats(attachment.offset, slot, samples_t_m, samples_phase_01);
        let severity = stats.severity(thresholds);
        before_stats.push((stats, severity));
    }

    let targeted_indices: Vec<usize> = attachment
        .animations
        .iter()
        .enumerate()
        .filter(|(_, slot)| channels_for_bias.contains(slot.channel.as_ref()))
        .map(|(i, _)| i)
        .collect();
    let targeted_need_fix = targeted_indices.iter().any(|&i| {
        let stats = before_stats[i].0;
        let sev = stats.severity(thresholds);
        match target {
            RestBiasTarget::Warn => sev.is_some(),
            RestBiasTarget::Error => matches!(sev, Some(JointRestBiasSeverity::Error)),
        }
    });
    if !targeted_need_fix {
        return AttachmentRecenterDecision::Noop {
            reason: "no joint_rest_bias_large issues at requested target on selected channels",
        };
    }

    let candidates = bias_candidate_rotations_for_attachment(
        attachment,
        joint_kind,
        hinge_axis_join,
        channels_for_bias,
        thresholds,
        samples_t_m,
        samples_phase_01,
    );

    let mut best: Option<(
        Quat,
        Option<f32>,
        Vec<(JointRestBiasStats, Option<JointRestBiasSeverity>)>,
    )> = None;

    for (candidate_rot, hinge_deg_opt) in candidates.iter().copied() {
        let candidate_rot = if candidate_rot.is_finite() {
            candidate_rot.normalize()
        } else {
            Quat::IDENTITY
        };

        // Evaluate after stats for all slots.
        let mut after_stats: Vec<(JointRestBiasStats, Option<JointRestBiasSeverity>)> = Vec::new();
        for slot in attachment.animations.iter() {
            let stats = joint_rest_bias_stats_with_bias(
                attachment.offset,
                slot,
                candidate_rot,
                samples_t_m,
                samples_phase_01,
            );
            let severity = stats.severity(thresholds);
            after_stats.push((stats, severity));
        }

        // Constraints:
        // - Targeted: must meet target.
        // - Non-targeted: must not worsen severity rank.
        let mut ok = true;
        for (slot_idx, slot) in attachment.animations.iter().enumerate() {
            let channel = slot.channel.as_ref();
            let before_sev = before_stats[slot_idx].1;
            let after_sev = after_stats[slot_idx].1;
            if channels_for_bias.contains(channel) {
                match target {
                    RestBiasTarget::Warn => {
                        if after_sev.is_some() {
                            ok = false;
                            break;
                        }
                    }
                    RestBiasTarget::Error => {
                        if matches!(after_sev, Some(JointRestBiasSeverity::Error)) {
                            ok = false;
                            break;
                        }
                    }
                }
            } else {
                let before_rank = before_sev.map(|s| s.rank()).unwrap_or(0);
                let after_rank = after_sev.map(|s| s.rank()).unwrap_or(0);
                if after_rank > before_rank {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }

        // Prefer candidate that minimizes the worst targeted min_angle_deg.
        let worst_targeted_min = targeted_indices
            .iter()
            .map(|&i| after_stats[i].0.min_angle_deg)
            .fold(0.0f32, |acc, v| acc.max(v));

        let is_better = match best.as_ref() {
            None => true,
            Some((_, _, best_after)) => {
                let best_worst = targeted_indices
                    .iter()
                    .map(|&i| best_after[i].0.min_angle_deg)
                    .fold(0.0f32, |acc, v| acc.max(v));
                worst_targeted_min < best_worst - 1e-4
            }
        };
        if is_better {
            best = Some((candidate_rot, hinge_deg_opt, after_stats));
        }
    }

    let Some((bias_rot, hinge_bias_deg, after_stats)) = best else {
        return AttachmentRecenterDecision::Refuse {
            reason:
                "no safe bias found (would introduce new rest-bias issues on other channels/slots); re-author motion instead",
        };
    };

    // If the selected bias is effectively identity, no-op.
    let bias_angle_deg = quat_angle_deg(bias_rot);
    if !bias_angle_deg.is_finite() || bias_angle_deg <= 1e-3 {
        return AttachmentRecenterDecision::Noop {
            reason: "computed bias is identity",
        };
    }

    AttachmentRecenterDecision::Apply {
        bias_rot,
        hinge_bias_deg,
        before_stats,
        after_stats,
    }
}

pub(super) fn recenter_attachment_motion_v1(
    job: &mut Gen3dAiJob,
    draft: &mut Gen3dDraft,
    call_id: Option<&str>,
    args_json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut args: RecenterAttachmentMotionArgsV1 = serde_json::from_value(args_json)
        .map_err(|err| format!("Invalid recenter_attachment_motion_v1 args JSON: {err}"))?;
    if args.version == 0 {
        args.version = 1;
    }
    if args.version != 1 {
        return Err(format!(
            "Unsupported recenter_attachment_motion_v1 version {} (expected 1)",
            args.version
        ));
    }

    if args.child_components.is_empty() {
        return Err("recenter_attachment_motion_v1 requires non-empty child_components".into());
    }

    let target = args
        .target
        .as_deref()
        .and_then(RestBiasTarget::from_str)
        .unwrap_or(RestBiasTarget::Warn);
    let thresholds = JointRestBiasThresholds::default();

    let channels_filter: Option<HashSet<String>> = args.channels.as_ref().map(|channels| {
        channels
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect::<HashSet<_>>()
    });
    if matches!(channels_filter.as_ref(), Some(set) if set.is_empty()) {
        return Err(
            "recenter_attachment_motion_v1.channels must be non-empty when provided".into(),
        );
    }

    let cycle_m = infer_cycle_m(job.rig_move_cycle_m, &job.planned_components).max(1e-3);
    let samples_t_m: Vec<f32> = (0..SAMPLE_COUNT)
        .map(|i| (i as f32 / SAMPLE_COUNT as f32) * cycle_m)
        .collect();
    let samples_phase_01: Vec<f32> = samples_t_m.iter().map(|t| *t / cycle_m).collect();

    let assembly_rev_before = job.assembly_rev();
    let mut any_applied = false;
    let mut children_json: Vec<serde_json::Value> = Vec::new();

    let mut resolved_indices: Vec<usize> = Vec::new();
    for cref in args.child_components.iter() {
        let idx = match cref {
            ComponentRefJsonV1::Index(idx) => *idx as usize,
            ComponentRefJsonV1::Name(name) => {
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                job.planned_components
                    .iter()
                    .position(|c| c.name == name)
                    .or_else(|| resolve_component_index_by_name_hint(&job.planned_components, name))
                    .unwrap_or(usize::MAX)
            }
        };
        if idx != usize::MAX {
            resolved_indices.push(idx);
        }
    }
    resolved_indices.sort_unstable();
    resolved_indices.dedup();

    for child_idx in resolved_indices.iter().copied() {
        let Some(child) = job.planned_components.get(child_idx) else {
            children_json.push(serde_json::json!({
                "ok": false,
                "child_component_index": child_idx,
                "reason": format!("child_component_index out of range: {child_idx}"),
            }));
            continue;
        };
        let child_name = child.name.clone();
        let Some(attachment) = child.attach_to.as_ref() else {
            children_json.push(serde_json::json!({
                "ok": false,
                "child_component": child_name,
                "child_component_index": child_idx,
                "reason": "child component has no attach_to (is it the root?)",
            }));
            continue;
        };
        if attachment.animations.is_empty() {
            children_json.push(serde_json::json!({
                "ok": true,
                "child_component": child_name,
                "child_component_index": child_idx,
                "applied": false,
                "reason": "edge has no animation slots",
            }));
            continue;
        }

        let mut channels_for_bias: HashSet<String> = match channels_filter.as_ref() {
            Some(filter) => filter.clone(),
            None => attachment
                .animations
                .iter()
                .map(|s| s.channel.as_ref().trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
        };
        channels_for_bias.retain(|ch| {
            attachment
                .animations
                .iter()
                .any(|s| s.channel.as_ref() == ch)
        });
        if channels_for_bias.is_empty() {
            children_json.push(serde_json::json!({
                "ok": false,
                "child_component": child_name,
                "child_component_index": child_idx,
                "reason": "no matching slots for requested channels on this edge",
            }));
            continue;
        }

        let joint_kind = attachment.joint.as_ref().map(|j| j.kind);
        let hinge_axis_join = attachment
            .joint
            .as_ref()
            .and_then(|j| (j.kind == AiJointKindJson::Hinge).then_some(j.axis_join))
            .flatten()
            .map(|arr| Vec3::new(arr[0], arr[1], arr[2]))
            .filter(|v| v.is_finite() && v.length_squared() > 1e-6)
            .map(|v| v.normalize());

        if matches!(joint_kind, Some(AiJointKindJson::Hinge)) && hinge_axis_join.is_none() {
            children_json.push(serde_json::json!({
                "ok": false,
                "child_component": child_name,
                "child_component_index": child_idx,
                "reason": "hinge joint is missing a valid axis_join",
            }));
            continue;
        }

        // Ensure spin clips (if any) can be represented after applying a hinge bias.
        if let Some(axis_join) = hinge_axis_join {
            let spin_ok = attachment
                .animations
                .iter()
                .all(|slot| spin_slot_supported_for_hinge_bias(slot, axis_join));
            if !spin_ok {
                children_json.push(serde_json::json!({
                    "ok": true,
                    "child_component": child_name,
                    "child_component_index": child_idx,
                    "applied": false,
                    "reason": "edge contains a spin clip whose axis is not aligned with hinge axis_join; cannot recenter without changing motion",
                }));
                continue;
            }
        } else {
            let has_spin = attachment
                .animations
                .iter()
                .any(|slot| matches!(slot.spec.clip, PartAnimationDef::Spin { .. }));
            if has_spin {
                children_json.push(serde_json::json!({
                    "ok": true,
                    "child_component": child_name,
                    "child_component_index": child_idx,
                    "applied": false,
                    "reason": "edge contains a spin clip on a non-hinge joint; cannot recenter without changing motion",
                }));
                continue;
            }
        }

        // Before stats (per slot).
        let decision = plan_recenter_for_attachment(
            attachment,
            joint_kind,
            hinge_axis_join,
            &channels_for_bias,
            target,
            thresholds,
            &samples_t_m,
            &samples_phase_01,
        );
        let (bias_rot, hinge_bias_deg, before_stats, after_stats) = match decision {
            AttachmentRecenterDecision::Noop { reason } => {
                children_json.push(serde_json::json!({
                    "ok": true,
                    "child_component": child_name,
                    "child_component_index": child_idx,
                    "applied": false,
                    "reason": reason,
                }));
                continue;
            }
            AttachmentRecenterDecision::Refuse { reason } => {
                children_json.push(serde_json::json!({
                    "ok": true,
                    "child_component": child_name,
                    "child_component_index": child_idx,
                    "applied": false,
                    "reason": reason,
                }));
                continue;
            }
            AttachmentRecenterDecision::Apply {
                bias_rot,
                hinge_bias_deg,
                before_stats,
                after_stats,
            } => (bias_rot, hinge_bias_deg, before_stats, after_stats),
        };

        // Build per-slot report.
        let mut slots_json: Vec<serde_json::Value> = Vec::new();
        for (slot_idx, slot) in attachment.animations.iter().enumerate() {
            let channel = slot.channel.as_ref().to_string();
            let before = before_stats[slot_idx].0;
            let after = after_stats[slot_idx].0;
            let before_sev = before_stats[slot_idx].1.map(|s| s.as_str());
            let after_sev = after_stats[slot_idx].1.map(|s| s.as_str());
            slots_json.push(serde_json::json!({
                "slot_index": slot_idx,
                "channel": channel,
                "driver": format!("{:?}", slot.spec.driver),
                "clip_kind": match slot.spec.clip {
                    PartAnimationDef::Loop{..} => "loop",
                    PartAnimationDef::Once{..} => "once",
                    PartAnimationDef::PingPong{..} => "ping_pong",
                    PartAnimationDef::Spin{..} => "spin",
                },
                "before": {
                    "min_angle_degrees": before.min_angle_deg,
                    "min_angle_at_phase_01": before.min_phase_01,
                    "max_angle_degrees": before.max_angle_deg,
                    "max_angle_at_phase_01": before.max_phase_01,
                    "span_degrees": before.span_deg,
                    "severity": before_sev,
                },
                "after": {
                    "min_angle_degrees": after.min_angle_deg,
                    "min_angle_at_phase_01": after.min_phase_01,
                    "max_angle_degrees": after.max_angle_deg,
                    "max_angle_at_phase_01": after.max_phase_01,
                    "span_degrees": after.span_deg,
                    "severity": after_sev,
                }
            }));
        }

        let mut applied = false;
        if !args.dry_run {
            if let Some(att_mut) = job.planned_components[child_idx].attach_to.as_mut() {
                apply_bias_to_attachment_in_place(
                    att_mut,
                    bias_rot,
                    hinge_bias_deg,
                    hinge_axis_join,
                );
                applied = true;
            }
        }

        any_applied |= applied;

        let channels_used_sorted: Vec<String> = {
            let mut v: Vec<String> = channels_for_bias.iter().cloned().collect();
            v.sort();
            v
        };
        children_json.push(serde_json::json!({
            "ok": true,
            "child_component": child_name,
            "child_component_index": child_idx,
            "applied": applied,
            "dry_run": args.dry_run,
            "channels_used_for_bias": channels_used_sorted,
            "bias": {
                "rot_quat_xyzw": [bias_rot.x, bias_rot.y, bias_rot.z, bias_rot.w],
                "hinge_degrees": hinge_bias_deg,
            },
            "slots": slots_json,
        }));
    }

    if any_applied && !args.dry_run {
        if let Some(root_idx) = job
            .planned_components
            .iter()
            .position(|c| c.attach_to.is_none())
        {
            convert::resolve_planned_component_transforms(&mut job.planned_components, root_idx)?;
        }
        convert::sync_attachment_tree_to_defs(&job.planned_components, draft)?;
        convert::update_root_def_from_planned_components(
            &job.planned_components,
            &job.plan_collider,
            draft,
        );

        if let Some(dir) = job.pass_dir_path() {
            write_gen3d_assembly_snapshot(Some(dir), &job.planned_components);
        }

        job.assembly_rev = job.assembly_rev.saturating_add(1);
    }

    let result = serde_json::json!({
        "ok": true,
        "version": 1,
        "dry_run": args.dry_run,
        "assembly_rev_before": assembly_rev_before,
        "new_assembly_rev": job.assembly_rev(),
        "applied_any": any_applied && !args.dry_run,
        "target": match target { RestBiasTarget::Warn => "warn", RestBiasTarget::Error => "error" },
        "thresholds": {
            "warn_min_angle_degrees": thresholds.warn_min_angle_deg,
            "error_min_angle_degrees": thresholds.error_min_angle_deg,
            "max_span_degrees": thresholds.max_span_deg,
        },
        "children": children_json,
    });

    if let Some(dir) = job.pass_dir_path() {
        if let Some(call_id) = call_id {
            let prefix =
                super::agent_utils::sanitize_prefix(&format!("tool_recenter_motion_{}", call_id));
            write_gen3d_json_artifact(Some(dir), format!("{prefix}.json"), &result);
        }
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        append_gen3d_jsonl_artifact(
            Some(dir),
            "draft_ops.jsonl",
            &serde_json::json!({
                "ts_ms": ts_ms,
                "tool": "recenter_attachment_motion_v1",
                "call_id": call_id.unwrap_or(""),
                "active_workspace": job.active_workspace_id(),
                "assembly_rev_before": assembly_rev_before,
                "assembly_rev_after": job.assembly_rev(),
                "dry_run": args.dry_run,
                "applied_any": any_applied && !args.dry_run,
            }),
        );
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{AnchorDef, PartAnimationSpec};

    fn anchor(name: &str, translation: Vec3) -> AnchorDef {
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform::from_translation(translation),
        }
    }

    fn stub_component(name: &str, anchors: Vec<AnchorDef>) -> Gen3dPlannedComponent {
        Gen3dPlannedComponent {
            display_name: name.to_string(),
            name: name.to_string(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors,
            contacts: Vec::new(),
            root_animations: Vec::new(),
            attach_to: None,
        }
    }

    fn assert_transforms_close(a: Transform, b: Transform) {
        let dp = (a.translation - b.translation).length();
        assert!(
            dp.is_finite() && dp <= 1e-4,
            "translation mismatch dp={dp} a={a:?} b={b:?}"
        );

        let qa = if a.rotation.is_finite() {
            a.rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        let qb = if b.rotation.is_finite() {
            b.rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        let dq = quat_angle_deg((qa.inverse() * qb).normalize());
        assert!(
            dq.is_finite() && dq <= 1e-3,
            "rotation mismatch dq_deg={dq} a={a:?} b={b:?}"
        );

        let ds = (a.scale - b.scale).length();
        assert!(
            ds.is_finite() && ds <= 1e-4,
            "scale mismatch ds={ds} a={a:?} b={b:?}"
        );
    }

    #[test]
    fn hinge_rest_bias_can_be_recentered_without_changing_motion() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let base_rot = Quat::from_rotation_x(85.0f32.to_radians());
        let bump_rot = Quat::from_rotation_x(90.0f32.to_radians());

        let idle_spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform {
                            rotation: base_rot,
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform {
                            translation: Vec3::new(0.1, 0.0, 0.0),
                            rotation: bump_rot,
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 2.0,
                        delta: Transform {
                            rotation: base_rot,
                            ..default()
                        },
                    },
                ],
            },
        };

        let mut limb = stub_component("limb", vec![anchor("mount", Vec3::ZERO)]);
        limb.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: Some(super::super::schema::AiJointJson {
                kind: AiJointKindJson::Hinge,
                axis_join: Some([1.0, 0.0, 0.0]),
                limits_degrees: Some([-120.0, 120.0]),
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![PartAnimationSlot {
                channel: "idle".into(),
                spec: idle_spec,
            }],
        });

        let components_before = vec![root.clone(), limb.clone()];
        let report_before = super::super::motion_validation::build_motion_validation_report(
            Some(1.0),
            &components_before,
        );
        let issues_before = report_before
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues_before.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("joint_rest_bias_large")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
            }),
            "expected joint_rest_bias_large issue on idle channel, got {issues_before:?}"
        );

        let cycle_m = 1.0f32.max(1e-3);
        let samples_t_m: Vec<f32> = (0..SAMPLE_COUNT)
            .map(|i| (i as f32 / SAMPLE_COUNT as f32) * cycle_m)
            .collect();
        let samples_phase_01: Vec<f32> = samples_t_m.iter().map(|t| *t / cycle_m).collect();

        let channels_for_bias: HashSet<String> = ["idle".to_string()].into_iter().collect();
        let attachment = limb.attach_to.as_ref().unwrap();
        let decision = plan_recenter_for_attachment(
            attachment,
            attachment.joint.as_ref().map(|j| j.kind),
            Some(Vec3::X),
            &channels_for_bias,
            RestBiasTarget::Warn,
            JointRestBiasThresholds::default(),
            &samples_t_m,
            &samples_phase_01,
        );
        let (bias_rot, hinge_bias_deg) = match decision {
            AttachmentRecenterDecision::Apply {
                bias_rot,
                hinge_bias_deg,
                ..
            } => (bias_rot, hinge_bias_deg),
            other => panic!("expected Apply, got {other:?}"),
        };

        let mut limb_after = limb.clone();
        apply_bias_to_attachment_in_place(
            limb_after.attach_to.as_mut().unwrap(),
            bias_rot,
            hinge_bias_deg,
            Some(Vec3::X),
        );

        // Motion must be unchanged: (offset * delta(t)) is invariant.
        let att_before = limb.attach_to.as_ref().unwrap();
        let slot_before = &att_before.animations[0];
        let att_after = limb_after.attach_to.as_ref().unwrap();
        let slot_after = &att_after.animations[0];
        for (i, &t_m) in samples_t_m.iter().enumerate() {
            let phase = samples_phase_01[i];
            let delta_before = sample_animation_slot_delta(slot_before, t_m, phase);
            let animated_before = mul_transform(&att_before.offset, &delta_before);
            let delta_after = sample_animation_slot_delta(slot_after, t_m, phase);
            let animated_after = mul_transform(&att_after.offset, &delta_after);
            assert_transforms_close(animated_before, animated_after);
        }

        let components_after = vec![root, limb_after];
        let report_after = super::super::motion_validation::build_motion_validation_report(
            Some(1.0),
            &components_after,
        );
        let issues_after = report_after
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues_after.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("joint_rest_bias_large")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
            }),
            "expected joint_rest_bias_large to be cleared on idle channel, got {issues_after:?}"
        );
    }

    #[test]
    fn hinge_recenter_finds_compromise_bias_when_other_channel_is_near_neutral() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let base_rot = Quat::from_rotation_x(85.0f32.to_radians());
        let bump_rot = Quat::from_rotation_x(90.0f32.to_radians());

        let idle_spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform {
                            rotation: base_rot,
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform {
                            rotation: bump_rot,
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 2.0,
                        delta: Transform {
                            rotation: base_rot,
                            ..default()
                        },
                    },
                ],
            },
        };

        let attack_spec = PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Once {
                duration_secs: 1.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform::IDENTITY,
                    },
                ],
            },
        };

        let mut limb = stub_component("limb", vec![anchor("mount", Vec3::ZERO)]);
        limb.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: Some(super::super::schema::AiJointJson {
                kind: AiJointKindJson::Hinge,
                axis_join: Some([1.0, 0.0, 0.0]),
                limits_degrees: Some([-120.0, 120.0]),
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![
                PartAnimationSlot {
                    channel: "idle".into(),
                    spec: idle_spec,
                },
                PartAnimationSlot {
                    channel: "attack_primary".into(),
                    spec: attack_spec,
                },
            ],
        });

        let cycle_m = 1.0f32.max(1e-3);
        let samples_t_m: Vec<f32> = (0..SAMPLE_COUNT)
            .map(|i| (i as f32 / SAMPLE_COUNT as f32) * cycle_m)
            .collect();
        let samples_phase_01: Vec<f32> = samples_t_m.iter().map(|t| *t / cycle_m).collect();

        let channels_for_bias: HashSet<String> = ["idle".to_string()].into_iter().collect();
        let attachment = limb.attach_to.as_ref().unwrap();
        let decision = plan_recenter_for_attachment(
            attachment,
            attachment.joint.as_ref().map(|j| j.kind),
            Some(Vec3::X),
            &channels_for_bias,
            RestBiasTarget::Warn,
            JointRestBiasThresholds::default(),
            &samples_t_m,
            &samples_phase_01,
        );
        let (bias_rot, hinge_bias_deg) = match decision {
            AttachmentRecenterDecision::Apply {
                bias_rot,
                hinge_bias_deg,
                ..
            } => (bias_rot, hinge_bias_deg),
            other => panic!("expected Apply (compromise bias), got {other:?}"),
        };

        let mut limb_after = limb.clone();
        apply_bias_to_attachment_in_place(
            limb_after.attach_to.as_mut().unwrap(),
            bias_rot,
            hinge_bias_deg,
            Some(Vec3::X),
        );

        let components_after = vec![root, limb_after];
        let report_after = super::super::motion_validation::build_motion_validation_report(
            Some(1.0),
            &components_after,
        );
        let issues_after = report_after
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues_after.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("joint_rest_bias_large")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
            }),
            "expected joint_rest_bias_large to be cleared on idle channel, got {issues_after:?}"
        );
        assert!(
            !issues_after.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("joint_rest_bias_large")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("attack_primary")
            }),
            "expected attack_primary to avoid joint_rest_bias_large after recenter, got {issues_after:?}"
        );
    }
}
