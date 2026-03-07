use bevy::log::{debug, warn};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

use crate::object::registry::{
    builtin_object_id, AimProfile, AnchorRef, ColliderProfile, MeleeAttackProfile, MeshKey,
    MobilityDef, MobilityMode, ObjectDef, ObjectInteraction, ObjectPartDef, ObjectPartKind,
    PartAnimationDef, PrimitiveParams, PrimitiveVisualDef, ProjectileObstacleRule,
    ProjectileProfile, RangedAttackProfile, UnitAttackKind, UnitAttackProfile,
};

use super::super::state::Gen3dDraft;
use super::super::{gen3d_draft_object_id, gen3d_draft_projectile_object_id, GEN3D_MAX_COMPONENTS};
use super::artifacts::append_gen3d_jsonl_artifact;
use super::schema::*;
use super::{Gen3dPlannedAttachment, Gen3dPlannedComponent};

fn rotated_half_extents(half: Vec3, rotation: Quat) -> Vec3 {
    let abs = Mat3::from_quat(rotation).abs();
    abs * half
}

pub(super) fn plan_rotation_from_forward_up_lossy(forward: Vec3, up: Option<Vec3>) -> Quat {
    const EPS: f32 = 1e-5;

    let mut f = forward;
    if !f.is_finite() {
        f = Vec3::Z;
    }
    if f.length_squared() < EPS {
        f = Vec3::Z;
    }
    f = f.normalize();

    let mut u = up.unwrap_or(Vec3::Y);
    if !u.is_finite() {
        u = Vec3::Y;
    }
    if u.length_squared() < EPS {
        u = Vec3::Y;
    }
    u = u.normalize();

    // If `up` is nearly parallel to `forward`, pick a fallback up to avoid degeneracy.
    if u.dot(f).abs() > 0.98 {
        u = if Vec3::Y.dot(f).abs() < 0.98 {
            Vec3::Y
        } else {
            Vec3::X
        };
    }

    let mut r = u.cross(f);
    if r.length_squared() < EPS {
        // Last-ditch fallback.
        r = Vec3::X.cross(f);
        if r.length_squared() < EPS {
            r = Vec3::Z.cross(f);
        }
    }
    if r.length_squared() < EPS {
        return Quat::IDENTITY;
    }
    r = r.normalize();

    let u2 = f.cross(r).normalize();
    Quat::from_mat3(&Mat3::from_cols(r, u2, f)).normalize()
}

pub(super) fn plan_rotation_from_forward_up_strict(
    forward: Vec3,
    up: Vec3,
) -> Result<Quat, String> {
    const EPS: f32 = 1e-5;

    if !forward.is_finite() || forward.length_squared() < EPS {
        return Err("forward must be a finite, non-zero vec3".into());
    }
    if !up.is_finite() || up.length_squared() < EPS {
        return Err("up must be a finite, non-zero vec3".into());
    }

    let f = forward.normalize();
    let u = up.normalize();

    if u.dot(f).abs() > 0.98 {
        return Err(
            "up must not be nearly parallel to forward (provide a distinct up vector)".into(),
        );
    }

    let r = u.cross(f);
    if r.length_squared() < EPS {
        return Err("forward/up produced a degenerate basis".into());
    }
    let r = r.normalize();

    let u2 = f.cross(r);
    if !u2.is_finite() || u2.length_squared() < EPS {
        return Err("forward/up produced a degenerate basis".into());
    }
    let u2 = u2.normalize();

    let q = Quat::from_mat3(&Mat3::from_cols(r, u2, f)).normalize();
    if !q.is_finite() {
        return Err("forward/up produced a non-finite rotation quaternion".into());
    }
    Ok(q)
}

fn collider_profile_from_ai(
    collider: Option<AiColliderJson>,
    default_size: Vec3,
) -> Result<ColliderProfile, String> {
    Ok(match collider {
        None => ColliderProfile::AabbXZ {
            half_extents: Vec2::new(default_size.x * 0.5, default_size.z * 0.5),
        },
        Some(AiColliderJson::None) => ColliderProfile::None,
        Some(AiColliderJson::CircleXz { radius }) => ColliderProfile::CircleXZ {
            radius: radius.max(0.01),
        },
        Some(AiColliderJson::AabbXz {
            half_extents,
            min,
            max,
        }) => {
            if let Some(half) = half_extents {
                ColliderProfile::AabbXZ {
                    half_extents: Vec2::new(half[0].abs().max(0.01), half[1].abs().max(0.01)),
                }
            } else if let (Some(min), Some(max)) = (min, max) {
                let hx = ((max[0] - min[0]).abs() * 0.5).max(0.01);
                let hz = ((max[1] - min[1]).abs() * 0.5).max(0.01);
                ColliderProfile::AabbXZ {
                    half_extents: Vec2::new(hx, hz),
                }
            } else {
                ColliderProfile::AabbXZ {
                    half_extents: Vec2::new(default_size.x * 0.5, default_size.z * 0.5),
                }
            }
        }
    })
}

fn mobility_from_ai(mobility: &AiMobilityJson) -> Option<MobilityDef> {
    match mobility {
        AiMobilityJson::Static => None,
        AiMobilityJson::Ground { max_speed } => {
            let speed = if max_speed.is_finite() {
                *max_speed
            } else {
                0.0
            };
            if speed <= 0.01 {
                None
            } else {
                Some(MobilityDef {
                    mode: MobilityMode::Ground,
                    max_speed: speed,
                })
            }
        }
        AiMobilityJson::Air { max_speed } => {
            let speed = if max_speed.is_finite() {
                *max_speed
            } else {
                0.0
            };
            if speed <= 0.01 {
                None
            } else {
                Some(MobilityDef {
                    mode: MobilityMode::Air,
                    max_speed: speed,
                })
            }
        }
    }
}

fn attack_anim_window_secs_from_planned_components(
    planned: &[Gen3dPlannedComponent],
) -> Option<f32> {
    let mut best: Option<f32> = None;
    for comp in planned.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        for slot in att.animations.iter() {
            if slot.channel.as_ref() != "attack_primary" {
                continue;
            }
            let duration_secs = match slot.spec.clip {
                PartAnimationDef::Loop { duration_secs, .. }
                | PartAnimationDef::Once { duration_secs, .. }
                | PartAnimationDef::PingPong { duration_secs, .. } => duration_secs,
                PartAnimationDef::Spin { .. } => continue,
            };
            let speed = slot.spec.speed_scale.max(1e-3);
            let wall_duration = (duration_secs / speed).clamp(0.05, 10.0);
            best = Some(best.map_or(wall_duration, |b| b.max(wall_duration)));
        }
    }
    best
}

fn gen3d_projectile_obstacle_rule_from_ai(
    rule: Option<AiProjectileObstacleRuleJson>,
) -> ProjectileObstacleRule {
    match rule.unwrap_or(AiProjectileObstacleRuleJson::BulletsBlockers) {
        AiProjectileObstacleRuleJson::BulletsBlockers => ProjectileObstacleRule::BulletsBlockers,
        AiProjectileObstacleRuleJson::LaserBlockers => ProjectileObstacleRule::LaserBlockers,
    }
}

fn gen3d_projectile_def_from_ai(spec: &AiProjectileSpecJson) -> Result<ObjectDef, String> {
    let id = gen3d_draft_projectile_object_id();

    let rgba = {
        let rgba = spec.color;
        let ok = rgba
            .iter()
            .copied()
            .all(|v| v.is_finite() && (0.0..=1.0).contains(&v));
        if !ok {
            return Err(format!(
                "projectile.color must be RGBA floats in the range 0..1, got {rgba:?}"
            ));
        }
        rgba
    };
    let color = Color::srgba(rgba[0], rgba[1], rgba[2], rgba[3]);

    let (part, size, collider_radius) = match spec.shape {
        AiProjectileShapeJson::Sphere => {
            let radius = spec.radius.unwrap_or(0.14).abs().clamp(0.01, 10.0);
            let scale = Vec3::splat(radius * 2.0);
            (
                ObjectPartDef::primitive(
                    PrimitiveVisualDef::Primitive {
                        mesh: MeshKey::UnitSphere,
                        params: None,
                        color,
                        unlit: spec.unlit,
                    },
                    Transform::from_scale(scale),
                ),
                Vec3::splat(radius * 2.0),
                radius,
            )
        }
        AiProjectileShapeJson::Capsule => {
            let radius = spec.radius.unwrap_or(0.10).abs().clamp(0.01, 10.0);
            let length = spec
                .length
                .unwrap_or(radius * 4.0)
                .abs()
                .clamp(radius * 2.0, 100.0);
            let half_length = ((length - radius * 2.0) * 0.5).max(0.0);
            (
                ObjectPartDef::primitive(
                    PrimitiveVisualDef::Primitive {
                        mesh: MeshKey::UnitCapsule,
                        params: Some(PrimitiveParams::Capsule {
                            radius,
                            half_length,
                        }),
                        color,
                        unlit: spec.unlit,
                    },
                    Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ),
                Vec3::new(radius * 2.0, radius * 2.0, length),
                radius,
            )
        }
        AiProjectileShapeJson::Cuboid => {
            let size = spec.size.unwrap_or_else(|| {
                let radius = spec.radius.unwrap_or(0.10).abs().clamp(0.01, 10.0);
                let length = spec
                    .length
                    .unwrap_or(radius * 4.0)
                    .abs()
                    .clamp(radius * 2.0, 100.0);
                [radius * 2.0, radius * 2.0, length]
            });
            let sx = size[0].abs().max(0.01);
            let sy = size[1].abs().max(0.01);
            let sz = size[2].abs().max(0.01);
            (
                ObjectPartDef::primitive(
                    PrimitiveVisualDef::Primitive {
                        mesh: MeshKey::UnitCube,
                        params: None,
                        color,
                        unlit: spec.unlit,
                    },
                    Transform::from_scale(Vec3::new(sx, sy, sz)),
                ),
                Vec3::new(sx, sy, sz),
                (sx.max(sy) * 0.5).max(0.01),
            )
        }
        AiProjectileShapeJson::Cylinder => {
            let radius = spec.radius.unwrap_or(0.10).abs().clamp(0.01, 10.0);
            let length = spec
                .length
                .unwrap_or(radius * 4.0)
                .abs()
                .clamp(radius * 2.0, 100.0);
            let scale = Vec3::new(radius * 2.0, length, radius * 2.0);
            (
                ObjectPartDef::primitive(
                    PrimitiveVisualDef::Primitive {
                        mesh: MeshKey::UnitCylinder,
                        params: None,
                        color,
                        unlit: spec.unlit,
                    },
                    Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))
                        .with_scale(scale),
                ),
                Vec3::new(radius * 2.0, radius * 2.0, length),
                radius,
            )
        }
    };

    let collider_radius = if collider_radius.is_finite() {
        collider_radius
    } else {
        0.1
    };
    let size = size.abs().max(Vec3::splat(0.01));

    let speed = if spec.speed.is_finite() {
        spec.speed
    } else {
        0.0
    }
    .max(0.01);
    let ttl_secs = if spec.ttl_secs.is_finite() {
        spec.ttl_secs
    } else {
        0.0
    }
    .clamp(0.05, 60.0);

    Ok(ObjectDef {
        object_id: id,
        label: "gen3d_projectile".into(),
        size,
        ground_origin_y: None,
        collider: ColliderProfile::CircleXZ {
            radius: collider_radius,
        },
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors: Vec::new(),
        parts: vec![part],
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: Some(ProjectileProfile {
            obstacle_rule: gen3d_projectile_obstacle_rule_from_ai(spec.obstacle_rule),
            speed,
            ttl_secs,
            damage: spec.damage,
            spawn_energy_impact: spec.spawn_energy_impact,
        }),
        attack: None,
    })
}

fn ai_vec3(value: [f32; 3]) -> Vec3 {
    Vec3::new(value[0], value[1], value[2])
}

fn ai_vec3_opt(value: Option<[f32; 3]>) -> Option<Vec3> {
    const EPS: f32 = 1e-6;
    let v = value.map(ai_vec3)?;
    if !v.is_finite() || v.length_squared() <= EPS {
        return None;
    }
    Some(v)
}

fn quat_from_forward_up_or_identity(
    context: &str,
    forward: Option<[f32; 3]>,
    up: Option<[f32; 3]>,
) -> Result<Quat, String> {
    let forward_v = ai_vec3_opt(forward);
    if forward.is_some() && forward_v.is_none() {
        return Err(format!(
            "{context}: forward must be a finite, non-zero vec3"
        ));
    }
    let up_v = ai_vec3_opt(up);
    if up.is_some() && up_v.is_none() {
        return Err(format!("{context}: up must be a finite, non-zero vec3"));
    }

    match (forward_v, up_v) {
        (None, None) => Ok(Quat::IDENTITY),
        (Some(_), None) | (None, Some(_)) => Err(format!(
            "{context}: rotation basis must include BOTH `forward` and `up` (or omit both for identity). Expected component-local axes (+X right, +Y up, +Z forward)."
        )),
        (Some(f), Some(u)) => plan_rotation_from_forward_up_strict(f, u).map_err(|err| {
            format!(
                "{context}: invalid forward/up basis: {err}. Expected non-degenerate basis vectors in component-local axes (+X right, +Y up, +Z forward)."
            )
        }),
    }
}

fn anchor_transform_from_defs(
    anchors: &[crate::object::registry::AnchorDef],
    name: &str,
) -> Option<Transform> {
    if name == "origin" {
        return Some(Transform::IDENTITY);
    }
    anchors
        .iter()
        .find(|a| a.name.as_ref() == name)
        .map(|a| a.transform)
}

fn anchors_from_ai(
    source: &str,
    component: &str,
    anchors: &[AiAnchorJson],
) -> Result<Vec<crate::object::registry::AnchorDef>, String> {
    let mut out: Vec<crate::object::registry::AnchorDef> = Vec::with_capacity(anchors.len());
    let mut seen: HashSet<String> = HashSet::new();

    for anchor in anchors {
        let name = anchor.name.trim();
        if name.is_empty() {
            continue;
        }
        if name == "origin" {
            // `origin` is implicit; keep it out of the explicit list to avoid ambiguity.
            continue;
        }
        if !seen.insert(name.to_string()) {
            return Err(format!(
                "AI plan: duplicate anchor name `{name}` in component `{component}`",
            ));
        }

        let pos = ai_vec3(anchor.pos);
        if !pos.is_finite() {
            return Err(format!(
                "AI {source}: anchor `{name}` in component `{component}` has non-finite pos",
            ));
        }

        let forward = ai_vec3(anchor.forward);
        let up = ai_vec3(anchor.up);
        let rot = plan_rotation_from_forward_up_strict(forward, up).map_err(|err| {
            format!(
                "AI {source}: anchor `{name}` in component `{component}` has invalid forward/up basis: {err}. Expected non-degenerate basis vectors in component-local axes (+X right, +Y up, +Z forward)."
            )
        })?;

        out.push(crate::object::registry::AnchorDef {
            name: name.to_string().into(),
            transform: Transform::from_translation(pos).with_rotation(rot),
        });
    }

    Ok(out)
}

fn merge_component_anchors_from_plan_and_draft(
    component_name: &str,
    required_plan_anchors: &[crate::object::registry::AnchorDef],
    draft_anchors: Vec<crate::object::registry::AnchorDef>,
) -> Result<Vec<crate::object::registry::AnchorDef>, String> {
    if required_plan_anchors.is_empty() {
        return Ok(draft_anchors);
    }

    let mut draft_by_name: std::collections::HashMap<String, crate::object::registry::AnchorDef> =
        std::collections::HashMap::new();
    for anchor in draft_anchors {
        draft_by_name.insert(anchor.name.as_ref().to_string(), anchor);
    }

    let mut missing: Vec<String> = Vec::new();
    for required in required_plan_anchors.iter() {
        if !draft_by_name.contains_key(required.name.as_ref()) {
            missing.push(required.name.as_ref().to_string());
        }
    }
    if !missing.is_empty() {
        missing.sort();
        return Err(format!(
            "AI draft missing required anchors for component `{}`: {}",
            component_name,
            missing.join(", ")
        ));
    }

    let mut merged: Vec<crate::object::registry::AnchorDef> =
        Vec::with_capacity(required_plan_anchors.len());
    for required in required_plan_anchors.iter() {
        if let Some(anchor) = draft_by_name.remove(required.name.as_ref()) {
            merged.push(anchor);
        } else {
            merged.push(required.clone());
        }
    }

    if !draft_by_name.is_empty() {
        let mut extras: Vec<String> = draft_by_name.keys().cloned().collect();
        extras.sort();
        debug!(
            "Gen3D: ignoring extra anchors in component `{}`: {}",
            component_name,
            extras.join(", ")
        );
    }

    Ok(merged)
}

fn override_required_anchor_rotations_from_plan(
    component_name: &str,
    required_plan_anchors: &[crate::object::registry::AnchorDef],
    anchors: &mut [crate::object::registry::AnchorDef],
    run_dir: Option<&Path>,
) {
    if required_plan_anchors.is_empty() || anchors.is_empty() {
        return;
    }

    for required in required_plan_anchors.iter() {
        let name = required.name.as_ref();
        if name.trim().is_empty() || name == "origin" {
            continue;
        }

        let Some(anchor) = anchors.iter_mut().find(|a| a.name.as_ref() == name) else {
            continue;
        };

        let desired = if required.transform.rotation.is_finite() {
            required.transform.rotation.normalize()
        } else {
            Quat::IDENTITY
        };

        let current = if anchor.transform.rotation.is_finite() {
            anchor.transform.rotation.normalize()
        } else {
            Quat::IDENTITY
        };

        // Join frames are part of the plan contract: draft geometry may adjust anchor *positions*,
        // but letting the draft override anchor orientation breaks attachment and animation axes
        // (e.g. melee swing yaw becomes a twist).
        if current.dot(desired).abs() < 0.999 {
            debug!(
                "Gen3D: overriding anchor rotation from draft to plan for component `{}` anchor `{}`",
                component_name, name
            );
            append_gen3d_jsonl_artifact(
                run_dir,
                "applied_defaults.jsonl",
                &serde_json::json!({
                    "kind": "override_anchor_rotation",
                    "component": component_name,
                    "anchor": name,
                }),
            );
        }
        anchor.transform.rotation = desired;
    }
}

fn attachment_offset_from_ai(
    offset: Option<&AiAttachmentOffsetJson>,
    parent_anchor_rot: Option<Quat>,
) -> Result<Transform, String> {
    let Some(offset) = offset else {
        return Ok(Transform::IDENTITY);
    };
    let mut translation = ai_vec3(offset.pos);
    if !translation.is_finite() {
        translation = Vec3::ZERO;
    }
    if matches!(offset.rot_frame, Some(AiRotationFrameJson::Unknown)) {
        return Err("rot_frame must be \"join\" or \"parent\"".into());
    }
    let rotation_requested =
        offset.forward.is_some() || offset.up.is_some() || offset.rot_quat_xyzw.is_some();
    if rotation_requested && offset.rot_frame.is_none() {
        return Err(
            "rot_frame is required when authoring a rotation (use \"join\" or \"parent\")".into(),
        );
    }
    let rot_frame = offset.rot_frame.unwrap_or(AiRotationFrameJson::Join);
    let rotation = if offset.forward.is_some() || offset.up.is_some() {
        let Some(forward) = offset.forward else {
            return Err(
                "offset rotation basis requires both `forward` and `up` (missing `forward`)".into(),
            );
        };
        let Some(up) = offset.up else {
            return Err(
                "offset rotation basis requires both `forward` and `up` (missing `up`)".into(),
            );
        };
        let mut forward =
            ai_vec3_opt(Some(forward)).ok_or("offset.forward must be a finite, non-zero vec3")?;
        let mut up = ai_vec3_opt(Some(up)).ok_or("offset.up must be a finite, non-zero vec3")?;
        if matches!(rot_frame, AiRotationFrameJson::Parent) {
            let parent_anchor_rot = parent_anchor_rot
                .ok_or("rot_frame=\"parent\" requires a valid parent anchor rotation")?;
            let inv = parent_anchor_rot.inverse();
            forward = inv * forward;
            up = inv * up;
        }
        plan_rotation_from_forward_up_strict(forward, up)?
    } else if let Some(q) = offset.rot_quat_xyzw {
        let q_raw = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
        if !q_raw.is_finite() {
            return Err("offset.rot_quat_xyzw must be a finite quaternion".into());
        }
        if q_raw.length_squared() <= 1e-10 {
            return Err("offset.rot_quat_xyzw must be a non-zero quaternion".into());
        }
        let mut q = q_raw.normalize();
        if matches!(rot_frame, AiRotationFrameJson::Parent) {
            let parent_anchor_rot = parent_anchor_rot
                .ok_or("rot_frame=\"parent\" requires a valid parent anchor rotation")?;
            let r = if parent_anchor_rot.is_finite() {
                parent_anchor_rot.normalize()
            } else {
                Quat::IDENTITY
            };
            q = (r.inverse() * q * r).normalize();
        }
        if !q.is_finite() {
            return Err("offset.rot_quat_xyzw must be a valid quaternion".into());
        }
        q
    } else {
        Quat::IDENTITY
    };
    let scale = match offset.scale {
        Some(scale) => {
            let scale = ai_vec3(scale);
            if !scale.is_finite() {
                return Err("offset.scale must be finite when provided".into());
            }
            scale
        }
        None => Vec3::ONE,
    };
    Ok(Transform::from_translation(translation)
        .with_rotation(rotation)
        .with_scale(scale))
}

pub(super) fn resolve_planned_component_transforms(
    planned: &mut [Gen3dPlannedComponent],
    root_idx: usize,
) -> Result<(), String> {
    let mut name_to_idx: HashMap<String, usize> = HashMap::new();
    for (idx, comp) in planned.iter().enumerate() {
        name_to_idx.insert(comp.name.clone(), idx);
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); planned.len()];
    for (idx, comp) in planned.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(parent_idx) = name_to_idx.get(att.parent.as_str()).copied() else {
            return Err(format!(
                "AI plan: component `{}` attach_to parent `{}` not found.",
                comp.name, att.parent
            ));
        };
        if parent_idx == idx {
            return Err(format!(
                "AI plan: component `{}` attach_to parent cannot be itself.",
                comp.name
            ));
        }
        children[parent_idx].push(idx);
    }

    planned[root_idx].pos = Vec3::ZERO;
    planned[root_idx].rot = Quat::IDENTITY;

    fn dfs(
        idx: usize,
        planned: &mut [Gen3dPlannedComponent],
        children: &[Vec<usize>],
        visiting: &mut Vec<bool>,
        visited: &mut Vec<bool>,
        name_to_idx: &std::collections::HashMap<String, usize>,
    ) -> Result<(), String> {
        if visiting[idx] {
            return Err("AI plan attachments contain a cycle.".into());
        }
        if visited[idx] {
            return Ok(());
        }
        visiting[idx] = true;

        let parent_world =
            Transform::from_translation(planned[idx].pos).with_rotation(planned[idx].rot);
        let parent_world_mat = parent_world.to_matrix();

        for &child_idx in children[idx].iter() {
            let att = planned[child_idx]
                .attach_to
                .as_ref()
                .ok_or_else(|| "Internal error: missing attachment".to_string())?;

            let parent_anchor = anchor_transform_from_defs(
                &planned[idx].anchors,
                att.parent_anchor.as_str(),
            )
            .ok_or_else(|| {
                format!(
                    "AI plan: attachment `{}` -> `{}` references missing parent_anchor `{}` on component `{}`.",
                    planned[idx].name, planned[child_idx].name, att.parent_anchor, planned[idx].name
                )
            })?;
            let child_anchor = anchor_transform_from_defs(
                &planned[child_idx].anchors,
                att.child_anchor.as_str(),
            )
            .ok_or_else(|| {
                format!(
                    "AI plan: attachment `{}` -> `{}` references missing child_anchor `{}` on component `{}`.",
                    planned[idx].name, planned[child_idx].name, att.child_anchor, planned[child_idx].name
                )
            })?;
            let composed = parent_world_mat
                * parent_anchor.to_matrix()
                * att.offset.to_matrix()
                * child_anchor.to_matrix().inverse();
            let decomposed = crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
                .ok_or_else(|| {
                    format!(
                        "AI plan: attachment `{}` -> `{}` resolved to a non-finite transform (degenerate scale or invalid rotation).",
                        planned[idx].name, planned[child_idx].name
                    )
                })?;
            planned[child_idx].pos = decomposed.translation;
            planned[child_idx].rot = decomposed.rotation.normalize();

            dfs(child_idx, planned, children, visiting, visited, name_to_idx)?;
        }

        visiting[idx] = false;
        visited[idx] = true;
        Ok(())
    }

    let mut visiting = vec![false; planned.len()];
    let mut visited = vec![false; planned.len()];
    dfs(
        root_idx,
        planned,
        &children,
        &mut visiting,
        &mut visited,
        &name_to_idx,
    )?;

    Ok(())
}

pub(super) fn ai_plan_to_initial_draft_defs(
    plan: AiPlanJsonV1,
) -> Result<(Vec<Gen3dPlannedComponent>, String, Vec<ObjectDef>), String> {
    if plan.components.is_empty() {
        return Err("AI plan has no components.".into());
    }

    let mut components = plan.components;
    if components.len() > GEN3D_MAX_COMPONENTS {
        debug!(
            "Gen3D: truncating plan components from {} to {GEN3D_MAX_COMPONENTS}",
            components.len()
        );
        components.truncate(GEN3D_MAX_COMPONENTS);
    }

    let mut name_to_idx: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (idx, c) in components.iter().enumerate() {
        let name = c.name.trim();
        if name.is_empty() {
            return Err("AI plan has a component with empty `name`.".into());
        }
        if name_to_idx.insert(name.to_string(), idx).is_some() {
            return Err(format!("AI plan has duplicate component name `{name}`."));
        }
    }

    // Reuse groups are allowed to omit duplicate anchor definitions on targets, because geometry
    // (and most anchors) will be produced by deterministic copy later. However, the plan
    // conversion phase still needs the attachment interface anchors to exist so it can validate
    // and resolve the assembly tree. Hydrate ONLY missing required anchors for reuse targets.
    hydrate_reuse_target_attachment_anchors(&plan.reuse_groups, &name_to_idx, &mut components);

    let root_idx = if let Some(root_name) = plan
        .root_component
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    {
        name_to_idx.get(root_name).copied().ok_or_else(|| {
            format!("AI plan root_component `{root_name}` not found in components.")
        })?
    } else {
        let roots: Vec<usize> = components
            .iter()
            .enumerate()
            .filter(|(_, c)| c.attach_to.is_none())
            .map(|(i, _)| i)
            .collect();
        if roots.len() != 1 {
            return Err(format!(
                "AI plan must have exactly 1 root component (component with no attach_to); found {}.",
                roots.len()
            ));
        }
        roots[0]
    };

    let mut planned: Vec<Gen3dPlannedComponent> = Vec::with_capacity(components.len());
    for (idx, comp) in components.iter().enumerate() {
        let planned_size = Vec3::new(comp.size[0], comp.size[1], comp.size[2])
            .abs()
            .max(Vec3::splat(0.01));

        let anchors = anchors_from_ai("plan", &comp.name, &comp.anchors)?;
        let contacts = {
            let mut out: Vec<AiContactJson> = Vec::new();
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            for contact in comp.contacts.iter() {
                let name = contact.name.trim();
                let anchor = contact.anchor.trim();
                if name.is_empty() || anchor.is_empty() {
                    continue;
                }
                if !seen.insert(name.to_string()) {
                    continue;
                }
                if anchor != "origin" && !anchors.iter().any(|a| a.name.as_ref() == anchor) {
                    debug!(
                        "Gen3D: plan component `{}` contact `{}` references missing anchor `{}`; skipping contact",
                        comp.name, name, anchor
                    );
                    continue;
                }
                out.push(AiContactJson {
                    name: name.to_string(),
                    anchor: anchor.to_string(),
                    kind: contact.kind,
                    stance: contact.stance.clone(),
                });
            }
            out
        };

        let attach_to = match comp.attach_to.as_ref() {
            None => None,
            Some(att) => {
                let parent = att.parent.trim();
                let parent_anchor = att.parent_anchor.trim();
                let child_anchor = att.child_anchor.trim();
                if parent.is_empty() || parent_anchor.is_empty() || child_anchor.is_empty() {
                    return Err(format!(
                        "AI plan: component `{}` has invalid attach_to (empty fields).",
                        comp.name
                    ));
                }
                let parent_anchor_rot = if parent_anchor == "origin" {
                    Some(Quat::IDENTITY)
                } else {
                    name_to_idx
                        .get(parent)
                        .and_then(|idx| components.get(*idx))
                        .and_then(|pc| pc.anchors.iter().find(|a| a.name.trim() == parent_anchor))
                        .map(|a| {
                            let forward = ai_vec3(a.forward);
                            let up = ai_vec3(a.up);
                            plan_rotation_from_forward_up_strict(forward, up)
                        })
                        .transpose()
                        .map_err(|err| {
                            format!(
                                "AI plan: component `{}` attach_to references invalid parent anchor basis on `{}`.`{}`: {err}. Expected non-degenerate basis vectors in component-local axes (+X right, +Y up, +Z forward).",
                                comp.name, parent, parent_anchor
                            )
                        })?
                };
                let offset = attachment_offset_from_ai(att.offset.as_ref(), parent_anchor_rot)
                    .map_err(|err| {
                        format!(
                            "AI plan: component `{}` attach_to.offset is invalid: {err}",
                            comp.name
                        )
                    })?;
                Some(Gen3dPlannedAttachment {
                    parent: parent.to_string(),
                    parent_anchor: parent_anchor.to_string(),
                    child_anchor: child_anchor.to_string(),
                    offset,
                    joint: att.joint.clone(),
                    animations: Vec::new(),
                })
            }
        };

        planned.push(Gen3dPlannedComponent {
            display_name: format!("{}. {}", idx + 1, comp.name),
            name: comp.name.clone(),
            purpose: comp.purpose.clone(),
            modeling_notes: comp.modeling_notes.clone(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size,
            actual_size: None,
            anchors,
            contacts,
            attach_to,
        });
    }

    // Validate tree structure and referenced names/anchors.
    for (idx, comp) in planned.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            if idx != root_idx {
                return Err(format!(
                    "AI plan: only the root component may omit attach_to (found another root-like component `{}`)",
                    comp.name
                ));
            }
            continue;
        };

        let Some(parent_idx) = name_to_idx.get(att.parent.as_str()).copied() else {
            return Err(format!(
                "AI plan: component `{}` attach_to parent `{}` not found.",
                comp.name, att.parent
            ));
        };
        if parent_idx == idx {
            return Err(format!(
                "AI plan: component `{}` attach_to parent cannot be itself.",
                comp.name
            ));
        }
        if idx == root_idx {
            return Err(format!(
                "AI plan: root component `{}` must not have attach_to.",
                comp.name
            ));
        }

        if att.child_anchor != "origin"
            && !comp
                .anchors
                .iter()
                .any(|a| a.name.as_ref() == att.child_anchor)
        {
            return Err(format!(
                "AI plan: component `{}` missing required child_anchor `{}` in its anchors.",
                comp.name, att.child_anchor
            ));
        }

        if att.parent_anchor != "origin"
            && !planned[parent_idx]
                .anchors
                .iter()
                .any(|a| a.name.as_ref() == att.parent_anchor)
        {
            return Err(format!(
                "AI plan: parent component `{}` missing required parent_anchor `{}` in its anchors (child `{}`).",
                planned[parent_idx].name, att.parent_anchor, comp.name
            ));
        }
    }

    // Validate attachment join frames (avoid 180° flips caused by opposing anchor frames).
    //
    // Contract: parent/child anchors describe the SAME join frame, but each is expressed in its
    // own component-local coordinates. They do NOT need to numerically match. However, they must
    // not be 180° opposed (which produces confusing flips); if a flip is desired, it must be
    // authored explicitly via `attach_to.offset` rotation (with `rot_frame` set).
    for comp in planned.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let Some(parent_idx) = name_to_idx.get(att.parent.as_str()).copied() else {
            continue;
        };
        let parent_anchor = anchor_transform_from_defs(
            &planned[parent_idx].anchors,
            att.parent_anchor.as_str(),
        )
        .ok_or_else(|| {
            format!(
                "AI plan: parent component `{}` missing required parent_anchor `{}` in its anchors (child `{}`).",
                planned[parent_idx].name, att.parent_anchor, comp.name
            )
        })?;
        let child_anchor = anchor_transform_from_defs(&comp.anchors, att.child_anchor.as_str())
            .ok_or_else(|| {
                format!(
                    "AI plan: component `{}` missing required child_anchor `{}` in its anchors.",
                    comp.name, att.child_anchor
                )
            })?;

        let parent_forward = parent_anchor.rotation * Vec3::Z;
        let child_forward = child_anchor.rotation * Vec3::Z;
        let forward_dot = parent_forward.dot(child_forward);
        if !forward_dot.is_finite() || forward_dot < 0.0 {
            return Err(format!(
                "AI plan: attachment `{}` -> `{}` uses opposing anchor forward vectors (parent `{}` vs child `{}`): dot={:.3}. Anchors must describe a shared JOIN FRAME (but expressed in each component's local axes): do NOT oppose the join forward axis. If you need a flip, encode it via attach_to.offset.forward/up (with rot_frame set).",
                att.parent,
                comp.name,
                att.parent_anchor,
                att.child_anchor,
                forward_dot
            ));
        }
        let parent_up = parent_anchor.rotation * Vec3::Y;
        let child_up = child_anchor.rotation * Vec3::Y;
        let up_dot = parent_up.dot(child_up);
        if !up_dot.is_finite() || up_dot < 0.0 {
            return Err(format!(
                "AI plan: attachment `{}` -> `{}` uses opposing anchor up vectors (parent `{}` vs child `{}`): dot={:.3}. Anchors must describe a shared JOIN FRAME (but expressed in each component's local axes): do NOT oppose the join up axis. If you need a 180° roll, encode it via attach_to.offset.forward/up (with rot_frame set).",
                att.parent,
                comp.name,
                att.parent_anchor,
                att.child_anchor,
                up_dot
            ));
        }
    }

    resolve_planned_component_transforms(&mut planned, root_idx)?;

    let mut defs: Vec<ObjectDef> = Vec::with_capacity(planned.len() + 1);
    let mut component_ids: Vec<u128> = Vec::with_capacity(planned.len());

    for comp in planned.iter() {
        let id = builtin_object_id(&format!("gravimera/gen3d/component/{}", comp.name));
        component_ids.push(id);
        let size = comp.planned_size.abs().max(Vec3::splat(0.01));
        defs.push(ObjectDef {
            object_id: id,
            label: format!("gen3d_component_{}", comp.name).into(),
            size,
            ground_origin_y: None,
            collider: ColliderProfile::None,
            interaction: ObjectInteraction::none(),
            aim: None,
            mobility: None,
            anchors: comp.anchors.clone(),
            parts: Vec::new(),
            minimap_color: None,
            health_bar_offset_y: None,
            enemy: None,
            muzzle: None,
            projectile: None,
            attack: None,
        });
    }

    // Add child references according to the attachment tree.
    for (child_idx, comp) in planned.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let parent_idx = name_to_idx
            .get(att.parent.as_str())
            .copied()
            .ok_or_else(|| {
                format!(
                    "AI plan: internal error: missing parent `{}` for component `{}`",
                    att.parent, comp.name
                )
            })?;
        let child_id = component_ids[child_idx];
        let attachment = crate::object::registry::AttachmentDef {
            parent_anchor: att.parent_anchor.clone().into(),
            child_anchor: att.child_anchor.clone().into(),
        };
        let mut part = ObjectPartDef::object_ref(child_id, att.offset).with_attachment(attachment);
        part.animations.extend(att.animations.clone());
        defs[parent_idx].parts.push(part);
    }

    let mobility = mobility_from_ai(&plan.mobility);
    let anim_window_secs =
        attack_anim_window_secs_from_planned_components(&planned).unwrap_or(0.35);
    let mut attack_profile: Option<UnitAttackProfile> = None;
    let mut ranged_muzzle_component_idx: Option<usize> = None;
    if mobility.is_some() {
        match plan.attack.as_ref() {
            None | Some(AiAttackJson::None) => {}
            Some(AiAttackJson::Melee {
                cooldown_secs,
                damage,
                range,
                radius,
                arc_degrees,
            }) => {
                if let (Some(damage), Some(range), Some(radius), Some(arc_degrees)) =
                    (*damage, *range, *radius, *arc_degrees)
                {
                    let cooldown_secs = cooldown_secs.unwrap_or(0.8);
                    let cooldown_secs = if cooldown_secs.is_finite() {
                        cooldown_secs
                    } else {
                        0.0
                    }
                    .clamp(0.05, 60.0);

                    attack_profile = Some(UnitAttackProfile {
                        kind: UnitAttackKind::Melee,
                        cooldown_secs,
                        damage,
                        anim_window_secs,
                        melee: Some(MeleeAttackProfile {
                            range: range.abs().clamp(0.05, 50.0),
                            radius: radius.abs().clamp(0.01, 50.0),
                            arc_degrees: arc_degrees.abs().clamp(1.0, 360.0),
                        }),
                        ranged: None,
                    });
                } else {
                    warn!("Gen3D: AI plan melee attack missing required fields; ignoring attack.");
                }
            }
            Some(AiAttackJson::RangedProjectile {
                cooldown_secs,
                muzzle,
                projectile,
            }) => {
                if let (Some(muzzle), Some(projectile)) = (muzzle.as_ref(), projectile.as_ref()) {
                    let cooldown_secs = cooldown_secs.unwrap_or(0.6);
                    let cooldown_secs = if cooldown_secs.is_finite() {
                        cooldown_secs
                    } else {
                        0.0
                    }
                    .clamp(0.05, 60.0);
                    let component_name = muzzle.component.trim();
                    let anchor_name = muzzle.anchor.trim();
                    if component_name.is_empty() || anchor_name.is_empty() {
                        return Err("AI plan: attack.muzzle has empty fields.".into());
                    }
                    let Some(&component_idx) = name_to_idx.get(component_name) else {
                        return Err(format!(
                            "AI plan: attack.muzzle.component `{component_name}` not found in components."
                        ));
                    };
                    let component_def_anchors = planned[component_idx].anchors.as_slice();
                    if anchor_name != "origin"
                        && !component_def_anchors
                            .iter()
                            .any(|a| a.name.as_ref() == anchor_name)
                    {
                        return Err(format!(
                            "AI plan: attack.muzzle.anchor `{anchor_name}` not found on component `{component_name}`."
                        ));
                    }
                    let muzzle_ref = AnchorRef {
                        object_id: component_ids[component_idx],
                        anchor: anchor_name.to_string().into(),
                    };
                    ranged_muzzle_component_idx = Some(component_idx);

                    let projectile_def = gen3d_projectile_def_from_ai(projectile)?;
                    let projectile_prefab = projectile_def.object_id;
                    defs.push(projectile_def);

                    attack_profile = Some(UnitAttackProfile {
                        kind: UnitAttackKind::RangedProjectile,
                        cooldown_secs,
                        damage: projectile.damage,
                        anim_window_secs,
                        melee: None,
                        ranged: Some(RangedAttackProfile {
                            projectile_prefab,
                            muzzle: muzzle_ref,
                        }),
                    });
                } else {
                    warn!(
                        "Gen3D: AI plan ranged attack missing `muzzle`/`projectile`; ignoring attack."
                    );
                }
            }
        }
    }

    let mut aim_profile: Option<AimProfile> = None;
    if let Some(ai_aim) = plan.aim.as_ref() {
        let mut component_object_ids: Vec<u128> = Vec::new();
        for name in ai_aim.components.iter() {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(&idx) = name_to_idx.get(trimmed) else {
                return Err(format!(
                    "AI plan: aim.components includes unknown component `{trimmed}`."
                ));
            };
            component_object_ids.push(component_ids[idx]);
        }

        component_object_ids.sort_unstable();
        component_object_ids.dedup();

        if ai_aim.max_yaw_delta_degrees.is_some() || !component_object_ids.is_empty() {
            aim_profile = Some(AimProfile {
                max_yaw_delta_degrees: ai_aim.max_yaw_delta_degrees,
                components: component_object_ids,
            });
        }
    }

    if aim_profile.is_none() {
        if let Some(attack) = attack_profile.as_ref() {
            if matches!(attack.kind, UnitAttackKind::RangedProjectile) {
                if let Some(ranged) = attack.ranged.as_ref() {
                    let muzzle_idx = ranged_muzzle_component_idx.or_else(|| {
                        component_ids
                            .iter()
                            .position(|id| *id == ranged.muzzle.object_id)
                    });
                    let aim_object_id = muzzle_idx
                        .and_then(|idx| planned.get(idx).and_then(|c| c.attach_to.as_ref()))
                        .and_then(|att| name_to_idx.get(att.parent.as_str()).copied())
                        .filter(|idx| *idx != root_idx)
                        .map(|idx| component_ids[idx])
                        .unwrap_or(ranged.muzzle.object_id);
                    aim_profile = Some(AimProfile {
                        max_yaw_delta_degrees: None,
                        components: vec![aim_object_id],
                    });
                }
            }
        }
    }

    let mut root_size = compute_gen3d_root_size_from_planned_components(&planned);
    if !root_size.x.is_finite() || root_size.length_squared() <= 1e-6 {
        root_size = Vec3::ONE;
    }

    let root_component_id = component_ids[root_idx];
    let root_part = ObjectPartDef::object_ref(root_component_id, Transform::IDENTITY)
        .with_attachment(crate::object::registry::AttachmentDef {
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
        });

    // Gen3D outputs are static-only: no prefab-authored animation clips by default.

    defs.push(ObjectDef {
        object_id: gen3d_draft_object_id(),
        label: "gen3d_draft".into(),
        size: root_size,
        ground_origin_y: None,
        collider: collider_profile_from_ai(plan.collider, root_size)?,
        interaction: ObjectInteraction::none(),
        aim: aim_profile,
        mobility,
        anchors: Vec::new(),
        parts: vec![root_part],
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: attack_profile,
    });

    Ok((planned, plan.assembly_notes, defs))
}

fn hydrate_reuse_target_attachment_anchors(
    reuse_groups: &[AiReuseGroupJson],
    name_to_idx: &HashMap<String, usize>,
    components: &mut [AiPlanComponentJson],
) {
    fn is_reuse_kind(kind: AiReuseGroupKindJson) -> bool {
        matches!(
            kind,
            AiReuseGroupKindJson::Component
                | AiReuseGroupKindJson::CopyComponent
                | AiReuseGroupKindJson::Subtree
                | AiReuseGroupKindJson::CopyComponentSubtree
        )
    }

    fn find_anchor<'a>(anchors: &'a [AiAnchorJson], name: &str) -> Option<&'a AiAnchorJson> {
        anchors.iter().find(|a| a.name.trim() == name)
    }

    fn has_anchor(anchors: &[AiAnchorJson], name: &str) -> bool {
        anchors.iter().any(|a| a.name.trim() == name)
    }

    fn canonical_anchor(name: &str) -> AiAnchorJson {
        AiAnchorJson {
            name: name.to_string(),
            pos: [0.0, 0.0, 0.0],
            forward: [0.0, 0.0, 1.0],
            up: [0.0, 1.0, 0.0],
        }
    }

    if reuse_groups.is_empty() || components.is_empty() {
        return;
    }

    // Map each reuse target component index to its chosen source index.
    let mut target_to_source: HashMap<usize, usize> = HashMap::new();
    for group in reuse_groups.iter() {
        if !is_reuse_kind(group.kind) {
            continue;
        }
        let source_name = group.source.trim();
        if source_name.is_empty() {
            continue;
        }
        let Some(&source_idx) = name_to_idx.get(source_name) else {
            continue;
        };
        for raw_target in group.targets.iter() {
            let target_name = raw_target.trim();
            if target_name.is_empty() || target_name == source_name {
                continue;
            }
            let Some(&target_idx) = name_to_idx.get(target_name) else {
                continue;
            };
            target_to_source.entry(target_idx).or_insert(source_idx);
        }
    }
    if target_to_source.is_empty() {
        return;
    }

    // Collect anchor names that are REQUIRED by attachments:
    // - attach_to.child_anchor on each component
    // - attach_to.parent_anchor on the parent component for each edge
    let mut required_by_component: Vec<HashSet<String>> = vec![HashSet::new(); components.len()];
    for (idx, comp) in components.iter().enumerate() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let child_anchor = att.child_anchor.trim();
        if !child_anchor.is_empty() && child_anchor != "origin" {
            required_by_component[idx].insert(child_anchor.to_string());
        }
    }
    for comp in components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let parent_name = att.parent.trim();
        let parent_anchor = att.parent_anchor.trim();
        if parent_anchor.is_empty() || parent_anchor == "origin" {
            continue;
        }
        let Some(&parent_idx) = name_to_idx.get(parent_name) else {
            continue;
        };
        if parent_idx < required_by_component.len() {
            required_by_component[parent_idx].insert(parent_anchor.to_string());
        }
    }

    for (target_idx, source_idx) in target_to_source {
        if target_idx >= components.len()
            || source_idx >= components.len()
            || target_idx == source_idx
        {
            continue;
        }

        let required = match required_by_component.get(target_idx) {
            Some(required) if !required.is_empty() => required,
            _ => continue,
        };

        let target_snapshot = &components[target_idx];
        let source_snapshot = &components[source_idx];

        let mut to_add: Vec<AiAnchorJson> = Vec::new();
        for required_name in required.iter() {
            let name = required_name.trim();
            if name.is_empty() || name == "origin" {
                continue;
            }
            if has_anchor(&target_snapshot.anchors, name) {
                continue;
            }

            let is_mount_anchor = target_snapshot
                .attach_to
                .as_ref()
                .is_some_and(|att| att.child_anchor.trim() == name);

            let synthesized = if is_mount_anchor {
                // Mount interface anchor: match the parent's join frame basis so join-frame
                // validation can proceed deterministically for reuse targets.
                let (forward, up) = (|| -> Option<([f32; 3], [f32; 3])> {
                    let att = target_snapshot.attach_to.as_ref()?;
                    let parent_name = att.parent.trim();
                    let parent_anchor_name = att.parent_anchor.trim();
                    if parent_anchor_name.is_empty() || parent_anchor_name == "origin" {
                        return Some(([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]));
                    }
                    let &parent_idx = name_to_idx.get(parent_name)?;
                    let parent = components.get(parent_idx)?;
                    let parent_anchor = find_anchor(&parent.anchors, parent_anchor_name)?;
                    Some((parent_anchor.forward, parent_anchor.up))
                })()
                .unwrap_or(([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]));

                let pos = find_anchor(&source_snapshot.anchors, name)
                    .map(|a| a.pos)
                    .unwrap_or([0.0, 0.0, 0.0]);

                AiAnchorJson {
                    name: name.to_string(),
                    pos,
                    forward,
                    up,
                }
            } else if let Some(source_anchor) = find_anchor(&source_snapshot.anchors, name) {
                AiAnchorJson {
                    name: name.to_string(),
                    pos: source_anchor.pos,
                    forward: source_anchor.forward,
                    up: source_anchor.up,
                }
            } else {
                canonical_anchor(name)
            };

            debug!(
                "Gen3D: hydrating missing attachment anchor `{}` on reuse target `{}` (source `{}`)",
                name,
                target_snapshot.name.trim(),
                source_snapshot.name.trim()
            );
            to_add.push(synthesized);
        }

        if to_add.is_empty() {
            continue;
        }

        if let Some(target) = components.get_mut(target_idx) {
            for anchor in to_add {
                if !has_anchor(&target.anchors, anchor.name.trim()) {
                    target.anchors.push(anchor);
                }
            }
        }
    }
}

#[allow(dead_code)]
fn normalize_bottom_radial_components(_components: &mut [AiPlanComponentJson]) {}

#[allow(dead_code)]
fn normalize_radial_facing_sheet_components(_components: &mut [AiPlanComponentJson]) {}

fn compute_gen3d_root_size_from_planned_components(components: &[Gen3dPlannedComponent]) -> Vec3 {
    if components.is_empty() {
        return Vec3::ONE;
    }

    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for comp in components.iter() {
        let size = comp.planned_size.abs();
        let half = size * 0.5;
        let center = comp.pos;
        let ext = rotated_half_extents(half, comp.rot);
        min = min.min(center - ext);
        max = max.max(center + ext);
    }

    let mut size = (max - min).abs();
    if !size.x.is_finite() || size.length_squared() <= 1e-6 {
        size = Vec3::ONE;
    }
    size
}

pub(super) fn update_root_def_from_planned_components(
    components: &[Gen3dPlannedComponent],
    plan_collider: &Option<AiColliderJson>,
    draft: &mut Gen3dDraft,
) {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);

    for comp in components.iter() {
        let size = comp
            .actual_size
            .unwrap_or(comp.planned_size)
            .abs()
            .max(Vec3::splat(0.01));
        let half = size * 0.5;
        let center = comp.pos;
        let ext = rotated_half_extents(half, comp.rot);
        min = min.min(center - ext);
        max = max.max(center + ext);
    }

    let mut root_size = (max - min).abs();
    if !root_size.x.is_finite() || root_size.length_squared() <= 1e-6 {
        root_size = Vec3::splat(1.0);
    }

    if let Some(root) = draft
        .defs
        .iter_mut()
        .find(|def| def.object_id == gen3d_draft_object_id())
    {
        root.size = root_size;
        if let Ok(collider) = collider_profile_from_ai(plan_collider.clone(), root_size) {
            root.collider = collider;
        }
    }
}

// NOTE: Gen3D placement is driven strictly by the AI plan + component outputs.
// We intentionally avoid engine-side heuristic placement tweaks so the assembled result matches
// AI-provided anchors and offsets exactly.

#[derive(Clone, Debug, Default)]
pub(super) struct AiReviewDeltaApplyResult {
    pub(super) accepted: bool,
    pub(super) had_actions: bool,
    pub(super) regen_indices: Vec<usize>,
    pub(super) replan_reason: Option<String>,
    pub(super) tooling_feedback: Vec<AiToolingFeedbackJsonV1>,
}

fn parse_component_id_u128(value: &str) -> Option<u128> {
    let raw = value.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(uuid) = Uuid::parse_str(raw) {
        return Some(uuid.as_u128());
    }
    let hex = raw.strip_prefix("0x").unwrap_or(raw);
    if hex.len() == 32 && hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return u128::from_str_radix(hex, 16).ok();
    }
    None
}

fn component_object_id_for_name(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name))
}

fn component_index_from_object_id(
    components: &[Gen3dPlannedComponent],
    object_id: u128,
) -> Option<usize> {
    components
        .iter()
        .position(|c| component_object_id_for_name(&c.name) == object_id)
}

pub(super) fn sync_attachment_tree_to_defs(
    components: &[Gen3dPlannedComponent],
    draft: &mut Gen3dDraft,
) -> Result<(), String> {
    // Clear all attachment parts from component defs (but keep primitives/models).
    for def in draft.defs.iter_mut() {
        def.parts.retain(|p| {
            !(matches!(p.kind, ObjectPartKind::ObjectRef { .. }) && p.attachment.is_some())
        });
    }

    // Determine root component.
    let root_idx = components
        .iter()
        .position(|c| c.attach_to.is_none())
        .ok_or_else(|| "Internal error: no root component (missing attach_to=None).".to_string())?;
    let root_component_id = component_object_id_for_name(&components[root_idx].name);

    // Draft root always references the root component at origin.
    let root_id = gen3d_draft_object_id();
    let root_def = draft
        .defs
        .iter_mut()
        .find(|d| d.object_id == root_id)
        .ok_or_else(|| "Internal error: missing Gen3D draft root def.".to_string())?;
    let root_part = ObjectPartDef::object_ref(root_component_id, Transform::IDENTITY)
        .with_attachment(crate::object::registry::AttachmentDef {
            parent_anchor: "origin".into(),
            child_anchor: "origin".into(),
        });
    root_def.parts.push(root_part);

    // Rebuild component child references according to the plan tree.
    for child in components.iter() {
        let Some(att) = child.attach_to.as_ref() else {
            continue;
        };
        let parent_id = component_object_id_for_name(&att.parent);
        let child_id = component_object_id_for_name(&child.name);
        let Some(parent_def) = draft.defs.iter_mut().find(|d| d.object_id == parent_id) else {
            continue;
        };
        let attachment = crate::object::registry::AttachmentDef {
            parent_anchor: att.parent_anchor.clone().into(),
            child_anchor: att.child_anchor.clone().into(),
        };
        let mut part = ObjectPartDef::object_ref(child_id, att.offset).with_attachment(attachment);
        part.animations.extend(att.animations.clone());
        parent_def.parts.push(part);
    }

    Ok(())
}

pub(super) fn apply_ai_review_delta_actions(
    delta: AiReviewDeltaJsonV1,
    components: &mut [Gen3dPlannedComponent],
    plan_collider: &Option<AiColliderJson>,
    draft: &mut Gen3dDraft,
    run_dir: Option<&Path>,
) -> Result<AiReviewDeltaApplyResult, String> {
    let mut result = AiReviewDeltaApplyResult::default();
    if components.is_empty() {
        return Ok(result);
    }

    let accept_present = delta
        .actions
        .iter()
        .any(|a| matches!(a, AiReviewDeltaActionJsonV1::Accept));
    let accept_has_conflicts = accept_present
        && delta.actions.iter().any(|a| {
            !matches!(
                a,
                AiReviewDeltaActionJsonV1::Accept
                    | AiReviewDeltaActionJsonV1::ToolingFeedback { .. }
            )
        });
    if accept_has_conflicts {
        return Err(
            "review_delta_v1: `accept` cannot be combined with other actions (except tooling_feedback)."
                .into(),
        );
    }

    let mut regen: HashSet<usize> = HashSet::new();

    let root_id = gen3d_draft_object_id();

    for action in delta.actions {
        match action {
            AiReviewDeltaActionJsonV1::Accept => {
                result.accepted = true;
            }
            AiReviewDeltaActionJsonV1::ToolingFeedback { feedback } => {
                result.tooling_feedback.push(feedback);
            }
            AiReviewDeltaActionJsonV1::Replan { reason } => {
                let reason = reason.trim();
                result.replan_reason = Some(if reason.is_empty() {
                    "replan requested".to_string()
                } else {
                    reason.to_string()
                });
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::RegenComponent {
                component_id,
                updated_modeling_notes,
                reason,
            } => {
                let Some(object_id) = parse_component_id_u128(&component_id) else {
                    continue;
                };
                let Some(idx) = component_index_from_object_id(components, object_id) else {
                    continue;
                };
                let notes = updated_modeling_notes.trim();
                if !notes.is_empty() {
                    components[idx].modeling_notes = notes.to_string();
                }
                regen.insert(idx);
                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta regen_component {} ({}) reason={}",
                        component_id,
                        components[idx].name,
                        reason.trim()
                    );
                }
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::TweakComponentTransform {
                component_id,
                set,
                delta,
                reason,
            } => {
                let Some(object_id) = parse_component_id_u128(&component_id) else {
                    continue;
                };
                let Some(idx) = component_index_from_object_id(components, object_id) else {
                    continue;
                };
                let Some(att) = components[idx].attach_to.as_mut() else {
                    continue;
                };

                if let Some(set) = set.as_ref() {
                    if let Some(pos) = set.pos {
                        let v = Vec3::new(pos[0], pos[1], pos[2]);
                        if v.is_finite() {
                            att.offset.translation = v;
                        }
                    }
                    if let Some(scale) = set.scale {
                        let v = Vec3::new(scale[0], scale[1], scale[2]);
                        if v.is_finite() {
                            att.offset.scale = v;
                        }
                    }
                    if let Some(rot) = set.rot.as_ref() {
                        let q = match rot {
                            AiRotationJsonV1::Basis { forward, up } => {
                                plan_rotation_from_forward_up_strict(
                                    Vec3::new(forward[0], forward[1], forward[2]),
                                    Vec3::new(up[0], up[1], up[2]),
                                )
                                .map_err(|err| {
                                    format!(
                                        "review_delta_v1: tweak_component_transform set.rot basis is invalid: {err}. Expected non-degenerate basis vectors in component-local axes (+X right, +Y up, +Z forward)."
                                    )
                                })?
                            }
                            AiRotationJsonV1::Quat { quat_xyzw } => Quat::from_xyzw(
                                quat_xyzw[0],
                                quat_xyzw[1],
                                quat_xyzw[2],
                                quat_xyzw[3],
                            )
                            .normalize(),
                        };
                        if q.is_finite() {
                            att.offset.rotation = q;
                        }
                    }
                }
                if let Some(delta) = delta.as_ref() {
                    if let Some(pos) = delta.pos {
                        let v = Vec3::new(pos[0], pos[1], pos[2]);
                        if v.is_finite() {
                            att.offset.translation += v;
                        }
                    }
                    if let Some(scale) = delta.scale {
                        let v = Vec3::new(scale[0], scale[1], scale[2]);
                        if v.is_finite() {
                            att.offset.scale = att.offset.scale * v;
                        }
                    }
                    if let Some(quat_xyzw) = delta.rot_quat_xyzw {
                        let q =
                            Quat::from_xyzw(quat_xyzw[0], quat_xyzw[1], quat_xyzw[2], quat_xyzw[3]);
                        if q.is_finite() {
                            att.offset.rotation = (q * att.offset.rotation).normalize();
                        }
                    }
                }

                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta tweak_component_transform {} ({}) reason={}",
                        component_id,
                        components[idx].name,
                        reason.trim()
                    );
                }
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::TweakAnchor {
                component_id,
                anchor_name,
                set,
                delta,
                reason,
            } => {
                let Some(object_id) = parse_component_id_u128(&component_id) else {
                    continue;
                };
                let Some(idx) = component_index_from_object_id(components, object_id) else {
                    continue;
                };
                let Some(anchor_idx) = components[idx]
                    .anchors
                    .iter()
                    .position(|a| a.name.as_ref() == anchor_name)
                else {
                    continue;
                };

                let component_name = components[idx].name.clone();

                let old_anchor_tf = components[idx].anchors[anchor_idx].transform;
                {
                    let anchor = &mut components[idx].anchors[anchor_idx];

                    if let Some(set) = set.as_ref() {
                        if let Some(pos) = set.pos {
                            let v = Vec3::new(pos[0], pos[1], pos[2]);
                            if v.is_finite() {
                                anchor.transform.translation = v;
                            }
                        }
                        if set.forward.is_some() || set.up.is_some() {
                            let Some(forward) = set.forward else {
                                return Err(format!(
                                    "review_delta_v1: tweak_anchor {} ({}) anchor `{}`: `set.up` provided without `set.forward`; provide both `set.forward` and `set.up` (component-local axes +X right, +Y up, +Z forward).",
                                    component_id, component_name, anchor_name
                                ));
                            };
                            let Some(up) = set.up else {
                                return Err(format!(
                                    "review_delta_v1: tweak_anchor {} ({}) anchor `{}`: `set.forward` provided without `set.up`; provide both `set.forward` and `set.up` (component-local axes +X right, +Y up, +Z forward).",
                                    component_id, component_name, anchor_name
                                ));
                            };
                            let f = Vec3::new(forward[0], forward[1], forward[2]);
                            let u = Vec3::new(up[0], up[1], up[2]);
                            let q = plan_rotation_from_forward_up_strict(f, u).map_err(|err| {
                                format!(
                                    "review_delta_v1: tweak_anchor {} ({}) anchor `{}` has invalid set.forward/set.up basis: {err}. Expected non-degenerate basis vectors in component-local axes (+X right, +Y up, +Z forward).",
                                    component_id, component_name, anchor_name
                                )
                            })?;
                            anchor.transform.rotation = q;
                        }
                    }
                    if let Some(delta) = delta.as_ref() {
                        if let Some(pos) = delta.pos {
                            let v = Vec3::new(pos[0], pos[1], pos[2]);
                            if v.is_finite() {
                                anchor.transform.translation += v;
                            }
                        }
                        if let Some(quat_xyzw) = delta.rot_quat_xyzw {
                            let q = Quat::from_xyzw(
                                quat_xyzw[0],
                                quat_xyzw[1],
                                quat_xyzw[2],
                                quat_xyzw[3],
                            );
                            if q.is_finite() {
                                anchor.transform.rotation =
                                    (q * anchor.transform.rotation).normalize();
                            }
                        }
                    }
                }

                let new_anchor_tf = components[idx].anchors[anchor_idx].transform;
                if old_anchor_tf != new_anchor_tf {
                    let old_anchor_mat = old_anchor_tf.to_matrix();
                    let new_anchor_mat = new_anchor_tf.to_matrix();
                    let old_anchor_inv = old_anchor_mat.inverse();
                    let new_anchor_inv = new_anchor_mat.inverse();

                    fn tf_json(t: Transform) -> serde_json::Value {
                        serde_json::json!({
                            "pos": [t.translation.x, t.translation.y, t.translation.z],
                            "rot_quat_xyzw": [t.rotation.x, t.rotation.y, t.rotation.z, t.rotation.w],
                            "scale": [t.scale.x, t.scale.y, t.scale.z],
                        })
                    }

                    // If this anchor is used as the component's child anchor, changing it would
                    // otherwise "jump" the whole component in the assembly. Compensate by updating
                    // the attachment offset so the assembled rest pose stays the same:
                    // parent_anchor * offset' * inv(child_anchor_new) == parent_anchor * offset * inv(child_anchor_old)
                    if let Some(att) = components[idx].attach_to.as_mut() {
                        if att.child_anchor.as_str() == anchor_name {
                            let offset_mat = att.offset.to_matrix();
                            let compensated = offset_mat * old_anchor_inv * new_anchor_mat;
                            if let Some(next) =
                                crate::geometry::mat4_to_transform_allow_degenerate_scale(
                                    compensated,
                                )
                            {
                                if next != att.offset {
                                    append_gen3d_jsonl_artifact(
                                        run_dir,
                                        "applied_defaults.jsonl",
                                        &serde_json::json!({
                                            "kind": "rebase_child_offset_for_child_anchor_change",
                                            "component": component_name.as_str(),
                                            "child_anchor": anchor_name.as_str(),
                                            "offset_before": tf_json(att.offset),
                                            "offset_after": tf_json(next),
                                        }),
                                    );
                                }
                                att.offset = next;
                            }
                        }
                    }

                    // If any children attach to this component via this anchor, changing the parent
                    // anchor would otherwise "jump" those children. Compensate by rebasing their
                    // offsets into the new parent-anchor frame:
                    // old_parent_anchor * offset == new_parent_anchor * offset'
                    // => offset' = inv(new_parent_anchor) * old_parent_anchor * offset
                    let parent_name = components[idx].name.clone();
                    for child in components.iter_mut() {
                        let Some(att) = child.attach_to.as_mut() else {
                            continue;
                        };
                        if att.parent != parent_name || att.parent_anchor.as_str() != anchor_name {
                            continue;
                        }
                        let offset_mat = att.offset.to_matrix();
                        let compensated = new_anchor_inv * old_anchor_mat * offset_mat;
                        if let Some(next) =
                            crate::geometry::mat4_to_transform_allow_degenerate_scale(compensated)
                        {
                            if next != att.offset {
                                append_gen3d_jsonl_artifact(
                                    run_dir,
                                    "applied_defaults.jsonl",
                                    &serde_json::json!({
                                        "kind": "rebase_child_offset_for_parent_anchor_change",
                                        "parent_component": parent_name.as_str(),
                                        "parent_anchor": anchor_name.as_str(),
                                        "child_component": child.name.as_str(),
                                        "offset_before": tf_json(att.offset),
                                        "offset_after": tf_json(next),
                                    }),
                                );
                            }
                            att.offset = next;
                        }
                    }
                }

                // Keep the runtime def anchor list in sync too.
                if let Some(def) = draft.defs.iter_mut().find(|d| d.object_id == object_id) {
                    if let Some(def_anchor) = def
                        .anchors
                        .iter_mut()
                        .find(|a| a.name.as_ref() == anchor_name)
                    {
                        *def_anchor = components[idx].anchors[anchor_idx].clone();
                    }
                }

                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta tweak_anchor {} ({}) anchor={} reason={}",
                        component_id,
                        components[idx].name,
                        anchor_name,
                        reason.trim()
                    );
                }
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::TweakAttachment {
                component_id,
                set,
                reason,
            } => {
                let Some(object_id) = parse_component_id_u128(&component_id) else {
                    continue;
                };
                let Some(idx) = component_index_from_object_id(components, object_id) else {
                    continue;
                };

                let Some(parent_object_id) = parse_component_id_u128(&set.parent_component_id)
                else {
                    continue;
                };
                let Some(parent_idx) = component_index_from_object_id(components, parent_object_id)
                else {
                    continue;
                };

                let parent_anchor = set.parent_anchor.trim();
                let child_anchor = set.child_anchor.trim();
                if parent_anchor.is_empty() || child_anchor.is_empty() {
                    return Err(format!(
                        "review_delta_v1: tweak_attachment {} ({}) has empty parent_anchor/child_anchor.",
                        component_id, components[idx].name
                    ));
                }
                if parent_idx == idx {
                    return Err(format!(
                        "review_delta_v1: tweak_attachment {} ({}) parent_component_id cannot be itself.",
                        component_id, components[idx].name
                    ));
                }
                let parent_anchor_rot = if parent_anchor == "origin" {
                    Some(Quat::IDENTITY)
                } else {
                    Some(
                        components[parent_idx]
                            .anchors
                            .iter()
                            .find(|a| a.name.as_ref() == parent_anchor)
                            .map(|a| a.transform.rotation)
                            .ok_or_else(|| {
                                format!(
                                    "review_delta_v1: tweak_attachment {} ({}) references missing parent_anchor `{}` on parent component `{}`.",
                                    component_id, components[idx].name, parent_anchor, components[parent_idx].name
                                )
                            })?,
                    )
                };
                if child_anchor != "origin"
                    && !components[idx]
                        .anchors
                        .iter()
                        .any(|a| a.name.as_ref() == child_anchor)
                {
                    return Err(format!(
                        "review_delta_v1: tweak_attachment {} ({}) references missing child_anchor `{}` on component `{}`.",
                        component_id, components[idx].name, child_anchor, components[idx].name
                    ));
                }
                let offset = attachment_offset_from_ai(set.offset.as_ref(), parent_anchor_rot)
                    .map_err(|err| {
                        format!(
                            "review_delta_v1: tweak_attachment {} ({}) has invalid offset: {err}",
                            component_id, components[idx].name
                        )
                    })?;
                let animations = components[idx]
                    .attach_to
                    .as_ref()
                    .map(|att| att.animations.clone())
                    .unwrap_or_default();
                let joint = components[idx].attach_to.as_ref().and_then(|att| {
                    if att.parent == components[parent_idx].name
                        && att.parent_anchor == parent_anchor
                        && att.child_anchor == child_anchor
                    {
                        att.joint.clone()
                    } else {
                        None
                    }
                });

                components[idx].attach_to = Some(Gen3dPlannedAttachment {
                    parent: components[parent_idx].name.clone(),
                    parent_anchor: set.parent_anchor.trim().to_string(),
                    child_anchor: child_anchor.to_string(),
                    offset,
                    joint,
                    animations,
                });

                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta tweak_attachment {} ({}) reason={}",
                        component_id,
                        components[idx].name,
                        reason.trim()
                    );
                }
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::TweakContact {
                component_id,
                contact_name,
                stance,
                reason,
            } => {
                let Some(object_id) = parse_component_id_u128(&component_id) else {
                    continue;
                };
                let Some(idx) = component_index_from_object_id(components, object_id) else {
                    continue;
                };
                let contact_name = contact_name.trim();
                if contact_name.is_empty() {
                    continue;
                }
                let Some(contact) = components[idx]
                    .contacts
                    .iter_mut()
                    .find(|c| c.name.trim() == contact_name)
                else {
                    continue;
                };

                let before = contact.stance.clone();
                if let Some(stance) = stance {
                    let move_is_spin = components[idx]
                        .attach_to
                        .as_ref()
                        .and_then(|att| {
                            att.animations.iter().find(|s| s.channel.as_ref() == "move")
                        })
                        .map(|slot| matches!(slot.spec.clip, PartAnimationDef::Spin { .. }))
                        .unwrap_or(false);

                    // Guardrail: for ground contacts, clearing stance makes motion validation
                    // thrash between slip and stance-missing. The only generic exception is a
                    // wheel-like move that is a pure `spin` (stance validation is skipped).
                    let clearing_ground_stance =
                        stance.is_none() && contact.kind == AiContactKindJson::Ground;
                    if !(clearing_ground_stance && !move_is_spin) {
                        contact.stance = stance;
                    }
                }

                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta tweak_contact {} ({}) contact={} reason={}",
                        component_id,
                        components[idx].name,
                        contact_name,
                        reason.trim()
                    );
                }
                if contact.stance != before {
                    result.had_actions = true;
                }
            }
            AiReviewDeltaActionJsonV1::TweakMobility { mobility, reason } => {
                let mobility = mobility_from_ai(&mobility);
                let root_def = draft
                    .defs
                    .iter_mut()
                    .find(|def| def.object_id == root_id)
                    .ok_or_else(|| "Internal error: missing Gen3D draft root def.".to_string())?;
                root_def.mobility = mobility;
                if root_def.mobility.is_none() {
                    root_def.attack = None;
                }
                if !reason.trim().is_empty() {
                    debug!(
                        "Gen3D: review-delta tweak_mobility reason={}",
                        reason.trim()
                    );
                }
                result.had_actions = true;
            }
            AiReviewDeltaActionJsonV1::TweakAttack { attack, reason } => {
                let movable = draft
                    .defs
                    .iter()
                    .find(|def| def.object_id == root_id)
                    .and_then(|def| def.mobility.as_ref())
                    .is_some();
                if !movable {
                    let root_def = draft
                        .defs
                        .iter_mut()
                        .find(|def| def.object_id == root_id)
                        .ok_or_else(|| {
                            "Internal error: missing Gen3D draft root def.".to_string()
                        })?;
                    root_def.attack = None;
                    result.had_actions = true;
                    continue;
                }

                let mut next_attack: Option<UnitAttackProfile> = None;
                match attack {
                    AiAttackJson::None => {}
                    AiAttackJson::Melee {
                        cooldown_secs,
                        damage,
                        range,
                        radius,
                        arc_degrees,
                    } => {
                        let (Some(damage), Some(range), Some(radius), Some(arc_degrees)) =
                            (damage, range, radius, arc_degrees)
                        else {
                            return Err("Melee attack missing required fields.".into());
                        };
                        let cooldown_secs = cooldown_secs.unwrap_or(0.8).clamp(0.05, 60.0);
                        let anim_window_secs =
                            attack_anim_window_secs_from_planned_components(components)
                                .unwrap_or(0.35);
                        next_attack = Some(UnitAttackProfile {
                            kind: UnitAttackKind::Melee,
                            cooldown_secs,
                            damage,
                            anim_window_secs,
                            melee: Some(MeleeAttackProfile {
                                range: range.abs().clamp(0.05, 50.0),
                                radius: radius.abs().clamp(0.01, 50.0),
                                arc_degrees: arc_degrees.abs().clamp(1.0, 360.0),
                            }),
                            ranged: None,
                        });
                    }
                    AiAttackJson::RangedProjectile {
                        cooldown_secs,
                        muzzle,
                        projectile,
                    } => {
                        let Some(muzzle) = muzzle.as_ref() else {
                            return Err("Ranged attack missing `muzzle`.".into());
                        };
                        let Some(projectile) = projectile.as_ref() else {
                            return Err("Ranged attack missing `projectile`.".into());
                        };

                        let cooldown_secs = cooldown_secs.unwrap_or(0.6).clamp(0.05, 60.0);
                        let component_name = muzzle.component.trim();
                        let anchor_name = muzzle.anchor.trim();
                        if component_name.is_empty() || anchor_name.is_empty() {
                            return Err("attack.muzzle has empty fields.".into());
                        }

                        let Some(component_idx) =
                            components.iter().position(|c| c.name == component_name)
                        else {
                            return Err(format!(
                                "attack.muzzle.component `{component_name}` not found in components."
                            ));
                        };
                        if anchor_name != "origin"
                            && !components[component_idx]
                                .anchors
                                .iter()
                                .any(|a| a.name.as_ref() == anchor_name)
                        {
                            return Err(format!(
                                "attack.muzzle.anchor `{anchor_name}` not found on component `{component_name}`."
                            ));
                        }
                        let muzzle_ref = AnchorRef {
                            object_id: component_object_id_for_name(component_name),
                            anchor: anchor_name.to_string().into(),
                        };

                        let projectile_def = gen3d_projectile_def_from_ai(projectile)?;
                        let projectile_prefab = projectile_def.object_id;
                        if let Some(existing) = draft
                            .defs
                            .iter_mut()
                            .find(|d| d.object_id == projectile_prefab)
                        {
                            *existing = projectile_def;
                        } else {
                            draft.defs.push(projectile_def);
                        }

                        let anim_window_secs =
                            attack_anim_window_secs_from_planned_components(components)
                                .unwrap_or(0.35);
                        next_attack = Some(UnitAttackProfile {
                            kind: UnitAttackKind::RangedProjectile,
                            cooldown_secs,
                            damage: projectile.damage,
                            anim_window_secs,
                            melee: None,
                            ranged: Some(RangedAttackProfile {
                                projectile_prefab,
                                muzzle: muzzle_ref,
                            }),
                        });
                    }
                }

                let root_def = draft
                    .defs
                    .iter_mut()
                    .find(|def| def.object_id == root_id)
                    .ok_or_else(|| "Internal error: missing Gen3D draft root def.".to_string())?;
                root_def.attack = next_attack;
                if !reason.trim().is_empty() {
                    debug!("Gen3D: review-delta tweak_attack reason={}", reason.trim());
                }
                result.had_actions = true;
            }
        }
    }

    // Clamp regen list for safety.
    let mut regen_indices: Vec<usize> = regen.into_iter().collect();
    regen_indices.sort_unstable();
    if regen_indices.len() > 16 {
        regen_indices.truncate(16);
    }
    result.regen_indices = regen_indices;

    if result.had_actions {
        if let Some(root_idx) = components.iter().position(|c| c.attach_to.is_none()) {
            resolve_planned_component_transforms(components, root_idx)?;
        }
        sync_attachment_tree_to_defs(components, draft)?;
        update_root_def_from_planned_components(components, plan_collider, draft);
    }

    Ok(result)
}

pub(super) fn ai_to_component_def(
    component: &Gen3dPlannedComponent,
    mut ai: AiDraftJsonV1,
    run_dir: Option<&Path>,
) -> Result<ObjectDef, String> {
    if ai.version == 0 {
        ai.version = 2;
    }
    if ai.version != 2 {
        return Err(format!(
            "Unsupported AI draft version {} (expected 2)",
            ai.version
        ));
    }

    let component_name = component.name.as_str();
    let object_id = builtin_object_id(&format!("gravimera/gen3d/component/{}", component_name));

    let draft_anchors = anchors_from_ai("draft", component_name, &ai.anchors)?;
    let mut anchors = merge_component_anchors_from_plan_and_draft(
        component_name,
        &component.anchors,
        draft_anchors,
    )?;

    let mut parts: Vec<ObjectPartDef> = Vec::with_capacity(ai.parts.len());
    for (part_idx, part) in ai.parts.iter().enumerate() {
        let mut mesh = match part.primitive {
            AiPrimitiveJson::Cuboid => MeshKey::UnitCube,
            AiPrimitiveJson::Cylinder => MeshKey::UnitCylinder,
            AiPrimitiveJson::Cone => MeshKey::UnitCone,
            AiPrimitiveJson::Sphere => MeshKey::UnitSphere,
        };

        let params = part
            .params
            .as_ref()
            .and_then(|v| primitive_params_from_ai(v, mesh).ok())
            .flatten();
        if let Some(params) = params {
            mesh = match params {
                PrimitiveParams::Capsule { .. } => MeshKey::UnitCapsule,
                PrimitiveParams::ConicalFrustum { .. } => MeshKey::UnitConicalFrustum,
                PrimitiveParams::Torus { .. } => MeshKey::UnitTorus,
            };
        }

        let color = part.color.map(|rgba| {
            Color::srgba(
                rgba[0].clamp(0.0, 1.0),
                rgba[1].clamp(0.0, 1.0),
                rgba[2].clamp(0.0, 1.0),
                rgba[3].clamp(0.0, 1.0),
            )
        });

        let rot = quat_from_forward_up_or_identity(
            &format!("AI draft: component `{component_name}` part[{part_idx}]"),
            part.forward,
            part.up,
        )?;

        let mut part_def = ObjectPartDef::primitive(
            PrimitiveVisualDef::Primitive {
                mesh,
                params,
                color: color.unwrap_or(Color::srgb(0.85, 0.87, 0.90)),
                unlit: false,
            },
            Transform::from_translation(Vec3::new(part.pos[0], part.pos[1], part.pos[2]))
                .with_rotation(rot)
                .with_scale(Vec3::new(part.scale[0], part.scale[1], part.scale[2])),
        );
        part_def.part_id = Some(builtin_object_id(&format!(
            "gravimera/gen3d/part/{}/{}",
            component_name, part_idx
        )));
        part_def.render_priority = part.render_priority;
        parts.push(part_def);
    }

    validate_component_part_transforms(component_name, &parts)?;
    canonicalize_component_parts(component_name, &mut parts, &mut anchors, run_dir);
    override_required_anchor_rotations_from_plan(
        component_name,
        &component.anchors,
        &mut anchors,
        run_dir,
    );

    let size = size_from_primitive_parts(&parts);
    error_if_component_axis_permutation(component_name, component.planned_size, size)?;
    let collider = collider_profile_from_ai(ai.collider.clone(), size)?;

    Ok(ObjectDef {
        object_id,
        label: format!("gen3d_component_{}", component_name).into(),
        size,
        ground_origin_y: None,
        collider,
        interaction: ObjectInteraction::none(),
        aim: None,
        mobility: None,
        anchors,
        parts,
        minimap_color: None,
        health_bar_offset_y: None,
        enemy: None,
        muzzle: None,
        projectile: None,
        attack: None,
    })
}

fn size_match_error(actual: Vec3, planned: Vec3) -> f32 {
    let planned = planned.abs().max(Vec3::splat(0.01));
    let actual = actual.abs().max(Vec3::splat(0.01));
    ((actual - planned).abs() / planned).x
        + ((actual - planned).abs() / planned).y
        + ((actual - planned).abs() / planned).z
}

fn error_if_component_axis_permutation(
    component_name: &str,
    planned_size: Vec3,
    measured_size: Vec3,
) -> Result<(), String> {
    // If a component comes back with its AABB dimensions effectively permuted relative to the
    // plan's `size`, we should NOT silently rotate it. The plan is a contract; the AI must obey
    // local axes conventions (+X right, +Y up, +Z forward).
    //
    // We still detect the "likely-permuted axes" case so we can fail fast and trigger regeneration
    // with an explicit, actionable error message.
    const MAX_OK_ERROR: f32 = 0.20;
    const MIN_BAD_ERROR: f32 = 0.70;
    const MIN_IMPROVEMENT: f32 = 0.50;

    let planned = planned_size.abs().max(Vec3::splat(0.01));
    let measured = measured_size.abs().max(Vec3::splat(0.01));
    let err_current = size_match_error(measured, planned);
    if err_current < MIN_BAD_ERROR {
        return Ok(());
    }

    let mut best_perm = "xyz";
    let mut best_size = measured;
    let mut best_err = err_current;

    for (perm, perm_size) in [
        ("xyz", Vec3::new(measured.x, measured.y, measured.z)),
        ("xzy", Vec3::new(measured.x, measured.z, measured.y)),
        ("yxz", Vec3::new(measured.y, measured.x, measured.z)),
        ("yzx", Vec3::new(measured.y, measured.z, measured.x)),
        ("zxy", Vec3::new(measured.z, measured.x, measured.y)),
        ("zyx", Vec3::new(measured.z, measured.y, measured.x)),
    ] {
        if perm == "xyz" {
            continue;
        }
        let err = size_match_error(perm_size, planned);
        if err < best_err {
            best_err = err;
            best_perm = perm;
            best_size = perm_size;
        }
    }

    if best_perm == "xyz" {
        return Ok(());
    }
    if best_err > MAX_OK_ERROR {
        return Ok(());
    }
    if err_current - best_err < MIN_IMPROVEMENT {
        return Ok(());
    }

    let hint = match best_perm {
        "xzy" => "swap Y and Z",
        "yxz" => "swap X and Y",
        "yzx" => "cycle axes (X<-Y<-Z)",
        "zxy" => "cycle axes (X<-Z<-Y)",
        "zyx" => "swap X and Z",
        _ => "permute axes",
    };

    Err(format!(
        "AI draft for component `{component_name}` appears to have permuted local axes ({hint}). Planned target_size=[{:.3},{:.3},{:.3}] but measured local AABB=[{:.3},{:.3},{:.3}] (err_before={:.3}). A permuted AABB `{best_perm}` would match much better: [{:.3},{:.3},{:.3}] (err_after={:.3}). The engine will not auto-rotate components; regenerate this component with correct local axes (+X right, +Y up, +Z forward) and ensure each part's `scale=[x,y,z]` corresponds to those axes (do not swap axes).",
        planned.x,
        planned.y,
        planned.z,
        measured.x,
        measured.y,
        measured.z,
        err_current,
        best_size.x,
        best_size.y,
        best_size.z,
        best_err,
    ))
}

fn validate_component_part_transforms(
    component_name: &str,
    parts: &[ObjectPartDef],
) -> Result<(), String> {
    for (part_idx, part) in parts.iter().enumerate() {
        let t = part.transform;
        if !t.translation.is_finite() {
            return Err(format!(
                "AI draft: component `{component_name}` part[{part_idx}] has non-finite `pos`"
            ));
        }
        if !t.rotation.is_finite() {
            return Err(format!(
                "AI draft: component `{component_name}` part[{part_idx}] has non-finite rotation"
            ));
        }
        if !t.scale.is_finite() {
            return Err(format!(
                "AI draft: component `{component_name}` part[{part_idx}] has non-finite `scale`"
            ));
        }
    }
    Ok(())
}

fn canonicalize_component_parts(
    component_name: &str,
    parts: &mut [ObjectPartDef],
    anchors: &mut [crate::object::registry::AnchorDef],
    run_dir: Option<&Path>,
) {
    const CENTER_EPS: f32 = 0.001;

    let Some((min, max)) = primitive_parts_aabb(parts) else {
        return;
    };
    let size = (max - min).abs();
    if !size.is_finite() || size.length_squared() <= 1e-8 {
        return;
    }

    let center = (min + max) * 0.5;
    if center.length_squared() > CENTER_EPS * CENTER_EPS {
        for part in parts.iter_mut() {
            part.transform.translation -= center;
        }
        for anchor in anchors.iter_mut() {
            anchor.transform.translation -= center;
        }
        debug!(
            "Gen3D: recentered component {component_name} by [{:.3},{:.3},{:.3}]",
            center.x, center.y, center.z
        );
        append_gen3d_jsonl_artifact(
            run_dir,
            "applied_defaults.jsonl",
            &serde_json::json!({
                "kind": "recenter_component",
                "component": component_name,
                "center_delta": [center.x, center.y, center.z],
            }),
        );
    }
}

pub(super) fn primitive_params_from_ai(
    value: &serde_json::Value,
    mesh: MeshKey,
) -> Result<Option<PrimitiveParams>, String> {
    let kind = value
        .get("kind")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match kind.as_str() {
        "capsule" => {
            let half_length = value
                .get("half_length")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5) as f32;
            let radius = value.get("radius").and_then(|v| v.as_f64()).unwrap_or(0.25) as f32;
            Ok(Some(PrimitiveParams::Capsule {
                half_length: half_length.max(0.01),
                radius: radius.max(0.01),
            }))
        }
        "conical_frustum" => {
            let top_radius = value
                .get("top_radius")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.25) as f32;
            let bottom_radius = value
                .get("bottom_radius")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.35) as f32;
            let height = value.get("height").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32;
            Ok(Some(PrimitiveParams::ConicalFrustum {
                radius_top: top_radius.max(0.01),
                radius_bottom: bottom_radius.max(0.01),
                height: height.max(0.01),
            }))
        }
        "torus" => {
            let minor_radius = value
                .get("minor_radius")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.15) as f32;
            let major_radius = value
                .get("major_radius")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.5) as f32;
            Ok(Some(PrimitiveParams::Torus {
                minor_radius: minor_radius.max(0.01),
                major_radius: major_radius.max(0.01),
            }))
        }
        "" => Ok(None),
        other => {
            debug!("Gen3D: ignoring unknown primitive params kind={other} for mesh={mesh:?}");
            Ok(None)
        }
    }
}

fn primitive_base_size(mesh: MeshKey, params: Option<&PrimitiveParams>) -> Vec3 {
    match mesh {
        MeshKey::UnitCube => Vec3::ONE,
        MeshKey::UnitCylinder => Vec3::ONE,
        MeshKey::UnitCone => Vec3::ONE,
        MeshKey::UnitSphere => Vec3::ONE,
        MeshKey::UnitPlane => Vec3::ONE,
        MeshKey::UnitTriangle => Vec3::ONE,
        MeshKey::UnitTetrahedron => Vec3::ONE,
        MeshKey::UnitCapsule => match params {
            Some(PrimitiveParams::Capsule {
                half_length,
                radius,
            }) => Vec3::new(radius * 2.0, (half_length + radius) * 2.0, radius * 2.0),
            _ => Vec3::ONE,
        },
        MeshKey::UnitConicalFrustum => match params {
            Some(PrimitiveParams::ConicalFrustum {
                radius_top,
                radius_bottom,
                height,
            }) => {
                let r = radius_top.max(*radius_bottom);
                Vec3::new(r * 2.0, *height, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        MeshKey::UnitTorus => match params {
            Some(PrimitiveParams::Torus {
                minor_radius,
                major_radius,
            }) => {
                let r = major_radius + minor_radius;
                Vec3::new(r * 2.0, minor_radius * 2.0, r * 2.0)
            }
            _ => Vec3::ONE,
        },
        MeshKey::TreeTrunk | MeshKey::TreeCone => Vec3::ONE,
    }
}

fn primitive_parts_aabb(parts: &[ObjectPartDef]) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut any = false;
    for part in parts.iter() {
        let (mesh, params) = match &part.kind {
            ObjectPartKind::Primitive { primitive } => match primitive {
                PrimitiveVisualDef::Primitive { mesh, params, .. } => (*mesh, params.as_ref()),
                PrimitiveVisualDef::Mesh { mesh, .. } => (*mesh, None),
            },
            ObjectPartKind::ObjectRef { .. } => continue,
            ObjectPartKind::Model { .. } => continue,
        };
        let base = primitive_base_size(mesh, params);
        let scaled = base * part.transform.scale;
        let half = scaled.abs() * 0.5;
        let center = part.transform.translation;
        let rot = part.transform.rotation;
        if !center.is_finite() || !half.is_finite() || !rot.is_finite() {
            continue;
        }
        let ext = rotated_half_extents(half, rot);
        min = min.min(center - ext);
        max = max.max(center + ext);
        any = true;
    }
    any.then_some((min, max))
}

pub(super) fn size_from_primitive_parts(parts: &[ObjectPartDef]) -> Vec3 {
    let Some((min, max)) = primitive_parts_aabb(parts) else {
        return Vec3::ONE;
    };
    let mut size = (max - min).abs();
    if !size.x.is_finite() || size.length_squared() <= 1e-6 {
        size = Vec3::ONE;
    }
    size
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use crate::object::registry::{PartAnimationDriver, PartAnimationSlot, PartAnimationSpec};

    #[test]
    fn recenters_component_parts_and_anchors_together() {
        let planned = Gen3dPlannedComponent {
            display_name: "1. test_component".into(),
            name: "test_component".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: vec![crate::object::registry::AnchorDef {
                name: "mount".into(),
                transform: Transform::from_translation(Vec3::new(1.0, 0.0, 0.0)),
            }],
            contacts: Vec::new(),
            attach_to: None,
        };

        let ai = AiDraftJsonV1 {
            version: 2,
            collider: None,
            anchors: vec![AiAnchorJson {
                name: "mount".into(),
                pos: [1.0, 0.0, 0.0],
                forward: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
            }],
            parts: vec![AiPartJson {
                primitive: AiPrimitiveJson::Cuboid,
                params: None,
                color: Some([0.2, 0.3, 0.4, 1.0]),
                render_priority: None,
                pos: [1.0, 0.0, 0.0],
                forward: None,
                up: None,
                scale: [1.0, 1.0, 1.0],
            }],
        };

        let def = ai_to_component_def(&planned, ai, None).expect("component def should build");
        let part = def
            .parts
            .iter()
            .find(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
            .expect("primitive part");
        assert!(part.transform.translation.length() < 1e-3);
        let anchor = def
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "mount")
            .expect("anchor");
        assert!(anchor.transform.translation.length() < 1e-3);
    }

    #[test]
    fn rejects_axis_permutation_against_planned_size() {
        // The engine must not silently rotate geometry to match the plan. If a component comes
        // back with permuted AABB axes relative to `target_size`, reject it and trigger regen.
        let planned = Gen3dPlannedComponent {
            display_name: "1. permuted_component".into(),
            name: "permuted_component".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::new(0.35, 1.0, 1.0),
            actual_size: None,
            anchors: vec![crate::object::registry::AnchorDef {
                name: "mount".into(),
                transform: Transform::IDENTITY,
            }],
            contacts: Vec::new(),
            attach_to: None,
        };

        // Geometry is effectively [1.0, 0.38, 1.0] (X/Y swapped compared to planned [0.35, 1.0, 1.0]).
        let ai = AiDraftJsonV1 {
            version: 2,
            collider: None,
            anchors: vec![AiAnchorJson {
                name: "mount".into(),
                pos: [0.0, 1.0, 0.0],
                forward: [0.0, 0.0, 1.0],
                up: [0.0, 1.0, 0.0],
            }],
            parts: vec![AiPartJson {
                primitive: AiPrimitiveJson::Cuboid,
                params: None,
                color: Some([0.2, 0.3, 0.4, 1.0]),
                render_priority: None,
                pos: [0.0, 0.0, 0.0],
                forward: None,
                up: None,
                scale: [1.0, 0.38, 1.0],
            }],
        };

        let err =
            ai_to_component_def(&planned, ai, None).expect_err("expected axis-permutation error");
        assert!(err.contains("permuted local axes"));
        assert!(err.contains("permuted AABB"));
    }

    #[test]
    fn does_not_auto_align_axially_symmetric_spinner_with_spin_axis() {
        // The engine must not rotate component geometry based on inferred symmetry/spin alignment.
        // If a spinner tumbles, the AI should regenerate/rotate the primitives explicitly.
        let planned = Gen3dPlannedComponent {
            display_name: "1. tail_rotor".into(),
            name: "tail_rotor".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::new(0.60, 0.60, 0.12),
            actual_size: None,
            anchors: vec![crate::object::registry::AnchorDef {
                name: "root_attach".into(),
                transform: Transform::from_translation(Vec3::ZERO)
                    .with_rotation(plan_rotation_from_forward_up_lossy(Vec3::X, Some(Vec3::Y))),
            }],
            contacts: Vec::new(),
            attach_to: Some(Gen3dPlannedAttachment {
                parent: "tail_boom".into(),
                parent_anchor: "tail_rotor_mount".into(),
                child_anchor: "root_attach".into(),
                offset: Transform::IDENTITY,
                joint: None,
                animations: vec![PartAnimationSlot {
                    channel: "ambient".into(),
                    spec: PartAnimationSpec {
                        driver: PartAnimationDriver::Always,
                        speed_scale: 1.0,
                        time_offset_units: 0.0,
                        clip: PartAnimationDef::Spin {
                            axis: Vec3::Z,
                            radians_per_unit: 1.0,
                        },
                    },
                }],
            }),
        };

        // Geometry is a flat disc in the XY plane (thin in +Z), but the attachment axis points +X.
        let ai = AiDraftJsonV1 {
            version: 2,
            collider: None,
            anchors: vec![AiAnchorJson {
                name: "root_attach".into(),
                pos: [0.0, 0.0, 0.0],
                forward: [1.0, 0.0, 0.0],
                up: [0.0, 1.0, 0.0],
            }],
            parts: vec![AiPartJson {
                primitive: AiPrimitiveJson::Cuboid,
                params: None,
                color: Some([0.2, 0.3, 0.4, 1.0]),
                render_priority: None,
                pos: [0.0, 0.0, 0.0],
                forward: None,
                up: None,
                scale: [0.60, 0.60, 0.12],
            }],
        };

        let def = ai_to_component_def(&planned, ai, None).expect("component def should build");
        assert!(
            def.size.z < def.size.x && def.size.z < def.size.y,
            "expected component to remain thin along +Z (no auto-alignment); size={:?}",
            def.size
        );
        let anchor = def
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "root_attach")
            .expect("root_attach anchor");
        let forward = anchor.transform.rotation * Vec3::Z;
        assert!(
            (forward - Vec3::X).length() < 1e-3,
            "expected attachment anchor forward to stay +X; forward={:?}",
            forward
        );
    }

    #[test]
    fn component_def_uses_plan_anchor_rotations_over_draft() {
        // Regression: letting the draft override required anchor orientation breaks join-frame
        // axes used by attachments and animations (e.g. melee swing yaw turns into a twist).
        let planned = Gen3dPlannedComponent {
            display_name: "1. arm".into(),
            name: "arm".into(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: vec![crate::object::registry::AnchorDef {
                name: "hand_grip".into(),
                transform: Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)),
            }],
            contacts: Vec::new(),
            attach_to: None,
        };

        let ai = AiDraftJsonV1 {
            version: 2,
            collider: None,
            anchors: vec![AiAnchorJson {
                name: "hand_grip".into(),
                pos: [1.0, 2.0, 3.0],
                forward: [0.0, -1.0, 0.0],
                up: [0.0, 0.0, 1.0],
            }],
            parts: vec![AiPartJson {
                primitive: AiPrimitiveJson::Cuboid,
                params: None,
                color: Some([0.2, 0.3, 0.4, 1.0]),
                render_priority: None,
                pos: [0.0, 0.0, 0.0],
                forward: None,
                up: None,
                scale: [1.0, 1.0, 1.0],
            }],
        };

        let def = ai_to_component_def(&planned, ai, None).expect("component def should build");
        let anchor = def
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "hand_grip")
            .expect("hand_grip anchor");

        assert!(
            (anchor.transform.translation - Vec3::new(1.0, 2.0, 3.0)).length() < 1e-5,
            "expected draft anchor translation to be preserved; got={:?}",
            anchor.transform.translation
        );

        let expected = planned.anchors[0].transform.rotation.normalize();
        let got = anchor.transform.rotation.normalize();
        assert!(
            got.dot(expected).abs() > 0.9999,
            "expected required anchor rotation to match plan; expected={:?} got={:?}",
            expected,
            got
        );
    }

    #[test]
    fn parses_plan_v8_with_partial_attack_without_failing() {
        let text = r##"{
          "version": 8,
          "mobility": { "kind": "ground", "max_speed": 5.0 },
          "attack": {
            "kind": "ranged_projectile",
            "muzzle": { "component": "root", "anchor": "origin" }
          },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": []
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(text).expect("plan should parse");
        let (_planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let root_id = gen3d_draft_object_id();
        let root = defs
            .iter()
            .find(|d| d.object_id == root_id)
            .expect("root def");
        assert!(root.mobility.is_some());
        // Attack is ignored because the plan omitted projectile details (should not fail the build).
        assert!(root.attack.is_none());
    }

    #[test]
    fn defaults_ranged_aim_to_parent_of_nested_muzzle_component() {
        // Regression: if the plan omits `aim`, Gen3D previously defaulted to aiming the muzzle's
        // COMPONENT object id. When the muzzle anchor lives on a small nested helper component
        // (e.g. a VFX mouth emitter), only that helper yaws and the visible head/weapon stays
        // fixed. Default to aiming the parent component when the muzzle is nested.
        let text = r##"{
          "version": 8,
          "mobility": { "kind": "ground", "max_speed": 6.0 },
          "attack": {
            "kind": "ranged_projectile",
            "cooldown_secs": 0.6,
            "muzzle": { "component": "vfx_heat", "anchor": "mouth" },
            "projectile": {
              "shape": "sphere",
              "radius": 0.2,
              "color": [1.0, 0.5, 0.0, 1.0],
              "unlit": true,
              "speed": 10.0,
              "ttl_secs": 1.0,
              "damage": 1
            }
          },
          "components": [
            {
              "name": "torso",
              "size": [2.0, 2.0, 2.0],
              "anchors": [
                { "name": "neck_mount", "pos": [0.0, 0.0, 1.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ]
            },
            {
              "name": "head",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "neck_mount", "pos": [0.0, 0.0, -0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                { "name": "vfx_mount", "pos": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ],
              "attach_to": { "parent": "torso", "parent_anchor": "neck_mount", "child_anchor": "neck_mount" }
            },
            {
              "name": "vfx_heat",
              "size": [0.5, 0.5, 0.5],
              "anchors": [
                { "name": "head_mount", "pos": [0.0, 0.0, -0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                { "name": "mouth", "pos": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ],
              "attach_to": { "parent": "head", "parent_anchor": "vfx_mount", "child_anchor": "head_mount" }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(text).expect("plan should parse");
        let (_planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let root_id = gen3d_draft_object_id();
        let root = defs
            .iter()
            .find(|d| d.object_id == root_id)
            .expect("root def");
        let aim = root.aim.as_ref().expect("aim profile");
        assert_eq!(aim.components.len(), 1);
        assert_eq!(
            aim.components[0],
            builtin_object_id("gravimera/gen3d/component/head")
        );
    }

    #[test]
    fn defaults_ranged_aim_to_muzzle_when_muzzle_is_direct_child_of_root() {
        let text = r##"{
          "version": 8,
          "mobility": { "kind": "ground", "max_speed": 6.0 },
          "attack": {
            "kind": "ranged_projectile",
            "cooldown_secs": 0.6,
            "muzzle": { "component": "gun", "anchor": "muzzle" },
            "projectile": {
              "shape": "sphere",
              "radius": 0.2,
              "color": [0.2, 0.8, 1.0, 1.0],
              "unlit": true,
              "speed": 10.0,
              "ttl_secs": 1.0,
              "damage": 1
            }
          },
          "components": [
            {
              "name": "body",
              "size": [2.0, 2.0, 2.0],
              "anchors": [
                { "name": "gun_mount", "pos": [0.0, 0.0, 1.0], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ]
            },
            {
              "name": "gun",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "gun_mount", "pos": [0.0, 0.0, -0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] },
                { "name": "muzzle", "pos": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ],
              "attach_to": { "parent": "body", "parent_anchor": "gun_mount", "child_anchor": "gun_mount" }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(text).expect("plan should parse");
        let (_planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let root_id = gen3d_draft_object_id();
        let root = defs
            .iter()
            .find(|d| d.object_id == root_id)
            .expect("root def");
        let aim = root.aim.as_ref().expect("aim profile");
        assert_eq!(aim.components.len(), 1);
        assert_eq!(
            aim.components[0],
            builtin_object_id("gravimera/gen3d/component/gun")
        );
    }

    #[test]
    fn parses_plan_projectile_rgba_color_and_default_obstacle_rule() {
        let text = r##"{
          "version": 8,
          "mobility": { "kind": "ground", "max_speed": 5.0 },
          "attack": {
            "kind": "ranged_projectile",
            "cooldown_secs": 0.6,
            "muzzle": { "component": "root", "anchor": "muzzle" },
            "projectile": {
              "shape": "sphere",
              "radius": 0.1,
              "color": [1.0, 0.15, 0.15, 1.0],
              "unlit": true,
              "speed": 10.0,
              "ttl_secs": 1.0,
              "damage": 3
            }
          },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "muzzle", "pos": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ]
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(text).expect("plan should parse");
        let (_planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let root_id = gen3d_draft_object_id();
        let root = defs
            .iter()
            .find(|d| d.object_id == root_id)
            .expect("root def");
        assert!(root.attack.is_some());

        let projectile_id = gen3d_draft_projectile_object_id();
        let projectile = defs
            .iter()
            .find(|d| d.object_id == projectile_id)
            .expect("projectile def");
        let profile = projectile.projectile.as_ref().expect("projectile profile");
        assert!(matches!(
            profile.obstacle_rule,
            ProjectileObstacleRule::BulletsBlockers
        ));
    }

    #[test]
    fn parses_plan_projectile_rgba_color_and_cylinder_shape() {
        let text = r##"{
          "version": 8,
          "mobility": { "kind": "ground", "max_speed": 5.0 },
          "attack": {
            "kind": "ranged_projectile",
            "cooldown_secs": 0.6,
            "muzzle": { "component": "root", "anchor": "muzzle" },
            "projectile": {
              "shape": "cylinder",
              "radius": 0.05,
              "length": 1.2,
              "color": [1.0, 0.25, 0.25, 1.0],
              "unlit": true,
              "speed": 10.0,
              "ttl_secs": 1.0,
              "damage": 3,
              "obstacle_rule": "laser_blockers"
            }
          },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "muzzle", "pos": [0.0, 0.0, 0.5], "forward": [0.0, 0.0, 1.0], "up": [0.0, 1.0, 0.0] }
              ]
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(text).expect("plan should parse");
        let (_planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let root_id = gen3d_draft_object_id();
        let root = defs
            .iter()
            .find(|d| d.object_id == root_id)
            .expect("root def");
        assert!(root.attack.is_some());

        let projectile_id = gen3d_draft_projectile_object_id();
        let projectile = defs
            .iter()
            .find(|d| d.object_id == projectile_id)
            .expect("projectile def");
        let profile = projectile.projectile.as_ref().expect("projectile profile");
        assert!(matches!(
            profile.obstacle_rule,
            ProjectileObstacleRule::LaserBlockers
        ));

        let Some(part) = projectile.parts.first() else {
            panic!("projectile should have one part");
        };
        let ObjectPartKind::Primitive { primitive } = &part.kind else {
            panic!("projectile part should be primitive");
        };
        let PrimitiveVisualDef::Primitive { mesh, .. } = primitive else {
            panic!("projectile primitive should use a generated mesh");
        };
        assert!(matches!(mesh, MeshKey::UnitCylinder));
    }

    #[test]
    fn applies_review_delta_tweak_contact_does_not_clear_ground_stance() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "body",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "foot_anchor", "pos": [0.0, -0.5, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "contacts": [
                {
                  "name": "foot_contact",
                  "kind": "ground",
                  "anchor": "foot_anchor",
                  "stance": { "phase_01": 0.0, "duty_factor_01": 0.6 }
                }
              ]
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        assert_eq!(planned.len(), 1);
        assert_eq!(planned[0].contacts.len(), 1);
        assert!(planned[0].contacts[0].stance.is_some());

        let component_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/body")).to_string();
        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakContact {
                component_id,
                contact_name: "foot_contact".into(),
                stance: Some(None),
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let apply =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .expect("apply should succeed");
        assert!(!apply.had_actions);
        assert!(planned[0].contacts[0].stance.is_some());
    }

    #[test]
    fn applies_review_delta_tweak_contact_allows_clearing_stance_for_move_spin() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "body",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "wheel_mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "wheel",
              "size": [1.0, 1.0, 0.2],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] },
                { "name": "rim_contact", "pos": [0.0, -0.5, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "contacts": [
                {
                  "name": "rim",
                  "kind": "ground",
                  "anchor": "rim_contact",
                  "stance": { "phase_01": 0.0, "duty_factor_01": 1.0 }
                }
              ],
              "attach_to": {
                "parent": "body",
                "parent_anchor": "wheel_mount",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        let wheel_idx = planned.iter().position(|c| c.name == "wheel").unwrap();
        let Some(att) = planned[wheel_idx].attach_to.as_mut() else {
            panic!("wheel should be attached");
        };
        att.animations.push(PartAnimationSlot {
            channel: "move".into(),
            spec: PartAnimationSpec {
                driver: PartAnimationDriver::MoveDistance,
                speed_scale: 1.0,
                time_offset_units: 0.0,
                clip: PartAnimationDef::Spin {
                    axis: Vec3::Z,
                    radians_per_unit: 1.0,
                },
            },
        });

        assert!(planned[wheel_idx].contacts[0].stance.is_some());

        let component_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/wheel")).to_string();
        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakContact {
                component_id,
                contact_name: "rim".into(),
                stance: Some(None),
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let apply =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .expect("apply should succeed");
        assert!(apply.had_actions);
        assert!(planned[wheel_idx].contacts[0].stance.is_none());
    }

    #[test]
    fn review_delta_tweak_attachment_errors_on_missing_anchors() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 1.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let child_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/child")).to_string();
        let root_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/root")).to_string();

        // Missing parent anchor.
        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakAttachment {
                component_id: child_id.clone(),
                set: AiAttachmentSetJsonV1 {
                    parent_component_id: root_id.clone(),
                    parent_anchor: "missing_anchor".into(),
                    child_anchor: "mount".into(),
                    offset: None,
                },
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let err =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .unwrap_err();
        assert!(
            err.contains("missing parent_anchor"),
            "unexpected error: {err}"
        );

        // Missing child anchor.
        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakAttachment {
                component_id: child_id,
                set: AiAttachmentSetJsonV1 {
                    parent_component_id: root_id,
                    parent_anchor: "socket".into(),
                    child_anchor: "missing_anchor".into(),
                    offset: None,
                },
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let err =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .unwrap_err();
        assert!(
            err.contains("missing child_anchor"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn plan_attachment_errors_on_opposing_anchor_up_vectors() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,-1,0] }
              ],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let err = ai_plan_to_initial_draft_defs(plan).unwrap_err();
        assert!(
            err.contains("opposing anchor up vectors"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn plan_attachment_errors_on_missing_child_anchor_without_reuse() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let err = ai_plan_to_initial_draft_defs(plan).unwrap_err();
        assert!(
            err.contains("missing required child_anchor `mount`"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn reuse_target_hydrates_missing_child_anchor_for_plan_conversion() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "reuse_groups": [
            { "kind": "component", "source": "child_source", "targets": ["child_target"], "alignment": "rotation" }
          ],
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 0.0, 0.0], "forward": [0.707,0.0,0.707], "up": [0,1,0] }
              ]
            },
            {
              "name": "child_source",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            },
            {
              "name": "child_target",
              "size": [1.0, 1.0, 1.0],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let (planned, _notes, _defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");

        let target = planned
            .iter()
            .find(|c| c.name.trim() == "child_target")
            .expect("target should exist");
        let mount = target
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == "mount")
            .expect("mount anchor should be hydrated");
        let forward = (mount.transform.rotation * Vec3::Z).normalize();
        let expected = Vec3::new(0.707, 0.0, 0.707).normalize();
        assert!(
            (forward - expected).length() < 1e-4,
            "unexpected hydrated forward: got={:?} expected={:?}",
            forward,
            expected
        );
    }

    fn assembled_child_transform(
        planned: &[Gen3dPlannedComponent],
        parent_name: &str,
        parent_anchor_name: &str,
        child_name: &str,
        child_anchor_name: &str,
    ) -> Transform {
        let parent = planned
            .iter()
            .find(|c| c.name == parent_name)
            .expect("parent component should exist");
        let parent_anchor = parent
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == parent_anchor_name)
            .map(|a| a.transform)
            .unwrap_or(Transform::IDENTITY);

        let child = planned
            .iter()
            .find(|c| c.name == child_name)
            .expect("child component should exist");
        let child_anchor = child
            .anchors
            .iter()
            .find(|a| a.name.as_ref() == child_anchor_name)
            .map(|a| a.transform)
            .unwrap_or(Transform::IDENTITY);
        let offset = child
            .attach_to
            .as_ref()
            .map(|a| a.offset)
            .unwrap_or(Transform::IDENTITY);

        let composed =
            parent_anchor.to_matrix() * offset.to_matrix() * child_anchor.to_matrix().inverse();
        crate::geometry::mat4_to_transform_allow_degenerate_scale(composed)
            .expect("assembled transform should decompose")
    }

    fn assert_transform_close(a: Transform, b: Transform) {
        let dp = (a.translation - b.translation).length();
        assert!(dp < 1e-4, "translation mismatch: a={:?} b={:?}", a, b);

        let qa = a.rotation.normalize();
        let qb = b.rotation.normalize();
        let dot = qa.dot(qb).abs();
        assert!(dot > 1.0 - 1e-4, "rotation mismatch: a={:?} b={:?}", qa, qb);

        let ds = (a.scale - b.scale).length();
        assert!(ds < 1e-4, "scale mismatch: a={:?} b={:?}", a.scale, b.scale);
    }

    #[test]
    fn tweak_anchor_rebases_component_offset_when_child_anchor_changes() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 1.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        let before = assembled_child_transform(&planned, "root", "socket", "child", "mount");

        let component_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/child")).to_string();
        let q = Quat::from_rotation_y(core::f32::consts::FRAC_PI_2).normalize();
        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakAnchor {
                component_id,
                anchor_name: "mount".into(),
                set: None,
                delta: Some(AiAnchorDeltaJsonV1 {
                    pos: None,
                    rot_quat_xyzw: Some([q.x, q.y, q.z, q.w]),
                }),
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let apply =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .expect("apply should succeed");
        assert!(apply.had_actions);

        let after = assembled_child_transform(&planned, "root", "socket", "child", "mount");
        assert_transform_close(before, after);
    }

    #[test]
    fn tweak_anchor_rebases_child_offsets_when_parent_anchor_changes() {
        let plan_text = r##"{
          "version": 8,
          "mobility": { "kind": "static" },
          "components": [
            {
              "name": "root",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "socket", "pos": [0.0, 1.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ]
            },
            {
              "name": "child",
              "size": [1.0, 1.0, 1.0],
              "anchors": [
                { "name": "mount", "pos": [0.0, 0.0, 0.0], "forward": [0,0,1], "up": [0,1,0] }
              ],
              "attach_to": {
                "parent": "root",
                "parent_anchor": "socket",
                "child_anchor": "mount",
                "offset": { "pos": [0.0, 0.0, 0.0] }
              }
            }
          ]
        }"##;

        let plan = parse::parse_ai_plan_from_text(plan_text).expect("plan should parse");
        let plan_collider = plan.collider.clone();
        let (mut planned, _notes, defs) = ai_plan_to_initial_draft_defs(plan).expect("defs build");
        let mut draft = Gen3dDraft { defs };

        let before = assembled_child_transform(&planned, "root", "socket", "child", "mount");

        let component_id =
            Uuid::from_u128(builtin_object_id("gravimera/gen3d/component/root")).to_string();
        let q = Quat::from_rotation_x(core::f32::consts::FRAC_PI_2).normalize();
        let delta = AiReviewDeltaJsonV1 {
            version: 1,
            applies_to: AiReviewDeltaAppliesToJsonV1 {
                run_id: "test".into(),
                attempt: 0,
                plan_hash: "sha256:test".into(),
                assembly_rev: 0,
            },
            actions: vec![AiReviewDeltaActionJsonV1::TweakAnchor {
                component_id,
                anchor_name: "socket".into(),
                set: None,
                delta: Some(AiAnchorDeltaJsonV1 {
                    pos: None,
                    rot_quat_xyzw: Some([q.x, q.y, q.z, q.w]),
                }),
                reason: String::new(),
            }],
            summary: None,
            notes: None,
        };

        let apply =
            apply_ai_review_delta_actions(delta, &mut planned, &plan_collider, &mut draft, None)
                .expect("apply should succeed");
        assert!(apply.had_actions);

        let after = assembled_child_transform(&planned, "root", "socket", "child", "mount");
        assert_transform_close(before, after);
    }
}
