use bevy::prelude::*;
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, PartAnimationDef, PartAnimationDriver, PartAnimationSlot,
};

use super::schema::{AiContactKindJson, AiJointKindJson};
use super::{Gen3dPlannedAttachment, Gen3dPlannedComponent};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MotionSeverity {
    Warn,
    Error,
}

impl MotionSeverity {
    fn as_str(self) -> &'static str {
        match self {
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }

    fn sort_key(self) -> u8 {
        match self {
            Self::Error => 0,
            Self::Warn => 1,
        }
    }
}

#[derive(Clone, Debug)]
struct MotionIssue {
    severity: MotionSeverity,
    kind: &'static str,
    component_id: String,
    component_name: String,
    channel: String,
    message: String,
    evidence: serde_json::Value,
    score: f32,
}

impl MotionIssue {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "severity": self.severity.as_str(),
            "kind": self.kind,
            "component_id": self.component_id,
            "component_name": self.component_name,
            "channel": self.channel,
            "message": self.message,
            "evidence": self.evidence,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) struct MotionValidationReport {
    pub(super) rig_summary: serde_json::Value,
    pub(super) motion_validation: serde_json::Value,
}

const DEFAULT_CYCLE_M: f32 = 1.0;
const DEFAULT_ATTACK_WINDOW_SECS: f32 = 0.35;
const SAMPLE_COUNT: usize = 24;
const ATTACK_SAMPLE_COUNT: usize = 12;
const MAX_ISSUES: usize = 8;

pub(super) fn build_motion_validation_report(
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
) -> MotionValidationReport {
    let joints_total: usize = components
        .iter()
        .filter(|c| {
            c.attach_to
                .as_ref()
                .and_then(|a| a.joint.as_ref())
                .is_some()
        })
        .count();
    let contacts_total: usize = components.iter().map(|c| c.contacts.len()).sum();
    let contacts_with_stance: usize = components
        .iter()
        .flat_map(|c| c.contacts.iter())
        .filter(|c| c.stance.is_some())
        .count();

    let (cycle_m, cycle_source) = infer_cycle_m(rig_move_cycle_m, components);
    let cycle_m = cycle_m.max(1e-3);

    let samples_t_m: Vec<f32> = (0..SAMPLE_COUNT)
        .map(|i| (i as f32 / SAMPLE_COUNT as f32) * cycle_m)
        .collect();
    let samples_phase_01: Vec<f32> = samples_t_m.iter().map(|t| *t / cycle_m).collect();

    let root_idx = components
        .iter()
        .position(|c| c.attach_to.is_none())
        .unwrap_or(0);
    let root_forward = Vec3::Z;
    let mut root_forward_xz = Vec3::new(root_forward.x, 0.0, root_forward.z);
    if !root_forward_xz.is_finite() || root_forward_xz.length_squared() <= 1e-6 {
        root_forward_xz = Vec3::Z;
    } else {
        root_forward_xz = root_forward_xz.normalize();
    }

    let mut issues: Vec<MotionIssue> = Vec::new();

    validate_chain_anchor_axes(components, &mut issues);
    validate_joints(&samples_t_m, &samples_phase_01, components, &mut issues);
    validate_contacts(
        &samples_t_m,
        &samples_phase_01,
        cycle_m,
        root_idx,
        root_forward_xz,
        components,
        &mut issues,
    );
    validate_attack_self_intersection(root_idx, components, &mut issues);

    issues.sort_by(|a, b| {
        a.severity
            .sort_key()
            .cmp(&b.severity.sort_key())
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.kind.cmp(b.kind))
            .then_with(|| a.component_name.cmp(&b.component_name))
    });
    if issues.len() > MAX_ISSUES {
        issues.truncate(MAX_ISSUES);
    }

    let ok = issues.iter().all(|i| i.severity != MotionSeverity::Error);
    let issues_json: Vec<serde_json::Value> = issues.iter().map(|i| i.to_json()).collect();

    MotionValidationReport {
        rig_summary: serde_json::json!({
            "cycle_m": cycle_m,
            "cycle_source": cycle_source,
            "sample_count": SAMPLE_COUNT,
            "joints_total": joints_total,
            "contacts_total": contacts_total,
            "contacts_with_stance": contacts_with_stance,
        }),
        motion_validation: serde_json::json!({
            "ok": ok,
            "issues": issues_json,
        }),
    }
}

fn validate_chain_anchor_axes(components: &[Gen3dPlannedComponent], issues: &mut Vec<MotionIssue>) {
    const EPS: f32 = 1e-6;
    const DOT_ERROR_THRESHOLD: f32 = 0.2;

    let mut name_to_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (idx, c) in components.iter().enumerate() {
        name_to_idx.insert(c.name.as_str(), idx);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (idx, c) in components.iter().enumerate() {
        let Some(att) = c.attach_to.as_ref() else {
            continue;
        };
        if let Some(parent_idx) = name_to_idx.get(att.parent.as_str()).copied() {
            children[parent_idx].push(idx);
        }
    }

    for (idx, comp) in components.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(joint) = att.joint.as_ref() else {
            continue;
        };
        if !matches!(joint.kind, AiJointKindJson::Hinge | AiJointKindJson::Ball) {
            continue;
        }
        // Restrict to the most common limb-link case to avoid false positives:
        // - exactly 2 anchors (a proximal + distal joint)
        // - exactly 1 child (a simple chain segment)
        if comp.anchors.len() != 2 {
            continue;
        }
        if children.get(idx).map_or(0, |v| v.len()) != 1 {
            continue;
        }

        let child_idx = children[idx][0];
        let Some(child_att) = components.get(child_idx).and_then(|c| c.attach_to.as_ref()) else {
            continue;
        };

        // In the component-local frame, the vector from the anchor that attaches to the parent
        // (proximal) to the anchor that attaches to the child (distal) should be aligned with the
        // join-frame +Z direction ("forward") at the proximal anchor.
        let proximal_anchor = att.child_anchor.as_str();
        let distal_anchor = child_att.parent_anchor.as_str();
        if proximal_anchor == distal_anchor {
            continue;
        }

        let proximal_tf = anchor_transform_from_component(comp, proximal_anchor);
        let distal_tf = anchor_transform_from_component(comp, distal_anchor);
        let axis = distal_tf.translation - proximal_tf.translation;
        if !axis.is_finite() || axis.length_squared() <= EPS {
            continue;
        }
        let axis_dir = axis.normalize();

        let forward = if proximal_tf.rotation.is_finite() {
            (proximal_tf.rotation.normalize() * Vec3::Z)
        } else {
            Vec3::Z
        };
        let forward = if forward.length_squared() > EPS {
            forward.normalize()
        } else {
            Vec3::Z
        };

        let dot = axis_dir.dot(forward);
        if dot >= DOT_ERROR_THRESHOLD {
            continue;
        }

        let angle_deg = dot.clamp(-1.0, 1.0).acos().to_degrees();

        let mut suggested_up = if proximal_tf.rotation.is_finite() {
            proximal_tf.rotation.normalize() * Vec3::Y
        } else {
            Vec3::Y
        };
        suggested_up = suggested_up - axis_dir * suggested_up.dot(axis_dir);
        if !suggested_up.is_finite() || suggested_up.length_squared() <= EPS {
            suggested_up = Vec3::Y - axis_dir * axis_dir.dot(Vec3::Y);
        }
        if !suggested_up.is_finite() || suggested_up.length_squared() <= EPS {
            suggested_up = Vec3::Z - axis_dir * axis_dir.dot(Vec3::Z);
        }
        if !suggested_up.is_finite() || suggested_up.length_squared() <= EPS {
            suggested_up = Vec3::X - axis_dir * axis_dir.dot(Vec3::X);
        }
        if suggested_up.length_squared() > EPS {
            suggested_up = suggested_up.normalize();
        } else {
            suggested_up = Vec3::Y;
        }

        issues.push(MotionIssue {
            severity: MotionSeverity::Error,
            kind: "chain_axis_mismatch",
            component_id: component_id_uuid_for_name(&comp.name),
            component_name: comp.name.clone(),
            channel: "rig".to_string(),
            message: "Intermediate component chain axis (between its parent and child anchors) is not aligned with join forward (+Z) at the parent joint; this usually makes limbs point the wrong direction and makes animation deltas rotate around the wrong axis.".into(),
            evidence: serde_json::json!({
                "proximal_anchor": proximal_anchor,
                "distal_anchor": distal_anchor,
                "axis_dir_component_local": [axis_dir.x, axis_dir.y, axis_dir.z],
                "forward_component_local": [forward.x, forward.y, forward.z],
                "dot": dot,
                "angle_degrees": angle_deg,
                "suggested_forward_component_local": [axis_dir.x, axis_dir.y, axis_dir.z],
                "suggested_up_component_local": [suggested_up.x, suggested_up.y, suggested_up.z],
                "thresholds": { "error_dot_min": DOT_ERROR_THRESHOLD },
                "hint": "Fix by reorienting the component's proximal and distal anchors in COMPONENT-LOCAL space so their +Z (forward) points from the parent joint toward the child joint. Do not blindly copy the parent's anchor forward/up vectors into the child component.",
            }),
            score: angle_deg,
        });
    }
}

fn infer_cycle_m(
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
) -> (f32, &'static str) {
    if let Some(v) = rig_move_cycle_m
        .filter(|v| v.is_finite())
        .map(|v| v.abs())
        .filter(|v| *v > 1e-3)
    {
        return (v, "rig.move_cycle_m");
    }

    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(slot) = find_move_slot(att) else {
            continue;
        };
        if !matches!(
            slot.spec.driver,
            PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
        ) {
            continue;
        }
        let PartAnimationDef::Loop { duration_secs, .. } = &slot.spec.clip else {
            continue;
        };
        if !duration_secs.is_finite() || *duration_secs <= 0.0 {
            continue;
        }
        let speed_scale = slot.spec.speed_scale.max(1e-6);
        let effective = (*duration_secs / speed_scale).abs();
        if effective.is_finite() && effective > 1e-3 {
            return (effective, "move.loop.duration_secs");
        }
    }

    (DEFAULT_CYCLE_M, "default")
}

fn component_id_uuid_for_name(name: &str) -> String {
    let id = builtin_object_id(&format!("gravimera/gen3d/component/{}", name));
    Uuid::from_u128(id).to_string()
}

fn find_move_slot(
    att: &Gen3dPlannedAttachment,
) -> Option<&crate::object::registry::PartAnimationSlot> {
    att.animations.iter().find(|s| s.channel.as_ref() == "move")
}

fn sample_move_delta(
    move_slot: &crate::object::registry::PartAnimationSlot,
    driver_t: f32,
) -> Transform {
    let driver_t = if driver_t.is_finite() { driver_t } else { 0.0 };
    let mut t = driver_t * move_slot.spec.speed_scale.max(0.0);
    if move_slot.spec.time_offset_units.is_finite() {
        t += move_slot.spec.time_offset_units;
    }
    sample_part_animation(&move_slot.spec.clip, t)
}

fn sample_animation_slot_delta(
    slot: &PartAnimationSlot,
    sample_t_m: f32,
    sample_phase_01: f32,
) -> Transform {
    let driver_t = match slot.spec.driver {
        PartAnimationDriver::Always => match &slot.spec.clip {
            PartAnimationDef::Loop { duration_secs, .. }
                if duration_secs.is_finite() && *duration_secs > 0.0 =>
            {
                sample_phase_01 * *duration_secs
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

fn sample_part_animation(animation: &PartAnimationDef, time_secs: f32) -> Transform {
    match animation {
        PartAnimationDef::Loop {
            duration_secs,
            keyframes,
        } => {
            let duration = (*duration_secs).max(1e-6);
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
        PartAnimationDef::Spin {
            axis,
            radians_per_unit,
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
    let (scale, rotation, translation) = composed.to_scale_rotation_translation();
    if !translation.is_finite() || !rotation.is_finite() || !scale.is_finite() {
        return *b;
    }
    Transform {
        translation,
        rotation,
        scale,
    }
}

fn anchor_transform_from_component(comp: &Gen3dPlannedComponent, name: &str) -> Transform {
    if name == "origin" {
        return Transform::IDENTITY;
    }
    comp.anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
        .unwrap_or(Transform::IDENTITY)
}

fn compute_world_transforms_at_t(
    components: &[Gen3dPlannedComponent],
    children: &[Vec<usize>],
    root_idx: usize,
    t_m: f32,
) -> Vec<Transform> {
    let mut world: Vec<Transform> = vec![Transform::IDENTITY; components.len()];
    let mut visiting = vec![false; components.len()];
    let mut visited = vec![false; components.len()];
    world[root_idx] = Transform::IDENTITY;

    fn dfs(
        idx: usize,
        components: &[Gen3dPlannedComponent],
        children: &[Vec<usize>],
        t_m: f32,
        world: &mut [Transform],
        visiting: &mut [bool],
        visited: &mut [bool],
    ) {
        if visited[idx] || visiting[idx] {
            return;
        }
        visiting[idx] = true;

        let parent_world = world[idx];
        for &child_idx in &children[idx] {
            let Some(att) = components[child_idx].attach_to.as_ref() else {
                continue;
            };
            let parent_anchor =
                anchor_transform_from_component(&components[idx], att.parent_anchor.as_str());
            let child_anchor =
                anchor_transform_from_component(&components[child_idx], att.child_anchor.as_str());

            let mut animated_offset = att.offset;
            if let Some(move_slot) = find_move_slot(att) {
                if matches!(
                    move_slot.spec.driver,
                    PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
                ) {
                    let delta = sample_move_delta(move_slot, t_m);
                    animated_offset = mul_transform(&animated_offset, &delta);
                }
            }

            let inv_child = child_anchor.to_matrix().inverse();
            let composed = parent_world.to_matrix()
                * parent_anchor.to_matrix()
                * animated_offset.to_matrix()
                * inv_child;
            let (scale, rot, translation) = composed.to_scale_rotation_translation();
            if translation.is_finite() && rot.is_finite() && scale.is_finite() {
                world[child_idx] = Transform {
                    translation,
                    rotation: rot.normalize(),
                    scale,
                };
            } else {
                world[child_idx] = Transform::IDENTITY;
            }

            dfs(
                child_idx, components, children, t_m, world, visiting, visited,
            );
        }

        visiting[idx] = false;
        visited[idx] = true;
    }

    dfs(
        root_idx,
        components,
        children,
        t_m,
        &mut world,
        &mut visiting,
        &mut visited,
    );

    world
}

fn phase_in_stance(phase_01: f32, start_01: f32, duty_01: f32) -> bool {
    let phase_01 = phase_01.rem_euclid(1.0);
    let start_01 = start_01.rem_euclid(1.0);
    let duty_01 = duty_01.clamp(0.0, 1.0);
    let end_01 = (start_01 + duty_01).rem_euclid(1.0);
    if duty_01 >= 1.0 - 1e-6 {
        return true;
    }
    if start_01 <= end_01 {
        phase_01 >= start_01 && phase_01 <= end_01
    } else {
        phase_01 >= start_01 || phase_01 <= end_01
    }
}

fn circular_distance(a: f32, b: f32) -> f32 {
    let d = (a - b).abs();
    d.min(1.0 - d)
}

fn quat_angle_deg(q: Quat) -> f32 {
    let w = q.w.clamp(-1.0, 1.0).abs();
    (2.0 * w.acos()).to_degrees()
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

fn validate_joints(
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
    components: &[Gen3dPlannedComponent],
    issues: &mut Vec<MotionIssue>,
) {
    fn maybe_push_joint_rest_bias_issue(
        issues: &mut Vec<MotionIssue>,
        component_id: &str,
        component_name: &str,
        channel: &str,
        min_angle_deg: f32,
        min_phase: f32,
        max_angle_deg: f32,
        max_phase: f32,
    ) {
        if !min_angle_deg.is_finite() || !max_angle_deg.is_finite() {
            return;
        }
        let span_deg = (max_angle_deg - min_angle_deg).max(0.0);
        let bias_warn_deg = 50.0;
        let bias_error_deg = 75.0;
        let max_span_deg = 70.0;
        if min_angle_deg <= bias_warn_deg || span_deg > max_span_deg {
            return;
        }
        let severity = if min_angle_deg >= bias_error_deg {
            MotionSeverity::Error
        } else {
            MotionSeverity::Warn
        };
        issues.push(MotionIssue {
            severity,
            kind: "joint_rest_bias_large",
            component_id: component_id.to_string(),
            component_name: component_name.to_string(),
            channel: channel.to_string(),
            message: "Animation channel keeps this joint far from neutral for the full cycle (likely absolute-frame animation instead of delta).".into(),
            evidence: serde_json::json!({
                "min_angle_degrees": min_angle_deg,
                "min_angle_at_phase_01": min_phase,
                "max_angle_degrees": max_angle_deg,
                "max_angle_at_phase_01": max_phase,
                "span_degrees": span_deg,
                "tolerances": {
                    "warn_min_angle_degrees": bias_warn_deg,
                    "error_min_angle_degrees": bias_error_deg,
                    "max_span_degrees": max_span_deg,
                },
            }),
            score: min_angle_deg,
        });
    }

    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(joint) = att.joint.as_ref() else {
            continue;
        };
        if att.animations.is_empty() {
            continue;
        }

        let component_name = comp.name.clone();
        let component_id = component_id_uuid_for_name(&component_name);

        for slot in att.animations.iter() {
            let channel = slot.channel.as_ref().to_string();
            if channel.is_empty() {
                continue;
            }

            let mut max_angle_deg: f32 = 0.0;
            let mut max_angle_phase: f32 = 0.0;
            let mut min_angle_deg: f32 = f32::INFINITY;
            let mut min_angle_phase: f32 = 0.0;

            let mut max_off_axis_deg: f32 = 0.0;
            let mut max_off_axis_phase: f32 = 0.0;
            let mut max_abs_hinge_angle_deg: f32 = 0.0;
            let mut max_abs_hinge_angle_phase: f32 = 0.0;
            let mut max_limit_exceed_deg: f32 = 0.0;
            let mut max_limit_exceed_phase: f32 = 0.0;
            let mut max_translation_m: f32 = 0.0;
            let mut max_translation_phase: f32 = 0.0;

            let axis_join = if joint.kind == AiJointKindJson::Hinge {
                let Some(axis_join_arr) = joint.axis_join else {
                    issues.push(MotionIssue {
                        severity: MotionSeverity::Error,
                        kind: "hinge_axis_missing",
                        component_id: component_id.clone(),
                        component_name: component_name.clone(),
                        channel: channel.clone(),
                        message: "Hinge joint is missing axis_join; cannot validate hinge motion."
                            .into(),
                        evidence: serde_json::json!({}),
                        score: 1.0,
                    });
                    continue;
                };
                let axis_join = Vec3::new(axis_join_arr[0], axis_join_arr[1], axis_join_arr[2]);
                if !axis_join.is_finite() || axis_join.length_squared() <= 1e-6 {
                    issues.push(MotionIssue {
                        severity: MotionSeverity::Error,
                        kind: "hinge_axis_invalid",
                        component_id: component_id.clone(),
                        component_name: component_name.clone(),
                        channel: channel.clone(),
                        message:
                            "Hinge joint axis_join is non-finite or near-zero; cannot validate hinge motion."
                                .into(),
                        evidence: serde_json::json!({
                            "axis_join": axis_join_arr,
                        }),
                        score: 1.0,
                    });
                    continue;
                }
                Some(axis_join.normalize())
            } else {
                None
            };

            for (i, &sample_t_m) in samples_t_m.iter().enumerate() {
                let sample_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
                let delta = sample_animation_slot_delta(slot, sample_t_m, sample_phase_01);
                let animated_offset = mul_transform(&att.offset, &delta);
                let q_delta =
                    (att.offset.rotation.inverse() * animated_offset.rotation).normalize();
                let angle_deg = quat_angle_deg(q_delta).abs();
                let translation_m = delta.translation.length();

                if angle_deg > max_angle_deg {
                    max_angle_deg = angle_deg;
                    max_angle_phase = sample_phase_01;
                }
                if angle_deg < min_angle_deg {
                    min_angle_deg = angle_deg;
                    min_angle_phase = sample_phase_01;
                }
                if translation_m.is_finite() && translation_m > max_translation_m {
                    max_translation_m = translation_m;
                    max_translation_phase = sample_phase_01;
                }

                if let Some(axis_join) = axis_join {
                    let (hinge_angle_deg, off_axis_deg) =
                        hinge_signed_angle_and_off_axis_deg(q_delta, axis_join);

                    if off_axis_deg > max_off_axis_deg {
                        max_off_axis_deg = off_axis_deg;
                        max_off_axis_phase = sample_phase_01;
                    }
                    if hinge_angle_deg.abs() > max_abs_hinge_angle_deg {
                        max_abs_hinge_angle_deg = hinge_angle_deg.abs();
                        max_abs_hinge_angle_phase = sample_phase_01;
                    }
                    if let Some([min_deg, max_deg]) = joint.limits_degrees {
                        let (min_deg, max_deg) = if min_deg <= max_deg {
                            (min_deg, max_deg)
                        } else {
                            (max_deg, min_deg)
                        };
                        let exceed = if hinge_angle_deg < min_deg {
                            min_deg - hinge_angle_deg
                        } else if hinge_angle_deg > max_deg {
                            hinge_angle_deg - max_deg
                        } else {
                            0.0
                        };
                        if exceed > max_limit_exceed_deg {
                            max_limit_exceed_deg = exceed;
                            max_limit_exceed_phase = sample_phase_01;
                        }
                    }
                }
            }

            match joint.kind {
                AiJointKindJson::Hinge => {
                    let Some(axis_join) = axis_join else {
                        continue;
                    };
                    let off_axis_warn_deg: f32 = 8.0;
                    let off_axis_error_deg: f32 = 18.0;
                    if max_off_axis_deg.is_finite() && max_off_axis_deg > off_axis_warn_deg {
                        let severity = if max_off_axis_deg >= off_axis_error_deg {
                            MotionSeverity::Error
                        } else {
                            MotionSeverity::Warn
                        };
                        issues.push(MotionIssue {
                            severity,
                            kind: "hinge_off_axis",
                            component_id: component_id.clone(),
                            component_name: component_name.clone(),
                            channel: channel.clone(),
                            message: "Hinge joint motion includes off-axis rotation (likely wrong frame or wrong degrees-of-freedom)."
                                .into(),
                            evidence: serde_json::json!({
                                "axis_join": [axis_join.x, axis_join.y, axis_join.z],
                                "max_off_axis_degrees": max_off_axis_deg,
                                "at_phase_01": max_off_axis_phase,
                                "max_abs_hinge_angle_degrees": max_abs_hinge_angle_deg,
                                "hinge_angle_at_phase_01": max_abs_hinge_angle_phase,
                                "tolerances": { "warn_off_axis_degrees": off_axis_warn_deg, "error_off_axis_degrees": off_axis_error_deg },
                            }),
                            score: max_off_axis_deg,
                        });
                    }
                    if max_limit_exceed_deg.is_finite() && max_limit_exceed_deg > 0.1 {
                        issues.push(MotionIssue {
                            severity: MotionSeverity::Error,
                            kind: "hinge_limit_exceeded",
                            component_id: component_id.clone(),
                            component_name: component_name.clone(),
                            channel: channel.clone(),
                            message: "Hinge joint motion exceeds declared limits.".into(),
                            evidence: serde_json::json!({
                                "limits_degrees": joint.limits_degrees,
                                "max_exceed_degrees": max_limit_exceed_deg,
                                "at_phase_01": max_limit_exceed_phase,
                            }),
                            score: max_limit_exceed_deg,
                        });
                    }
                    maybe_push_joint_rest_bias_issue(
                        issues,
                        &component_id,
                        &component_name,
                        channel.as_str(),
                        min_angle_deg,
                        min_angle_phase,
                        max_angle_deg,
                        max_angle_phase,
                    );
                }
                AiJointKindJson::Fixed => {
                    let warn_deg = 2.0;
                    let error_deg = 6.0;
                    if max_angle_deg.is_finite() && max_angle_deg > warn_deg {
                        let severity = if max_angle_deg >= error_deg {
                            MotionSeverity::Error
                        } else {
                            MotionSeverity::Warn
                        };
                        issues.push(MotionIssue {
                            severity,
                            kind: "fixed_joint_rotates",
                            component_id: component_id.clone(),
                            component_name: component_name.clone(),
                            channel: channel.clone(),
                            message: "Fixed joint rotates under animation (expected no rotation)."
                                .into(),
                            evidence: serde_json::json!({
                                "max_angle_degrees": max_angle_deg,
                                "at_phase_01": max_angle_phase,
                                "tolerances": { "warn_degrees": warn_deg, "error_degrees": error_deg },
                            }),
                            score: max_angle_deg,
                        });
                    }
                }
                AiJointKindJson::Ball => {
                    // Generic rule: any joint channel should not stay far from neutral for the
                    // entire cycle unless it is intentionally spanning a broad arc.
                    maybe_push_joint_rest_bias_issue(
                        issues,
                        &component_id,
                        &component_name,
                        channel.as_str(),
                        min_angle_deg,
                        min_angle_phase,
                        max_angle_deg,
                        max_angle_phase,
                    );
                }
                AiJointKindJson::Free | AiJointKindJson::Unknown => {}
            }

            if matches!(joint.kind, AiJointKindJson::Hinge | AiJointKindJson::Ball) {
                let max_dim = comp
                    .planned_size
                    .x
                    .abs()
                    .max(comp.planned_size.y.abs())
                    .max(comp.planned_size.z.abs())
                    .max(0.01);
                let warn_ratio = 0.05;
                let warn_m = (max_dim * warn_ratio).max(0.01);
                if max_translation_m.is_finite() && max_translation_m > warn_m {
                    issues.push(MotionIssue {
                        severity: MotionSeverity::Warn,
                        kind: "constrained_joint_translates",
                        component_id: component_id.clone(),
                        component_name: component_name.clone(),
                        channel: channel.clone(),
                        message: "Constrained joint uses translation deltas; prefer rotation-only deltas to avoid sliding/gap artifacts."
                            .into(),
                        evidence: serde_json::json!({
                            "joint_kind": format!("{:?}", joint.kind),
                            "max_translation_m": max_translation_m,
                            "at_phase_01": max_translation_phase,
                            "component_max_dim_m": max_dim,
                            "tolerances": { "warn_ratio_of_max_dim": warn_ratio, "warn_translation_m": warn_m },
                        }),
                        score: max_translation_m,
                    });
                }
            }
        }
    }
}

fn validate_contacts(
    samples_t_m: &[f32],
    samples_phase_01: &[f32],
    cycle_m: f32,
    root_idx: usize,
    root_forward_xz: Vec3,
    components: &[Gen3dPlannedComponent],
    issues: &mut Vec<MotionIssue>,
) {
    let contacts_total: usize = components.iter().map(|c| c.contacts.len()).sum();
    if contacts_total == 0 {
        return;
    }

    let mut name_to_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (idx, c) in components.iter().enumerate() {
        name_to_idx.insert(c.name.as_str(), idx);
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (idx, c) in components.iter().enumerate() {
        let Some(att) = c.attach_to.as_ref() else {
            continue;
        };
        if let Some(parent_idx) = name_to_idx.get(att.parent.as_str()).copied() {
            children[parent_idx].push(idx);
        }
    }

    let world_per_sample: Vec<Vec<Transform>> = samples_t_m
        .iter()
        .map(|&t_m| compute_world_transforms_at_t(components, &children, root_idx, t_m))
        .collect();

    let slip_warn_m: f32 = (0.08 + 0.08 * cycle_m).clamp(0.12, 0.35);
    let lift_warn_m: f32 = (0.06 + 0.06 * cycle_m).clamp(0.10, 0.30);

    for (component_idx, comp) in components.iter().enumerate() {
        // Contact stance validation assumes the anchor is a planted point that should stay roughly
        // fixed in world space during stance. That is not true for rolling wheels: a single anchor
        // on the wheel rim will necessarily "slip" because the physical contact point moves around
        // the wheel as it rotates. If a component's `move` channel is a pure `spin`, skip stance
        // validation for its contacts.
        let move_is_spin = comp
            .attach_to
            .as_ref()
            .and_then(|att| find_move_slot(att))
            .map(|slot| matches!(slot.spec.clip, PartAnimationDef::Spin { .. }))
            .unwrap_or(false);

        for contact in comp.contacts.iter() {
            if contact.kind != AiContactKindJson::Ground {
                continue;
            }
            let Some(stance) = contact.stance.as_ref() else {
                continue;
            };
            if move_is_spin {
                continue;
            }

            let phase_start = if stance.phase_01.is_finite() {
                stance.phase_01.rem_euclid(1.0)
            } else {
                0.0
            };
            let duty = if stance.duty_factor_01.is_finite() {
                stance.duty_factor_01.clamp(0.0, 1.0)
            } else {
                0.0
            };
            if duty <= 1e-4 {
                continue;
            }

            let anchor_name = contact.anchor.trim();
            let anchor_local = anchor_transform_from_component(comp, anchor_name);

            let mut positions_world: Vec<Vec3> = Vec::with_capacity(samples_t_m.len());
            for (i, &t_m) in samples_t_m.iter().enumerate() {
                let component_world = world_per_sample
                    .get(i)
                    .and_then(|w| w.get(component_idx))
                    .copied()
                    .unwrap_or(Transform::IDENTITY);
                let p = component_world
                    .to_matrix()
                    .transform_point3(anchor_local.translation);
                let root_translation = root_forward_xz * t_m;
                positions_world.push(root_translation + p);
            }

            let stance_mid_phase = (phase_start + duty * 0.5).rem_euclid(1.0);
            let mut stance_indices: Vec<usize> = Vec::new();
            for (i, &phase) in samples_phase_01.iter().enumerate() {
                if phase_in_stance(phase, phase_start, duty) {
                    stance_indices.push(i);
                }
            }
            if stance_indices.len() < 2 {
                continue;
            }

            let baseline_idx = stance_indices
                .iter()
                .copied()
                .min_by(|&a, &b| {
                    let da = circular_distance(samples_phase_01[a], stance_mid_phase);
                    let db = circular_distance(samples_phase_01[b], stance_mid_phase);
                    da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or(stance_indices[0]);
            let baseline = positions_world[baseline_idx];
            let ground_y = baseline.y;

            let mut max_slip_m: f32 = 0.0;
            let mut max_slip_phase: f32 = 0.0;
            let mut max_lift_m: f32 = 0.0;
            let mut max_lift_phase: f32 = 0.0;

            for &i in &stance_indices {
                let p = positions_world[i];
                let dx = p.x - baseline.x;
                let dz = p.z - baseline.z;
                let slip = (dx * dx + dz * dz).sqrt();
                if slip > max_slip_m {
                    max_slip_m = slip;
                    max_slip_phase = samples_phase_01.get(i).copied().unwrap_or(0.0);
                }
                let lift = (p.y - ground_y).abs();
                if lift > max_lift_m {
                    max_lift_m = lift;
                    max_lift_phase = samples_phase_01.get(i).copied().unwrap_or(0.0);
                }
            }

            if max_slip_m.is_finite() && max_slip_m > slip_warn_m {
                let severity = if max_slip_m >= slip_warn_m * 2.0 {
                    MotionSeverity::Error
                } else {
                    MotionSeverity::Warn
                };
                issues.push(MotionIssue {
                    severity,
                    kind: "contact_slip",
                    component_id: component_id_uuid_for_name(&comp.name),
                    component_name: comp.name.clone(),
                    channel: "move".to_string(),
                    message:
                        "Contact slips too much during declared stance (foot should be mostly planted)."
                            .into(),
                    evidence: serde_json::json!({
                        "contact_name": contact.name.trim(),
                        "anchor": anchor_name,
                        "stance": { "phase_01": phase_start, "duty_factor_01": duty },
                        "cycle_m": cycle_m,
                        "slip_m": max_slip_m,
                        "at_phase_01": max_slip_phase,
                        "tolerances": { "warn_slip_m": slip_warn_m, "error_slip_m": slip_warn_m * 2.0 },
                    }),
                    score: max_slip_m,
                });
            }
            if max_lift_m.is_finite() && max_lift_m > lift_warn_m {
                let severity = if max_lift_m >= lift_warn_m * 2.0 {
                    MotionSeverity::Error
                } else {
                    MotionSeverity::Warn
                };
                issues.push(MotionIssue {
                    severity,
                    kind: "contact_lift",
                    component_id: component_id_uuid_for_name(&comp.name),
                    component_name: comp.name.clone(),
                    channel: "move".to_string(),
                    message:
                        "Contact lifts too much during declared stance (expected near-constant ground height)."
                            .into(),
                    evidence: serde_json::json!({
                        "contact_name": contact.name.trim(),
                        "anchor": anchor_name,
                        "stance": { "phase_01": phase_start, "duty_factor_01": duty },
                        "cycle_m": cycle_m,
                        "lift_m": max_lift_m,
                        "at_phase_01": max_lift_phase,
                        "tolerances": { "warn_lift_m": lift_warn_m, "error_lift_m": lift_warn_m * 2.0 },
                    }),
                    score: max_lift_m,
                });
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct PairDeltaSample {
    delta_m: f32,
    baseline_m: f32,
    attack_m: f32,
    at_phase_01: f32,
    at_time_secs: f32,
}

#[derive(Clone, Copy, Debug)]
struct Obb {
    center: Vec3,
    axes: [Vec3; 3],
    half_extents: Vec3,
}

fn validate_attack_self_intersection(
    root_idx: usize,
    components: &[Gen3dPlannedComponent],
    issues: &mut Vec<MotionIssue>,
) {
    if components.len() < 2 {
        return;
    }

    let has_attack_primary = components.iter().any(|c| {
        c.attach_to.as_ref().is_some_and(|att| {
            att.animations
                .iter()
                .any(|slot| slot.channel.as_ref() == "attack_primary")
        })
    });
    if !has_attack_primary {
        return;
    }

    let mut name_to_idx: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for (idx, c) in components.iter().enumerate() {
        name_to_idx.insert(c.name.as_str(), idx);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); components.len()];
    for (idx, c) in components.iter().enumerate() {
        let Some(att) = c.attach_to.as_ref() else {
            continue;
        };
        if let Some(p) = name_to_idx.get(att.parent.as_str()).copied() {
            children[p].push(idx);
        }
    }

    let has_attack_slot: Vec<bool> = components
        .iter()
        .map(|c| {
            c.attach_to.as_ref().is_some_and(|att| {
                att.animations
                    .iter()
                    .any(|slot| slot.channel.as_ref() == "attack_primary")
            })
        })
        .collect();

    let mut depth: Vec<usize> = vec![0; components.len()];
    let mut nearest_attack_ancestor: Vec<Option<usize>> = vec![None; components.len()];
    let mut influenced_by_attack: Vec<bool> = vec![false; components.len()];

    fn dfs_attack_influence(
        idx: usize,
        depth_here: usize,
        parent_influenced: bool,
        parent_nearest_attack: Option<usize>,
        children: &[Vec<usize>],
        has_attack_slot: &[bool],
        depth: &mut [usize],
        nearest_attack_ancestor: &mut [Option<usize>],
        influenced_by_attack: &mut [bool],
    ) {
        depth[idx] = depth_here;
        let nearest = if has_attack_slot[idx] {
            Some(idx)
        } else {
            parent_nearest_attack
        };
        let influenced = parent_influenced || has_attack_slot[idx];
        nearest_attack_ancestor[idx] = nearest;
        influenced_by_attack[idx] = influenced;

        for &child in children.get(idx).into_iter().flatten() {
            dfs_attack_influence(
                child,
                depth_here.saturating_add(1),
                influenced,
                nearest,
                children,
                has_attack_slot,
                depth,
                nearest_attack_ancestor,
                influenced_by_attack,
            );
        }
    }

    dfs_attack_influence(
        root_idx,
        0,
        false,
        None,
        &children,
        &has_attack_slot,
        &mut depth,
        &mut nearest_attack_ancestor,
        &mut influenced_by_attack,
    );

    let mut inferred_window_secs: Option<f32> = None;
    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        for slot in att.animations.iter() {
            if slot.channel.as_ref() != "attack_primary" {
                continue;
            }
            let PartAnimationDef::Loop { duration_secs, .. } = &slot.spec.clip else {
                continue;
            };
            if !duration_secs.is_finite() || *duration_secs <= 0.0 {
                continue;
            }
            let speed = slot.spec.speed_scale.max(1e-3);
            let wall_duration = (*duration_secs / speed).abs();
            if wall_duration.is_finite() && wall_duration > 1e-3 {
                inferred_window_secs =
                    Some(inferred_window_secs.map_or(wall_duration, |b| b.max(wall_duration)));
            }
        }
    }
    let attack_window_secs = inferred_window_secs
        .unwrap_or(DEFAULT_ATTACK_WINDOW_SECS)
        .max(1e-3);

    let sizes: Vec<Vec3> = components
        .iter()
        .map(|c| c.actual_size.unwrap_or(c.planned_size).abs().max(Vec3::splat(0.001)))
        .collect();
    let max_dims: Vec<f32> = sizes.iter().map(|s| s.max_element()).collect();

    let n = components.len();
    let mut pair_eps_m: Vec<f32> = vec![0.0; n * n];
    for i in 0..n {
        for j in (i + 1)..n {
            let scale = max_dims[i].min(max_dims[j]).max(1e-3);
            pair_eps_m[i * n + j] = (0.01 * scale).clamp(0.002, 0.05);
        }
    }

    let mut best: Vec<PairDeltaSample> = vec![PairDeltaSample::default(); n * n];
    let mut idle_world: Vec<Transform>;
    let mut attack_world: Vec<Transform>;
    let mut idle_obbs: Vec<Option<Obb>> = vec![None; n];
    let mut attack_obbs: Vec<Option<Obb>> = vec![None; n];

    for s in 0..ATTACK_SAMPLE_COUNT {
        let phase_01 = (s as f32) / (ATTACK_SAMPLE_COUNT as f32);
        let t_secs = phase_01 * attack_window_secs;

        idle_world = compute_world_transforms_for_channels(
            components,
            &children,
            root_idx,
            t_secs,
            0.0,
            0.0,
            0.0,
            false,
            false,
            true,
        );
        attack_world = compute_world_transforms_for_channels(
            components,
            &children,
            root_idx,
            t_secs,
            0.0,
            0.0,
            t_secs,
            true,
            false,
            false,
        );

        for idx in 0..n {
            idle_obbs[idx] = obb_from_transform_and_size(idle_world[idx], sizes[idx]);
            attack_obbs[idx] = obb_from_transform_and_size(attack_world[idx], sizes[idx]);
        }

        for i in 0..n {
            for j in (i + 1)..n {
                if nearest_attack_ancestor[i].is_none() && nearest_attack_ancestor[j].is_none() {
                    continue;
                }
                if !influenced_by_attack[i] && !influenced_by_attack[j] {
                    continue;
                }

                let Some(a_idle) = idle_obbs[i] else {
                    continue;
                };
                let Some(b_idle) = idle_obbs[j] else {
                    continue;
                };
                let Some(a_attack) = attack_obbs[i] else {
                    continue;
                };
                let Some(b_attack) = attack_obbs[j] else {
                    continue;
                };

                let baseline_m = obb_penetration_m(&a_idle, &b_idle);
                let attack_m = obb_penetration_m(&a_attack, &b_attack);
                let delta_m = attack_m - baseline_m;
                if !delta_m.is_finite() {
                    continue;
                }

                let idx_flat = i * n + j;
                if delta_m > best[idx_flat].delta_m {
                    best[idx_flat] = PairDeltaSample {
                        delta_m,
                        baseline_m,
                        attack_m,
                        at_phase_01: phase_01,
                        at_time_secs: t_secs,
                    };
                }
            }
        }
    }

    let mut candidates: Vec<(f32, usize, usize, usize, PairDeltaSample, f32)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let idx_flat = i * n + j;
            let sample = best[idx_flat];
            if sample.delta_m <= 0.0 {
                continue;
            }
            let eps_m = pair_eps_m[idx_flat];
            if sample.delta_m <= eps_m {
                continue;
            }

            let a = nearest_attack_ancestor[i];
            let b = nearest_attack_ancestor[j];
            let blame = match (a, b) {
                (Some(x), Some(y)) if x == y => Some(x),
                (Some(x), Some(y)) => {
                    let dx = depth[x];
                    let dy = depth[y];
                    if dx != dy {
                        Some(if dx > dy { x } else { y })
                    } else {
                        let nx = components[x].name.as_str();
                        let ny = components[y].name.as_str();
                        Some(if nx <= ny { x } else { y })
                    }
                }
                (Some(x), None) => Some(x),
                (None, Some(y)) => Some(y),
                (None, None) => None,
            };
            let Some(blame_idx) = blame else {
                continue;
            };

            candidates.push((
                sample.delta_m,
                blame_idx,
                i,
                j,
                sample,
                eps_m,
            ));
        }
    }

    candidates.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    for (delta_m, blame_idx, i, j, sample, eps_m) in candidates.into_iter().take(MAX_ISSUES) {
        let component_name = components[blame_idx].name.clone();
        issues.push(MotionIssue {
            // Self-intersection is often a cosmetic issue (and OBB tests can be conservative).
            // Keep reporting it, but do not block Gen3D acceptance or encourage "disable attack_primary"
            // fixes that remove all attack motion.
            severity: MotionSeverity::Warn,
            kind: "attack_self_intersection",
            component_id: component_id_uuid_for_name(&component_name),
            component_name,
            channel: "attack_primary".to_string(),
            message: "Attack animation increases self-intersection relative to idle pose."
                .into(),
            evidence: serde_json::json!({
                "attack_window_secs": attack_window_secs,
                "pair": {
                    "a": { "component_name": components[i].name.as_str(), "component_id": component_id_uuid_for_name(&components[i].name) },
                    "b": { "component_name": components[j].name.as_str(), "component_id": component_id_uuid_for_name(&components[j].name) },
                },
                "penetration_m": {
                    "baseline": sample.baseline_m,
                    "attack": sample.attack_m,
                    "delta": sample.delta_m,
                },
                "at": { "time_secs": sample.at_time_secs, "phase_01": sample.at_phase_01 },
                "tolerances": { "min_delta_m": eps_m },
            }),
            score: delta_m,
        });
    }
}

fn compute_world_transforms_for_channels(
    components: &[Gen3dPlannedComponent],
    children: &[Vec<usize>],
    root_idx: usize,
    wall_time_secs: f32,
    move_phase_m: f32,
    move_distance_m: f32,
    attack_elapsed_secs: f32,
    attacking_primary: bool,
    moving: bool,
    idle: bool,
) -> Vec<Transform> {
    let mut world: Vec<Transform> = vec![Transform::IDENTITY; components.len()];
    let mut visiting = vec![false; components.len()];
    let mut visited = vec![false; components.len()];
    world[root_idx] = Transform::IDENTITY;

    fn choose_slot<'a>(
        att: &'a Gen3dPlannedAttachment,
        attacking_primary: bool,
        moving: bool,
        idle: bool,
    ) -> Option<&'a PartAnimationSlot> {
        for channel in ["attack_primary", "move", "idle", "ambient"] {
            let active = match channel {
                "attack_primary" => attacking_primary,
                "move" => moving,
                "idle" => idle,
                "ambient" => true,
                _ => false,
            };
            if !active {
                continue;
            }
            if let Some(slot) = att
                .animations
                .iter()
                .find(|slot| slot.channel.as_ref() == channel)
            {
                return Some(slot);
            }
        }
        None
    }

    fn sample_slot_delta_runtime(
        slot: &PartAnimationSlot,
        wall_time_secs: f32,
        move_phase_m: f32,
        move_distance_m: f32,
        attack_elapsed_secs: f32,
    ) -> Transform {
        let driver_t = match slot.spec.driver {
            PartAnimationDriver::Always => wall_time_secs,
            PartAnimationDriver::MovePhase => move_phase_m,
            PartAnimationDriver::MoveDistance => move_distance_m,
            PartAnimationDriver::AttackTime => attack_elapsed_secs,
        };

        let mut t = if driver_t.is_finite() { driver_t } else { 0.0 };
        t *= slot.spec.speed_scale.max(0.0);
        if slot.spec.time_offset_units.is_finite() {
            t += slot.spec.time_offset_units;
        }
        sample_part_animation(&slot.spec.clip, t)
    }

    fn dfs(
        idx: usize,
        components: &[Gen3dPlannedComponent],
        children: &[Vec<usize>],
        wall_time_secs: f32,
        move_phase_m: f32,
        move_distance_m: f32,
        attack_elapsed_secs: f32,
        attacking_primary: bool,
        moving: bool,
        idle: bool,
        world: &mut [Transform],
        visiting: &mut [bool],
        visited: &mut [bool],
    ) {
        if visited[idx] || visiting[idx] {
            return;
        }
        visiting[idx] = true;

        let parent_world = world[idx];
        for &child_idx in &children[idx] {
            let Some(att) = components[child_idx].attach_to.as_ref() else {
                continue;
            };

            let parent_anchor =
                anchor_transform_from_component(&components[idx], att.parent_anchor.as_str());
            let child_anchor =
                anchor_transform_from_component(&components[child_idx], att.child_anchor.as_str());

            let mut animated_offset = att.offset;
            if let Some(slot) = choose_slot(att, attacking_primary, moving, idle) {
                let delta = sample_slot_delta_runtime(
                    slot,
                    wall_time_secs,
                    move_phase_m,
                    move_distance_m,
                    attack_elapsed_secs,
                );
                animated_offset = mul_transform(&animated_offset, &delta);
            }

            let inv_child = child_anchor.to_matrix().inverse();
            let composed = parent_world.to_matrix()
                * parent_anchor.to_matrix()
                * animated_offset.to_matrix()
                * inv_child;
            let (scale, rot, translation) = composed.to_scale_rotation_translation();
            if translation.is_finite() && rot.is_finite() && scale.is_finite() {
                world[child_idx] = Transform {
                    translation,
                    rotation: rot.normalize(),
                    scale,
                };
            } else {
                world[child_idx] = Transform::IDENTITY;
            }

            dfs(
                child_idx,
                components,
                children,
                wall_time_secs,
                move_phase_m,
                move_distance_m,
                attack_elapsed_secs,
                attacking_primary,
                moving,
                idle,
                world,
                visiting,
                visited,
            );
        }

        visiting[idx] = false;
        visited[idx] = true;
    }

    dfs(
        root_idx,
        components,
        children,
        wall_time_secs,
        move_phase_m,
        move_distance_m,
        attack_elapsed_secs,
        attacking_primary,
        moving,
        idle,
        &mut world,
        &mut visiting,
        &mut visited,
    );

    world
}

fn obb_from_transform_and_size(transform: Transform, size: Vec3) -> Option<Obb> {
    let mut half_extents = size * 0.5;
    if !half_extents.is_finite() {
        return None;
    }
    half_extents = half_extents.max(Vec3::splat(1e-4));

    let scale = if transform.scale.is_finite() {
        transform.scale.abs()
    } else {
        Vec3::ONE
    };
    half_extents *= scale;

    let rot = if transform.rotation.is_finite() {
        transform.rotation.normalize()
    } else {
        Quat::IDENTITY
    };

    let mut x = rot * Vec3::X;
    let mut y = rot * Vec3::Y;
    let mut z = rot * Vec3::Z;
    if x.length_squared() > 1e-8 {
        x = x.normalize();
    }
    if y.length_squared() > 1e-8 {
        y = y.normalize();
    }
    if z.length_squared() > 1e-8 {
        z = z.normalize();
    }

    let center = transform.translation;
    if !center.is_finite() {
        return None;
    }

    Some(Obb {
        center,
        axes: [x, y, z],
        half_extents,
    })
}

fn obb_projection_radius_m(obb: &Obb, axis: Vec3) -> f32 {
    let axis = if axis.length_squared() > 1e-12 {
        axis.normalize()
    } else {
        return 0.0;
    };
    let [ax, ay, az] = obb.axes;
    obb.half_extents.x * axis.dot(ax).abs()
        + obb.half_extents.y * axis.dot(ay).abs()
        + obb.half_extents.z * axis.dot(az).abs()
}

fn obb_penetration_m(a: &Obb, b: &Obb) -> f32 {
    let t = b.center - a.center;
    if !t.is_finite() {
        return 0.0;
    }

    let axes_a = a.axes;
    let axes_b = b.axes;
    let axes = [
        axes_a[0],
        axes_a[1],
        axes_a[2],
        axes_b[0],
        axes_b[1],
        axes_b[2],
        axes_a[0].cross(axes_b[0]),
        axes_a[0].cross(axes_b[1]),
        axes_a[0].cross(axes_b[2]),
        axes_a[1].cross(axes_b[0]),
        axes_a[1].cross(axes_b[1]),
        axes_a[1].cross(axes_b[2]),
        axes_a[2].cross(axes_b[0]),
        axes_a[2].cross(axes_b[1]),
        axes_a[2].cross(axes_b[2]),
    ];

    let mut min_overlap = f32::INFINITY;
    for axis in axes {
        if !axis.is_finite() || axis.length_squared() <= 1e-8 {
            continue;
        }
        let l = axis.normalize();
        let ra = obb_projection_radius_m(a, l);
        let rb = obb_projection_radius_m(b, l);
        let dist = t.dot(l).abs();
        let overlap = ra + rb - dist;
        if overlap <= 0.0 {
            return 0.0;
        }
        if overlap < min_overlap {
            min_overlap = overlap;
        }
    }

    if min_overlap.is_finite() {
        min_overlap.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::registry::{
        AnchorDef, PartAnimationKeyframeDef, PartAnimationSlot, PartAnimationSpec,
    };

    fn anchor(name: &str, pos: Vec3) -> AnchorDef {
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform::from_translation(pos),
        }
    }

    fn anchor_with_rot(name: &str, pos: Vec3, rot: Quat) -> AnchorDef {
        AnchorDef {
            name: name.to_string().into(),
            transform: Transform::from_translation(pos).with_rotation(rot),
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
            actual_size: Some(Vec3::ONE),
            anchors,
            contacts: Vec::new(),
            attach_to: None,
        }
    }

    #[test]
    fn chain_axis_mismatch_reports_error() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        // Forward axis at the proximal anchor points along +Y, but the chain axis points along +Z.
        let bad_rot = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);
        let mut link = stub_component(
            "link",
            vec![
                anchor_with_rot("prox", Vec3::new(0.0, 0.0, -0.5), bad_rot),
                anchor_with_rot("dist", Vec3::new(0.0, 0.0, 0.5), bad_rot),
            ],
        );
        link.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "prox".into(),
            offset: Transform::IDENTITY,
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Ball,
                axis_join: None,
                limits_degrees: None,
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: Vec::new(),
        });

        let mut child = stub_component("child", vec![anchor("prox", Vec3::ZERO)]);
        child.attach_to = Some(Gen3dPlannedAttachment {
            parent: "link".into(),
            parent_anchor: "dist".into(),
            child_anchor: "prox".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        let components = vec![root, link, child];
        let report = build_motion_validation_report(Some(1.0), &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        assert!(!ok, "expected motion validation to fail");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues
                .iter()
                .any(|i| i.get("kind").and_then(|v| v.as_str()) == Some("chain_axis_mismatch")),
            "expected chain_axis_mismatch issue, got {issues:?}"
        );
    }

    #[test]
    fn chain_axis_mismatch_is_not_reported_when_aligned() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let mut link = stub_component(
            "link",
            vec![
                anchor("prox", Vec3::new(0.0, 0.0, -0.5)),
                anchor("dist", Vec3::new(0.0, 0.0, 0.5)),
            ],
        );
        link.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "prox".into(),
            offset: Transform::IDENTITY,
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Ball,
                axis_join: None,
                limits_degrees: None,
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: Vec::new(),
        });

        let mut child = stub_component("child", vec![anchor("prox", Vec3::ZERO)]);
        child.attach_to = Some(Gen3dPlannedAttachment {
            parent: "link".into(),
            parent_anchor: "dist".into(),
            child_anchor: "prox".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: Vec::new(),
        });

        let components = vec![root, link, child];
        let report = build_motion_validation_report(Some(1.0), &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(ok, "expected motion validation to pass");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues
                .iter()
                .any(|i| { i.get("kind").and_then(|v| v.as_str()) == Some("chain_axis_mismatch") }),
            "expected no chain_axis_mismatch issue, got {issues:?}"
        );
    }

    #[test]
    fn hinge_off_axis_reports_error() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform {
                            rotation: Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
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
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Hinge,
                axis_join: Some([1.0, 0.0, 0.0]),
                limits_degrees: Some([-20.0, 20.0]),
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![PartAnimationSlot {
                channel: "move".into(),
                spec: move_spec,
            }],
        });

        let components = vec![root, limb];
        let report = build_motion_validation_report(Some(1.0), &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        assert!(!ok, "expected motion validation to fail");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues
                .iter()
                .any(|i| i.get("kind").and_then(|v| v.as_str()) == Some("hinge_off_axis")),
            "expected hinge_off_axis issue, got {issues:?}"
        );
    }

    #[test]
    fn hinge_off_axis_is_checked_for_idle_channel() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let idle_spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform {
                            rotation: Quat::from_rotation_y(core::f32::consts::FRAC_PI_2),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 2.0,
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
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Hinge,
                axis_join: Some([1.0, 0.0, 0.0]),
                limits_degrees: Some([-20.0, 20.0]),
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![PartAnimationSlot {
                channel: "idle".into(),
                spec: idle_spec,
            }],
        });

        let components = vec![root, limb];
        let report = build_motion_validation_report(Some(1.0), &components);
        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("hinge_off_axis")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
            }),
            "expected hinge_off_axis issue on idle channel, got {issues:?}"
        );
    }

    #[test]
    fn ball_joint_rest_bias_is_reported() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);
        let base_rot = Quat::from_rotation_x(-core::f32::consts::FRAC_PI_2);

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
                            rotation: (Quat::from_rotation_z(0.08) * base_rot).normalize(),
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

        let mut head = stub_component("head", vec![anchor("mount", Vec3::ZERO)]);
        head.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Ball,
                axis_join: None,
                limits_degrees: None,
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![PartAnimationSlot {
                channel: "idle".into(),
                spec: idle_spec,
            }],
        });

        let components = vec![root, head];
        let report = build_motion_validation_report(Some(1.0), &components);
        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("joint_rest_bias_large")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
            }),
            "expected joint_rest_bias_large issue on idle channel, got {issues:?}"
        );
    }

    #[test]
    fn constrained_joint_translation_is_warned() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let idle_spec = PartAnimationSpec {
            driver: PartAnimationDriver::Always,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform {
                            translation: Vec3::new(0.2, 0.0, 0.0),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 2.0,
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
            joint: Some(super::super::AiJointJson {
                kind: AiJointKindJson::Hinge,
                axis_join: Some([1.0, 0.0, 0.0]),
                limits_degrees: Some([-20.0, 20.0]),
                swing_limits_degrees: None,
                twist_limits_degrees: None,
            }),
            animations: vec![PartAnimationSlot {
                channel: "idle".into(),
                spec: idle_spec,
            }],
        });

        let components = vec![root, limb];
        let report = build_motion_validation_report(Some(1.0), &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(
            ok,
            "expected motion validation to pass with warn-only issues"
        );

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("constrained_joint_translates")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("idle")
                    && i.get("severity").and_then(|v| v.as_str()) == Some("warn")
            }),
            "expected constrained_joint_translates warn on idle channel, got {issues:?}"
        );
    }

    #[test]
    fn contact_slip_reports_error_when_large() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 2.0,
                keyframes: vec![PartAnimationKeyframeDef {
                    time_secs: 0.0,
                    delta: Transform::IDENTITY,
                }],
            },
        };

        let mut foot = stub_component(
            "foot",
            vec![anchor("mount", Vec3::ZERO), anchor("contact", Vec3::ZERO)],
        );
        foot.contacts.push(super::super::AiContactJson {
            name: "foot_contact".into(),
            anchor: "contact".into(),
            kind: AiContactKindJson::Ground,
            stance: Some(super::super::AiContactStanceJson {
                phase_01: 0.0,
                duty_factor_01: 0.5,
            }),
        });
        foot.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "move".into(),
                spec: move_spec,
            }],
        });

        let components = vec![root, foot];
        let report = build_motion_validation_report(Some(2.0), &components);
        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues
                .iter()
                .any(|i| i.get("kind").and_then(|v| v.as_str()) == Some("contact_slip")),
            "expected contact_slip issue, got {issues:?}"
        );
    }

    #[test]
    fn contact_stance_is_ignored_for_spin_move_components() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MoveDistance,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Spin {
                axis: Vec3::X,
                radians_per_unit: 4.0,
            },
        };

        let mut wheel = stub_component(
            "wheel",
            vec![
                anchor("mount", Vec3::ZERO),
                anchor("contact", Vec3::new(0.0, -0.3, 0.0)),
            ],
        );
        wheel.contacts.push(super::super::AiContactJson {
            name: "wheel_contact".into(),
            anchor: "contact".into(),
            kind: AiContactKindJson::Ground,
            stance: Some(super::super::AiContactStanceJson {
                phase_01: 0.0,
                duty_factor_01: 1.0,
            }),
        });
        wheel.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "move".into(),
                spec: move_spec,
            }],
        });

        let components = vec![root, wheel];
        let report = build_motion_validation_report(Some(1.0), &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(ok, "expected motion validation to pass for spin contacts");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues.iter().any(|i| matches!(
                i.get("kind").and_then(|v| v.as_str()),
                Some("contact_slip" | "contact_lift")
            )),
            "expected no contact slip/lift issues for spin move component, got {issues:?}"
        );
    }

    #[test]
    fn attack_self_intersection_is_reported() {
        let mut root = stub_component("root", Vec::new());
        root.planned_size = Vec3::splat(1.0);

        let attack_spec = PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.5,
                        delta: Transform::from_translation(Vec3::new(-1.0, 0.0, 0.0)),
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform::IDENTITY,
                    },
                ],
            },
        };

        let mut child = stub_component("child", Vec::new());
        child.planned_size = Vec3::splat(1.0);
        child.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
            offset: Transform::from_translation(Vec3::new(1.5, 0.0, 0.0)),
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "attack_primary".into(),
                spec: attack_spec,
            }],
        });

        let components = vec![root, child];
        let report = build_motion_validation_report(None, &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(ok, "expected motion validation to pass (warn-only issue)");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("attack_self_intersection")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("attack_primary")
                    && i.get("component_name").and_then(|v| v.as_str()) == Some("child")
                    && i.get("severity").and_then(|v| v.as_str()) == Some("warn")
            }),
            "expected attack_self_intersection issue, got {issues:?}"
        );
    }

    #[test]
    fn attack_self_intersection_is_not_reported_without_increase() {
        let mut root = stub_component("root", Vec::new());
        root.planned_size = Vec3::splat(1.0);

        let attack_spec = PartAnimationSpec {
            driver: PartAnimationDriver::AttackTime,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.0,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.5,
                        delta: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.0,
                        delta: Transform::IDENTITY,
                    },
                ],
            },
        };

        let mut child = stub_component("child", Vec::new());
        child.planned_size = Vec3::splat(1.0);
        // Idle pose already intersects; attack moves away, so there should be no "increased"
        // self-intersection relative to idle.
        child.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
            offset: Transform::from_translation(Vec3::new(0.5, 0.0, 0.0)),
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "attack_primary".into(),
                spec: attack_spec,
            }],
        });

        let components = vec![root, child];
        let report = build_motion_validation_report(None, &components);
        let ok = report
            .motion_validation
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        assert!(ok, "expected motion validation to pass");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("attack_self_intersection")
            }),
            "expected no attack_self_intersection issues, got {issues:?}"
        );
    }
}
