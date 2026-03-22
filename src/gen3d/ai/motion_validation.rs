use bevy::prelude::*;
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
    PartAnimationSlot, PartAnimationSpinAxisSpace,
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
    validate_time_offset_effectiveness(components, &mut issues);
    validate_move_phase_cycle_alignment(cycle_m, cycle_source, root_idx, components, &mut issues);
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

pub(super) fn build_motion_metrics_report_v1(
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
    sample_count: usize,
) -> serde_json::Value {
    fn finite_f32(v: f32) -> Option<f32> {
        v.is_finite().then_some(v)
    }

    fn vec3_json(v: Vec3) -> Option<[f32; 3]> {
        v.is_finite().then_some([v.x, v.y, v.z])
    }

    fn stats_json(values: &[f32]) -> serde_json::Value {
        let mut min_v = f32::INFINITY;
        let mut max_v = f32::NEG_INFINITY;
        let mut sum_v = 0.0;
        let mut count: u32 = 0;

        for &v in values {
            if !v.is_finite() {
                continue;
            }
            min_v = min_v.min(v);
            max_v = max_v.max(v);
            sum_v += v;
            count = count.saturating_add(1);
        }

        if count == 0 || !min_v.is_finite() || !max_v.is_finite() {
            return serde_json::json!({ "count": 0 });
        }

        let mean = (sum_v / count as f32).max(0.0);
        serde_json::json!({
            "count": count,
            "min": min_v.max(0.0),
            "max": max_v.max(0.0),
            "mean": mean,
        })
    }

    let sample_count = sample_count.clamp(8, 256);

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

    let (cycle_m, cycle_source) = infer_cycle_m(rig_move_cycle_m, components);
    let cycle_m = cycle_m.max(1e-3);

    let samples_t_m: Vec<f32> = (0..sample_count)
        .map(|i| (i as f32 / sample_count as f32) * cycle_m)
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
    let mut root_right_xz = Vec3::Y.cross(root_forward_xz);
    if !root_right_xz.is_finite() || root_right_xz.length_squared() <= 1e-6 {
        root_right_xz = Vec3::X;
    } else {
        root_right_xz = root_right_xz.normalize();
    }

    let rig_max_dim_m: f32 = components
        .iter()
        .map(|c| c.planned_size.abs().max_element())
        .filter(|v| v.is_finite())
        .fold(0.0_f32, |acc, v| acc.max(v))
        .max(0.01_f32);

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

    let mut contacts_ground_total: usize = 0;
    let mut contacts_ground_with_stance: usize = 0;

    let mut stance_slip_max_xz_values: Vec<f32> = Vec::new();
    let mut stance_lift_max_values: Vec<f32> = Vec::new();
    let mut root_frame_forward_range_values: Vec<f32> = Vec::new();

    let mut ground_contacts: Vec<serde_json::Value> = Vec::new();

    for (component_idx, comp) in components.iter().enumerate() {
        // Stance metrics assume the contact point is planted in world space during stance. That's
        // not true for rolling wheels/rollers where a rim anchor rotates around the hub.
        let move_is_spin = comp
            .attach_to
            .as_ref()
            .and_then(find_move_slot)
            .map(|slot| matches!(slot.spec.clip, PartAnimationDef::Spin { .. }))
            .unwrap_or(false);

        for contact in comp.contacts.iter() {
            if contact.kind != AiContactKindJson::Ground {
                continue;
            }
            contacts_ground_total = contacts_ground_total.saturating_add(1);

            let anchor_name = contact.anchor.trim();
            if anchor_name.is_empty() {
                continue;
            }

            let anchor_local = anchor_transform_from_component(comp, anchor_name);

            let mut positions_root: Vec<Vec3> = Vec::with_capacity(sample_count);
            let mut positions_world: Vec<Vec3> = Vec::with_capacity(sample_count);

            for (i, &t_m) in samples_t_m.iter().enumerate() {
                let component_world = world_per_sample
                    .get(i)
                    .and_then(|w| w.get(component_idx))
                    .copied()
                    .unwrap_or(Transform::IDENTITY);
                let p_root = component_world
                    .to_matrix()
                    .transform_point3(anchor_local.translation);
                positions_root.push(p_root);
                positions_world.push(root_forward_xz * t_m + p_root);
            }

            let mut f_min = f32::INFINITY;
            let mut f_max = f32::NEG_INFINITY;
            let mut r_min = f32::INFINITY;
            let mut r_max = f32::NEG_INFINITY;
            let mut y_min = f32::INFINITY;
            let mut y_max = f32::NEG_INFINITY;

            for p in &positions_root {
                if !p.is_finite() {
                    continue;
                }
                let f = p.dot(root_forward_xz);
                let r = p.dot(root_right_xz);
                let y = p.y;
                if f.is_finite() {
                    f_min = f_min.min(f);
                    f_max = f_max.max(f);
                }
                if r.is_finite() {
                    r_min = r_min.min(r);
                    r_max = r_max.max(r);
                }
                if y.is_finite() {
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }

            let forward_range_m = if f_min.is_finite() && f_max.is_finite() {
                (f_max - f_min).abs()
            } else {
                0.0
            };
            let right_range_m = if r_min.is_finite() && r_max.is_finite() {
                (r_max - r_min).abs()
            } else {
                0.0
            };
            let up_range_m = if y_min.is_finite() && y_max.is_finite() {
                (y_max - y_min).abs()
            } else {
                0.0
            };
            if forward_range_m.is_finite() {
                root_frame_forward_range_values.push(forward_range_m);
            }

            let stance_json = contact.stance.as_ref().map(|s| {
                serde_json::json!({
                    "phase_01": finite_f32(s.phase_01),
                    "duty_factor_01": finite_f32(s.duty_factor_01),
                })
            });

            let mut stance_metrics_json: Option<serde_json::Value> = None;
            if let Some(stance) = contact.stance.as_ref().filter(|_| !move_is_spin) {
                contacts_ground_with_stance = contacts_ground_with_stance.saturating_add(1);

                let phase_start = if stance.phase_01.is_finite() {
                    stance.phase_01.rem_euclid(1.0)
                } else {
                    0.0
                };
                let duty = if stance.duty_factor_01.is_finite() {
                    stance.duty_factor_01.clamp(0.0, 1.0)
                } else {
                    0.5
                };

                let stance_mid_phase = (phase_start + duty * 0.5).rem_euclid(1.0);
                let mut stance_indices: Vec<usize> = Vec::new();
                for (i, &phase) in samples_phase_01.iter().enumerate() {
                    if phase_in_stance(phase, phase_start, duty) {
                        stance_indices.push(i);
                    }
                }

                if stance_indices.len() >= 2 {
                    let baseline_idx = stance_indices
                        .iter()
                        .copied()
                        .min_by(|&a, &b| {
                            let da = circular_distance(samples_phase_01[a], stance_mid_phase);
                            let db = circular_distance(samples_phase_01[b], stance_mid_phase);
                            da.partial_cmp(&db).unwrap_or(std::cmp::Ordering::Equal)
                        })
                        .unwrap_or(stance_indices[0]);
                    let baseline_world = positions_world[baseline_idx];
                    let baseline_phase_01 =
                        samples_phase_01.get(baseline_idx).copied().unwrap_or(0.0);
                    let baseline_t_m = samples_t_m.get(baseline_idx).copied().unwrap_or(0.0);

                    let mut max_lift_m: f32 = 0.0;
                    let mut max_lift_phase_01: f32 = baseline_phase_01;
                    let mut max_slip_xz_m: f32 = 0.0;
                    let mut max_slip_phase_01: f32 = baseline_phase_01;

                    let mut max_slip_forward_m: f32 = 0.0;
                    let mut max_slip_right_m: f32 = 0.0;

                    for &i in &stance_indices {
                        let p = positions_world[i];
                        if !p.is_finite() || !baseline_world.is_finite() {
                            continue;
                        }
                        let delta = p - baseline_world;
                        let lift = (p.y - baseline_world.y).abs();
                        if lift.is_finite() && lift > max_lift_m {
                            max_lift_m = lift;
                            max_lift_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
                        }

                        let dxz = Vec2::new(delta.x, delta.z);
                        let slip_xz = dxz.length();
                        if slip_xz.is_finite() && slip_xz > max_slip_xz_m {
                            max_slip_xz_m = slip_xz;
                            max_slip_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
                        }

                        let slip_forward = delta.dot(root_forward_xz).abs();
                        if slip_forward.is_finite() && slip_forward > max_slip_forward_m {
                            max_slip_forward_m = slip_forward;
                        }
                        let slip_right = delta.dot(root_right_xz).abs();
                        if slip_right.is_finite() && slip_right > max_slip_right_m {
                            max_slip_right_m = slip_right;
                        }
                    }

                    if max_slip_xz_m.is_finite() {
                        stance_slip_max_xz_values.push(max_slip_xz_m);
                    }
                    if max_lift_m.is_finite() {
                        stance_lift_max_values.push(max_lift_m);
                    }

                    stance_metrics_json = Some(serde_json::json!({
                        "stance_phase_01": phase_start,
                        "stance_duty_factor_01": duty,
                        "baseline": {
                            "phase_01": finite_f32(baseline_phase_01),
                            "t_m": finite_f32(baseline_t_m),
                            "pos_world_m": vec3_json(baseline_world),
                        },
                        "lift_max_m": finite_f32(max_lift_m),
                        "lift_at_phase_01": finite_f32(max_lift_phase_01),
                        "slip_max_m_xz": finite_f32(max_slip_xz_m),
                        "slip_at_phase_01": finite_f32(max_slip_phase_01),
                        "slip_max_m_forward": finite_f32(max_slip_forward_m),
                        "slip_max_m_right": finite_f32(max_slip_right_m),
                    }));
                }
            }

            ground_contacts.push(serde_json::json!({
                "component": comp.name.as_str(),
                "component_id": component_id_uuid_for_name(&comp.name),
                "contact": contact.name.trim(),
                "anchor": anchor_name,
                "move_is_spin": move_is_spin,
                "stance": stance_json,
                "root_frame": {
                    "forward_range_m": finite_f32(forward_range_m),
                    "right_range_m": finite_f32(right_range_m),
                    "up_range_m": finite_f32(up_range_m),
                    "forward_range_fraction_of_cycle": finite_f32(forward_range_m / cycle_m),
                    "cycle_fraction_of_rig_max_dim": finite_f32(cycle_m / rig_max_dim_m),
                },
                "stance_metrics": stance_metrics_json,
            }));
        }
    }

    serde_json::json!({
        "version": 1,
        "rig_summary": {
            "cycle_m": cycle_m,
            "cycle_source": cycle_source,
            "sample_count": sample_count,
            "root_forward_xz": vec3_json(root_forward_xz),
            "root_right_xz": vec3_json(root_right_xz),
            "joints_total": joints_total,
            "contacts_total": contacts_total,
            "contacts_ground_total": contacts_ground_total,
            "contacts_ground_with_stance": contacts_ground_with_stance,
            "rig_max_dim_m": rig_max_dim_m,
        },
        "definitions": {
            "root_frame.forward_range_m": "Range of the contact anchor position along root +Z (forward) over one move cycle, measured in root frame (visual step excursion relative to the body).",
            "stance_metrics.slip_max_m_xz": "Max horizontal drift of the contact point in WORLD space during its stance window (ideal planted contact ~= 0).",
            "stance_metrics.lift_max_m": "Max vertical lift of the contact point in WORLD space during its stance window (ideal planted contact ~= 0).",
        },
        "summary": {
            "stance_slip_max_m_xz": stats_json(&stance_slip_max_xz_values),
            "stance_lift_max_m": stats_json(&stance_lift_max_values),
            "root_frame_forward_range_m": stats_json(&root_frame_forward_range_values),
        },
        "ground_contacts": ground_contacts,
    })
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
            proximal_tf.rotation.normalize() * Vec3::Z
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
            severity: MotionSeverity::Warn,
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
            return (effective, "move.loop.duration_secs");
        }
    }

    (DEFAULT_CYCLE_M, "default")
}

fn validate_move_phase_cycle_alignment(
    cycle_m: f32,
    cycle_source: &str,
    root_idx: usize,
    components: &[Gen3dPlannedComponent],
    issues: &mut Vec<MotionIssue>,
) {
    const MIN_RATIO_OK: f32 = 0.5;
    const MAX_RATIO_OK: f32 = 2.0;

    #[derive(Clone)]
    struct LoopInfo {
        component_name: String,
        clip_kind: &'static str,
        duration_units: f32,
        speed_scale: f32,
        repeats: f32,
        effective_loop_m: f32,
        ratio_to_cycle_m: f32,
    }

    if cycle_source != "rig.move_cycle_m" {
        return;
    }
    if !cycle_m.is_finite() || cycle_m <= 1e-3 {
        return;
    }
    let root_name = components
        .get(root_idx)
        .map(|c| c.name.as_str())
        .unwrap_or("root");

    let mut loops: Vec<LoopInfo> = Vec::new();
    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(move_slot) = find_move_slot(att) else {
            continue;
        };
        if move_slot.spec.driver != PartAnimationDriver::MovePhase {
            continue;
        }

        let (duration_units, repeats, clip_kind) = match &move_slot.spec.clip {
            PartAnimationDef::Loop { duration_secs, .. } => (*duration_secs, 1.0, "loop"),
            PartAnimationDef::PingPong { duration_secs, .. } => (*duration_secs, 2.0, "ping_pong"),
            PartAnimationDef::Once { .. } => continue,
            PartAnimationDef::Spin { .. } => continue,
        };
        if !duration_units.is_finite() || duration_units <= 1e-6 {
            continue;
        }
        let speed_scale = move_slot.spec.speed_scale.max(0.0);
        if !speed_scale.is_finite() || speed_scale <= 1e-6 {
            continue;
        }

        let effective_loop_m = (repeats * duration_units / speed_scale).abs();
        if !effective_loop_m.is_finite() || effective_loop_m <= 1e-6 {
            continue;
        }
        let ratio_to_cycle_m = (effective_loop_m / cycle_m).abs();
        if !ratio_to_cycle_m.is_finite() || ratio_to_cycle_m <= 1e-6 {
            continue;
        }

        loops.push(LoopInfo {
            component_name: comp.name.clone(),
            clip_kind,
            duration_units,
            speed_scale,
            repeats,
            effective_loop_m,
            ratio_to_cycle_m,
        });
    }

    if loops.len() < 2 {
        return;
    }

    let mut effective: Vec<f32> = loops.iter().map(|v| v.effective_loop_m).collect();
    effective.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_effective = if effective.len() % 2 == 1 {
        effective[effective.len() / 2]
    } else {
        let i = effective.len() / 2;
        (effective[i - 1] + effective[i]) * 0.5
    };
    let median_ratio = (median_effective / cycle_m).abs();

    if median_ratio.is_finite() && median_ratio >= MIN_RATIO_OK && median_ratio <= MAX_RATIO_OK {
        return;
    }

    loops.sort_by(|a, b| {
        a.effective_loop_m
            .partial_cmp(&b.effective_loop_m)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let fastest: Vec<serde_json::Value> = loops
        .iter()
        .take(4)
        .map(|v| {
            serde_json::json!({
                "component_name": v.component_name,
                "clip_kind": v.clip_kind,
                "duration_units": v.duration_units,
                "speed_scale": v.speed_scale,
                "repeats_per_loop": v.repeats,
                "effective_loop_m": v.effective_loop_m,
                "ratio_to_cycle_m": v.ratio_to_cycle_m,
                "loops_per_cycle": (cycle_m / v.effective_loop_m).abs(),
            })
        })
        .collect();
    let slowest: Vec<serde_json::Value> = loops
        .iter()
        .rev()
        .take(4)
        .map(|v| {
            serde_json::json!({
                "component_name": v.component_name,
                "clip_kind": v.clip_kind,
                "duration_units": v.duration_units,
                "speed_scale": v.speed_scale,
                "repeats_per_loop": v.repeats,
                "effective_loop_m": v.effective_loop_m,
                "ratio_to_cycle_m": v.ratio_to_cycle_m,
                "loops_per_cycle": (cycle_m / v.effective_loop_m).abs(),
            })
        })
        .collect();

    let score = if median_ratio.is_finite() && median_ratio > 0.0 {
        if median_ratio < 1.0 {
            1.0 / median_ratio
        } else {
            median_ratio
        }
    } else {
        0.0
    };

    issues.push(MotionIssue {
        severity: MotionSeverity::Warn,
        kind: "move_phase_cycle_m_mismatch",
        component_id: component_id_uuid_for_name(root_name),
        component_name: root_name.to_string(),
        channel: "move".to_string(),
        message: "MovePhase move loops appear out of scale with rig.move_cycle_m (MovePhase time is meters traveled); motion may oscillate too fast/slow.".into(),
        evidence: serde_json::json!({
            "cycle_m": cycle_m,
            "cycle_source": cycle_source,
            "median_effective_loop_m": median_effective,
            "median_ratio_to_cycle_m": median_ratio,
            "move_phase_loops_count": loops.len(),
            "fastest_edges": fastest,
            "slowest_edges": slowest,
            "tolerances": { "ratio_min_ok": MIN_RATIO_OK, "ratio_max_ok": MAX_RATIO_OK },
            "notes": "For MovePhase loops, effective meters-per-loop is roughly (repeats * duration_units / speed_scale).",
        }),
        score,
    });
}

fn validate_time_offset_effectiveness(
    components: &[Gen3dPlannedComponent],
    issues: &mut Vec<MotionIssue>,
) {
    const OFFSET_EPS: f32 = 1e-4;
    const MIN_OFFSET_FRACTION: f32 = 0.10;
    const MIN_OFFSET_ABS_UNITS: f32 = 0.01;

    const MIN_MOTION_ROT_DEG: f32 = 5.0;
    const MIN_MOTION_TRANSL_M: f32 = 0.005;
    const MIN_MOTION_SCALE: f32 = 0.005;

    const MAX_NO_EFFECT_ROT_DEG: f32 = 1.0;
    const MAX_NO_EFFECT_TRANSL_M: f32 = 0.002;
    const MAX_NO_EFFECT_SCALE: f32 = 0.002;

    fn safe_transform(t: Transform) -> Transform {
        let translation = if t.translation.is_finite() {
            t.translation
        } else {
            Vec3::ZERO
        };
        let rotation = if t.rotation.is_finite() {
            t.rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        let scale = if t.scale.is_finite() {
            t.scale
        } else {
            Vec3::ONE
        };
        Transform {
            translation,
            rotation,
            scale,
        }
    }

    fn rotation_diff_deg(a: Quat, b: Quat) -> f32 {
        let a = if a.is_finite() {
            a.normalize()
        } else {
            Quat::IDENTITY
        };
        let b = if b.is_finite() {
            b.normalize()
        } else {
            Quat::IDENTITY
        };
        quat_angle_deg((a.inverse() * b).normalize()).abs()
    }

    fn build_sample_times(
        duration: f32,
        keyframes: &[crate::object::registry::PartAnimationKeyframeDef],
    ) -> Vec<f32> {
        if !duration.is_finite() || duration <= 0.0 {
            return Vec::new();
        }
        let mut times: Vec<f32> = Vec::new();
        for i in 0..SAMPLE_COUNT {
            times.push((i as f32 / SAMPLE_COUNT as f32) * duration);
        }

        let mut key_times: Vec<f32> = keyframes
            .iter()
            .filter_map(|k| {
                k.time_secs
                    .is_finite()
                    .then_some(k.time_secs.rem_euclid(duration))
            })
            .collect();
        key_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        key_times.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);

        times.extend(key_times.iter().copied());
        for (idx, t0) in key_times.iter().copied().enumerate() {
            let t1 = if idx + 1 < key_times.len() {
                key_times[idx + 1]
            } else {
                key_times[0] + duration
            };
            times.push(((t0 + t1) * 0.5).rem_euclid(duration));
        }

        times.retain(|t| t.is_finite());
        times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        times.dedup_by(|a, b| (*a - *b).abs() <= 1e-4);
        times
    }

    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        if att.animations.is_empty() {
            continue;
        }

        let component_name = comp.name.clone();
        let component_id = component_id_uuid_for_name(&component_name);

        for slot in att.animations.iter() {
            let channel = slot.channel.as_ref().trim();
            if channel.is_empty() {
                continue;
            }

            let time_offset = slot.spec.time_offset_units;
            if !time_offset.is_finite() || time_offset.abs() <= OFFSET_EPS {
                continue;
            }

            let PartAnimationDef::Loop {
                duration_secs,
                keyframes,
            } = &slot.spec.clip
            else {
                continue;
            };

            if !duration_secs.is_finite() || *duration_secs <= 0.0 {
                continue;
            }
            let duration = duration_secs.max(1e-6);
            let offset_mod = time_offset.rem_euclid(duration);
            let offset_dist = offset_mod.min(duration - offset_mod);
            let min_offset = (duration * MIN_OFFSET_FRACTION).max(MIN_OFFSET_ABS_UNITS);
            if !offset_dist.is_finite() || offset_dist < min_offset {
                continue;
            }

            let sample_times = build_sample_times(duration, keyframes);
            if sample_times.is_empty() {
                continue;
            }

            let base = safe_transform(sample_part_animation(&slot.spec.clip, 0.0));
            let mut max_motion_rot_deg: f32 = 0.0;
            let mut max_motion_trans_m: f32 = 0.0;
            let mut max_motion_scale: f32 = 0.0;

            let mut max_effect_rot_deg: f32 = 0.0;
            let mut max_effect_trans_m: f32 = 0.0;
            let mut max_effect_scale: f32 = 0.0;

            for &t in &sample_times {
                let a = safe_transform(sample_part_animation(&slot.spec.clip, t));
                let b = safe_transform(sample_part_animation(&slot.spec.clip, t + offset_mod));

                let motion_rot = rotation_diff_deg(base.rotation, a.rotation);
                let motion_trans = (base.translation - a.translation).length();
                let motion_scale = (base.scale - a.scale).length();

                if motion_rot.is_finite() && motion_rot > max_motion_rot_deg {
                    max_motion_rot_deg = motion_rot;
                }
                if motion_trans.is_finite() && motion_trans > max_motion_trans_m {
                    max_motion_trans_m = motion_trans;
                }
                if motion_scale.is_finite() && motion_scale > max_motion_scale {
                    max_motion_scale = motion_scale;
                }

                let effect_rot = rotation_diff_deg(a.rotation, b.rotation);
                let effect_trans = (a.translation - b.translation).length();
                let effect_scale = (a.scale - b.scale).length();

                if effect_rot.is_finite() && effect_rot > max_effect_rot_deg {
                    max_effect_rot_deg = effect_rot;
                }
                if effect_trans.is_finite() && effect_trans > max_effect_trans_m {
                    max_effect_trans_m = effect_trans;
                }
                if effect_scale.is_finite() && effect_scale > max_effect_scale {
                    max_effect_scale = effect_scale;
                }
            }

            // Skip near-static clips to avoid spamming repairs when time_offset is irrelevant.
            if max_motion_rot_deg < MIN_MOTION_ROT_DEG
                && max_motion_trans_m < MIN_MOTION_TRANSL_M
                && max_motion_scale < MIN_MOTION_SCALE
            {
                continue;
            }

            if max_effect_rot_deg < MAX_NO_EFFECT_ROT_DEG
                && max_effect_trans_m < MAX_NO_EFFECT_TRANSL_M
                && max_effect_scale < MAX_NO_EFFECT_SCALE
            {
                let score = max_motion_rot_deg
                    .max(max_motion_trans_m * 100.0)
                    .max(max_motion_scale * 100.0);
                issues.push(MotionIssue {
                    severity: MotionSeverity::Error,
                    kind: "time_offset_no_effect",
                    component_id: component_id.clone(),
                    component_name: component_name.clone(),
                    channel: channel.to_string(),
                    message: "Animation time_offset_units has no effect: shifting the loop by the configured offset produces (nearly) identical deltas. This can cause duplicated/in-phase limbs even when an offset is set."
                        .into(),
                    evidence: serde_json::json!({
                        "clip_kind": "loop",
                        "duration_units": duration,
                        "time_offset_units": time_offset,
                        "time_offset_mod_units": offset_mod,
                        "offset_distance_units": offset_dist,
                        "offset_fraction_of_duration": offset_dist / duration,
                        "sample_count": sample_times.len(),
                        "max_motion": {
                            "rot_degrees": max_motion_rot_deg,
                            "translation_m": max_motion_trans_m,
                            "scale": max_motion_scale,
                        },
                        "max_effect": {
                            "rot_degrees": max_effect_rot_deg,
                            "translation_m": max_effect_trans_m,
                            "scale": max_effect_scale,
                        },
                        "tolerances": {
                            "min_offset_fraction_of_duration": MIN_OFFSET_FRACTION,
                            "min_offset_abs_units": MIN_OFFSET_ABS_UNITS,
                            "min_motion_rot_degrees": MIN_MOTION_ROT_DEG,
                            "min_motion_translation_m": MIN_MOTION_TRANSL_M,
                            "min_motion_scale": MIN_MOTION_SCALE,
                            "max_no_effect_rot_degrees": MAX_NO_EFFECT_ROT_DEG,
                            "max_no_effect_translation_m": MAX_NO_EFFECT_TRANSL_M,
                            "max_no_effect_scale": MAX_NO_EFFECT_SCALE,
                        },
                    }),
                    score,
                });
            }
        }
    }
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

fn apply_child_local_delta_to_attachment_offset(
    base_offset: Transform,
    child_anchor: Transform,
    delta_child_local: Transform,
) -> Transform {
    let child_anchor_mat = child_anchor.to_matrix();
    let inv_child_anchor = child_anchor_mat.inverse();
    if !inv_child_anchor.is_finite() {
        return mul_transform(&base_offset, &delta_child_local);
    }

    // `offset` is applied as: parent_anchor * offset * inv(child_anchor).
    // If we want a delta in the CHILD's local frame, the desired composition is:
    //   parent_anchor * offset * inv(child_anchor) * delta_child_local
    // Rebase into the offset slot:
    //   offset' = offset * inv(child_anchor) * delta * child_anchor
    let rebased_mat = inv_child_anchor * delta_child_local.to_matrix() * child_anchor_mat;
    let rebased = crate::geometry::mat4_to_transform_allow_degenerate_scale(rebased_mat)
        .unwrap_or(delta_child_local);
    mul_transform(&base_offset, &rebased)
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
                    animated_offset = match &move_slot.spec.clip {
                        PartAnimationDef::Spin {
                            axis_space: PartAnimationSpinAxisSpace::ChildLocal,
                            ..
                        } => apply_child_local_delta_to_attachment_offset(
                            animated_offset,
                            child_anchor,
                            delta,
                        ),
                        _ => mul_transform(&animated_offset, &delta),
                    };
                }
            }

            let inv_child = child_anchor.to_matrix().inverse();
            let composed = parent_world.to_matrix()
                * parent_anchor.to_matrix()
                * animated_offset.to_matrix()
                * inv_child;
            world[child_idx] = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
                .unwrap_or(Transform::IDENTITY);

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

fn quat_axis(q: Quat) -> Option<Vec3> {
    let q = if q.is_finite() {
        q.normalize()
    } else {
        return None;
    };
    let v = Vec3::new(q.x, q.y, q.z);
    let sin_half = v.length();
    if !sin_half.is_finite() || sin_half <= 1e-6 {
        return None;
    }
    Some((v / sin_half).normalize())
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
            let mut max_angle_q_delta = Quat::IDENTITY;
            let mut min_angle_deg: f32 = f32::INFINITY;
            let mut min_angle_phase: f32 = 0.0;

            let mut max_off_axis_deg: f32 = 0.0;
            let mut max_off_axis_phase: f32 = 0.0;
            let mut max_off_axis_q_delta = Quat::IDENTITY;
            let mut max_abs_hinge_angle_deg: f32 = 0.0;
            let mut max_abs_hinge_angle_phase: f32 = 0.0;
            let mut max_limit_exceed_deg: f32 = 0.0;
            let mut max_limit_exceed_phase: f32 = 0.0;
            let mut max_limit_exceed_hinge_angle_deg: f32 = 0.0;
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

            let child_anchor = anchor_transform_from_component(comp, att.child_anchor.as_str());
            for (i, &sample_t_m) in samples_t_m.iter().enumerate() {
                let sample_phase_01 = samples_phase_01.get(i).copied().unwrap_or(0.0);
                let delta = sample_animation_slot_delta(slot, sample_t_m, sample_phase_01);
                let animated_offset = match &slot.spec.clip {
                    PartAnimationDef::Spin {
                        axis_space: PartAnimationSpinAxisSpace::ChildLocal,
                        ..
                    } => apply_child_local_delta_to_attachment_offset(
                        att.offset,
                        child_anchor,
                        delta,
                    ),
                    _ => mul_transform(&att.offset, &delta),
                };
                let q_delta =
                    (att.offset.rotation.inverse() * animated_offset.rotation).normalize();
                let angle_deg = quat_angle_deg(q_delta).abs();
                let translation_m = delta.translation.length();

                if angle_deg > max_angle_deg {
                    max_angle_deg = angle_deg;
                    max_angle_phase = sample_phase_01;
                    max_angle_q_delta = q_delta;
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
                        max_off_axis_q_delta = q_delta;
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
                            max_limit_exceed_hinge_angle_deg = hinge_angle_deg;
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
                        let observed_axis = quat_axis(max_off_axis_q_delta);
                        let axis_alignment_abs =
                            observed_axis.map(|a| a.dot(axis_join).abs()).unwrap_or(0.0);
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
                                "observed_axis_join": observed_axis.map(|v| [v.x, v.y, v.z]),
                                "axis_alignment_abs": axis_alignment_abs,
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
                                "hinge_angle_degrees": max_limit_exceed_hinge_angle_deg,
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
                    if max_angle_deg.is_finite() && max_angle_deg > warn_deg {
                        let observed_axis = quat_axis(max_angle_q_delta);
                        issues.push(MotionIssue {
                            severity: MotionSeverity::Warn,
                            kind: "fixed_joint_rotates",
                            component_id: component_id.clone(),
                            component_name: component_name.clone(),
                            channel: channel.clone(),
                            message: "Fixed joint rotates under animation (expected no rotation)."
                                .into(),
                            evidence: serde_json::json!({
                                "max_angle_degrees": max_angle_deg,
                                "at_phase_01": max_angle_phase,
                                "observed_axis_join": observed_axis.map(|v| [v.x, v.y, v.z]),
                                "tolerances": { "warn_degrees": warn_deg },
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

    let lift_warn_m: f32 = (0.06 + 0.06 * cycle_m).clamp(0.10, 0.30);

    for (component_idx, comp) in components.iter().enumerate() {
        // Contact stance validation assumes the anchor is a planted point that should stay roughly
        // fixed in world space during stance. That is not true for rolling wheels/rollers: a single
        // anchor on the rim will move around as it rotates. If a component's `move` channel is a
        // pure `spin`, skip stance validation for its contacts.
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
            if move_is_spin {
                continue;
            }
            let Some(stance) = contact.stance.as_ref() else {
                issues.push(MotionIssue {
                    severity: MotionSeverity::Error,
                    kind: "contact_stance_missing",
                    component_id: component_id_uuid_for_name(&comp.name),
                    component_name: comp.name.clone(),
                    channel: "move".to_string(),
                    message: "Ground contact is missing `stance`; planted contacts must declare a stance schedule so contact lift can be validated. (For wheels/rollers using a `move` `spin`, omit stance; stance validation is skipped.)"
                        .into(),
                    evidence: serde_json::json!({
                        "contact_name": contact.name.trim(),
                        "anchor": contact.anchor.trim(),
                    }),
                    score: 1.0,
                });
                continue;
            };

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

            let mut max_lift_m: f32 = 0.0;
            let mut max_lift_phase: f32 = 0.0;

            for &i in &stance_indices {
                let p = positions_world[i];
                let lift = (p.y - ground_y).abs();
                if lift > max_lift_m {
                    max_lift_m = lift;
                    max_lift_phase = samples_phase_01.get(i).copied().unwrap_or(0.0);
                }
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
        .map(|c| {
            c.actual_size
                .unwrap_or(c.planned_size)
                .abs()
                .max(Vec3::splat(0.001))
        })
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
            components, &children, root_idx, t_secs, 0.0, 0.0, 0.0, false, false, true,
        );
        attack_world = compute_world_transforms_for_channels(
            components, &children, root_idx, t_secs, 0.0, 0.0, t_secs, true, false, false,
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

            candidates.push((sample.delta_m, blame_idx, i, j, sample, eps_m));
        }
    }

    candidates.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

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
                animated_offset = match &slot.spec.clip {
                    PartAnimationDef::Spin {
                        axis_space: PartAnimationSpinAxisSpace::ChildLocal,
                        ..
                    } => apply_child_local_delta_to_attachment_offset(
                        animated_offset,
                        child_anchor,
                        delta,
                    ),
                    _ => mul_transform(&animated_offset, &delta),
                };
            }

            let inv_child = child_anchor.to_matrix().inverse();
            let composed = parent_world.to_matrix()
                * parent_anchor.to_matrix()
                * animated_offset.to_matrix()
                * inv_child;
            world[child_idx] = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
                .unwrap_or(Transform::IDENTITY);

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
            root_animations: Vec::new(),
            attach_to: None,
        }
    }

    #[test]
    fn time_offset_no_effect_reports_error_for_repeating_loop() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        // Loop repeats every 0.7 units (A,B,A,B,A), so time_offset_units=0.7 is a no-op.
        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.7,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.4,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.35,
                        delta: Transform {
                            rotation: Quat::from_rotation_x(0.6),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.7,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.05,
                        delta: Transform {
                            rotation: Quat::from_rotation_x(0.6),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.4,
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
            joint: None,
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
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("time_offset_no_effect")
                    && i.get("channel").and_then(|v| v.as_str()) == Some("move")
            }),
            "expected time_offset_no_effect issue on move channel, got {issues:?}"
        );
    }

    #[test]
    fn time_offset_no_effect_is_not_reported_when_offset_changes_pose() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.7,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.4,
                keyframes: vec![
                    PartAnimationKeyframeDef {
                        time_secs: 0.0,
                        delta: Transform::IDENTITY,
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.35,
                        delta: Transform {
                            rotation: Quat::from_rotation_x(0.6),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 0.7,
                        delta: Transform {
                            rotation: Quat::from_rotation_x(-0.4),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.05,
                        delta: Transform {
                            rotation: Quat::from_rotation_x(0.2),
                            ..default()
                        },
                    },
                    PartAnimationKeyframeDef {
                        time_secs: 1.4,
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
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "move".into(),
                spec: move_spec,
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
            !issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("time_offset_no_effect")
            }),
            "expected no time_offset_no_effect issues, got {issues:?}"
        );
    }

    #[test]
    fn time_offset_no_effect_is_not_reported_for_static_loop() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.7,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.4,
                keyframes: vec![PartAnimationKeyframeDef {
                    time_secs: 0.0,
                    delta: Transform::IDENTITY,
                }],
            },
        };

        let mut limb = stub_component("limb", vec![anchor("mount", Vec3::ZERO)]);
        limb.attach_to = Some(Gen3dPlannedAttachment {
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

        let components = vec![root, limb];
        let report = build_motion_validation_report(Some(1.0), &components);
        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            !issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("time_offset_no_effect")
            }),
            "expected no time_offset_no_effect issues for static loop, got {issues:?}"
        );
    }

    #[test]
    fn chain_axis_mismatch_reports_warn() {
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
            .unwrap_or(false);
        assert!(ok, "expected motion validation to pass (warn only)");

        let issues = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let issue = issues
            .iter()
            .find(|i| i.get("kind").and_then(|v| v.as_str()) == Some("chain_axis_mismatch"));
        let issue =
            issue.unwrap_or_else(|| panic!("expected chain_axis_mismatch issue, got {issues:?}"));
        assert_eq!(
            issue.get("severity").and_then(|v| v.as_str()),
            Some("warn"),
            "expected chain_axis_mismatch severity=warn, got {issue:?}"
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
    fn contact_stance_missing_reports_error() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        let move_spec = PartAnimationSpec {
            driver: PartAnimationDriver::MovePhase,
            speed_scale: 1.0,
            time_offset_units: 0.0,
            clip: PartAnimationDef::Loop {
                duration_secs: 1.0,
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
            stance: None,
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
            issues.iter().any(|i| {
                i.get("kind").and_then(|v| v.as_str()) == Some("contact_stance_missing")
                    && i.get("severity").and_then(|v| v.as_str()) == Some("error")
            }),
            "expected contact_stance_missing error, got {issues:?}"
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
                axis_space: crate::object::registry::PartAnimationSpinAxisSpace::Join,
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
            !issues
                .iter()
                .any(|i| { i.get("kind").and_then(|v| v.as_str()) == Some("contact_lift") }),
            "expected no contact_lift issues for spin move component, got {issues:?}"
        );
    }

    #[test]
    fn attachment_spin_axis_space_child_local_rebases_through_child_anchor() {
        let root = stub_component("root", vec![anchor("mount", Vec3::ZERO)]);

        // A rotated anchor frame where +Z points up (common for rotor mounts / mirrored wheels).
        let child_anchor_rot =
            Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_X, Vec3::Z, Vec3::Y)).normalize();

        let mut wheel = stub_component(
            "wheel",
            vec![anchor_with_rot("mount", Vec3::ZERO, child_anchor_rot)],
        );
        wheel.attach_to = Some(Gen3dPlannedAttachment {
            parent: "root".into(),
            parent_anchor: "mount".into(),
            child_anchor: "mount".into(),
            offset: Transform::IDENTITY,
            joint: None,
            animations: vec![PartAnimationSlot {
                channel: "move".into(),
                spec: PartAnimationSpec {
                    driver: PartAnimationDriver::MoveDistance,
                    speed_scale: 1.0,
                    time_offset_units: 0.0,
                    clip: PartAnimationDef::Spin {
                        axis: Vec3::Y,
                        radians_per_unit: 1.0,
                        axis_space: crate::object::registry::PartAnimationSpinAxisSpace::ChildLocal,
                    },
                },
            }],
        });

        let components = vec![root, wheel];
        let children = vec![vec![1usize], vec![]];

        let world = compute_world_transforms_at_t(&components, &children, 0, 1.0);
        let got = if world[1].rotation.is_finite() {
            world[1].rotation.normalize()
        } else {
            Quat::IDENTITY
        };
        let delta = Quat::from_axis_angle(Vec3::Y, 1.0);
        let expected = (child_anchor_rot.inverse() * delta).normalize();
        assert!(
            got.angle_between(expected) < 1e-3,
            "expected child-local spin axis to be rebased through child anchor: got={:?} expected={:?}",
            got,
            expected,
        );

        let join = (delta * child_anchor_rot.inverse()).normalize();
        assert!(
            got.angle_between(join) > 1e-2,
            "expected child-local spin to differ from join-frame spin: got={:?} join={:?}",
            got,
            join,
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
