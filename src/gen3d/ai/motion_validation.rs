use bevy::prelude::*;
use uuid::Uuid;

use crate::object::registry::{builtin_object_id, PartAnimationDef, PartAnimationDriver};

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
    channel: &'static str,
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
const SAMPLE_COUNT: usize = 24;
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
    let t = driver_t * move_slot.spec.speed_scale.max(0.0);
    sample_part_animation(&move_slot.spec.clip, t)
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
    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(joint) = att.joint.as_ref() else {
            continue;
        };
        let Some(move_slot) = find_move_slot(att) else {
            continue;
        };
        if !matches!(
            move_slot.spec.driver,
            PartAnimationDriver::MovePhase | PartAnimationDriver::MoveDistance
        ) {
            continue;
        }

        let component_name = comp.name.clone();
        let component_id = component_id_uuid_for_name(&component_name);

        match joint.kind {
            AiJointKindJson::Hinge => {
                let Some(axis_join_arr) = joint.axis_join else {
                    issues.push(MotionIssue {
                        severity: MotionSeverity::Error,
                        kind: "hinge_axis_missing",
                        component_id,
                        component_name,
                        channel: "move",
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
                        component_id,
                        component_name,
                        channel: "move",
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
                let axis_join = axis_join.normalize();

                let mut max_off_axis_deg: f32 = 0.0;
                let mut max_off_axis_phase: f32 = 0.0;
                let mut max_abs_hinge_angle_deg: f32 = 0.0;
                let mut max_abs_hinge_angle_phase: f32 = 0.0;
                let mut max_limit_exceed_deg: f32 = 0.0;
                let mut max_limit_exceed_phase: f32 = 0.0;

                for (i, &t_m) in samples_t_m.iter().enumerate() {
                    let delta = sample_move_delta(move_slot, t_m);
                    let animated_offset = mul_transform(&att.offset, &delta);
                    let q_delta =
                        (att.offset.rotation.inverse() * animated_offset.rotation).normalize();
                    let (hinge_angle_deg, off_axis_deg) =
                        hinge_signed_angle_and_off_axis_deg(q_delta, axis_join);
                    let phase = samples_phase_01.get(i).copied().unwrap_or(0.0);

                    if off_axis_deg > max_off_axis_deg {
                        max_off_axis_deg = off_axis_deg;
                        max_off_axis_phase = phase;
                    }
                    if hinge_angle_deg.abs() > max_abs_hinge_angle_deg {
                        max_abs_hinge_angle_deg = hinge_angle_deg.abs();
                        max_abs_hinge_angle_phase = phase;
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
                            max_limit_exceed_phase = phase;
                        }
                    }
                }

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
                        component_id: component_id_uuid_for_name(&comp.name),
                        component_name: comp.name.clone(),
                        channel: "move",
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
                        component_id,
                        component_name,
                        channel: "move",
                        message: "Hinge joint motion exceeds declared limits.".into(),
                        evidence: serde_json::json!({
                            "limits_degrees": joint.limits_degrees,
                            "max_exceed_degrees": max_limit_exceed_deg,
                            "at_phase_01": max_limit_exceed_phase,
                        }),
                        score: max_limit_exceed_deg,
                    });
                }
            }
            AiJointKindJson::Fixed => {
                let mut max_angle_deg: f32 = 0.0;
                let mut max_phase: f32 = 0.0;
                for (i, &t_m) in samples_t_m.iter().enumerate() {
                    let delta = sample_move_delta(move_slot, t_m);
                    let animated_offset = mul_transform(&att.offset, &delta);
                    let q_delta =
                        (att.offset.rotation.inverse() * animated_offset.rotation).normalize();
                    let angle_deg = quat_angle_deg(q_delta).abs();
                    if angle_deg > max_angle_deg {
                        max_angle_deg = angle_deg;
                        max_phase = samples_phase_01.get(i).copied().unwrap_or(0.0);
                    }
                }
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
                        component_id,
                        component_name,
                        channel: "move",
                        message:
                            "Fixed joint rotates under `move` animation (expected no rotation)."
                                .into(),
                        evidence: serde_json::json!({
                            "max_angle_degrees": max_angle_deg,
                            "at_phase_01": max_phase,
                            "tolerances": { "warn_degrees": warn_deg, "error_degrees": error_deg },
                        }),
                        score: max_angle_deg,
                    });
                }
            }
            AiJointKindJson::Ball | AiJointKindJson::Free | AiJointKindJson::Unknown => {}
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
                    channel: "move",
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
                    channel: "move",
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
    fn hinge_off_axis_reports_error() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
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
    fn contact_slip_reports_error_when_large() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
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
}
