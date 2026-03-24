use bevy::prelude::*;

use crate::gen3d::ai::reuse_groups::Gen3dValidatedReuseGroup;
use crate::gen3d::ai::schema::AiColliderJson;
use crate::gen3d::state::Gen3dDraft;
use crate::object::registry::{
    builtin_object_id, MeshKey, ObjectDef, ObjectPartKind, PrimitiveParams, PrimitiveVisualDef,
    ProjectileObstacleRule, UnitAttackKind,
};

use super::job::{Gen3dPendingPlanAttempt, Gen3dPlannedComponent};

fn vec3_is_close(a: Vec3, b: Vec3) -> bool {
    (a - b).length_squared() <= 1e-10
}

fn quat_is_close(a: Quat, b: Quat) -> bool {
    // Account for q and -q representing the same rotation.
    a.dot(b).abs() >= 0.999_999
}

fn component_object_id_for_name(name: &str) -> u128 {
    builtin_object_id(&format!("gravimera/gen3d/component/{}", name.trim()))
}

fn component_name_for_object_id<'a>(
    planned_components: &'a [Gen3dPlannedComponent],
    object_id: u128,
) -> Option<&'a str> {
    planned_components
        .iter()
        .find(|c| component_object_id_for_name(&c.name) == object_id)
        .map(|c| c.name.as_str())
}

fn normalize_name_tokens(raw: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut buf = String::new();
    for ch in raw.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            buf.push(ch.to_ascii_lowercase());
        } else {
            if buf.len() >= 2 {
                out.push(buf.clone());
            }
            buf.clear();
        }
    }
    if buf.len() >= 2 {
        out.push(buf);
    }
    out.sort();
    out.dedup();
    out
}

fn suggest_existing_component_names(unknown: &str, existing_names: &[String]) -> Vec<String> {
    let unknown = unknown.trim();
    if unknown.is_empty() {
        return Vec::new();
    }
    let unknown_lc = unknown.to_ascii_lowercase();
    let unknown_tokens = normalize_name_tokens(unknown);

    let mut token_matches: Vec<String> = Vec::new();
    let mut substring_matches: Vec<String> = Vec::new();
    for name in existing_names.iter() {
        let candidate = name.trim();
        if candidate.is_empty() {
            continue;
        }
        let candidate_lc = candidate.to_ascii_lowercase();
        if unknown_tokens.iter().any(|t| t == &candidate_lc) {
            token_matches.push(candidate.to_string());
            continue;
        }
        if candidate_lc.len() >= 3 && unknown_lc.contains(&candidate_lc) {
            substring_matches.push(candidate.to_string());
        }
    }
    token_matches.sort();
    token_matches.dedup();
    substring_matches.sort();
    substring_matches.dedup();

    let mut out = token_matches;
    out.extend(substring_matches);
    out.dedup();
    out.truncate(5);
    out
}

fn build_ai_mobility_json(root_def: Option<&ObjectDef>) -> serde_json::Value {
    let Some(root_def) = root_def else {
        return serde_json::json!({ "kind": "static" });
    };
    let Some(mobility) = root_def.mobility else {
        return serde_json::json!({ "kind": "static" });
    };
    let max_speed = if mobility.max_speed.is_finite() {
        mobility.max_speed
    } else {
        0.0
    }
    .abs()
    .max(0.01);
    match mobility.mode {
        crate::object::registry::MobilityMode::Ground => {
            serde_json::json!({ "kind": "ground", "max_speed": max_speed })
        }
        crate::object::registry::MobilityMode::Air => {
            serde_json::json!({ "kind": "air", "max_speed": max_speed })
        }
    }
}

fn obstacle_rule_to_ai(rule: ProjectileObstacleRule) -> &'static str {
    match rule {
        ProjectileObstacleRule::BulletsBlockers => "bullets_blockers",
        ProjectileObstacleRule::LaserBlockers => "laser_blockers",
    }
}

fn infer_projectile_spec_json(projectile_def: &ObjectDef) -> Result<serde_json::Value, String> {
    let Some(projectile) = projectile_def.projectile.as_ref() else {
        return Err("Projectile prefab is missing `projectile` metadata.".into());
    };
    let Some(part) = projectile_def
        .parts
        .iter()
        .find(|p| matches!(p.kind, ObjectPartKind::Primitive { .. }))
    else {
        return Err("Projectile prefab has no primitive part.".into());
    };

    let (mesh, params, color, unlit) = match &part.kind {
        ObjectPartKind::Primitive {
            primitive:
                PrimitiveVisualDef::Primitive {
                    mesh,
                    params,
                    color,
                    unlit,
                },
        } => (*mesh, *params, *color, *unlit),
        _ => {
            return Err("Projectile prefab primitive part is not a Gen3D primitive.".into());
        }
    };

    let srgba = color.to_srgba();
    let rgba = [srgba.red, srgba.green, srgba.blue, srgba.alpha];

    let mut spec = serde_json::Map::new();
    spec.insert("color".into(), serde_json::json!(rgba));
    spec.insert("unlit".into(), serde_json::json!(unlit));
    spec.insert("speed".into(), serde_json::json!(projectile.speed));
    spec.insert("ttl_secs".into(), serde_json::json!(projectile.ttl_secs));
    spec.insert("damage".into(), serde_json::json!(projectile.damage));
    spec.insert(
        "obstacle_rule".into(),
        serde_json::json!(obstacle_rule_to_ai(projectile.obstacle_rule)),
    );
    spec.insert(
        "spawn_energy_impact".into(),
        serde_json::json!(projectile.spawn_energy_impact),
    );

    match mesh {
        MeshKey::UnitSphere => {
            let radius = (part.transform.scale.x.abs() * 0.5).max(0.01);
            spec.insert("shape".into(), serde_json::json!("sphere"));
            spec.insert("radius".into(), serde_json::json!(radius));
        }
        MeshKey::UnitCapsule => {
            let PrimitiveParams::Capsule {
                radius,
                half_length,
            } = params.ok_or_else(|| {
                "Projectile capsule missing capsule params (radius/half_length).".to_string()
            })?
            else {
                return Err("Projectile capsule has non-capsule params.".into());
            };
            let radius = radius.abs().max(0.01);
            let half_length = half_length.abs().max(0.0);
            let length = (half_length * 2.0 + radius * 2.0).max(radius * 2.0);
            spec.insert("shape".into(), serde_json::json!("capsule"));
            spec.insert("radius".into(), serde_json::json!(radius));
            spec.insert("length".into(), serde_json::json!(length));
        }
        MeshKey::UnitCube => {
            let sx = part.transform.scale.x.abs().max(0.01);
            let sy = part.transform.scale.y.abs().max(0.01);
            let sz = part.transform.scale.z.abs().max(0.01);
            spec.insert("shape".into(), serde_json::json!("cuboid"));
            spec.insert("size".into(), serde_json::json!([sx, sy, sz]));
        }
        MeshKey::UnitCylinder => {
            let radius = (part.transform.scale.x.abs() * 0.5).max(0.01);
            let length = part.transform.scale.y.abs().max(radius * 2.0);
            spec.insert("shape".into(), serde_json::json!("cylinder"));
            spec.insert("radius".into(), serde_json::json!(radius));
            spec.insert("length".into(), serde_json::json!(length));
        }
        other => {
            return Err(format!(
                "Projectile prefab uses unsupported mesh shape {other:?}."
            ));
        }
    }

    Ok(serde_json::Value::Object(spec))
}

fn build_ai_attack_json(
    draft: &Gen3dDraft,
    planned_components: &[Gen3dPlannedComponent],
    root_def: &ObjectDef,
) -> Result<Option<serde_json::Value>, String> {
    let Some(mobility) = root_def.mobility else {
        return Ok(None);
    };
    let _ = mobility;

    let Some(attack) = root_def.attack.as_ref() else {
        return Ok(None);
    };

    match attack.kind {
        UnitAttackKind::Melee => {
            let Some(melee) = attack.melee.as_ref() else {
                return Ok(None);
            };
            Ok(Some(serde_json::json!({
                "kind": "melee",
                "cooldown_secs": attack.cooldown_secs,
                "damage": attack.damage,
                "range": melee.range,
                "radius": melee.radius,
                "arc_degrees": melee.arc_degrees,
            })))
        }
        UnitAttackKind::RangedProjectile => {
            let Some(ranged) = attack.ranged.as_ref() else {
                return Ok(None);
            };
            let Some(muzzle_component) =
                component_name_for_object_id(planned_components, ranged.muzzle.object_id)
            else {
                return Err("Failed to map ranged muzzle object_id to a component name.".into());
            };
            let muzzle_anchor = ranged.muzzle.anchor.as_ref();

            let projectile_def = draft
                .defs
                .iter()
                .find(|def| def.object_id == ranged.projectile_prefab)
                .ok_or_else(|| "Projectile prefab def not found in draft defs.".to_string())?;
            let projectile_spec = infer_projectile_spec_json(projectile_def)?;

            Ok(Some(serde_json::json!({
                "kind": "ranged_projectile",
                "cooldown_secs": attack.cooldown_secs,
                "muzzle": { "component": muzzle_component, "anchor": muzzle_anchor },
                "projectile": projectile_spec,
            })))
        }
    }
}

fn build_ai_aim_json(
    planned_components: &[Gen3dPlannedComponent],
    root_def: &ObjectDef,
) -> Option<serde_json::Value> {
    let aim = root_def.aim.as_ref()?;
    let mut component_names: Vec<String> = aim
        .components
        .iter()
        .filter_map(|id| component_name_for_object_id(planned_components, *id))
        .map(|s| s.to_string())
        .collect();
    component_names.sort();
    component_names.dedup();
    Some(serde_json::json!({
        "max_yaw_delta_degrees": aim.max_yaw_delta_degrees,
        "components": component_names,
    }))
}

fn attachment_offset_to_ai_json(offset: Transform) -> Option<serde_json::Value> {
    let t = offset.translation;
    let r = if offset.rotation.is_finite() {
        offset.rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    let s = offset.scale;

    let has_translation = !vec3_is_close(t, Vec3::ZERO);
    let has_rotation = !quat_is_close(r, Quat::IDENTITY);
    let has_scale = !vec3_is_close(s, Vec3::ONE);

    if !has_translation && !has_rotation && !has_scale {
        return None;
    }

    let mut obj = serde_json::Map::new();
    obj.insert("pos".into(), serde_json::json!([t.x, t.y, t.z]));
    if has_rotation {
        obj.insert(
            "rot_quat_xyzw".into(),
            serde_json::json!([r.x, r.y, r.z, r.w]),
        );
        obj.insert("rot_frame".into(), serde_json::json!("join"));
    }
    if has_scale {
        obj.insert("scale".into(), serde_json::json!([s.x, s.y, s.z]));
    }
    Some(serde_json::Value::Object(obj))
}

fn anchors_to_ai_json(anchors: &[crate::object::registry::AnchorDef]) -> Vec<serde_json::Value> {
    anchors
        .iter()
        .map(|a| {
            let name = a.name.as_ref();
            let t = a.transform.translation;
            let r = if a.transform.rotation.is_finite() {
                a.transform.rotation.normalize()
            } else {
                Quat::IDENTITY
            };
            let forward = r * Vec3::Z;
            let up = r * Vec3::Y;
            serde_json::json!({
                "name": name,
                "pos": [t.x, t.y, t.z],
                "forward": [forward.x, forward.y, forward.z],
                "up": [up.x, up.y, up.z],
            })
        })
        .collect()
}

fn reuse_groups_to_ai_json(
    reuse_groups: &[Gen3dValidatedReuseGroup],
    planned_components: &[Gen3dPlannedComponent],
) -> Vec<serde_json::Value> {
    reuse_groups
        .iter()
        .filter_map(|g| {
            let source = planned_components.get(g.source_root_idx)?.name.as_str();
            let targets: Vec<String> = g
                .target_root_indices
                .iter()
                .filter_map(|idx| planned_components.get(*idx).map(|c| c.name.clone()))
                .collect();
            if targets.is_empty() {
                return None;
            }

            let kind = match g.kind {
                crate::gen3d::ai::reuse_groups::Gen3dReuseGroupKind::Component => "copy_component",
                crate::gen3d::ai::reuse_groups::Gen3dReuseGroupKind::Subtree => {
                    "copy_component_subtree"
                }
            };
            let alignment = match g.alignment {
                crate::gen3d::ai::copy_component::Gen3dCopyAlignmentMode::Rotation => "rotation",
                crate::gen3d::ai::copy_component::Gen3dCopyAlignmentMode::MirrorMountX => {
                    "mirror_mount_x"
                }
            };
            let mode = match g.mode {
                crate::gen3d::ai::copy_component::Gen3dCopyMode::Detached => "detached",
                crate::gen3d::ai::copy_component::Gen3dCopyMode::Linked => "linked",
            };
            let anchors = match g.anchors_mode {
                crate::gen3d::ai::copy_component::Gen3dCopyAnchorsMode::PreserveTargetAnchors => {
                    "preserve_target"
                }
                crate::gen3d::ai::copy_component::Gen3dCopyAnchorsMode::PreserveInterfaceAnchors => {
                    "preserve_interfaces"
                }
                crate::gen3d::ai::copy_component::Gen3dCopyAnchorsMode::CopySourceAnchors => {
                    "copy_source"
                }
            };
            let alignment_frame = match g.alignment_frame {
                crate::gen3d::ai::copy_component::Gen3dCopyAlignmentFrame::Join => "join",
                crate::gen3d::ai::copy_component::Gen3dCopyAlignmentFrame::ChildAnchor => {
                    "child_anchor"
                }
            };

            Some(serde_json::json!({
                "kind": kind,
                "source": source,
                "targets": targets,
                "alignment": alignment,
                "alignment_frame": alignment_frame,
                "mode": mode,
                "anchors": anchors,
            }))
        })
        .collect()
}

pub(super) fn build_preserve_mode_plan_template_json_v8(
    draft: &Gen3dDraft,
    planned_components: &[Gen3dPlannedComponent],
    assembly_notes: &str,
    plan_collider: Option<&AiColliderJson>,
    rig_move_cycle_m: Option<f32>,
    reuse_groups: &[Gen3dValidatedReuseGroup],
) -> Result<serde_json::Value, String> {
    if planned_components.is_empty() {
        return Err("No planned components; run llm_generate_plan_v1 first.".into());
    }
    let root_component = planned_components
        .iter()
        .find(|c| c.attach_to.is_none())
        .map(|c| c.name.as_str())
        .ok_or_else(|| "Current plan has no root component.".to_string())?;

    let root_def = draft.root_def();
    let mobility = build_ai_mobility_json(root_def);

    let attack = match root_def {
        Some(def) => build_ai_attack_json(draft, planned_components, def)?,
        None => None,
    };
    let aim = root_def.and_then(|def| build_ai_aim_json(planned_components, def));

    let components: Vec<serde_json::Value> = planned_components
        .iter()
        .map(|c| {
            let size = c.planned_size.abs().max(Vec3::splat(0.01));
            let anchors = anchors_to_ai_json(&c.anchors);
            let contacts = serde_json::to_value(&c.contacts).unwrap_or(serde_json::json!([]));

            let attach_to = c.attach_to.as_ref().map(|att| {
                let mut obj = serde_json::Map::new();
                obj.insert("parent".into(), serde_json::json!(att.parent.as_str()));
                obj.insert(
                    "parent_anchor".into(),
                    serde_json::json!(att.parent_anchor.as_str()),
                );
                obj.insert(
                    "child_anchor".into(),
                    serde_json::json!(att.child_anchor.as_str()),
                );
                if let Some(offset) = attachment_offset_to_ai_json(att.offset) {
                    obj.insert("offset".into(), offset);
                }
                if let Some(joint) = att.joint.as_ref() {
                    obj.insert(
                        "joint".into(),
                        serde_json::to_value(joint).unwrap_or(serde_json::Value::Null),
                    );
                }
                serde_json::Value::Object(obj)
            });

            let mut obj = serde_json::Map::new();
            obj.insert("name".into(), serde_json::json!(c.name.as_str()));
            obj.insert("purpose".into(), serde_json::json!(c.purpose.as_str()));
            obj.insert(
                "modeling_notes".into(),
                serde_json::json!(c.modeling_notes.as_str()),
            );
            obj.insert("size".into(), serde_json::json!([size.x, size.y, size.z]));
            obj.insert("anchors".into(), serde_json::Value::Array(anchors));
            obj.insert("contacts".into(), contacts);
            if let Some(attach_to) = attach_to {
                obj.insert("attach_to".into(), attach_to);
            }
            serde_json::Value::Object(obj)
        })
        .collect();

    let reuse_groups_json = reuse_groups_to_ai_json(reuse_groups, planned_components);

    let mut plan = serde_json::Map::new();
    plan.insert("version".into(), serde_json::json!(8));
    if let Some(move_cycle_m) = rig_move_cycle_m.filter(|v| v.is_finite()).map(f32::abs) {
        plan.insert(
            "rig".into(),
            serde_json::json!({ "move_cycle_m": move_cycle_m.max(0.01) }),
        );
    }
    plan.insert("mobility".into(), mobility);
    if let Some(attack) = attack {
        plan.insert("attack".into(), attack);
    }
    if let Some(aim) = aim {
        plan.insert("aim".into(), aim);
    }
    if let Some(collider) = plan_collider {
        plan.insert(
            "collider".into(),
            serde_json::to_value(collider).unwrap_or(serde_json::Value::Null),
        );
    }
    plan.insert("assembly_notes".into(), serde_json::json!(assembly_notes));
    plan.insert("root_component".into(), serde_json::json!(root_component));
    if !reuse_groups_json.is_empty() {
        plan.insert(
            "reuse_groups".into(),
            serde_json::Value::Array(reuse_groups_json),
        );
    }
    plan.insert("components".into(), serde_json::Value::Array(components));
    Ok(serde_json::Value::Object(plan))
}

pub(super) fn inspect_pending_plan_attempt_v1(
    pending: Option<&Gen3dPendingPlanAttempt>,
    current_components: &[Gen3dPlannedComponent],
    preserve_existing_components_mode: bool,
) -> serde_json::Value {
    let mut existing_names: Vec<String> =
        current_components.iter().map(|c| c.name.clone()).collect();
    existing_names.sort();
    existing_names.dedup();
    let existing_root = current_components
        .iter()
        .find(|c| c.attach_to.is_none())
        .map(|c| c.name.as_str())
        .unwrap_or("");

    let constraints = serde_json::json!({
        "preserve_existing_components_mode": preserve_existing_components_mode,
        "existing_component_names": existing_names,
        "existing_root_component": if existing_root.is_empty() { serde_json::Value::Null } else { serde_json::json!(existing_root) },
    });

    let Some(pending) = pending else {
        return serde_json::json!({
            "version": 1,
            "has_pending_plan": false,
            "pending": serde_json::Value::Null,
            "constraints": constraints,
            "analysis": {
                "ok": true,
                "errors": [],
                "fixits": [],
                "hints": [
                    "If llm_generate_plan_v1 fails semantically (unknown parent, root mismatch, preserve missing names), use this tool immediately instead of get_scene_graph_summary_v1.",
                    "For preserve mode, prefer get_plan_template_v1 + llm_generate_plan_v1.plan_template_kv to reduce invalid plans."
                ],
            }
        });
    };

    let mut plan_names: Vec<String> = pending
        .plan
        .components
        .iter()
        .map(|c| c.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    plan_names.sort();
    let mut duplicates: Vec<String> = Vec::new();
    for w in plan_names.windows(2) {
        if w[0] == w[1] {
            duplicates.push(w[0].clone());
        }
    }
    plan_names.dedup();
    duplicates.sort();
    duplicates.dedup();

    let mut errors: Vec<serde_json::Value> = Vec::new();
    let mut fixits: Vec<serde_json::Value> = Vec::new();

    if !duplicates.is_empty() {
        errors.push(serde_json::json!({
            "kind": "duplicate_component_names",
            "names": duplicates,
        }));
    }

    let plan_component_by_name: std::collections::HashMap<
        &str,
        &super::schema::AiPlanComponentJson,
    > = pending
        .plan
        .components
        .iter()
        .filter_map(|c| {
            let name = c.name.trim();
            (!name.is_empty()).then_some((name, c))
        })
        .collect();

    // Root selection should match convert.rs: root_component if provided, else exactly 1 component with attach_to omitted.
    let root_from_field = pending
        .plan
        .root_component
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let root_from_attach: Vec<String> = pending
        .plan
        .components
        .iter()
        .filter(|c| c.attach_to.is_none())
        .map(|c| c.name.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let plan_root = if let Some(root) = root_from_field.as_ref() {
        if !plan_names.iter().any(|n| n == root) {
            errors.push(serde_json::json!({
                "kind": "root_component_not_found",
                "root_component": root,
            }));
            None
        } else {
            Some(root.clone())
        }
    } else {
        if root_from_attach.len() != 1 {
            errors.push(serde_json::json!({
                "kind": "invalid_root_count",
                "root_candidates": root_from_attach,
            }));
            None
        } else {
            Some(root_from_attach[0].clone())
        }
    };

    // Unknown parents can be detected without full conversion.
    let plan_name_set: std::collections::HashSet<&str> =
        plan_names.iter().map(|s| s.as_str()).collect();

    // Enumerate missing referenced component names beyond attach_to.parent.
    // This stays purely structural and deterministic (no heuristics).
    let mut component_refs: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    {
        let mut add_ref = |name: &str, location: &str| {
            let name = name.trim();
            if name.is_empty() {
                return;
            }
            let location = location.trim();
            if location.is_empty() {
                return;
            }
            component_refs
                .entry(name.to_string())
                .or_default()
                .insert(location.to_string());
        };

        for comp in pending.plan.components.iter() {
            let Some(att) = comp.attach_to.as_ref() else {
                continue;
            };
            add_ref(att.parent.as_str(), "attach_to.parent");
        }
        if let Some(aim) = pending.plan.aim.as_ref() {
            for name in aim.components.iter() {
                add_ref(name.as_str(), "aim.components");
            }
        }
        if let Some(attack) = pending.plan.attack.as_ref() {
            match attack {
                super::schema::AiAttackJson::RangedProjectile { muzzle, .. } => {
                    if let Some(muzzle) = muzzle.as_ref() {
                        add_ref(muzzle.component.as_str(), "attack.muzzle.component");
                    }
                }
                _ => {}
            }
        }
        for group in pending.plan.reuse_groups.iter() {
            add_ref(group.source.as_str(), "reuse_groups.source");
            for target in group.targets.iter() {
                add_ref(target.as_str(), "reuse_groups.targets");
            }
        }
    }

    let plan_component_names_sample: Vec<String> = plan_names.iter().cloned().take(24).collect();
    for (missing, locations) in component_refs.iter() {
        if plan_name_set.contains(missing.as_str()) {
            continue;
        }
        let referenced_by: Vec<String> = locations.iter().cloned().take(12).collect();
        errors.push(serde_json::json!({
            "kind": "missing_component_reference",
            "name": missing,
            "referenced_by": referenced_by,
            "plan_component_names_sample": plan_component_names_sample.clone(),
        }));
    }

    for comp in pending.plan.components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let parent = att.parent.trim();
        if parent.is_empty() {
            continue;
        }
        if !plan_name_set.contains(parent) {
            let suggestions = suggest_existing_component_names(parent, &existing_names);
            errors.push(serde_json::json!({
                "kind": "unknown_parent",
                "component": comp.name.trim(),
                "parent": parent,
                "suggestions": suggestions,
            }));
        }
    }

    for comp in pending.plan.components.iter() {
        let Some(att) = comp.attach_to.as_ref() else {
            continue;
        };
        let child_anchor = att.child_anchor.trim();
        if !(child_anchor.is_empty() || child_anchor == "origin") {
            let anchors: std::collections::HashSet<&str> = comp
                .anchors
                .iter()
                .map(|a| a.name.trim())
                .filter(|s| !s.is_empty())
                .collect();
            if !anchors.contains(child_anchor) {
                let available: Vec<String> = comp
                    .anchors
                    .iter()
                    .map(|a| a.name.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .take(24)
                    .collect();
                errors.push(serde_json::json!({
                    "kind": "missing_child_anchor",
                    "component": comp.name.trim(),
                    "child_anchor": child_anchor,
                    "available_anchors": available,
                }));
            }
        }

        let parent = att.parent.trim();
        if parent.is_empty() || !plan_name_set.contains(parent) {
            continue;
        }
        let parent_anchor = att.parent_anchor.trim();
        if parent_anchor.is_empty() || parent_anchor == "origin" {
            continue;
        }
        let Some(parent_comp) = plan_component_by_name.get(parent).copied() else {
            continue;
        };
        let parent_anchors: std::collections::HashSet<&str> = parent_comp
            .anchors
            .iter()
            .map(|a| a.name.trim())
            .filter(|s| !s.is_empty())
            .collect();
        if !parent_anchors.contains(parent_anchor) {
            let available: Vec<String> = parent_comp
                .anchors
                .iter()
                .map(|a| a.name.trim().to_string())
                .filter(|s| !s.is_empty())
                .take(24)
                .collect();
            errors.push(serde_json::json!({
                "kind": "missing_parent_anchor",
                "component": comp.name.trim(),
                "parent": parent,
                "parent_anchor": parent_anchor,
                "available_parent_anchors": available,
            }));
        }
    }

    if pending.preserve_existing_components {
        let required: std::collections::HashSet<&str> = pending
            .existing_component_names
            .iter()
            .map(|s| s.as_str())
            .collect();
        let new: std::collections::HashSet<&str> = plan_names.iter().map(|s| s.as_str()).collect();
        let mut missing: Vec<String> = required
            .difference(&new)
            .map(|s| (*s).to_string())
            .collect();
        missing.sort();
        if !missing.is_empty() {
            errors.push(serde_json::json!({
                "kind": "preserve_missing_existing_component_names",
                "missing": missing,
            }));
        }

        if let (Some(old_root), Some(new_root)) =
            (pending.existing_root_component.as_ref(), plan_root.as_ref())
        {
            if old_root.trim() != new_root.trim() {
                errors.push(serde_json::json!({
                    "kind": "preserve_root_changed",
                    "old_root": old_root,
                    "new_root": new_root,
                }));
            }
        }
    }

    // Optional FixIts (suggestions only; no mutation). Only suggest when the repair is forced
    // by explicit plan intent (compiler-style diagnostics; no heuristics).
    //
    // If a reuse target is referenced elsewhere but missing from components[], the plan
    // necessarily intends that component to exist. Suggest adding a stub component; the agent
    // must still wire it into the attachment tree.
    if !component_refs.is_empty() && !pending.plan.reuse_groups.is_empty() {
        let mut suggested: Vec<String> = Vec::new();
        for (missing, locations) in component_refs.iter() {
            if plan_name_set.contains(missing.as_str()) {
                continue;
            }
            let in_reuse_targets = locations.iter().any(|loc| loc == "reuse_groups.targets");
            if !in_reuse_targets {
                continue;
            }
            let referenced_elsewhere = locations.iter().any(|loc| {
                loc == "attach_to.parent"
                    || loc == "aim.components"
                    || loc == "attack.muzzle.component"
            });
            if !referenced_elsewhere {
                continue;
            }
            suggested.push(missing.clone());
        }
        suggested.sort();
        suggested.dedup();
        suggested.truncate(8);

        for missing in suggested {
            let mut size = [0.0_f32, 0.0_f32, 0.0_f32];
            for group in pending.plan.reuse_groups.iter() {
                let source = group.source.trim();
                if source.is_empty() {
                    continue;
                }
                let is_target = group.targets.iter().any(|t| t.trim() == missing.as_str());
                if !is_target {
                    continue;
                }
                if let Some(source_comp) = plan_component_by_name.get(source).copied() {
                    size = source_comp.size;
                }
                break;
            }

            fixits.push(serde_json::json!({
                "title": format!("Add missing component `{}` (stub)", missing),
                "notes": "Suggestion only. You will likely still need to set attach_to (and any required anchors) so the plan forms a valid attachment tree.",
                "ops": [
                    {
                        "kind": "add_component",
                        "name": missing,
                        "size": size,
                        "anchors": [],
                        "contacts": [],
                    }
                ],
            }));
        }
    }

    serde_json::json!({
        "version": 1,
        "has_pending_plan": true,
        "pending": {
            "call_id": pending.call_id,
            "error": pending.error,
            "preserve_existing_components": pending.preserve_existing_components,
            "preserve_edit_policy": pending.preserve_edit_policy,
            "rewire_components": pending.rewire_components,
            "existing_root_component": pending.existing_root_component,
        },
        "constraints": constraints,
        "plan_summary": {
            "components_total": pending.plan.components.len(),
            "component_names": plan_names,
            "root_component": plan_root,
        },
        "analysis": {
            "ok": errors.is_empty(),
            "errors": errors,
            "fixits": fixits,
            "hints": [
                "Component/parent names must match EXACTLY (case-sensitive).",
                "In preserve mode: include ALL existing component names and keep the same root component.",
                "Attachment anchors must exist: child_anchor/parent_anchor may be `origin` (implicit) or a named anchor listed under that component. If anchors are missing in preserve mode, prefer get_plan_template_v1 so existing anchors are included.",
                "If the fix is local/deterministic (rename a parent, add a missing component definition, add missing anchors), prefer apply_plan_ops_v1 instead of rerunning llm_generate_plan_v1.",
                "If you need a safe starting point: run get_plan_template_v1, then pass llm_generate_plan_v1.plan_template_kv."
            ],
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen3d::ai::schema::{
        AiAimJson, AiAnchorRefJson, AiAttackJson, AiMobilityJson, AiPlanAttachmentJson,
        AiPlanComponentJson, AiPlanJsonV1, AiReuseAlignmentJson, AiReuseGroupJson,
        AiReuseGroupKindJson,
    };

    fn dummy_component(name: &str, parent: Option<&str>) -> Gen3dPlannedComponent {
        Gen3dPlannedComponent {
            display_name: name.to_string(),
            name: name.to_string(),
            purpose: String::new(),
            modeling_notes: String::new(),
            pos: Vec3::ZERO,
            rot: Quat::IDENTITY,
            planned_size: Vec3::ONE,
            actual_size: None,
            anchors: Vec::new(),
            contacts: Vec::new(),
            root_animations: Vec::new(),
	            attach_to: parent.map(|p| super::super::job::Gen3dPlannedAttachment {
	                parent: p.to_string(),
	                parent_anchor: "origin".into(),
	                child_anchor: "origin".into(),
	                offset: Transform::IDENTITY,
	                fallback_basis: Transform::IDENTITY,
	                joint: None,
	                animations: Vec::new(),
	            }),
	        }
	    }

    fn dummy_plan(
        components: Vec<AiPlanComponentJson>,
        root_component: Option<&str>,
    ) -> AiPlanJsonV1 {
        AiPlanJsonV1 {
            version: 8,
            rig: None,
            mobility: AiMobilityJson::Static,
            attack: None,
            aim: None,
            collider: None,
            assembly_notes: String::new(),
            root_component: root_component.map(|s| s.to_string()),
            reuse_groups: Vec::new(),
            components,
        }
    }

    #[test]
    fn inspect_detects_unknown_parent() {
        let existing = vec![
            dummy_component("body", None),
            dummy_component("neck", Some("body")),
        ];
        let plan = dummy_plan(
            vec![
                AiPlanComponentJson {
                    name: "body".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "rider_mount".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "dragon_neck".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
            Some("body"),
        );

        let pending = Gen3dPendingPlanAttempt {
            call_id: "call_1".into(),
            error: "AI plan: component `rider_mount` attach_to parent `dragon_neck` not found."
                .into(),
            preserve_existing_components: true,
            preserve_edit_policy: Some("additive".into()),
            rewire_components: Vec::new(),
            existing_component_names: vec!["body".into(), "neck".into()],
            existing_root_component: Some("body".into()),
            plan,
        };

        let report = inspect_pending_plan_attempt_v1(Some(&pending), &existing, true);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let unknown_parent = errors
            .iter()
            .find(|e| e.get("kind").and_then(|v| v.as_str()) == Some("unknown_parent"))
            .cloned()
            .expect("missing unknown_parent error");
        let suggestions: Vec<String> = unknown_parent
            .get("suggestions")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        assert!(suggestions.contains(&"neck".to_string()));
    }

    #[test]
    fn inspect_detects_missing_parent_and_child_anchors() {
        let existing = vec![
            dummy_component("body", None),
            dummy_component("neck", Some("body")),
        ];
        let plan = dummy_plan(
            vec![
                AiPlanComponentJson {
                    name: "body".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "neck".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "body".into(),
                        parent_anchor: "neck_mount".into(),
                        child_anchor: "body_socket".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
            Some("body"),
        );

        let pending = Gen3dPendingPlanAttempt {
            call_id: "call_3".into(),
            error: "anchors missing".into(),
            preserve_existing_components: true,
            preserve_edit_policy: Some("additive".into()),
            rewire_components: Vec::new(),
            existing_component_names: vec!["body".into(), "neck".into()],
            existing_root_component: Some("body".into()),
            plan,
        };

        let report = inspect_pending_plan_attempt_v1(Some(&pending), &existing, true);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(errors
            .iter()
            .any(|e| e.get("kind").and_then(|v| v.as_str()) == Some("missing_child_anchor")));
        assert!(errors
            .iter()
            .any(|e| e.get("kind").and_then(|v| v.as_str()) == Some("missing_parent_anchor")));
    }

    #[test]
    fn inspect_detects_preserve_missing_names_and_root_change() {
        let existing = vec![
            dummy_component("body", None),
            dummy_component("neck", Some("body")),
            dummy_component("head", Some("neck")),
        ];

        let plan = dummy_plan(
            vec![
                AiPlanComponentJson {
                    name: "neck".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "head".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "neck".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
            Some("neck"),
        );

        let pending = Gen3dPendingPlanAttempt {
            call_id: "call_2".into(),
            error: "preserve mode rejected".into(),
            preserve_existing_components: true,
            preserve_edit_policy: Some("additive".into()),
            rewire_components: Vec::new(),
            existing_component_names: vec!["body".into(), "neck".into(), "head".into()],
            existing_root_component: Some("body".into()),
            plan,
        };

        let report = inspect_pending_plan_attempt_v1(Some(&pending), &existing, true);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(errors.iter().any(|e| e.get("kind").and_then(|v| v.as_str())
            == Some("preserve_missing_existing_component_names")));
        assert!(errors
            .iter()
            .any(|e| e.get("kind").and_then(|v| v.as_str()) == Some("preserve_root_changed")));
    }

    fn pending_attempt_for_plan(plan: AiPlanJsonV1) -> Gen3dPendingPlanAttempt {
        Gen3dPendingPlanAttempt {
            call_id: "call_test".into(),
            error: "semantic error".into(),
            preserve_existing_components: false,
            preserve_edit_policy: None,
            rewire_components: Vec::new(),
            existing_component_names: Vec::new(),
            existing_root_component: None,
            plan,
        }
    }

    fn find_missing_component_ref_error<'a>(
        errors: &'a [serde_json::Value],
        name: &str,
    ) -> Option<&'a serde_json::Value> {
        errors.iter().find(|e| {
            e.get("kind").and_then(|v| v.as_str()) == Some("missing_component_reference")
                && e.get("name").and_then(|v| v.as_str()) == Some(name)
        })
    }

    #[test]
    fn inspect_reports_missing_component_reference_from_attach_to_parent() {
        let plan = dummy_plan(
            vec![
                AiPlanComponentJson {
                    name: "body".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "gun".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.2, 0.2, 0.6],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "arm_lower_r".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
            Some("body"),
        );

        let pending = pending_attempt_for_plan(plan);
        let report = inspect_pending_plan_attempt_v1(Some(&pending), &[], false);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let err = find_missing_component_ref_error(&errors, "arm_lower_r")
            .expect("missing missing_component_reference error");
        let referenced_by: Vec<&str> = err
            .get("referenced_by")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(referenced_by.contains(&"attach_to.parent"));
    }

    #[test]
    fn inspect_reports_missing_component_reference_from_aim_components() {
        let mut plan = dummy_plan(
            vec![AiPlanComponentJson {
                name: "body".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                size: [1.0, 1.0, 1.0],
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            }],
            Some("body"),
        );
        plan.aim = Some(AiAimJson {
            max_yaw_delta_degrees: None,
            components: vec!["turret".into()],
        });

        let pending = pending_attempt_for_plan(plan);
        let report = inspect_pending_plan_attempt_v1(Some(&pending), &[], false);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let err =
            find_missing_component_ref_error(&errors, "turret").expect("missing error for turret");
        let referenced_by: Vec<&str> = err
            .get("referenced_by")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(referenced_by.contains(&"aim.components"));
    }

    #[test]
    fn inspect_reports_missing_component_reference_from_attack_muzzle_component() {
        let mut plan = dummy_plan(
            vec![AiPlanComponentJson {
                name: "body".into(),
                purpose: String::new(),
                modeling_notes: String::new(),
                size: [1.0, 1.0, 1.0],
                anchors: Vec::new(),
                contacts: Vec::new(),
                attach_to: None,
            }],
            Some("body"),
        );
        plan.attack = Some(AiAttackJson::RangedProjectile {
            cooldown_secs: None,
            muzzle: Some(AiAnchorRefJson {
                component: "muzzle_comp".into(),
                anchor: "muzzle".into(),
            }),
            projectile: None,
        });

        let pending = pending_attempt_for_plan(plan);
        let report = inspect_pending_plan_attempt_v1(Some(&pending), &[], false);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let err = find_missing_component_ref_error(&errors, "muzzle_comp")
            .expect("missing error for muzzle_comp");
        let referenced_by: Vec<&str> = err
            .get("referenced_by")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(referenced_by.contains(&"attack.muzzle.component"));
    }

    #[test]
    fn inspect_reports_missing_component_reference_from_reuse_groups_targets() {
        let mut plan = dummy_plan(
            vec![
                AiPlanComponentJson {
                    name: "torso".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [1.0, 1.0, 1.0],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: None,
                },
                AiPlanComponentJson {
                    name: "arm_lower_l".into(),
                    purpose: String::new(),
                    modeling_notes: String::new(),
                    size: [0.2, 0.2, 0.2],
                    anchors: Vec::new(),
                    contacts: Vec::new(),
                    attach_to: Some(AiPlanAttachmentJson {
                        parent: "torso".into(),
                        parent_anchor: "origin".into(),
                        child_anchor: "origin".into(),
                        offset: None,
                        joint: None,
                    }),
                },
            ],
            Some("torso"),
        );
        plan.reuse_groups = vec![AiReuseGroupJson {
            kind: AiReuseGroupKindJson::Component,
            source: "arm_lower_l".into(),
            targets: vec!["arm_lower_r".into()],
            alignment: AiReuseAlignmentJson::MirrorMountX,
            alignment_frame: None,
            mode: None,
            anchors: None,
        }];

        let pending = pending_attempt_for_plan(plan);
        let report = inspect_pending_plan_attempt_v1(Some(&pending), &[], false);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let err = find_missing_component_ref_error(&errors, "arm_lower_r")
            .expect("missing error for arm_lower_r");
        let referenced_by: Vec<&str> = err
            .get("referenced_by")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(referenced_by.contains(&"reuse_groups.targets"));
    }

    const FIXTURE_REUSE_TARGET_MISSING_COMPONENT: &str = r#"
{
  "version": 8,
  "mobility": { "kind": "static" },
  "reuse_groups": [
    { "kind": "component", "source": "arm_lower_l", "targets": ["arm_lower_r"], "alignment": "mirror_mount_x" }
  ],
  "components": [
    { "name": "torso", "size": [1.0, 1.0, 1.0] },
    { "name": "arm_lower_l", "size": [0.2, 0.2, 0.2], "attach_to": { "parent": "torso", "parent_anchor": "origin", "child_anchor": "origin" } },
    { "name": "laser", "size": [0.2, 0.2, 0.6], "attach_to": { "parent": "arm_lower_r", "parent_anchor": "origin", "child_anchor": "origin" } }
  ]
}
"#;

    #[test]
    fn inspect_fixture_reuse_target_referenced_but_missing_component() {
        let plan: AiPlanJsonV1 =
            serde_json::from_str(FIXTURE_REUSE_TARGET_MISSING_COMPONENT).expect("fixture parses");
        let pending = pending_attempt_for_plan(plan);
        let report = inspect_pending_plan_attempt_v1(Some(&pending), &[], false);
        let errors = report
            .get("analysis")
            .and_then(|v| v.get("errors"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let err = find_missing_component_ref_error(&errors, "arm_lower_r")
            .expect("missing error for arm_lower_r");
        let referenced_by: Vec<&str> = err
            .get("referenced_by")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        assert!(referenced_by.contains(&"reuse_groups.targets"));
        assert!(referenced_by.contains(&"attach_to.parent"));
    }
}
