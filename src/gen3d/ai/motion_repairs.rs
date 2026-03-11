use crate::gen3d::ai::motion_validation;
use crate::gen3d::ai::schema::{AiJointJson, AiJointKindJson};

use super::Gen3dPlannedComponent;

fn json_f32(v: Option<&serde_json::Value>) -> Option<f32> {
    let v = v?;
    v.as_f64()
        .and_then(|n| n.is_finite().then_some(n as f32))
        .or_else(|| v.as_i64().map(|n| n as f32))
        .or_else(|| v.as_u64().map(|n| n as f32))
}

fn json_limits_degrees(v: Option<&serde_json::Value>) -> Option<[f32; 2]> {
    let v = v?.as_array()?;
    if v.len() != 2 {
        return None;
    }
    let a = json_f32(v.first())?;
    let b = json_f32(v.get(1))?;
    (a.is_finite() && b.is_finite()).then_some([a, b])
}

pub(super) fn suggest_motion_repairs_report_v1(
    rig_move_cycle_m: Option<f32>,
    components: &[Gen3dPlannedComponent],
    assembly_rev: u32,
    max_suggestions: usize,
    safety_margin_degrees: f32,
) -> serde_json::Value {
    let report = motion_validation::build_motion_validation_report(rig_move_cycle_m, components);
    let issues = report
        .motion_validation
        .get("issues")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut suggestions: Vec<serde_json::Value> = Vec::new();
    let max_suggestions = max_suggestions.max(1);
    let mut stopped_due_to_limit = false;

    for issue in issues.iter() {
        let Some(kind) = issue.get("kind").and_then(|v| v.as_str()) else {
            continue;
        };
        if kind != "hinge_limit_exceeded" {
            continue;
        }

        let Some(component_name) = issue.get("component_name").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(channel) = issue.get("channel").and_then(|v| v.as_str()) else {
            continue;
        };
        let evidence = issue.get("evidence");

        let Some(hinge_angle_deg) = json_f32(evidence.and_then(|e| e.get("hinge_angle_degrees")))
        else {
            continue;
        };
        let Some(limits_deg) = json_limits_degrees(evidence.and_then(|e| e.get("limits_degrees")))
        else {
            continue;
        };

        let Some(planned_child) = components.iter().find(|c| c.name == component_name) else {
            continue;
        };
        let Some(att) = planned_child.attach_to.as_ref() else {
            continue;
        };
        let Some(joint) = att.joint.as_ref() else {
            continue;
        };
        if joint.kind != AiJointKindJson::Hinge {
            continue;
        }

        let [min_deg, max_deg] = limits_deg;
        if !min_deg.is_finite() || !max_deg.is_finite() || min_deg >= max_deg {
            continue;
        }
        if !safety_margin_degrees.is_finite() || safety_margin_degrees < 0.0 {
            continue;
        }

        let mut updated_joint: AiJointJson = joint.clone();
        let mut impact = serde_json::Map::new();
        impact.insert(
            "old_limits_degrees".into(),
            serde_json::json!([min_deg, max_deg]),
        );
        impact.insert(
            "safety_margin_degrees".into(),
            serde_json::json!(safety_margin_degrees),
        );

        let (bound, new_limits, delta) = if hinge_angle_deg > max_deg {
            let new_max = hinge_angle_deg + safety_margin_degrees;
            ("upper", [min_deg, new_max], (new_max - max_deg).max(0.0))
        } else if hinge_angle_deg < min_deg {
            let new_min = hinge_angle_deg - safety_margin_degrees;
            ("lower", [new_min, max_deg], (min_deg - new_min).max(0.0))
        } else {
            continue;
        };

        if !new_limits[0].is_finite()
            || !new_limits[1].is_finite()
            || new_limits[0] >= new_limits[1]
        {
            continue;
        }

        updated_joint.limits_degrees = Some(new_limits);
        impact.insert("adjusted_bound".into(), serde_json::json!(bound));
        impact.insert(
            "new_limits_degrees".into(),
            serde_json::json!([new_limits[0], new_limits[1]]),
        );
        impact.insert("relax_degrees".into(), serde_json::json!(delta));

        let patch = serde_json::json!({
            "version": 1,
            "atomic": true,
            "if_assembly_rev": assembly_rev,
            "ops": [
                {
                    "kind": "set_attachment_joint",
                    "child_component": component_name,
                    "set_joint": updated_joint,
                }
            ]
        });

        suggestions.push(serde_json::json!({
            "id": format!("hinge_limit_exceeded/{}/{}/relax_joint_limits", component_name, channel),
            "kind": "relax_joint_limits",
            "issue_kind": "hinge_limit_exceeded",
            "component_name": component_name,
            "channel": channel,
            "message": format!(
                "Relax `{}` hinge {} limit by {:.3}° to fit observed motion (no silent apply).",
                component_name, bound, delta
            ),
            "impact": serde_json::Value::Object(impact),
            "apply_draft_ops_args": patch,
        }));

        if suggestions.len() >= max_suggestions {
            stopped_due_to_limit = true;
            break;
        }

        // Option 2: scale animation rotation down so the worst hinge angle fits within limits.
        // Only propose pure shrink scaling (0, 1] to avoid flipping/increasing amplitude.
        let target_deg = if hinge_angle_deg > max_deg {
            max_deg - safety_margin_degrees
        } else if hinge_angle_deg < min_deg {
            min_deg + safety_margin_degrees
        } else {
            hinge_angle_deg
        };
        let scale_factor = if hinge_angle_deg.abs() > 1e-6 {
            target_deg / hinge_angle_deg
        } else {
            f32::NAN
        };
        if scale_factor.is_finite() && scale_factor > 0.0 && scale_factor <= 1.0 {
            let has_slot = att
                .animations
                .iter()
                .filter(|s| s.channel.as_ref() == channel)
                .count();
            if has_slot == 1 {
                let patch = serde_json::json!({
                    "version": 1,
                    "atomic": true,
                    "if_assembly_rev": assembly_rev,
                    "ops": [
                        {
                            "kind": "scale_animation_slot_rotation",
                            "child_component": component_name,
                            "channel": channel,
                            "scale": scale_factor,
                        }
                    ]
                });
                suggestions.push(serde_json::json!({
                    "id": format!("hinge_limit_exceeded/{}/{}/scale_animation_slot_rotation", component_name, channel),
                    "kind": "scale_animation_slot_rotation",
                    "issue_kind": "hinge_limit_exceeded",
                    "component_name": component_name,
                    "channel": channel,
                    "message": format!(
                        "Scale `{}` `{}` rotation keyframes by {:.4} to fit hinge limits (explicit apply required).",
                        component_name, channel, scale_factor
                    ),
                    "impact": {
                        "scale_factor": scale_factor,
                        "target_degrees": target_deg,
                        "observed_degrees": hinge_angle_deg,
                        "limits_degrees": [min_deg, max_deg],
                        "safety_margin_degrees": safety_margin_degrees,
                    },
                    "apply_draft_ops_args": patch,
                }));
            }
        }

        if suggestions.len() >= max_suggestions {
            stopped_due_to_limit = true;
            break;
        }
    }

    let truncated = stopped_due_to_limit;

    serde_json::json!({
        "ok": true,
        "version": 1,
        "rig_summary": report.rig_summary,
        "motion_validation": report.motion_validation,
        "max_suggestions": max_suggestions,
        "safety_margin_degrees": safety_margin_degrees,
        "suggestions": suggestions,
        "truncated": truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::ai::draft_ops::apply_draft_ops_v1;
    use crate::gen3d::ai::job::Gen3dAiJob;
    use crate::gen3d::gen3d_draft_object_id;
    use crate::gen3d::state::Gen3dDraft;
    use crate::object::registry::{
        builtin_object_id, AnchorDef, ColliderProfile, MeshKey, ObjectDef, ObjectInteraction,
        ObjectPartDef, PartAnimationDef, PartAnimationDriver, PartAnimationKeyframeDef,
        PartAnimationSlot, PartAnimationSpec, PrimitiveVisualDef,
    };
    use bevy::prelude::*;

    fn component_object_id_for_name(name: &str) -> u128 {
        builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
    }

    fn make_component_def(name: &str) -> ObjectDef {
        let object_id = component_object_id_for_name(name);
        let mut part0 = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh: MeshKey::UnitCube,
                params: None,
                color: Color::srgb(0.5, 0.5, 0.5),
                unlit: false,
            },
            Transform::IDENTITY,
        );
        part0.part_id = Some(builtin_object_id(&format!("gravimera/gen3d/part/{name}/0")));

        ObjectDef {
            object_id,
            label: format!("gen3d_component_{name}").into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: vec![AnchorDef {
                name: "mount".into(),
                transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
            }],
            parts: vec![part0],
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    fn make_root_def() -> ObjectDef {
        ObjectDef {
            object_id: gen3d_draft_object_id(),
            label: "gen3d_draft".into(),
            size: Vec3::ONE,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: Vec::new(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        }
    }

    fn make_hinge_limit_exceeded_job_and_draft() -> (Gen3dAiJob, Gen3dDraft) {
        let mut job = Gen3dAiJob::default();
        job.planned_components = vec![
            Gen3dPlannedComponent {
                display_name: "1. root".into(),
                name: "root".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![AnchorDef {
                    name: "mount".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                }],
                contacts: Vec::new(),
                attach_to: None,
            },
            Gen3dPlannedComponent {
                display_name: "2. tongue".into(),
                name: "tongue".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                pos: Vec3::ZERO,
                rot: Quat::IDENTITY,
                planned_size: Vec3::ONE,
                actual_size: Some(Vec3::ONE),
                anchors: vec![AnchorDef {
                    name: "mount".into(),
                    transform: Transform::from_translation(Vec3::new(0.0, 1.0, 0.0)),
                }],
                contacts: Vec::new(),
                attach_to: Some(crate::gen3d::ai::job::Gen3dPlannedAttachment {
                    parent: "root".into(),
                    parent_anchor: "origin".into(),
                    child_anchor: "origin".into(),
                    offset: Transform::IDENTITY,
                    joint: Some(AiJointJson {
                        kind: AiJointKindJson::Hinge,
                        axis_join: Some([1.0, 0.0, 0.0]),
                        limits_degrees: Some([-30.0, 30.0]),
                        swing_limits_degrees: None,
                        twist_limits_degrees: None,
                    }),
                    animations: vec![PartAnimationSlot {
                        channel: "move".into(),
                        spec: PartAnimationSpec {
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
                                        time_secs: 0.5,
                                        delta: Transform {
                                            rotation: Quat::from_axis_angle(
                                                Vec3::X,
                                                40.0_f32.to_radians(),
                                            ),
                                            ..Default::default()
                                        },
                                    },
                                    PartAnimationKeyframeDef {
                                        time_secs: 1.0,
                                        delta: Transform::IDENTITY,
                                    },
                                ],
                            },
                        },
                    }],
                }),
            },
        ];

        let draft = Gen3dDraft {
            defs: vec![
                make_root_def(),
                make_component_def("root"),
                make_component_def("tongue"),
            ],
        };
        (job, draft)
    }

    fn has_issue_kind(report: &motion_validation::MotionValidationReport, kind: &str) -> bool {
        let Some(issues) = report
            .motion_validation
            .get("issues")
            .and_then(|v| v.as_array())
        else {
            return false;
        };
        issues
            .iter()
            .any(|issue| issue.get("kind").and_then(|v| v.as_str()) == Some(kind))
    }

    #[test]
    fn suggests_repairs_for_hinge_limit_exceeded() {
        let (job, _draft) = make_hinge_limit_exceeded_job_and_draft();
        let report =
            motion_validation::build_motion_validation_report(Some(1.0), &job.planned_components);
        assert!(has_issue_kind(&report, "hinge_limit_exceeded"));

        let out = suggest_motion_repairs_report_v1(
            Some(1.0),
            &job.planned_components,
            job.assembly_rev(),
            8,
            0.2,
        );
        let suggestions = out
            .get("suggestions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(suggestions
            .iter()
            .any(|s| s.get("kind").and_then(|v| v.as_str()) == Some("relax_joint_limits")));
        assert!(suggestions.iter().any(
            |s| s.get("kind").and_then(|v| v.as_str()) == Some("scale_animation_slot_rotation")
        ));
    }

    #[test]
    fn applying_scale_suggestion_fixes_hinge_limit_exceeded() {
        let (mut job, mut draft) = make_hinge_limit_exceeded_job_and_draft();
        let out = suggest_motion_repairs_report_v1(
            Some(1.0),
            &job.planned_components,
            job.assembly_rev(),
            8,
            0.2,
        );
        let suggestions = out
            .get("suggestions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let scale = suggestions
            .iter()
            .find(|s| {
                s.get("kind").and_then(|v| v.as_str()) == Some("scale_animation_slot_rotation")
            })
            .and_then(|s| s.get("apply_draft_ops_args"))
            .cloned()
            .expect("missing scale suggestion apply_draft_ops_args");

        let apply_out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), scale).unwrap();
        assert!(apply_out
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));

        let report =
            motion_validation::build_motion_validation_report(Some(1.0), &job.planned_components);
        assert!(!has_issue_kind(&report, "hinge_limit_exceeded"));
    }

    #[test]
    fn applying_relax_limits_suggestion_fixes_hinge_limit_exceeded() {
        let (mut job, mut draft) = make_hinge_limit_exceeded_job_and_draft();
        let out = suggest_motion_repairs_report_v1(
            Some(1.0),
            &job.planned_components,
            job.assembly_rev(),
            8,
            0.2,
        );
        let suggestions = out
            .get("suggestions")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let patch = suggestions
            .iter()
            .find(|s| s.get("kind").and_then(|v| v.as_str()) == Some("relax_joint_limits"))
            .and_then(|s| s.get("apply_draft_ops_args"))
            .cloned()
            .expect("missing relax_joint_limits apply_draft_ops_args");

        let apply_out = apply_draft_ops_v1(&mut job, &mut draft, Some("test"), patch).unwrap();
        assert!(apply_out
            .get("ok")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));

        let report =
            motion_validation::build_motion_validation_report(Some(1.0), &job.planned_components);
        assert!(!has_issue_kind(&report, "hinge_limit_exceeded"));
    }
}
